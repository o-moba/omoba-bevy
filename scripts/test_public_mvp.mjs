#!/usr/bin/env node
// Live public-protocol integration/load probe. Uses Node built-ins only, so the
// file alone runs on a host without the repository (LR-09 capacity measurement,
// ops/vps/ru/README.md "Arena capacity"). It launches its own loopback lobby;
// run it against a disposable database, never production.
//
// --scenario=ready-check verifies nine accepts, a 15 s timeout, FIFO requeue,
// late-answer rejection and one unanimous replacement roster before spawn.
// --scenario=party-draft --clients=10 --preference=humans_only proves a five-stack,
// signed draft Leave, FIFO/no-credit requeue and a fresh unanimous roster.
// Load options (scenario `load`):
//   --clients=N --preference=P      N players of one queue (bot_practice, quick, humans_only)
//   --mix=bot_practice:10,quick:5   players of several queues, interleaved (instead of the two above)
//   --capacity=N                    OMOBA_MATCH_CAPACITY of the probe's lobby (default max(16, players))
//   --max-wait-secs=S               every player must reach Running within S seconds of joining
//   --limits-from=UNIT              run the lobby and its workers under that systemd unit's
//                                   MemoryHigh/MemoryMax/CPUQuota/TasksMax (Linux, systemd-run --scope);
//                                   --memory-high/--memory-max/--cpu-quota/--tasks-max override one
//   --protocol-version=N | --contract=compatibility.json   when shared/src/protocol.rs is not beside it
//   --allow-large                   lift the 100-client cap to 1000 (capacity stays 1-100, the pool's maximum)
//   --queue-only                    sign in and FindMatch only, no handoff to a worker: a lobby-side load
//                                   (capacity=1; one disclosed normal vs-Bots helper holds the arena;
//                                   target clients stay queued; CPU/RSS cover lobby + helper worker)
//   --verify-presence               queue-only: also check all target presence rows in the isolated DB (psql)
// The report (report.json) records, per arena type: snapshot gap p50/p95/p99, time to Running,
// snapshot bytes per second per player, and the lobby + workers' CPU and RSS while all play.
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import dgram from 'node:dgram';
import {bindLoopbackPeer,listenForLoopbackPeer} from './public_probe_socket.mjs';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawn, spawnSync} from 'node:child_process';
import {performance} from 'node:perf_hooks';
import {inflateSync, deflateSync} from 'node:zlib';

// Complete framed payload only. Bound output before allocation and require one
// exact zlib stream, preserving JSON integers and floats as original bytes.
function decodeSnapshotPayload(bytes) {
 if(!bytes.length||bytes.length>65507)throw new Error('snapshot payload size');
 if(bytes.subarray(0,4).toString()!=='OMZ1')return bytes;
 if(bytes.length<=8)throw new Error('compressed snapshot header');
 const expected=bytes.readUInt32LE(4);
 if(!expected||expected>65507)throw new Error('decompressed snapshot size');
 const encoded=bytes.subarray(8),decoded=inflateSync(encoded,{maxOutputLength:expected+1,info:true});
 if(decoded.buffer.length!==expected||decoded.engine.bytesWritten!==encoded.length)
  throw new Error('incomplete, oversized or trailing compressed snapshot');
 return decoded.buffer;
}
if(process.argv.includes('--self-test-transport')) {
 const envelope=raw=>{const header=Buffer.alloc(8);header.write('OMZ1');header.writeUInt32LE(raw.length,4);return Buffer.concat([header,deflateSync(raw,{level:1})]);};
 const exact=Buffer.from('{"id":18446744073709551615,"x":1.0000000000000002}');
 for(const raw of [exact,Buffer.alloc(65507,120),crypto.randomBytes(3000)]) {
  assert.deepEqual(decodeSnapshotPayload(raw),raw);assert.deepEqual(decodeSnapshotPayload(envelope(raw)),raw);
 }
 const good=envelope(Buffer.alloc(65507,120));let refused=0;
 const reject=raw=>{assert.throws(()=>decodeSnapshotPayload(raw));refused++;};
 for(let n=4;n<good.length;n++)reject(good.subarray(0,n));
 for(const n of [0,1,65506,65508,0xffffffff]){const bad=Buffer.from(good);bad.writeUInt32LE(n,4);reject(bad);}
 reject(Buffer.concat([good,Buffer.from([0])]));reject(Buffer.concat([good,good.subarray(8)]));
 reject(Buffer.alloc(0));reject(Buffer.alloc(65508));
 const corrupt=Buffer.from(good);corrupt[corrupt.length-1]^=1;reject(corrupt);
 const bomb=envelope(Buffer.alloc(1000000,120));bomb.writeUInt32LE(4,4);reject(bomb);
 console.log(`snapshot_payload: PASS (exact roundtrips; ${refused} malformed/bounded refusals)`);process.exit(0);
}

// Match consent must belong to the current search. A retained ready view may
// still carry the previous request ID while a reconnect starts the next one.
// Retry FindMatch in that case; repeatedly answering the stale view starves the
// refresh and turns all innocent survivors into missed confirmations.
function nextQueueAction(client, now) {
 const view=client.career, ready=view?.match_service;
 if(client.autoReady&&view?.match_service_request_id===client.queueId&&ready?.state==='ready_check'&&!ready.accepted&&now-client.lastQueue>500)
  return {action:'ready_match',request_id:client.queueId,check_id:ready.check_id,accepted:true};
 if(client.autoQueue&&now-client.lastQueue>2000)
  return {action:'find_match',request_id:client.queueId,preference:client.preference};
 return null;
}

// Deterministic regression before any socket, output directory or DB access;
// keep the live probe portable as one standalone file on a capacity host.
if(process.argv.includes('--self-test-queue-policy')) {
 const client={queueId:2,preference:'humans_only',autoReady:true,autoQueue:true,lastQueue:0,
  career:{match_service_request_id:1,match_service:{state:'ready_check',check_id:'retained',accepted:false}}};
 assert.deepEqual(nextQueueAction(client,2500),{action:'find_match',request_id:2,preference:'humans_only'});
 assert.equal(nextQueueAction(client,1000),null);
 client.career.match_service_request_id=2;
 assert.deepEqual(nextQueueAction(client,1000),{action:'ready_match',request_id:2,check_id:'retained',accepted:true});
 client.career.match_service.accepted=true;
 assert.deepEqual(nextQueueAction(client,2500),{action:'find_match',request_id:2,preference:'humans_only'});
 client.autoQueue=false;client.autoReady=false;
 assert.equal(nextQueueAction(client,2500),null);
 console.log('queue_policy: PASS (5 correlated consent/refresh cases)');process.exit(0);
}
function availableDraftClass(players,id) {
 const me=players.find(p=>p.player_id===id);if(!me)return null;
 const team=players.filter(p=>p.team===me.team&&!p.is_bot).sort((a,b)=>Number(b.locked)-Number(a.locked)||(a.player_id<b.player_id?-1:a.player_id>b.player_id?1:0));
 const used=new Set();
 for(const p of team){const c=!used.has(p.hero_class)?p.hero_class:['warrior','mage','ranger','cleric','warden'].find(c=>!used.has(c));
  if(!c)return null;used.add(c);if(p.player_id===id)return c;}
 return null;
}
if(process.argv.includes('--self-test-draft-policy')) {
 const rows=Array.from({length:5},(_,i)=>({player_id:i+1,team:'green',hero_class:'warrior',locked:false,is_bot:false}));
 rows.push({player_id:10,team:'green',hero_class:'warrior',locked:true,is_bot:true},{player_id:11,team:'blue',hero_class:'warrior',locked:true,is_bot:false});
 assert.equal(availableDraftClass(rows,1),'warrior');rows[4].locked=true;
 assert.equal(availableDraftClass(rows,5),'warrior');assert.notEqual(availableDraftClass(rows,1),'warrior');
 assert.equal(new Set([1,2,3,4,5].map(id=>availableDraftClass(rows,id))).size,5);
 rows.reverse();assert.equal(availableDraftClass(rows,5),'warrior');assert.notEqual(availableDraftClass(rows,1),'warrior');
 console.log('draft_policy: PASS (6 human-lock/bot/mirror/order/unique cases)');process.exit(0);
}
function isolatedQueueDatabase(value) {
 try {const u=new URL(value);return ['postgres:','postgresql:'].includes(u.protocol)&&['localhost','127.0.0.1','::1','[::1]'].includes(u.hostname)&&u.pathname==='/omoba_test'&&!u.search&&!u.hash;}
 catch{return false;}
}
if(process.argv.includes('--self-test-queue-only-policy')) {
 assert(isolatedQueueDatabase('postgres://postgres:postgres@127.0.0.1:55581/omoba_test'));
 for(const value of ['', 'postgres://remote/omoba_test','postgres://localhost/production','postgres://localhost/omoba_test?host=remote','postgres://localhost/omoba_test#ignored','mysql://localhost/omoba_test'])assert(!isolatedQueueDatabase(value));
 console.log('queue_only_policy: PASS (7 isolated fixture boundary cases)');process.exit(0);
}
const opts = Object.fromEntries(process.argv.slice(2).map(s => {const [key,...rest]=s.replace(/^--/, '').split('=');return [key,rest.join('=')];}));
const QUEUES = ['bot_practice','quick','humans_only'];
// --mix=queue:count,…: players of several queues, interleaved as they arrive.
const mix = opts.mix ? opts.mix.split(',').map(part => {const [queue,count]=part.split(':');assert(QUEUES.includes(queue)&&Number(count)>0, `bad --mix part ${part}`);return [queue,Number(count)];}) : null;
const clientsCount = mix ? mix.reduce((n,[,count])=>n+count,0) : Number(opts.clients || 1);
const preference = mix ? 'mixed' : (opts.preference || 'bot_practice');
assert(mix || QUEUES.includes(preference), `unknown --preference ${preference}`);
const preferences = mix ? (() => {const left=mix.map(([queue,count])=>[queue,count]);const out=[];while(out.length<clientsCount)for(const entry of left)if(entry[1]>0){out.push(entry[0]);entry[1]--;}return out;})() : Array(clientsCount).fill(preference);
const scenario = opts.scenario || 'load';
const draftExit=opts['draft-exit']||'leave';
const exerciseStaleHandoff='exercise-stale-handoff' in opts;
assert(['leave','disconnect'].includes(draftExit)&&(!opts['draft-exit']||scenario==='party-draft'), '--draft-exit=leave|disconnect is a party-draft option');
assert(['load','lifecycle','lobby-restart','recovery','abandoned','capacity-wait','ready-check','cross-lobby-ready','legacy-penalty','termination','early-recovery','party-draft','unique-draft','handoff-release','remake','surrender','drain'].includes(scenario), 'unknown scenario');
if(['remake','surrender','unique-draft'].includes(scenario)) assert(!mix&&preference==='humans_only'&&clientsCount===10, `${scenario} uses --clients=10 --preference=humans_only`);
else if(scenario==='party-draft') assert(!mix&&preference==='humans_only'&&clientsCount===10, 'party-draft uses --clients=10 --preference=humans_only');
else if(scenario==='handoff-release') assert(!mix&&preference==='quick'&&clientsCount===10&&isolatedQueueDatabase(process.env.OMOBA_TEST_DATABASE_URL), 'handoff-release requires10 Casual humans and the isolated loopback omoba_test database');
else if(['recovery','early-recovery'].includes(scenario)) assert(!mix&&((preference==='humans_only'&&clientsCount===10)||(preference!=='humans_only'&&clientsCount>=1&&clientsCount<=10)), `${scenario} uses one roster (Ranked: ten humans)`);
else if(scenario!=='load') assert(clientsCount===1 && preference==='bot_practice', `${scenario} uses --clients=1 --preference=bot_practice`);
for(const [queue,count] of mix ?? [[preference,clientsCount]])assert(queue!=='humans_only'||count%10===0, 'Ranked (humans_only) forms only full ten-player matches: use a multiple of 10');
const maxWaitSecs = opts['max-wait-secs'] ? Number(opts['max-wait-secs']) : null;
assert(maxWaitSecs===null || maxWaitSecs>0, 'max-wait-secs must be positive');
const allowLarge = 'allow-large' in opts;
const queueOnly = 'queue-only' in opts;
const verifyPresence = 'verify-presence' in opts;
assert(!verifyPresence||queueOnly,'--verify-presence is a queue-only disposable-DB check requiring psql');
assert(!queueOnly || scenario==='load', '--queue-only is a load option');
assert(!queueOnly||!exerciseStaleHandoff, '--exercise-stale-handoff requires a worker handoff');
const soakSecs = Number(opts.seconds || (scenario==='early-recovery'?2:15));
assert(!verifyPresence||soakSecs>=12,'--verify-presence requires at least12s soak to observe the ordinary10s presence cadence');
const inputHz = Number(opts['input-hz'] ?? 20);
assert(Number.isFinite(inputHz)&&inputHz>=0&&inputHz<=60, 'input-hz must be between 0 and 60');
// The gameplay protocol of the server under test, so the probe follows bumps: the
// checkout's, or a release's (its compatibility.json), or given.
function serverProtocol(){
 if(opts['protocol-version'])return Number(opts['protocol-version']);
 if(opts.contract){const contract=JSON.parse(fs.readFileSync(opts.contract,'utf8'));return Number(contract.protocol ?? contract.contract?.protocol);}
 const source=new URL('../shared/src/protocol.rs', import.meta.url);
 assert(fs.existsSync(source), 'no shared/src/protocol.rs beside this script: pass --protocol-version=N or --contract=compatibility.json');
 return Number(/pub const PROTOCOL_VERSION: u16 = (\d+);/.exec(fs.readFileSync(source, 'utf8'))[1]);
}
const protocolVersion = serverProtocol();
assert(Number.isInteger(protocolVersion)&&protocolVersion>0, 'unknown protocol version');
const lobbyPort = Number(opts.port || 45500);
const workerPort = Number(opts['worker-port'] || 45600);
const root = path.resolve(opts.output || `.agent/tasks/TASK-PUBLIC-MVP-2026-09-22/raw/live-${Date.now()}`);
fs.mkdirSync(root, {recursive:true});
// An isolated physical-backup runner owns the DB fault/replacement. The probe
// exchanges only correlated receipts, never runs a supplied command or SQL.
const databaseRestoreControl=opts['database-restore-control']?fs.realpathSync(opts['database-restore-control']):null;
if(databaseRestoreControl)assert(scenario==='recovery'&&clientsCount===1&&preference==='bot_practice'&&isolatedQueueDatabase(process.env.OMOBA_TEST_DATABASE_URL)&&databaseRestoreControl.startsWith(fs.realpathSync(root)+path.sep),'database restore requires the one-roster loopback recovery fixture and a control directory inside its output');
assert(process.env.OMOBA_TEST_DATABASE_URL, 'Use a disposable OMOBA_TEST_DATABASE_URL');
assert(process.env.HARNESS_SERVER_BIN, 'Build server and set HARNESS_SERVER_BIN');
assert(clientsCount >= 1 && clientsCount <= (allowLarge ? 1000 : 100), allowLarge ? 'clients must be 1-1000' : 'clients must be 1-100 (--allow-large lifts the cap to 1000)');
const capacity = scenario==='capacity-wait' ? 1 : Number(opts.capacity || (queueOnly?1:Math.min(100, Math.max(16,clientsCount))));
assert(scenario!=='drain'||capacity===2,'--scenario=drain requires --capacity=2 to prove a free arena stays unused');
assert(!queueOnly||capacity===1,'--queue-only uses --capacity=1 and one disclosed normal capacity-holder');
assert(!queueOnly||isolatedQueueDatabase(process.env.OMOBA_TEST_DATABASE_URL),'--queue-only requires a loopback /omoba_test database without URI options');
assert(Number.isInteger(capacity)&&capacity>=1&&capacity<=100, 'capacity must be 1-100 (the server pool maximum)');
// The lobby unit's cgroup limits (LR-09): the lobby and every worker share them, as on the host.
const LIMIT_KEYS = {MemoryHigh:'memory-high', MemoryMax:'memory-max', CPUQuota:'cpu-quota', TasksMax:'tasks-max'};
const limits = {};
if(opts['limits-from']){
 for(const line of fs.readFileSync(opts['limits-from'],'utf8').split('\n')){
  const match=/^\s*(MemoryHigh|MemoryMax|CPUQuota|TasksMax)\s*=\s*(\S+)\s*$/.exec(line);
  if(match)limits[match[1]]=match[2];
 }
 assert(Object.keys(limits).length>0, `no MemoryHigh/MemoryMax/CPUQuota/TasksMax in ${opts['limits-from']}`);
}
for(const [property,option] of Object.entries(LIMIT_KEYS))if(opts[option])limits[property]=opts[option];
for(const value of Object.values(limits))assert(/^[0-9][0-9.]*[KMGT%]?$|^infinity$/.test(value), `bad limit ${value}`);
if(Object.keys(limits).length)assert(process.platform==='linux', 'cgroup limits need Linux with systemd-run');
const wire = obj => JSON.stringify(obj, (_,v) => typeof v === 'bigint' ? `@u64:${v}` : v).replace(/"@u64:(\d+)"/g, '$1');
const parse = bytes => JSON.parse(bytes.toString().replace(/([:\[,])(\d{16,})(?=[,}\]])/g, '$1"@u64:$2"'), (_,v) => typeof v==='string' && v.startsWith('@u64:') ? BigInt(v.slice(5)) : v);
const sign = (key, data) => crypto.sign(null, Buffer.from(wire(data)), key).toString('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const logfile = fs.openSync(path.join(root,'lobby.log'),'a');
const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('OMOBA_') && !k.startsWith('GAME_') && k !== 'SERVER_ADDR'));
Object.assign(env, {SERVER_ADDR:`127.0.0.1:${lobbyPort}`, OMOBA_SERVER_ROLE:'lobby', OMOBA_DATABASE_URL:process.env.OMOBA_TEST_DATABASE_URL,
 OMOBA_MATCH_ROOT:path.join(root,'matches'), OMOBA_CAREER_OUTBOX:path.join(root,'lobby-outbox'),
 OMOBA_MATCH_CAPACITY:String(capacity), OMOBA_MATCH_FIRST_PORT:String(workerPort),
 OMOBA_MATCH_PUBLIC_HOST:'127.0.0.1', OMOBA_MATCH_BIND_IP:'127.0.0.1',
 // Every synthetic player comes from 127.0.0.1: lift the per-source limits (LR-16), which
 // would otherwise refuse the second arena, the ninth endpoint and the eleventh profile.
 OMOBA_LIMIT_PROBES_PER_IP:'10000', OMOBA_LIMIT_PROBES_PER_NETWORK:'10000', OMOBA_LIMIT_PROBES_GLOBAL:'10000',
 OMOBA_LIMIT_ENDPOINTS_PER_IP:'2048', OMOBA_LIMIT_NEW_PROFILES_PER_IP:'1000000', OMOBA_LIMIT_NEW_PROFILES_TOTAL:'10000000',
 OMOBA_LIMIT_ARENAS_PER_IP:'100', OMOBA_LIMIT_PRACTICE_ABANDONS:'1000'});
const qualificationChildren=new Set();
let qualificationTail=Promise.resolve();
// Keep live UDP heartbeats flowing while the isolated helper writes fixtures.
// Serialize helpers to avoid ten competing store pools, without blocking Node.
function qualifyRankedProfile(profile) {
 const job=qualificationTail.then(()=>new Promise((resolve,reject)=>{
  if(stopping){reject(new Error('probe stopped before fixture preparation'));return;}
  const child=spawn(process.env.HARNESS_RANKED_FIXTURE_BIN,[],{
   env:{...process.env,OMOBA_HARNESS_DATABASE_URL:process.env.OMOBA_TEST_DATABASE_URL},stdio:['pipe','pipe','pipe'],
  });
  qualificationChildren.add(child);
  let stdout='',stderr='',timedOut=false;
  const timer=setTimeout(()=>{timedOut=true;child.kill('SIGTERM');},60000);
  child.stdout.on('data',chunk=>{stdout+=chunk;if(stdout.length>1048576)child.kill('SIGTERM');});
  child.stderr.on('data',chunk=>{stderr=(stderr+chunk).slice(-1200);});
  child.on('error',error=>{clearTimeout(timer);qualificationChildren.delete(child);reject(error);});
  child.on('close',code=>{
   clearTimeout(timer);qualificationChildren.delete(child);
   if(code!==0||timedOut){reject(new Error(`isolated Ranked fixture failed (${timedOut?'timeout':code}): ${stderr}`));return;}
   try{resolve(JSON.parse(stdout.trim()));}catch(error){reject(error);}
  });
  child.stdin.on('error',()=>{});child.stdin.end(JSON.stringify({profiles:[profile],target:10}));
 }));
 qualificationTail=job.catch(()=>{});return job;
}
const ownedGroups=new Set();
// A lobby client is in the queue once its match_service view says so (--queue-only).
const QUEUED_STATES=['waiting','allocating','assigned'];
const queued=c=>Boolean(c.auth&&QUEUED_STATES.includes(c.career?.match_service?.state));
const percentile=(sorted,q)=>sorted[Math.min(sorted.length-1,Math.floor(sorted.length*q))];
// With limits, `systemd-run --scope` execs the server in a transient scope (same PID), so the
// lobby and the workers it spawns share them like the lobby unit's cgroup.
function launchLobby(overrides={}){
 const scope=Object.keys(limits).length?['systemd-run','--scope','--quiet','--collect',...Object.entries(limits).map(([k,v])=>`--property=${k}=${v}`),process.env.HARNESS_SERVER_BIN]:[process.env.HARNESS_SERVER_BIN];
 const child=spawn(scope[0], scope.slice(1), {env:{...env,...overrides}, detached:true, stdio:['ignore',logfile,logfile]});if(child.pid)ownedGroups.add(child.pid);return child;
}
let lobby=launchLobby();
const all=[]; const support=[]; const report={scenario,queue_only:queueOnly||undefined,clients:clientsCount,preference,mix:mix?Object.fromEntries(mix):undefined,capacity,limits,max_wait_secs:maxWaitSecs,protocol:protocolVersion,host:{platform:process.platform,cpus:os.cpus().length,cpu_model:os.cpus()[0]?.model,memory_gib:Math.round(os.totalmem()/2**30*10)/10},input_hz_requested:inputHz,input_model:['remake','surrender'].includes(scenario)?'signed_bounded_movement_intent':'signed_current_pose_transform',soak_seconds:soakSecs,started_at:new Date().toISOString(),scenarios:[],status:'RUNNING'};
const resources=[];
let stopping=false;
// A process's CPU time in seconds: /proc/<pid>/stat utime+stime on Linux (10 ms ticks; ps
// prints whole seconds there), else ps's [dd-]hh:mm:ss.xx.
const CLOCK_TICKS=100;
function cpuSeconds(pid, text){
 if(process.platform==='linux'){
  try{const fields=fs.readFileSync(`/proc/${pid}/stat`,'utf8').split(') ').at(-1).split(' ');return (Number(fields[11])+Number(fields[12]))/CLOCK_TICKS;}catch{return 0;}
 }
 const [days,rest]=text.includes('-')?text.split('-'):[0,text];
 return Number(days)*86400+rest.split(':').reduce((total,part)=>total*60+Number(part),0);
}
function resourceSample(){
 const lines=spawnSync('ps',['-Ao','pid=,pgid=,%cpu=,rss=,time='],{encoding:'utf8'}).stdout || '';
 const owned=lines.trim().split('\n').map(l=>l.trim().split(/\s+/)).filter(r=>ownedGroups.has(Number(r[1])));
 const sample={at:new Date().toISOString(),wall_ms:performance.now(),processes:owned.length,cpu_percent:owned.reduce((s,r)=>s+Number(r[2]),0),rss_mib:owned.reduce((s,r)=>s+Number(r[3]),0)/1024,cpu_seconds:owned.reduce((s,r)=>s+cpuSeconds(r[0],r[4]),0)};
 // Interval CPU from cumulative CPU time: ps %cpu is a lifetime average on Linux.
 const previous=resources.at(-1);
 if(previous&&sample.wall_ms>previous.wall_ms)sample.cpu_percent_interval=Math.max(0,(sample.cpu_seconds-previous.cpu_seconds)*100000/(sample.wall_ms-previous.wall_ms));
 resources.push(sample);
}
function cleanup() {if(stopping)return;stopping=true;for(const child of qualificationChildren)child.kill('SIGTERM');for(const c of [...all,...support])c.close();for(const group of ownedGroups){try{process.kill(-group,'SIGTERM')}catch{}}fs.closeSync(logfile);}
process.on('SIGINT',()=>{cleanup();process.exit(130)});
process.on('SIGTERM',()=>{cleanup();process.exit(143)});
function isLobbyPort(port) {
 return port===lobbyPort || (['cross-lobby-ready','legacy-penalty'].includes(scenario) && port===lobbyPort+1);
}
class Client {
 constructor(index, target=lobbyPort, identity=null, mode={}) {
  this.keepActive=mode.keepActive===true;this.index=index;this.port=target;this.autoHandoff=mode.autoHandoff!==false;this.autoQueue=mode.autoQueue!==false;this.autoJoin=mode.autoJoin!==false;this.autoReady=mode.autoReady!==false;this.autoLock=mode.autoLock!==false;this.partyPresence=mode.partyPresence===true;this.preference=mode.preference || preferences[index] || preferences[0];this.active=true;this.createdAt=performance.now();this.runningBytes=0;this.worldSnapshots=0;this.bootstrapSnapshots=0;this.joinAttempts=0;this.allocationsSeen=[];this.profileResponses=new Map();this.careerGaps=[];this.lastCareer=0;this.key=identity?.key || crypto.generateKeyPairSync('ed25519').privateKey;
  this.publicKey=crypto.createPublicKey(this.key).export({type:'spki',format:'der'}).subarray(-32).toString('hex');
  this.session=identity?.session || `mvp-${Date.now()}-${index}`;this.nickname=identity?.nickname || `Probe-${index}`;
  this.packets=0;this.bytes=0;this.sentPackets=0;this.sentBytes=0;this.transforms=0;this.lastInput=0;this.firstTransform=null;this.lastTransform=null;this.snapshots=0;this.gaps=[];this.frames=new Map();this.errors=[];this.stage='probing';this.next=0;this.queueId=1;
  this.bind();
 }
 bind() {
  this.socketReady=false;this.bindError=null;
  this.nonce=crypto.randomBytes(16).toString('hex');this.epoch=0n;this.match=0;this.path=null;this.auth=null;this.authChallenge=null;this.lastProbe=0;this.accountSequence=0;this.sequence=0;this.prematchSequence=0;this.lastSend=0;this.lastQueue=0;this.next=0;
  bindLoopbackPeer({
   isReserved:port=>port===lobbyPort||(port>=workerPort&&port<workerPort+capacity)||(['cross-lobby-ready','legacy-penalty'].includes(scenario)&&(port===lobbyPort+1||(port>=workerPort+100&&port<workerPort+100+capacity))),
   active:()=>this.active&&!stopping,
   onSocket:socket=>{this.socket=socket;
    listenForLoopbackPeer(socket,()=>this.port,b=>{if(this.active&&this.socket===socket)this.receive(b);},()=>{this.foreignDatagrams=(this.foreignDatagrams||0)+1;});
    socket.on('error',e=>{if(this.active&&this.socket===socket)this.errors.push(e.message);});
   },
   onReady:socket=>{this.localPort=socket.address().port;this.socketReady=true;},
   onRejected:()=>{this.portRebinds=(this.portRebinds||0)+1;},
   onFailure:error=>{this.bindError=error;},
  });
 }
 send(value) {if(!stopping&&this.active&&this.socketReady){const bytes=Buffer.from(wire(value));this.sentPackets++;this.sentBytes+=bytes.length;this.socket.send(bytes,this.port,'127.0.0.1');}}

 bootstrap(payload) {this.send({type:'transport_bootstrap',path_nonce:this.path,payload:wire(payload)});}
 account(action) {if(!this.auth)return;const sequence=++this.accountSequence;this.bootstrap({type:'career',request:{action:'authorized',session_nonce:this.auth,sequence,request:action,signature:sign(this.key,['omoba.career.request.v1',this.epoch,this.auth,sequence,action])}});}
 gameplay(payload) {if(!this.auth)return;const command={server_epoch:this.epoch,match_id:this.match,session_id:this.session,session_nonce:this.auth,path_nonce:this.path,sequence:++this.sequence,payload:wire(payload),signature:''};command.signature=sign(this.key,['omoba.gameplay.command.v1',command.server_epoch,command.match_id,command.session_id,command.session_nonce,command.path_nonce,command.sequence,command.payload]);this.send({type:'signed_command',command});}
 receive(bytes) {
  this.packets++;this.bytes+=bytes.length;
  // Snapshot bytes per second per player (LR-28 input): what the worker sends while playing.
  if(this.stage==='running'&&!isLobbyPort(this.port))this.runningBytes+=bytes.length;
  if(bytes.subarray(0,4).toString()==='OMB1') {
   const id=`${bytes.readBigUInt64LE(6)}:${bytes.readBigUInt64LE(14)}`;const index=bytes.readUInt16LE(22),count=bytes.readUInt16LE(24);
   if(!this.frames.has(id))this.frames.set(id,{parts:new Array(count),received:0});
   const f=this.frames.get(id);if(!f.parts[index]){f.parts[index]=bytes.subarray(30);f.received++;}
   if(this.frames.size>8)this.frames.delete(this.frames.keys().next().value);
   if(f.received<count)return;this.frames.delete(id);bytes=Buffer.concat(f.parts);
  }
  let packet;try{packet=parse(decodeSnapshotPayload(bytes))}catch(e){this.errors.push('decode:'+e.message);return;}
  if(packet.type==='server_full') {if(packet.client_nonce===this.nonce)this.serverFull=(this.serverFull||0)+1;return;}
  if(packet.type==='transport_challenge') {if(packet.client_nonce!==this.nonce)return;this.path=packet.path_nonce;this.epoch=packet.server_epoch;this.send({type:'transport_proof',server_epoch:this.epoch,path_nonce:this.path});if(!this.auth)this.stage='authenticating';return;}
  if(packet.type==='party') {this.party=packet.party;return;}
  if(packet.type==='career') {
   const careerAt=performance.now();if(this.lastCareer)this.careerGaps.push(careerAt-this.lastCareer);this.lastCareer=careerAt;
   this.epoch=packet.server_epoch;const v=packet.career;this.career=v;if(v.response_id!=null&&v.profile){this.profileResponses.set(String(v.response_id),v.profile);if(this.profileResponses.size>16)this.profileResponses.delete(this.profileResponses.keys().next().value);}if(v.history_loaded)this.lastHistory=v.history;if(v.detail)this.lastDetail=v.detail;if(v.visited_profile)this.lastVisited=v.visited_profile;
   if(v.challenge && !v.auth_nonce && !this.auth && this.authChallenge?.nonce!==v.challenge.nonce) {this.authChallenge=v.challenge;this.next=0;}
   if(v.auth_nonce && v.profile){this.auth=v.auth_nonce;this.profile=v.profile;if(this.stage==='authenticating')this.stage=isLobbyPort(this.port)?(this.autoQueue?'queued':'authenticated'):(this.autoJoin?'joining':'authenticated');}
   if(isLobbyPort(this.port)&&v.match_service?.state==='waiting')this.maxQueueElapsed=Math.max(this.maxQueueElapsed||0,v.match_service.elapsed_secs);
   if(isLobbyPort(this.port)&&!this.queuedAt&&QUEUED_STATES.includes(v.match_service?.state))this.queuedAt=performance.now();
   if(isLobbyPort(this.port) && v.match_service?.state==='assigned' && v.match_service_request_id===this.queueId) {
    this.allocation=v.match_service.allocation;this.inMatch=v.match_service.in_match??null;this.allocationsSeen.push(this.allocation);if(!this.autoHandoff)return;
    const oldSocket=this.socket;this.snapshot=null;oldSocket.close();this.port=Number(this.allocation.endpoint.split(':').at(-1));this.stage='probing';this.bind();
    if(exerciseStaleHandoff){
     // Deliver a real old lobby view after the worker context exists. This
     // models a queued callback, independently of kernel/runner timing.
     const scope={socket:this.socket,epoch:this.epoch,auth:this.auth};
     oldSocket.emit('message',Buffer.from(bytes),{address:'127.0.0.1',port:lobbyPort});
     // The OS can also deliver old lobby datagrams to a reused *new* socket.
     const foreignBefore=this.foreignDatagrams||0;
     this.socket.emit('message',Buffer.from(bytes),{address:'127.0.0.1',port:lobbyPort});
     this.staleEndpointChecks=(this.staleEndpointChecks||0)+1;
     assert.equal(this.foreignDatagrams,foreignBefore+1,'old lobby packet entered worker context');
     this.staleHandoffChecks=(this.staleHandoffChecks||0)+1;
     this.staleHandoffFailure=this.socket!==scope.socket||this.epoch!==scope.epoch||this.auth!==scope.auth;
    }
   }
   return;
  }
  if(packet.type!=='snapshot')return;
  if(["players","structures","minions","neutrals","projectiles"].some(key=>packet[key]?.length))this.worldSnapshots++;
  else this.bootstrapSnapshots++;
  this.epoch=packet.server_epoch;this.match=packet.match_id;this.snapshot=packet;
  if(packet.join_error)this.lastJoinError=packet.join_error;
  if((packet.game_state==='running'||packet.game_state?.type==='running'||packet.game_state?.Running) && packet.players?.some(p=>p.id===packet.your_id&&!p.is_bot)) {
   const now=performance.now();if(this.lastSnapshot)this.gaps.push(now-this.lastSnapshot);this.lastSnapshot=now;this.snapshots++;this.stage='running';this.firstRunning ||= now;
  }
 }
 tick(now) {
  if(this.active&&this.bindError)throw this.bindError;
  if(this.staleHandoffFailure)throw new Error('old lobby callback mutated worker authentication');
  if(!this.active)return;
  if(this.qualificationError)throw this.qualificationError;
  // Retry admission until authentication completes, just like the native Hello
  // loop. A lost proof must not strand a synthetic client forever. Authenticate
  // retries are timer-driven, never echoed for every repeated server view.
  if(!this.auth&&now-this.lastProbe>=1000){this.lastProbe=now;this.send({type:'transport_probe',protocol_version:protocolVersion,client_nonce:this.nonce,padding:'0'.repeat(384)});}
  if(!this.path)return;
  if(!this.auth){if(now>=this.next){this.next=now+2000;const c=this.authChallenge;this.bootstrap({type:'career',request:c?{action:'authenticate',challenge:c,signature:sign(this.key,['omoba.career.auth.v1',c.public_key,c.nonce,c.server_epoch,c.session_id,c.nickname])}:{action:'challenge',public_key:this.publicKey,nickname:this.nickname,session_id:this.session}});}return;}
  // Representative signed movement-command verification/dispatch load. Current
  // authoritative pose avoids inventing movement or bypassing collision rules.
  const intentional=this.keepActive||['remake','surrender'].includes(scenario), hz=intentional?Math.max(1,inputHz):inputHz;
  if(hz>0&&this.stage==='running'&&!isLobbyPort(this.port)&&now-this.lastInput>=1000/hz){
   const me=this.snapshot?.players?.find(p=>p.id===this.snapshot.your_id&&!p.is_bot);
   if(me){
    this.lastInput=this.lastInput?this.lastInput+1000/hz:now;
    if(now-this.lastInput>1000/hz)this.lastInput=now;
    const direction=[[0.15,0],[0,0.15],[-0.15,0],[0,-0.15]][this.transforms%4];
    this.gameplay({type:'transform',x:me.x+(intentional?direction[0]:0),y:me.y,z:me.z+(intentional?direction[1]:0),yaw:me.yaw,dash_sequence:me.utility?.dash_sequence??0,...(intentional?{input_active:true}:{})});
    this.transforms++;this.firstTransform??=now;this.lastTransform=now;
   }
  }
  if(now-this.lastSend<300)return;this.lastSend=now;this.gameplay({type:'ping'});
  if(isLobbyPort(this.port)&&this.partyPresence&&now-(this.lastPresence||0)>2000){this.lastPresence=now;this.gameplay({type:'party',command:{command:'presence',nickname:this.nickname,avatar:null}});}
  if(isLobbyPort(this.port)){
   if(this.preference==='humans_only' && (this.profile?.completed_public_matches??0)<10 && process.env.HARNESS_RANKED_FIXTURE_BIN) {
    if(!this.qualificationRequested && this.profile) {
     this.qualificationRequested=true;
     qualifyRankedProfile(this.profile.profile_id).then(fixture=>{
      this.qualificationFixture=fixture;report.qualification_fixtures??=[];report.qualification_fixtures.push(fixture);
     }).catch(error=>{this.qualificationError=error;});
    }
    if(this.qualificationFixture && now-(this.lastQualificationRead||0)>1500) {
     this.lastQualificationRead=now;this.account({action:'profile',request_id:900000000+this.index,profile_id:this.profile.profile_id});
    }
    return;
   }
   const ready=this.career?.match_service;
   if(this.exerciseStaleReady&&!this.staleReadyChecks&&this.autoQueue&&this.autoReady&&ready?.state==='ready_check'){
    // Model a fresh local search while the actual server still reports its
    // previous request. No server deadline or acceptance is forged/accelerated.
    this.queueId++;this.staleReadyChecks=1;this.lastQueue=now-2100;
   }
   const action=nextQueueAction(this,now);
   if(action){
    if(action.action==='find_match'&&ready?.state==='ready_check'&&this.career.match_service_request_id!==this.queueId)this.staleReadyRefreshes=(this.staleReadyRefreshes||0)+1;
    this.lastQueue=now;this.account(action);
   }
   return;
  }
  if(!this.autoJoin)return;
  const s=this.snapshot;
  if(!s?.players?.some(p=>p.id===s.your_id)) {this.join();return;}
  if(s.prematch){const me=s.prematch.players.find(p=>p.player_id===s.your_id);const p=s.prematch;
   if(this.autoLock&&p.phase==='draft'&&me&&!me.locked){const c=availableDraftClass(p.players,s.your_id);if(c)this.gameplay({type:'prematch',request:{server_epoch:this.epoch,match_id:this.match,generation:p.generation,request_id:++this.prematchSequence,action:c===me.hero_class?{kind:'lock',locked:true}:{kind:'select',hero_class:c,character:'ipfs',avatar:null,sprite_character:null,role:me.role,passport_ticket:null}}});}
   if(p.phase==='loading'&&!me?.loaded)this.gameplay({type:'prematch',request:{server_epoch:this.epoch,match_id:this.match,generation:p.generation,request_id:++this.prematchSequence,action:{kind:'loaded'}}});
  }
 }
 join(){this.joinAttempts++;this.gameplay({type:'join',prematch:true,team:'green',character:'ipfs',hero_class:['warrior','mage','ranger','cleric','warden'][this.index%5],avatar:null,sprite_character:null,session_id:this.session,passport_ticket:null});}
 close(){this.active=false;try{this.socket.close()}catch{}}
 diagnostic(){return{index:this.index,queue_request_id:this.queueId,match_service_request_id:this.career?.match_service_request_id,stale_ready_checks:this.staleReadyChecks||0,stale_ready_refreshes:this.staleReadyRefreshes||0,preference:this.preference,stage:this.stage,running_after_secs:this.firstRunning?(this.firstRunning-this.createdAt)/1000:null,server_full_replies:this.serverFull||0,active:this.active,port:this.port,local_port:this.localPort,port_rebinds:this.portRebinds||0,foreign_datagrams:this.foreignDatagrams||0,world_snapshots:this.worldSnapshots,bootstrap_snapshots:this.bootstrapSnapshots,join_attempts:this.joinAttempts,profile:this.profile?.profile_id,allocation:this.allocation,join_error:this.lastJoinError,career_error:this.career?.error,match_service:this.career?.match_service,party:this.party,game_state:this.snapshot?.game_state,prematch:this.snapshot?.prematch,packets:this.packets,bytes:this.bytes,sent_packets:this.sentPackets,sent_bytes:this.sentBytes,transforms_sent:this.transforms,input_hz_observed:this.transforms>1?(this.transforms-1)*1000/(this.lastTransform-this.firstTransform):0,snapshots:this.snapshots,errors:this.errors.slice(-5)};}
}
// Keep every owned client alive while awaiting an observable protocol condition.
async function pumpUntil(label, predicate, timeout=20000, onTick=()=>{}) {
 const started=performance.now();let nextEvidence=0;
 while(performance.now()-started<timeout) {
  assert(lobby.exitCode===null, `${label}: lobby exited`);
  const now=performance.now();for(const c of [...all,...support])c.tick(now);onTick(now);
  if(predicate())return;
  if(now>=nextEvidence){nextEvidence=now+1000;fs.writeFileSync(path.join(root,'clients.json'),wire(all.map(c=>c.diagnostic())));}
  await delay(20);
 }
 throw new Error(`${label}: timed out after ${timeout}ms; inspect clients.json`);
}
async function pumpFor(ms, onTick=()=>{}) {
 const end=performance.now()+ms;
 await pumpUntil('observation window',()=>performance.now()>=end,ms+1500,onTick);
}
// Vision filters the snapshot's players; the live scoreboard lists every participant.
const ids = snapshot => (snapshot.scoreboard?.players??[]).map(p=>String(p.player_id)).sort();
const careerProgress = client => Object.fromEntries(['profile_id','rating','rated_matches','matches_played','wins','losses','progression_xp'].map(k=>[k,client.profile[k]]));

// The lobby may record intent, but only the owning worker can release a seat.
// One player cancels without ever joining; nine reach Running in the same arena.
async function casualHandoffReleaseLifecycle() {
 const peers=Array.from({length:10},(_,i)=>new Client(10000+i,lobbyPort,null,{preference:'quick',autoHandoff:false,autoLock:false}));
 all.push(...peers);
 await pumpUntil('ten Casual humans accept one real allocation',()=>peers.every(c=>c.allocation),45000);
 const allocation=peers[0].allocation, leaver=peers[0], survivors=peers.slice(1);
 assert(peers.every(c=>c.allocation.allocation_id===allocation.allocation_id));
 assert.equal(allocation.human_count,10);assert.equal(allocation.bot_count,0);assert.equal(allocation.rated,false);
 for(const c of peers)c.autoQueue=false;
 for(const c of survivors)c.autoHandoff=true;
 await pumpUntil('nine humans reach held Draft without the canceller',()=>survivors.every(c=>c.snapshot?.prematch?.phase==='draft'&&c.snapshot.prematch.players.length===9),30000);
 assert.equal(leaver.port,lobbyPort);assert.equal(leaver.joinAttempts,0);
 const workerPort=Number(allocation.endpoint.split(':').at(-1)), workerPid=ownedWorkerPid(workerPort);
 const originalIds=survivors.map(c=>String(c.snapshot.your_id)).sort();
 const baseline=careerProgress(leaver), checks=report.handoff_release={allocation_id:allocation.allocation_id,checks:[]};
 let nextCancel=0;
 await pumpUntil('owning worker acknowledges exact handoff release',()=>allocationStatus(allocation.allocation_id)?.released_sessions?.includes(leaver.session),10000,now=>{
  if(now>=nextCancel){nextCancel=now+300;leaver.account({action:'cancel_queue'});}
  assert(survivors.every(c=>!c.firstRunning),'held Draft ran early');
 });
 const releaseFile=path.join(root,'matches',allocation.allocation_id,'release-requests.json');
 const requests=JSON.parse(fs.readFileSync(releaseFile,'utf8'));
 assert.equal(requests.length,1);assert.equal(requests[0].profile_id,leaver.profile.profile_id);assert.equal(requests[0].session_id,leaver.session);
 assert(!fs.existsSync(path.join(root,'matches',allocation.allocation_id,'cancel.json')));
 assert.equal(ownedWorkerPid(workerPort),workerPid);
 assert.equal(allocationManifests().find(m=>m.allocation_id===allocation.allocation_id).humans.length,10,'immutable manifest changed');
 await pumpUntil('survivors keep ten-seat draft with one bot replacement',()=>survivors.every(c=>c.snapshot?.prematch?.players.length===10&&c.snapshot.prematch.players.filter(p=>p.is_bot).length===1),10000);
 checks.checks.push({name:'unjoined_cancel_is_worker_acknowledged_and_only_its_seat_is_bot_filled',status:'PASS',surviving_humans:9,bots:1,manifest_humans:10});
 for(const c of survivors)c.autoLock=true;
 await pumpUntil('nine survivors start the same Casual arena',()=>survivors.every(c=>c.stage==='running')&&allocationStatus(allocation.allocation_id)?.phase==='running',45000);
 assert.equal(ownedWorkerPid(workerPort),workerPid);
 assert(survivors.every(c=>c.allocation.allocation_id===allocation.allocation_id));
 assert.deepEqual(survivors.map(c=>String(c.snapshot.your_id)).sort(),originalIds);
 assert(survivors.every(c=>ids(c.snapshot).length===10));
 checks.checks.push({name:'nine_original_humans_reach_running_without_new_search_or_worker',status:'PASS'});
 const requestId=700001;let nextProfile=0;
 await pumpUntil('cancelled unjoined profile has fresh unchanged durable progress',()=>leaver.profileResponses.has(String(requestId)),10000,now=>{
  if(now>=nextProfile){nextProfile=now+1000;leaver.account({action:'profile',request_id:requestId,profile_id:baseline.profile_id});}
 });
 const actual=leaver.profileResponses.get(String(requestId));
 for(const [key,value] of Object.entries(baseline))assert.equal(actual[key],value,`unplayed canceller changed ${key}`);
 checks.checks.push({name:'unjoined_canceller_receives_no_match_xp_or_rating',status:'PASS'});
 const observer=new Client(11000,lobbyPort,survivors[0],{preference:'quick',autoHandoff:false,autoQueue:false,autoReady:false});
 all.push(observer);
 await pumpUntil('running survivor authenticates its original lobby session',()=>Boolean(observer.auth),15000);
 await pumpFor(150);observer.account({action:'cancel_queue'});
 await pumpFor(1500);
 await pumpUntil('late lobby cancellation retains resumable running assignment',()=>observer.career?.match_service?.state==='assigned'&&Boolean(observer.career.match_service.in_match),10000);
 const status=allocationStatus(allocation.allocation_id);
 assert.equal(status.phase,'running');assert.deepEqual(status.released_sessions,[leaver.session]);
 assert.equal(JSON.parse(fs.readFileSync(releaseFile,'utf8')).length,1,'late cancel wrote another release intent');
 assert(!fs.existsSync(path.join(root,'matches',allocation.allocation_id,'cancel.json')));
 assert.equal(ownedWorkerPid(workerPort),workerPid);
 checks.checks.push({name:'start_wins_late_cancel_keeps_admission_and_resume_without_aborting_other_players',status:'PASS'});
 report.scenarios.push('casual_handoff_release_ack_bot_fill_same_worker_and_running_race');
}
function assertRoster(client, expected) {
 assert.equal(ids(client.snapshot).length,10,'running match must retain ten participants');
 assert.deepEqual(ids(client.snapshot),expected,'a fresh identity entered or replaced an allocated participant');
 assert.equal(new Set(ids(client.snapshot)).size,10,'duplicate player identity');
}
// Real signed requests exercise both collision boundaries before ordinary play.
async function uniqueDraftLifecycle() {
 const peers=Array.from({length:10},(_,i)=>new Client(i,lobbyPort,null,{preference:'humans_only',autoLock:false}));
 all.push(...peers);
 await pumpUntil('ten qualified humans enter Draft',()=>peers.every(c=>c.snapshot?.prematch?.phase==='draft'&&c.snapshot.prematch.players.length===10),45000);
 const roster=peers[0].snapshot.prematch.players;
 const own=peers[0], row=c=>c.snapshot.prematch.players.find(p=>p.player_id===c.snapshot.your_id);
 const ally=peers.find(c=>c!==own&&row(c).team===row(own).team);
 const opponent=peers.find(c=>row(c).team!==row(own).team);
 const checks=report.unique_draft={checks:[],allocation_id:own.allocation.allocation_id,initial_roster:roster};
 const action=async(c,value,error=null)=>{
  const request_id=++c.prematchSequence,p=c.snapshot.prematch;
  const packet={type:'prematch',request:{server_epoch:c.epoch,match_id:c.match,generation:p.generation,request_id,action:value}};
  let next=0;
  await pumpUntil('correlated draft request ACK',()=>c.snapshot?.prematch?.last_request_id===request_id,4000,now=>{
   if(now>=next){next=now+300;c.gameplay(packet);}
  });
  assert.equal(c.snapshot.prematch.error,error);
 };
 const select=c=>({kind:'select',hero_class:'mage',character:'ipfs',avatar:null,sprite_character:null,role:row(c).role,passport_ticket:null});
 for(const c of [own,ally,opponent])await action(c,select(c));
 await action(own,{kind:'lock',locked:true});
 await action(ally,select(ally),'hero_taken');
 assert.equal(row(ally).hero_class,'mage');assert.equal(row(ally).locked,false);
 await action(ally,{kind:'lock',locked:true},'hero_taken');
 assert.equal(row(ally).locked,false);
 await action(opponent,{kind:'lock',locked:true});
 assert.equal(row(opponent).hero_class,'mage');assert.equal(row(opponent).locked,true);
 checks.checks.push({name:'select_and_lock_reject_same_team_but_allow_opposing_mirror',status:'PASS'});
 await action(own,{kind:'lock',locked:false});
 await action(ally,{kind:'lock',locked:true});
 await action(own,{kind:'lock',locked:true},'hero_taken');
 checks.checks.push({name:'unlock_releases_class_and_a_later_lock_reserves_it',status:'PASS'});
 for(const c of peers)c.autoLock=true;
 let frozen=null;
 await pumpUntil('unique drafted roster enters Running',()=>peers.every(c=>c.firstRunning),30000,()=>{
  for(const c of peers){const p=c.snapshot?.prematch;if(p&&['countdown','loading'].includes(p.phase))frozen=p.players;}
 });
 assert(frozen,'no frozen roster observed');
 for(const team of ['green','blue']){
  const rows=frozen.filter(p=>p.team===team);assert.equal(rows.length,5);
  assert.equal(new Set(rows.map(p=>p.hero_class)).size,5,'duplicate class passed into gameplay');
 }
 for(const c of [ally,opponent])assert.equal(frozen.find(p=>p.player_id===c.snapshot.your_id).hero_class,'mage','valid lock changed');
 checks.final_roster=frozen;checks.checks.push({name:'five_distinct_classes_per_team_and_valid_locks_preserved',status:'PASS'});
 report.scenarios.push('signed_unique_team_draft_to_running');
}

// One real five-stack plus five solos. Keep the first worker in Draft so a
// signed solo Leave exercises cancellation, not a played-match abandonment.
async function partyDraftLifecycle() {
 const peers=Array.from({length:10},(_,i)=>new Client(i,lobbyPort,null,{preference:'humans_only',autoQueue:false,autoLock:false,partyPresence:true}));
 all.push(...peers);
 const checks=report.party_draft={exit:draftExit,checks:[]};
 await pumpUntil('ten authenticated party subscribers',()=>peers.every(c=>c.auth&&c.party?.you),30000);
 const leader=peers[0], stack=peers.slice(0,5);
 for(const member of stack.slice(1)){
  let nextInvite=0,nextAccept=0;
  await pumpUntil('signed party invite and acceptance',()=>leader.party?.party?.members.length===stack.indexOf(member)+1&&member.party?.party?.party_id===leader.party?.party?.party_id,15000,now=>{
   if(now>=nextInvite&&!member.party?.invites?.some(i=>i.from_player_id===leader.party.you)){
    nextInvite=now+1000;leader.gameplay({type:'party',command:{command:'invite',player_id:member.party.you}});
   }
   const invite=member.party?.invites?.find(i=>i.from_player_id===leader.party.you);
   if(invite&&now>=nextAccept){nextAccept=now+400;member.gameplay({type:'party',command:{command:'accept',party_id:invite.party_id}});}
  });
 }
 const party=leader.party.party;
 assert.equal(party.members.length,5);
 assert.deepEqual(party.members.map(m=>String(m.player_id)).sort(),stack.map(c=>String(c.party.you)).sort());
 checks.checks.push({name:'five_stack_signed_invite_accept',status:'PASS',party_id:party.party_id,members:party.members.map(m=>m.player_id)});
 leader.gameplay({type:'party',command:{command:'launch',preference:'humans_only'}});
 const searchingAt=performance.now();
 // Hold nine real searches long enough to prove FIFO age, then add the tenth.
 for(const c of peers.slice(0,9)){c.autoQueue=true;c.lastQueue=0;}
 await pumpUntil('nine queued players before the last arrival',()=>peers.slice(0,9).every(c=>c.career?.match_service?.state==='waiting'),15000);
 await pumpFor(6000);
 peers[9].autoQueue=true;peers[9].lastQueue=0;
 await pumpUntil('party and solos reach one full Draft',()=>peers.every(c=>c.allocation&&c.snapshot?.prematch?.phase==='draft'&&c.snapshot.prematch.players.length===10),45000);
 const allocation=peers[0].allocation;
 assert(peers.every(c=>c.allocation.allocation_id===allocation.allocation_id));
 assert.equal(allocation.human_count,10);assert.equal(allocation.bot_count,0);assert.equal(allocation.rated,true);
 assert.equal(new Set(stack.map(c=>c.allocation.team)).size,1,'five-stack split across teams');
 const originalRoster=peers.map(c=>String(c.snapshot.your_id)).sort();
 assert.equal(new Set(originalRoster).size,10);
 assert(peers.every(c=>!c.firstRunning),'held Draft unexpectedly entered gameplay');
 const baselines=peers.map(c=>careerProgress(c));
 checks.checks.push({name:'ranked_party_team_and_held_draft',status:'PASS',allocation_id:allocation.allocation_id,team:stack[0].allocation.team,humans:10,bots:0});
 const leaver=peers[7];leaver.autoJoin=false;let nextLeave=0;
 const queueAges=peers.map(c=>c.maxQueueElapsed||0);
 assert(queueAges.slice(0,9).every(age=>age>=5),'FIFO rehearsal did not establish a waiting age');
 if(draftExit==='disconnect')leaver.close();
 await pumpUntil('draft departure cancels original worker',()=>['failed','settling'].includes(allocationStatus(allocation.allocation_id)?.phase),draftExit==='leave'?10000:Math.max(10000,Number(allocation.join_deadline_ms)-Date.now()+15000),now=>{
  if(draftExit==='leave'&&now>=nextLeave){nextLeave=now+300;leaver.gameplay({type:'leave'});}
  assert(peers.every(c=>!c.firstRunning),'cancelled Draft entered gameplay');
 });
 await pumpUntil('all nine survivors receive the automatic return code',()=>peers.filter(c=>c!==leaver).every(c=>c.snapshot?.prematch?.error==='allocation_cancelled_or_expired'),5000);
 // Return to the original lobby with the same device key and session, as the
 // native handoff flow does. No worker endpoint becomes the saved lobby.
 for(const c of peers)c.close();
 const survivors=peers.filter(c=>c!==leaver).map(c=>{
  const resumed=new Client(c.index,lobbyPort,c,{preference:'humans_only',autoHandoff:false,partyPresence:true});
  resumed.queueId=c.queueId+1;resumed.exerciseStaleReady=exerciseStaleHandoff;return resumed;
 });
 const dodger=new Client(leaver.index,lobbyPort,leaver,{preference:'humans_only',autoHandoff:false,autoReady:false,autoQueue:draftExit==='leave'});
 dodger.queueId=leaver.queueId+1;all.push(...survivors,dodger);
 await pumpUntil('nine retained searches and authenticated departed player',()=>survivors.every(c=>c.career?.match_service?.state==='waiting')&&dodger.auth&&(draftExit==='disconnect'||(dodger.career?.match_service?.state==='failed'&&dodger.career.match_service.code==='dodge_cooldown')),30000);
 dodger.autoQueue=false;
 const resumedStack=survivors.filter(c=>c.index<5);
 await pumpUntil('five-stack membership survives lobby return',()=>resumedStack.every(c=>c.party?.party?.party_id===party.party_id&&c.party.party.members.length===5),10000);
 assert.deepEqual(resumedStack[0].party.party.members.map(m=>String(m.player_id)).sort(),resumedStack.map(c=>String(c.party.you)).sort());
 for(const c of survivors){
  assert(c.career.match_service.elapsed_secs>=queueAges[c.index],'draft cancellation reset FIFO age');
  assert.deepEqual(careerProgress(c),baselines[c.index],'unplayed draft changed career');
 }
 assert.deepEqual(careerProgress(dodger),baselines[leaver.index]);
 let nextHistory=0;
 await pumpUntil('all ten unplayed histories contain only disclosed warmup receipts',()=>[...survivors,dodger].every(c=>Array.isArray(c.lastHistory)&&c.lastHistory.every(row=>row.result_id.startsWith('synthetic-public-'))),15000,now=>{
  if(now>=nextHistory){nextHistory=now+1000;for(const c of [...survivors,dodger])c.account({action:'history',request_id:500,before:null});}
 });
 const penaltiesFile=path.join(root,'matches','dodge-penalties.json');
 const penalties=fs.existsSync(penaltiesFile)?parse(fs.readFileSync(penaltiesFile)):{};
 if(draftExit==='disconnect')assert.equal(Object.keys(penalties).length,0,'a lost connection penalized the departed player or a survivor');
 else {
  assert.deepEqual(Object.keys(penalties),[dodger.profile.profile_id],'a survivor received a dodge penalty');
  assert.equal(penalties[dodger.profile.profile_id].strikes,1,'repeated Leave packets escalated one departure more than once');
 }
 checks.checks.push({name:'draft_departure_requeues_nine_without_credit',status:'PASS',exit:draftExit,original_queue_ages_secs:queueAges,elapsed_secs:survivors.map(c=>c.career.match_service.elapsed_secs),all_profiles_unchanged:true,all_histories_empty:true,all_survivors_received_return_code:true,dodger_code:dodger.career?.match_service?.code??null});
 const replacement=draftExit==='disconnect'?dodger:new Client(8100,lobbyPort,null,{preference:'humans_only',autoHandoff:false});
 if(replacement!==dodger)all.push(replacement);
 replacement.autoReady=true;replacement.autoQueue=true;replacement.lastQueue=0;replacement.exerciseStaleReady=exerciseStaleHandoff;
 const nextRoster=[...survivors,replacement];
 await pumpUntil('fresh full roster accepted after draft cancellation',()=>nextRoster.every(c=>c.allocation&&c.allocation.allocation_id!==allocation.allocation_id),45000);
 const fresh=nextRoster[0].allocation;
 assert(nextRoster.every(c=>c.allocation.allocation_id===fresh.allocation_id));
 assert.equal(new Set(nextRoster.filter(c=>c.index<5).map(c=>c.allocation.team)).size,1,'requeued party split');
 assert.equal(allocationManifests().length,2,'unexpected duplicate worker');
 // Enable handoff only after inspecting all recipient allocations. Repeated
 // Assigned views carry the same correlated search, so no extra FindMatch is needed.
 for(const c of nextRoster)c.autoHandoff=true;
 await pumpUntil('replacement Ranked party roster reaches Running',()=>nextRoster.every(c=>c.stage==='running'),45000);
 await pumpFor(Math.max(1000,soakSecs*1000));
 for(const c of nextRoster){assert.equal(c.allocation.allocation_id,fresh.allocation_id);assert.equal(c.errors.length,0);assert.equal(ids(c.snapshot).length,10);}
 if(exerciseStaleHandoff){
  assert(nextRoster.every(c=>c.staleReadyChecks===1&&c.staleReadyRefreshes>=1),'stale ready request was not refreshed before consent');
  checks.checks.push({name:'stale_ready_search_id_refresh',status:'PASS',clients:nextRoster.length,explicit_current_search_consent:true});
 }
 checks.checks.push({name:'replacement_party_ranked_running',status:'PASS',allocation_id:fresh.allocation_id,humans:10,bots:0,party_team:nextRoster.find(c=>c.index===0).allocation.team});
 report.arenas=1;report.elapsed_seconds=(performance.now()-searchingAt)/1000;
 report.scenarios.push('signed_five_stack_ranked','draft_departure_requeues_nine_without_credit','fresh_unanimous_party_roster_running');
}
async function gracefulTermination(original) {
 const allocation=original.allocation, progress=careerProgress(original);
 const before=allocationStatus(allocation.allocation_id);
 assert.equal(before.phase,'running'); assert(before.result_id);
 const observer=new Client(6000,lobbyPort,original,{autoQueue:false,autoJoin:false,autoHandoff:false});all.push(observer);
 await pumpUntil('termination observer authentication',()=>!!observer.auth);
 const worker=ownedWorkerPid(original.port), started=performance.now();
 process.kill(worker,'SIGTERM'); original.close();
 let query=600, nextQuery=0;
 await pumpUntil('SIGTERM persists an interrupted result before worker exit',()=>observer.lastDetail?.result_id===before.result_id&&observer.lastDetail.saved&&allocationStatus(allocation.allocation_id)?.phase==='failed',20000,now=>{
  if(now>=nextQuery){nextQuery=now+1000;observer.account({action:'detail',request_id:++query,result_id:before.result_id});}
 });
 const saved=observer.lastDetail;
 assert.equal(saved.outcome,'interrupted');assert.equal(saved.rated,false);
 assert(saved.participants.every(p=>p.progression_xp_gained===0&&p.rating==null));
 assert.equal(saved.server_epoch,before.server_epoch);
 const log=fs.readFileSync(path.join(root,'matches',allocation.allocation_id,'worker.log'),'utf8');
 assert(log.includes('SHUTDOWN_METRIC terminal_outbox=durable'),'worker exited without confirming durable outbox');
 nextQuery=0;const profileRequest=++query;
 await pumpUntil('interrupted match is counted exactly once',()=>observer.profileResponses.get(String(profileRequest))?.matches_played===progress.matches_played+1,10000,now=>{
  if(now>=nextQuery){nextQuery=now+1000;observer.account({action:'profile',request_id:profileRequest,profile_id:progress.profile_id});}
 });
 for(const key of ['rating','rated_matches','progression_xp','wins','losses'])assert.equal(observer.profile[key],progress[key],`SIGTERM changed ${key}`);
 report.termination={status:'PASS',signal:'SIGTERM',worker_pid:worker,terminal_result:saved.result_id,outcome:saved.outcome,outbox_durable:true,elapsed_seconds:(performance.now()-started)/1000};
 report.scenarios.push('sigterm_interrupts_and_fsyncs_terminal_result');
}
// Two real lobby processes with independent outboxes/match roots and the
// same disposable PostgreSQL. No directory/signing-key provisioning is implied.
async function legacyPenaltyLifecycle() {
 const secondPort=lobbyPort+1,secondRoot=path.join(root,'matches-b');
 const first=Array.from({length:2},(_,i)=>new Client(12400+i,lobbyPort,null,{preference:'bot_practice',autoQueue:false,autoReady:false,autoHandoff:false}));all.push(...first);
 await pumpUntil('legacy profiles exist before another lobby imports their snapshots',()=>first.every(c=>c.auth&&c.profile),20000);
 const before=first.map(c=>({...c.profile})),at=Date.now(),active=first[0],expired=first[1];
 const floors={
  [active.profile.profile_id]:{strikes:3,last_ms:at-30000,until_ms:at+870000},
  [expired.profile.profile_id]:{strikes:2,last_ms:at-400000,until_ms:at-100000},
 };
 fs.mkdirSync(secondRoot);fs.writeFileSync(path.join(secondRoot,'dodge-penalties.json'),JSON.stringify(floors),{flag:'wx'});
 const overrides={SERVER_ADDR:`127.0.0.1:${secondPort}`,OMOBA_MATCH_ROOT:secondRoot,OMOBA_CAREER_OUTBOX:path.join(root,'lobby-outbox-b'),OMOBA_MATCH_FIRST_PORT:String(workerPort+100)};
 let secondLobby=launchLobby(overrides);
 const completions=()=>fs.readFileSync(path.join(root,'lobby.log'),'utf8').split('QUEUE_FLOOR_SYNC pending=0').length-1;
 await pumpUntil('startup confirms both legacy aggregates committed',()=>completions()===1,20000);
 let second=first.map((c,i)=>new Client(12500+i,secondPort,c,{preference:'bot_practice',autoQueue:false,autoReady:false,autoHandoff:false}));all.push(...second);
 await pumpUntil('both imported profiles authenticate on the second lobby',()=>second.every(c=>c.auth&&c.profile),20000);
 active.autoQueue=true;second[0].autoQueue=true;
 await pumpUntil('old active cooldown is enforced on both independent lobbies',()=>[active,second[0]].every(c=>c.career?.match_service?.state==='failed'&&c.career.match_service.code==='dodge_cooldown'&&c.career.match_service_request_id===c.queueId),10000);
 expired.autoQueue=true;
 await pumpUntil('expired legacy cooldown permits fresh consent',()=>expired.career?.match_service?.state==='ready_check',15000);
 const declineCheck=expired.career.match_service.check_id;let nextDecline=0;
 await pumpUntil('new real decline advances the imported strike floor',()=>{
  try{const p=JSON.parse(fs.readFileSync(path.join(root,'matches','dodge-penalties.json'),'utf8'))[expired.profile.profile_id];return p?.strikes===3&&(p.pending_events||[]).length===0&&expired.career?.match_service?.code==='dodge_cooldown';}catch{return false;}
 },10000,now=>{if(now>=nextDecline&&expired.career?.match_service?.state==='ready_check'){nextDecline=now+500;expired.account({action:'ready_match',request_id:expired.queueId,check_id:declineCheck,accepted:false});}});
 second[1].autoQueue=true;
 await pumpUntil('another lobby sees the new decline without copying local counters',()=>second[1].career?.match_service?.code==='dodge_cooldown'&&second[1].career.match_service_request_id===second[1].queueId,10000);
 for(const c of second)c.close();secondLobby.kill('SIGTERM');
 await pumpUntil('only the owned second lobby exits',()=>secondLobby.exitCode!==null,10000);
 secondLobby=launchLobby(overrides);
 await pumpUntil('restart reimports unchanged persisted floors',()=>completions()===2,20000);
 second=first.map((c,i)=>new Client(12600+i,secondPort,c,{preference:'bot_practice',autoReady:false,autoHandoff:false}));all.push(...second);
 await pumpUntil('restart cannot pardon either shared cooldown',()=>second.every(c=>c.career?.match_service?.code==='dodge_cooldown'&&c.career.match_service_request_id===c.queueId),15000);
 assert.deepEqual(JSON.parse(fs.readFileSync(path.join(secondRoot,'dodge-penalties.json'),'utf8')),floors,'import must not manufacture producer events or rewrite source times');
 for(let i=0;i<first.length;i++){
  const c=first[i],request=950000+i;c.autoQueue=false;let nextRead=0;
  await pumpUntil(`legacy import/ready decline gives no career credit for ${c.index}`,()=>c.profileResponses.has(String(request)),10000,now=>{if(now>=nextRead){nextRead=now+1000;c.account({action:'profile',request_id:request,profile_id:c.profile.profile_id});}});
  const saved=c.profileResponses.get(String(request));for(const key of ['rating','matches_played','rated_matches','progression_xp','wins','losses'])assert.equal(saved[key],before[i][key],`legacy migration changed ${key}`);
 }
 assert.equal(fs.readdirSync(secondRoot,{withFileTypes:true}).filter(e=>e.isDirectory()&&fs.existsSync(path.join(secondRoot,e.name,'manifest.json'))).length,0);
 report.legacy_penalty={status:'PASS',lobby_ports:[lobbyPort,secondPort],versionless_snapshots:2,active_floor_strikes:3,expired_floor_strikes:2,new_decline_strikes:3,expired_cooldown_not_rearmed:true,shared_refusals:true,source_snapshot_unchanged:true,restart_preserves_shared_penalties:true,career_unchanged:true,new_workers:0,transport:'real UDP; signed account requests; separate lobby epochs/outboxes'};
 report.scenarios.push('legacy_floor_shared_without_duplicate_events_or_expired_time_revival');for(const c of [...first,...second]){c.autoQueue=false;c.close();}
}

async function crossLobbyReadyLifecycle() {
 const secondPort=lobbyPort+1,secondRoot=path.join(root,'matches-b');
 launchLobby({SERVER_ADDR:`127.0.0.1:${secondPort}`,OMOBA_MATCH_ROOT:secondRoot,
  OMOBA_CAREER_OUTBOX:path.join(root,'lobby-outbox-b'),OMOBA_MATCH_FIRST_PORT:String(workerPort+100)});
 const workers=dir=>fs.existsSync(dir)?fs.readdirSync(dir,{withFileTypes:true}).filter(e=>e.isDirectory()&&fs.existsSync(path.join(dir,e.name,'manifest.json'))).length:0;
 const firstRoot=path.join(root,'matches'),before=workers(firstRoot),prepared=performance.now();
 const first=Array.from({length:10},(_,i)=>new Client(12000+i,lobbyPort,null,{preference:'humans_only',autoQueue:false,autoHandoff:false,autoReady:i!==9}));
 const second=Array.from({length:9},(_,i)=>new Client(12100+i,secondPort,null,{preference:'humans_only',autoQueue:false,autoHandoff:false}));
 const departed=first[9];
 const duplicate=new Client(12200,secondPort,departed,{preference:'humans_only',autoQueue:false,autoReady:false,autoHandoff:false});duplicate.queueId=101;
 all.push(...first,...second,duplicate);
 await pumpUntil('both lobby rosters qualified before Search',()=>[...first,...second,duplicate].every(c=>c.auth&&(c.profile?.completed_public_matches??0)>=10),120000);
 assert.equal(duplicate.profile.profile_id,departed.profile.profile_id,'second endpoint must prove the same account key');
 assert.equal(new Set([...first,...second].map(c=>c.profile.profile_id)).size,19);
 const setupSeconds=(performance.now()-prepared)/1000;
 for(const c of [...first,...second])c.autoQueue=true;
 await pumpUntil('owning lobby has nine saved accepts',()=>first.every(c=>c.career?.match_service?.state==='ready_check')&&first[0].career.match_service.accepted_count===9,20000);
 const check=first[0].career.match_service.check_id;duplicate.autoQueue=true;
 await pumpUntil('another lobby refuses the account while original confirmation is open',()=>duplicate.career?.match_service?.state==='failed'&&duplicate.career.match_service.code==='profile_has_another_session'&&duplicate.career.match_service_request_id===duplicate.queueId,8000);
 assert.equal(workers(firstRoot),before,'owning lobby launched before unanimous consent');assert.equal(workers(secondRoot),0,'another lobby launched an unconfirmed roster');
 const firstRefusal=duplicate.career.match_service.code;
 await pumpUntil('other lobby sees committed timeout penalty without old-host outbox delivery',()=>departed.career?.match_service?.state==='failed'&&departed.career.match_service.code==='dodge_cooldown'&&duplicate.career?.match_service?.state==='failed'&&duplicate.career.match_service.code==='dodge_cooldown'&&first.slice(0,9).every(c=>c.career?.match_service?.state==='waiting')&&second.every(c=>c.career?.match_service?.state==='waiting'),20000);
 assert.equal(duplicate.career.match_service_request_id,101,'foreign refusal lost current search correlation');
 for(const c of first.slice(0,9))assert(c.career.match_service.elapsed_secs>=15,'owning survivors lost FIFO age');
 assert.equal(workers(firstRoot),before);assert.equal(workers(secondRoot),0);
 const penalty=JSON.parse(fs.readFileSync(path.join(firstRoot,'dodge-penalties.json'),'utf8'))[departed.profile.profile_id];
 assert.equal(penalty.strikes,1);assert.equal((penalty.pending_events||[]).length,0,'SQL ready timeout created a second producer event');
 const foreignFile=path.join(secondRoot,'dodge-penalties.json');
 assert(!fs.existsSync(foreignFile)||Object.keys(JSON.parse(fs.readFileSync(foreignFile,'utf8'))).length===0,'foreign lobby copied/fabricated local penalties');
 report.cross_lobby_ready={status:'PASS',lobby_ports:[lobbyPort,secondPort],distinct_ranked_profiles:19,authenticated_duplicate_endpoints:2,
  setup_seconds:setupSeconds,qualification:'disclosed synthetic prior public receipts',window_secs:15,first_refusal:firstRefusal,
  after_timeout_refusal:'dodge_cooldown',owning_accepted_survivors:9,foreign_waiting_survivors:9,fifo_age_preserved:true,
  new_workers_before_unanimity:0,foreign_local_penalty_copy:false,duplicate_producer_events:0,transport:'real UDP; signed player account requests; independent server epochs'};
 report.scenarios.push('two_lobbies_share_ready_ownership_and_timeout_discipline');
 for(const c of [...first,...second,duplicate]){c.autoQueue=false;c.close();}
}

async function readyCheckLifecycle() {
 const workers = () => fs.readdirSync(path.join(root,'matches'),{withFileTypes:true}).filter(e=>e.isDirectory()&&fs.existsSync(path.join(root,'matches',e.name,'manifest.json'))).length;
 const before=workers();
 const preparedAt=performance.now();
 const roster=Array.from({length:10},(_,i)=>new Client(5000+i,lobbyPort,null,{preference:'humans_only',autoQueue:false,autoHandoff:false,autoReady:i!==9}));
 all.push(...roster); const missing=roster[9];
 // Isolated fixture writes are setup, not player consent. Do not start the
 // gameplay deadline while ten serial PostgreSQL qualification jobs are pending.
 await pumpUntil('ready-check roster qualified before Search',()=>roster.every(c=>c.auth&&(c.profile?.completed_public_matches??0)>=10),90000);
 assert(roster.every(c=>!['waiting','ready_check','assigned'].includes(c.career?.match_service?.state)),'fixture setup started Search');
 const qualificationSetupSeconds=(performance.now()-preparedAt)/1000;
 for(const c of roster)c.autoQueue=true;
 await pumpUntil('nine explicit confirmations',()=>roster.every(c=>c.career?.match_service?.state==='ready_check')&&roster[0].career.match_service.accepted_count===9,20000);
 const check=roster[0].career.match_service.check_id;
 assert(roster.every(c=>c.career.match_service.check_id===check));
 assert.equal(workers(),before,'worker launched without unanimous consent');
 await pumpUntil('15-second ready timeout',()=>missing.career?.match_service?.state==='failed'&&missing.career.match_service.code==='dodge_cooldown'&&roster.slice(0,9).every(c=>c.career?.match_service?.state==='waiting'),20000);
 assert.equal(workers(),before,'an expired roster launched a worker');
 for(const c of roster.slice(0,9))assert(c.career.match_service.elapsed_secs>=15,'requeue lost FIFO age');
 missing.autoQueue=false;
 missing.account({action:'ready_match',request_id:missing.queueId,check_id:check,accepted:true});
 await pumpFor(500); assert.equal(workers(),before,'late accept revived an expired check');
 const replacement=new Client(5010,lobbyPort,null,{preference:'humans_only',autoQueue:false,autoHandoff:false});all.push(replacement);
 await pumpUntil('ready-check replacement qualified before Search',()=>replacement.auth&&(replacement.profile?.completed_public_matches??0)>=10,90000);
 replacement.autoQueue=true;
 const accepted=[...roster.slice(0,9),replacement];
 await pumpUntil('replacement unanimously accepts a new roster',()=>accepted.every(c=>c.career?.match_service?.state==='assigned'),25000);
 const allocation=accepted[0].career.match_service.allocation;
 assert(accepted.every(c=>c.career.match_service.allocation.allocation_id===allocation.allocation_id));
 assert.equal(workers(),before+1,'one unanimous roster must start exactly one worker');
 for(const c of accepted)c.autoHandoff=true;
 await pumpUntil('replacement ten-human roster reaches acknowledged Running',()=>accepted.every(c=>c.stage==='running')&&allocationStatus(allocation.allocation_id)?.phase==='running',45000);
 const readHealth=p=>{try{if(fs.statSync(p).size>4096)return null;return parse(fs.readFileSync(p));}catch{return null;}};
 const freshHealth=h=>h?.version===2&&h.sql_ready===true&&h.journal_ready===true&&h.staged_dodges===0&&h.legacy_floors===0&&h.unpersisted_results===0&&h.critical_replies===0&&h.reply_receiver_closed===false&&h.blocked_since_ms===null&&Number.isSafeInteger(h.updated_ms)&&h.updated_ms<=Date.now()+5000&&Date.now()-h.updated_ms<=60000&&Number.isSafeInteger(h.probe_checked_ms)&&Date.now()-h.probe_checked_ms<=30000;
 let lobbyHealth,workerHealth;
 await pumpUntil('fresh lobby and Ranked-worker storage health',()=>{
  lobbyHealth=readHealth(path.join(root,'lobby-outbox','health','status.json'));
  workerHealth=readHealth(path.join(root,'matches',allocation.allocation_id,'outbox','health','status.json'));
  const status=allocationStatus(allocation.allocation_id);
  return freshHealth(lobbyHealth)&&lobbyHealth.server_epoch===null&&freshHealth(workerHealth)&&status&&String(workerHealth.server_epoch)===String(status.server_epoch);
 },20000);
 report.storage_health={status:'PASS',lobby_sql_ready:true,worker_sql_ready:true,journals_ready:true,worker_epoch_matches:true,staged_dodges:0,legacy_floors:0,unpersisted_results:0,critical_replies:0,reply_receiver_closed:false,blocked:false,scope:'actual lobby and ten-human Ranked worker; fresh authenticated probes; no injected health report'};
 for(const c of [...roster,replacement]){c.autoQueue=false;c.close();}
 report.ready_check={status:'PASS',qualification_setup_seconds:qualificationSetupSeconds,fixture_setup_before_search:true,window_secs:15,accepted_before_timeout:9,worker_count_before_unanimity:0,fifo_age_preserved:true,late_accept_rejected:true,replacement_allocation:allocation.allocation_id,replacement_humans_running:accepted.length};
 report.scenarios.push('unanimous_15s_ready_timeout_fifo_replacement');
}
async function lifecycle(original) {
 console.log(wire({phase:'lifecycle',action:'humans_only_cancel'}));
 const baseline=ids(original.snapshot);
 const allocation=original.allocation;
 const checks=report.lifecycle={checks:[],allocation_id:allocation.allocation_id,endpoint:allocation.endpoint};
 const waiting=new Client(1001,lobbyPort,null,{preference:'humans_only',autoQueue:false,autoJoin:false});all.push(waiting);
 await pumpUntil('waiting player authentication',()=>!!waiting.auth);
 let nextQueue=0;
 await pumpUntil('humans-only waiting response',()=>waiting.career?.match_service?.state==='waiting'&&waiting.career?.match_service_request_id===waiting.queueId,20000,now=>{
  if(now>=nextQueue){nextQueue=now+1000;waiting.account({action:'find_match',request_id:waiting.queueId,preference:'humans_only'});}
 });
 const view=waiting.career.match_service;
 assert.equal(view.preference,'humans_only');assert.equal(view.bot_fill_after_secs,null);
 assert.equal(view.needed,10);assert.equal(view.humans,1);assert.equal(waiting.allocationsSeen.length,0);
 // Stop queue retries before sending the authenticated cancellation.
 let nextCancel=0;
 await pumpUntil('signed queue cancellation',()=>waiting.career?.match_service?.state==='idle',10000,now=>{
  if(now>=nextCancel){nextCancel=now+500;waiting.account({action:'cancel_queue'});}
 });
 await pumpFor(1200);
 assert.equal(waiting.career.match_service.state,'idle');assert.equal(waiting.allocationsSeen.length,0);
 assertRoster(original,baseline);
 checks.checks.push({name:'signed_humans_only_cancel',status:'PASS',waiting:view,final_state:'idle',allocations:0});
 report.scenarios.push('signed_humans_only_cancel_without_allocation');

 // Exercise the real 100ms account throttle: FindMatch and CancelQueue arrive
 // together. A subsequently signed gameplay Leave must still remove the queue.
 waiting.queueId++;
 waiting.account({action:'find_match',request_id:waiting.queueId,preference:'humans_only'});
 waiting.account({action:'cancel_queue'});
 await pumpUntil('queued after throttled account cancel',()=>waiting.career?.match_service?.state==='waiting'&&waiting.career?.match_service_request_id===waiting.queueId,10000);
 let nextLeave=0;
 await pumpUntil('signed Leave cancels throttled queue',()=>waiting.career?.match_service?.state==='idle',10000,now=>{
  if(now>=nextLeave){nextLeave=now+300;waiting.gameplay({type:'leave'});}
 });
 checks.checks.push({name:'signed_leave_after_throttled_cancel',status:'PASS',final_state:'idle'});
 report.scenarios.push('signed_leave_cancels_after_account_throttle');
 await pumpFor(300);
 waiting.queueId++;
 waiting.account({action:'find_match',request_id:waiting.queueId,preference:'humans_only'});
 await pumpUntil('queue intent for lost cancel test',()=>waiting.career?.match_service?.state==='waiting'&&waiting.career?.match_service_request_id===waiting.queueId,10000);
 const abandonedAt=performance.now();
 // Deliberately send neither cancellation packet. Home-style signed heartbeats
 // continue, but no explicit FindMatch retry may renew the queue lease.
 // C3 keeps search intent for 60 s, independently of the 15 s ready deadline.
 // Prove both sides: a short suspension preserves age, heartbeats cannot keep
 // an abandoned queue alive forever. The former 22 s expectation was stale.
 const heartbeatPackets=waiting.sentPackets;
 await pumpFor(30000);
 assert.equal(waiting.career?.match_service?.state,'waiting','short mobile pause lost queue intent');
 assert.equal(waiting.career.match_service_request_id,waiting.queueId);
 assert(waiting.career.match_service.elapsed_secs>=29,'short mobile pause reset FIFO age');
 assert(waiting.sentPackets>heartbeatPackets,'fixture must keep signed Home heartbeats active');
 await pumpUntil('60-second abandoned queue expires despite authenticated heartbeat',()=>waiting.career?.match_service?.state==='idle',45000);
 const expiredAfter=performance.now()-abandonedAt;
 assert(expiredAfter>=59000,'queue expired before its 60-second intent lease');
 assert.equal(waiting.allocationsSeen.length,0);
 checks.checks.push({name:'lost_cancel_expires_with_home_heartbeat',status:'PASS',short_pause_preserves_fifo:true,intent_lease_secs:60,elapsed_ms:expiredAfter,allocations:0});
 report.scenarios.push('abandoned_queue_expires_despite_authenticated_heartbeat');

 console.log(wire({phase:'lifecycle',action:'stranger_denied'}));
 const stranger=new Client(1002,original.port,null,{autoQueue:false,autoJoin:false});all.push(stranger);
 await pumpUntil('independent stranger authentication',()=>!!stranger.auth);
 assert.notEqual(stranger.profile.profile_id,original.profile.profile_id);
 // Use the real current worker match ID: rejection must test admission rather
 // than an obsolete match envelope. The key/session is a real authenticated outsider.
 stranger.match=original.match;
 let nextJoin=0;
 await pumpFor(3000,now=>{
  if(now>=nextJoin){nextJoin=now+500;stranger.join();}
  assertRoster(original,baseline);
 });
 assert(stranger.joinAttempts>=4,'stranger admission attempts were not exercised');
 assert.equal(stranger.worldSnapshots,0,'unallocated authenticated identity received world replication');
 assert.equal(stranger.stage,'authenticated');
 checks.checks.push({name:'authenticated_stranger_denied',status:'PASS',profile: stranger.profile.profile_id,attempts:stranger.joinAttempts,world_snapshots:0,bootstrap_snapshots:stranger.bootstrapSnapshots,retained_roster:baseline});
 report.scenarios.push('authenticated_stranger_cannot_join_or_observe_running_worker');

 console.log(wire({phase:'lifecycle',action:'participant_reconnect'}));
 const before=original.snapshot.players.find(p=>p.id===original.snapshot.your_id);
 const oldId=before.id,oldPort=original.port,oldProgress=careerProgress(original);
 original.close();
 // Real PLAYER_TIMEOUT is five seconds. Close without Leave so the allocation
 // and durable identity remain available within the thirty-second reclaim window.
 await pumpFor(5500);
 const resumed=new Client(original.index,lobbyPort,original);resumed.queueId=original.queueId+1;all.push(resumed);
 await pumpUntil('original participant reconnect via lobby',()=>resumed.stage==='running',25000);
 // LR-13: the lobby reports a running seat as the in-match state, with when the match ends at the latest.
 const inMatch=seconds=>Number.isInteger(seconds)&&seconds>0&&seconds<=2700;
 assert(inMatch(resumed.inMatch?.ends_within_secs),`lobby did not report the running seat as in-match: ${wire(resumed.inMatch)}`);
 assert.equal(resumed.port,oldPort,'reconnect allocated a second worker');
 assert.equal(resumed.allocation.allocation_id,allocation.allocation_id,'reconnect duplicated active allocation');
 assert.equal(resumed.snapshot.your_id,oldId,'reconnect replaced the player identity');
 assert.deepEqual(careerProgress(resumed),oldProgress,'career progress changed during reconnect');
 assertRoster(resumed,baseline);
 const after=resumed.snapshot.players.find(p=>p.id===oldId);
 assert(after.level>=before.level,'reconnect reset in-match level');
 if(after.level===before.level)assert(after.xp>=before.xp,'reconnect reset in-match experience');
 // The takeover bot may shop while the player is away: earned gold only grows
 // and no owned item is lost.
 const earned=(client,id)=>client.snapshot.scoreboard?.players.find(p=>String(p.player_id)===String(id))?.earned_gold??0;
 assert(earned(resumed,oldId)>=earned(original,oldId),'reconnect reset earned gold');
 const owned=[...after.inventory];
 for(const item of before.inventory){const index=owned.indexOf(item);assert(index>=0,'reconnect lost inventory');owned.splice(index,1);}
 assert.equal(resumed.snapshot.players.filter(p=>!p.is_bot).length,1,'reconnect created a duplicate human');
 await pumpFor(1000,()=>assertRoster(resumed,baseline));
 const manifests=fs.readdirSync(path.join(root,'matches'),{withFileTypes:true})
  .filter(entry=>entry.isDirectory()&&fs.existsSync(path.join(root,'matches',entry.name,'manifest.json')))
  .map(entry=>JSON.parse(fs.readFileSync(path.join(root,'matches',entry.name,'manifest.json'),'utf8')));
 assert.equal(manifests.filter(m=>m.humans.some(h=>h.profile_id===resumed.profile.profile_id)).length,1,'profile has duplicate worker manifests');
 assert.equal(manifests.filter(m=>m.humans.some(h=>h.profile_id===waiting.profile.profile_id)).length,0,'cancelled waiter was allocated');
 checks.checks.push({name:'same_participant_reconnect',status:'PASS',player_id:oldId,port:oldPort,allocation_id:allocation.allocation_id,progress:careerProgress(resumed),roster:baseline,in_match_before:{level:before.level,xp:before.xp,gold:before.gold},in_match_after:{level:after.level,xp:after.xp,gold:after.gold}});
 report.scenarios.push('same_profile_session_reconnect_preserves_player_progress_and_worker');

 // LR-13: a deliberate Leave keeps the running seat under bot control. Play
 // on the lobby then answers with the in-match state, never a fresh match,
 // and resuming reclaims the same hero.
 console.log(wire({phase:'lifecycle',action:'leave_then_play'}));
 resumed.gameplay({type:'leave'});
 await pumpFor(300);
 resumed.close();
 const back=new Client(original.index,lobbyPort,original);back.queueId=resumed.queueId+1;all.push(back);
 await pumpUntil('play after leave resumes the running seat',()=>back.stage==='running',25000);
 assert(inMatch(back.inMatch?.ends_within_secs),`lobby did not report the left seat as in-match: ${wire(back.inMatch)}`);
 assert.equal(back.allocation.allocation_id,allocation.allocation_id,'play after leave allocated another match');
 assert.equal(back.snapshot.your_id,oldId,'resume replaced the player identity');
 await pumpFor(1000,()=>assertRoster(back,baseline));
 assert.equal(allocationManifests().filter(m=>m.humans.some(h=>h.profile_id===back.profile.profile_id)).length,1,'play after leave created a second manifest');
 checks.checks.push({name:'leave_then_play_resumes_running_seat',status:'PASS',allocation_id:allocation.allocation_id,ends_within_secs:back.inMatch.ends_within_secs});
 report.scenarios.push('leave_then_play_shows_in_match_and_resumes_the_seat');
 for(const client of [waiting,stranger,resumed,back])assert.equal(client.errors.length,0,`client ${client.index} transport errors`);
}
function allocationStatus(id) {
 try{return parse(fs.readFileSync(path.join(root,'matches',id,'status.json')))}catch{return null;}
}
function allocationManifests() {
 return fs.readdirSync(path.join(root,'matches'),{withFileTypes:true})
  .filter(entry=>entry.isDirectory()&&fs.existsSync(path.join(root,'matches',entry.name,'manifest.json')))
  .map(entry=>JSON.parse(fs.readFileSync(path.join(root,'matches',entry.name,'manifest.json'),'utf8')));
}
function ownedWorkerPid(port) {
 const result=spawnSync('lsof',['-nP',`-iUDP:${port}`,'-t'],{encoding:'utf8'});
 assert(!result.error, 'recovery needs lsof to identify the exact owned worker');
 const pids=[...new Set((result.stdout||'').trim().split(/\s+/).filter(Boolean).map(Number))];
 assert.equal(pids.length,1,`expected one UDP owner on worker port ${port}`);
 const pid=pids[0];
 const group=Number(spawnSync('ps',['-o','pgid=','-p',String(pid)],{encoding:'utf8'}).stdout?.trim());
 assert(ownedGroups.has(group)&&pid!==lobby.pid, 'refusing to signal a process outside this probe');
 return pid;
}
// LR-11 + LR-13: the only human leaves a running bot practice. Until the worker
// reaps it (reclaim window + grace, about 210 s of real time), Play on the
// lobby answers with the in-match state, never a fresh handoff or a failure;
// then the seat frees, the leaver is cooled down, and a fresh player can play.
async function abandoned(original) {
 const allocation=original.allocation;
 const checks=report.abandoned={checks:[],allocation_id:allocation.allocation_id};
 console.log(wire({phase:'abandoned',action:'leave'}));
 original.gameplay({type:'leave'});
 await pumpFor(300);
 original.close();
 const leftAt=performance.now();
 const player=new Client(original.index,lobbyPort,original,{autoQueue:false,autoJoin:false,autoHandoff:false});
 player.queueId=original.queueId+1;all.push(player);
 await pumpUntil('lobby authentication after leave',()=>!!player.auth);
 let nextFind=0,nextProgress=0,inMatchAnswers=0,firstEnds=null;
 await pumpUntil('abandoned match reaped and durable cooldown applied',()=>player.career?.match_service?.state==='failed'&&player.career.match_service.code==='play_cooldown',330000,now=>{
  const view=player.career?.match_service;
  if(view?.state==='assigned'&&view.allocation.allocation_id===allocation.allocation_id){
   assert(view.in_match,'the left running match was handed out as a fresh match');
   inMatchAnswers++;firstEnds??=view.in_match.ends_within_secs;
  }
  if(view?.state==='failed')assert.equal(view.code,'play_cooldown',`unexpected search failure: ${wire(view)}`);
  if(now>=nextFind){nextFind=now+2000;player.account({action:'find_match',request_id:player.queueId,preference:'bot_practice'});}
  if(now>=nextProgress){nextProgress=now+15000;console.log(wire({phase:'abandoned',elapsed_seconds:Math.round((now-leftAt)/1000),match_service:view?.state,ends_within_secs:view?.in_match?.ends_within_secs}));}
 });
 const waited=(performance.now()-leftAt)/1000;
 assert(inMatchAnswers>0,'the lobby never reported the left match as in-match');
 assert(waited>=180,`the seat freed before the reclaim window: ${waited}s`);
 assert(firstEnds>0&&firstEnds<=215,`in-match end is not the reaping deadline: ${firstEnds}`);
 const status=allocationStatus(allocation.allocation_id);
 assert.equal(status?.phase,'failed','the abandoned worker did not retire');
 const log=fs.readFileSync(path.join(root,'matches',allocation.allocation_id,'worker.log'),'utf8');
 assert(/MATCH_METRIC event=abandoned_reaped outcome=Interrupted/.test(log),'worker did not log the reaping');
 assert.equal(allocationManifests().filter(m=>m.humans.some(h=>h.profile_id===player.profile.profile_id)).length,1,'cooled player received a new allocation');
 assert.equal(player.career.play_cooldown?.strikes,1);
 const remaining=Number(player.career.play_cooldown.until)-Date.now()/1000;
 assert(remaining>250&&remaining<=301,'first abandon did not persist its five-minute cooldown');
 const fresh=new Client(9500,lobbyPort,null,{preference:'bot_practice'});all.push(fresh);
 await pumpUntil('a fresh profile can start after the abandoned worker retires',()=>fresh.stage==='running',45000);
 assert.notEqual(fresh.allocation.allocation_id,allocation.allocation_id);
 checks.checks.push({name:'left_match_in_match_until_reaped_then_durable_cooldown',status:'PASS',waited_seconds:waited,in_match_answers:inMatchAnswers,first_ends_within_secs:firstEnds,cooldown_remaining_secs:remaining,new_allocation_id:fresh.allocation.allocation_id});
 report.scenarios.push('left_running_match_reaped_leaver_cooled_and_fresh_profile_can_play');
}
const quantiles=values=>{const v=values.filter(Number.isFinite).sort((a,b)=>a-b);return v.length?{p50:v[Math.floor(v.length*.5)],p95:v[Math.floor(v.length*.95)],p99:v[Math.floor(v.length*.99)],max:v.at(-1)}:null;};
const mean=values=>values.length?values.reduce((a,b)=>a+b,0)/values.length:null;
const round=(value,digits=1)=>value==null?null:Math.round(value*10**digits)/10**digits;
// LR-09: what the capacity decision needs, per arena type (the allocation's queue), and
// what the lobby with every worker used while all played.
function measure(allocations, runningAt) {
 const types={};
 for(const [id,players] of allocations){const type=players[0].allocation.preference;(types[type]??={arenas:0,players:[]}).arenas++;types[type].players.push(...players);}
 report.per_arena_type=Object.fromEntries(Object.entries(types).map(([type,{arenas,players}])=>[type,{
  arenas, players:players.length,
  snapshot_gap_ms:quantiles(players.flatMap(c=>c.gaps)),
  running_after_secs:quantiles(players.map(c=>(c.firstRunning-c.createdAt)/1000)),
  snapshot_bytes_per_second_per_player:quantiles(players.map(c=>c.runningBytes*1000/Math.max(1,c.lastSnapshot-c.firstRunning))),
 }]));
 const soak=resources.filter(r=>r.wall_ms>=runningAt);
 const cpu=soak.map(r=>r.cpu_percent_interval).filter(Number.isFinite), rss=soak.map(r=>r.rss_mib);
 report.soak_resources={samples:soak.length,processes:Math.max(0,...soak.map(r=>r.processes)),cpu_percent_mean:round(mean(cpu)),cpu_percent_peak:round(Math.max(0,...cpu)),rss_mib_mean:round(mean(rss)),rss_mib_peak:round(Math.max(0,...rss)),
  per_arena:{cpu_percent:round(mean(cpu)/allocations.size,2),rss_mib:round(Math.max(0,...rss)/allocations.size,1)}};
 const waits=all.map(c=>(c.firstRunning-c.createdAt)/1000);
 report.running_after_secs=quantiles(waits);
 if(maxWaitSecs!==null){const late=waits.filter(w=>w>maxWaitSecs).length;assert.equal(late,0,`${late} of ${all.length} players waited longer than ${maxWaitSecs}s to reach Running`);}
 // One line to paste into ops/vps/ru/README.md (Arena capacity).
 for(const [type,stats] of Object.entries(report.per_arena_type))console.log(`CAPACITY_RESULT type=${type} arenas=${stats.arenas} players=${stats.players} capacity=${capacity} gap_p95_ms=${round(stats.snapshot_gap_ms?.p95)} gap_p99_ms=${round(stats.snapshot_gap_ms?.p99)} running_after_max_s=${round(stats.running_after_secs?.max)} bytes_per_s_per_player_p50=${Math.round(stats.snapshot_bytes_per_second_per_player?.p50??0)} all_cpu_mean_pct=${report.soak_resources.cpu_percent_mean} all_rss_peak_mib=${report.soak_resources.rss_mib_peak} limits=${JSON.stringify(limits)}`);
}
// LR-09: a second vs Bots player while the only arena is busy waits in the line with its
// place, and the lobby reports the busy pool (capacity.json and CAPACITY_METRIC).
async function capacityWait() {
 console.log(wire({phase:'capacity-wait',action:'second_player'}));
 const second=new Client(3001,lobbyPort,null,{autoHandoff:false,autoJoin:false,preference:'bot_practice'});all.push(second);
 await pumpUntil('second player waits in the line for the busy arena',()=>{const v=second.career?.match_service;return v?.state==='waiting'&&v.capacity_wait===true&&v.capacity_line?.position===1;},20000);
 const file=path.join(root,'matches','capacity.json');
 const read=()=>{try{return JSON.parse(fs.readFileSync(file,'utf8'));}catch{return null;}};
 await pumpUntil('lobby reports the busy pool',()=>read()?.busy_since_ms!=null&&read()?.waiting_for_arena===1,10000);
 const busy=read();
 assert.equal(busy.capacity,1);assert.equal(busy.occupied,1);
 assert(/CAPACITY_METRIC event=busy capacity=1 occupied=1 searching=1 waiting_for_arena=1/.test(fs.readFileSync(path.join(root,'lobby.log'),'utf8')),'lobby did not log the busy pool');
 assert.equal(second.allocationsSeen.length,0,'a full pool allocated a second arena');
 report.capacity_wait={view:second.career.match_service,report:busy};
 report.scenarios.push('full_pool_shows_place_in_line_and_reports_busy');
}

async function maintenanceDrain(original) {
 const rootPath=path.join(root,'matches'), marker=path.join(rootPath,'drain');
 const operation=crypto.randomBytes(16).toString('hex');
 const second=new Client(3201,lobbyPort,null,{autoHandoff:false,autoJoin:false,autoReady:false,preference:'bot_practice'});all.push(second);
 await pumpUntil('second roster awaits explicit consent',()=>second.career?.match_service?.state==='ready_check');
 const oldCheck=second.career.match_service.check_id;
 assert.equal(allocationManifests().length,1);
 fs.mkdirSync(marker);fs.writeFileSync(path.join(marker,operation),'',{flag:'wx'});
 const markerAt=Date.now();
 const capacityReport=()=>{try{return JSON.parse(fs.readFileSync(path.join(rootPath,'capacity.json'),'utf8'));}catch{return null;}};
 await pumpUntil('lobby acknowledges this exact maintenance operation',()=>{
  const r=capacityReport();return r?.draining&&r.drain_operation_id===operation&&r.updated_ms>=markerAt&&r.occupied===1&&second.career?.match_service?.code==='server_draining';
 },10000);
 const firstTick=original.snapshot.snapshot_tick;
 second.account({action:'ready_match',request_id:second.queueId,check_id:oldCheck,accepted:false});
 const third=new Client(3202,lobbyPort,null,{autoHandoff:false,autoJoin:false,preference:'bot_practice'});all.push(third);
 await pumpUntil('new signed search receives correlated maintenance refusal',()=>third.career?.match_service_request_id===third.queueId&&third.career.match_service?.state==='failed'&&third.career.match_service.code==='server_draining');
 await pumpFor(16000,()=>{
  assert.equal(allocationManifests().length,1,'maintenance started another worker despite a free arena');
  assert.equal(allocationStatus(original.allocation.allocation_id)?.phase,'running','maintenance stopped the existing match');
 });
 assert(original.snapshot.snapshot_tick>firstTick,'existing worker stopped advancing');
 assert(performance.now()-original.lastSnapshot<2000,'existing match lost fresh snapshots');
 const penaltyPath=path.join(rootPath,'dodge-penalties.json');
 const penalties=fs.existsSync(penaltyPath)?JSON.parse(fs.readFileSync(penaltyPath,'utf8')):{};
 assert(!penalties[second.profile.profile_id],'maintenance/old decline created a dodge strike');
 assert.equal(third.allocationsSeen.length,0);
 third.autoQueue=false;third.account({action:'cancel_queue'});
 // Remove only this operation's marker, then require a fresh explicit check.
 assert.deepEqual(fs.readdirSync(marker),[operation]);
 fs.unlinkSync(path.join(marker,operation));fs.rmdirSync(marker);
 second.autoReady=true;second.autoJoin=true;second.autoHandoff=true;
 let freshCheck=null;
 await pumpUntil('normal search resumes through fresh consent and actual play',()=>second.stage==='running',25000,()=>{
  const r=second.career?.match_service;if(r?.state==='ready_check'){assert.notEqual(r.check_id,oldCheck);freshCheck=r.check_id;}
 });
 assert(freshCheck,'resume skipped explicit new consent');
 assert.notEqual(second.allocation.allocation_id,original.allocation.allocation_id);
 assert.equal(allocationManifests().length,2);
 await pumpFor(500);
 await pumpUntil('lobby publishes resumed state',()=>capacityReport()?.draining===false,10000);
 // A maintenance marker must survive a lobby restart without becoming a
 // phantom arena or disturbing adopted matches/resume admission.
 const restartOperation=crypto.randomBytes(16).toString('hex');
 fs.mkdirSync(marker);fs.writeFileSync(path.join(marker,restartOperation),'',{flag:'wx'});
 await pumpUntil('second maintenance operation is acknowledged',()=>capacityReport()?.draining&&capacityReport()?.drain_operation_id===restartOperation,10000);
 const restartAt=Date.now();
 await restartAndRecover(original,false);
 await pumpUntil('restarted lobby retains exact maintenance acknowledgement',()=>{
  const r=capacityReport();return r?.draining&&r.drain_operation_id===restartOperation&&r.updated_ms>=restartAt&&r.occupied===2;
 },10000);
 const afterRestart=new Client(3203,lobbyPort,null,{autoHandoff:false,autoJoin:false,preference:'bot_practice'});all.push(afterRestart);
 await pumpUntil('new search is refused by the restarted draining lobby',()=>afterRestart.career?.match_service_request_id===afterRestart.queueId&&afterRestart.career.match_service?.code==='server_draining');
 assert.equal(allocationManifests().length,2);
 assert.deepEqual(fs.readdirSync(marker),[restartOperation]);
 afterRestart.autoQueue=false;afterRestart.account({action:'cancel_queue'});
 fs.unlinkSync(path.join(marker,restartOperation));fs.rmdirSync(marker);
 await pumpUntil('restarted lobby resumes after explicit cancellation',()=>capacityReport()?.draining===false,10000);
 assert(all.every(c=>c.errors.length===0),'drain probe received transport errors');
 report.drain={status:'PASS',operation_id:operation,restart_operation_id:restartOperation,capacity:2,existing_allocation:original.allocation.allocation_id,new_allocation:second.allocation.allocation_id,paused_ready_without_penalty:true,late_decline_rejected:true,new_search_correlated_refusal:true,existing_worker_advanced:true,fresh_consent_required:true,lobby_restart_keeps_drain_and_adopted_workers:true,resume_allowed_during_drain:true,observation_seconds:16};
 report.scenarios.push('maintenance_pauses_new_matches_and_preserves_live_play_fifo_consent');
}
// A complete public roster is killed before creating observers, so the early
// case proves the durable marker before the normal 10-second checkpoint.
async function teamVoteJourney(peers, kind) {
 assert.equal(peers.length,10);
 const allocation=peers[0].allocation, status=allocationStatus(allocation.allocation_id);
 assert(status?.phase==='running'&&status.result_id);
 const baselines=peers.map(c=>({...careerProgress(c),abandons:c.profile.abandons??0}));
 const firstTeam=peers[0].snapshot.players.find(p=>p.id===peers[0].snapshot.your_id).team;
 const sameTeam=peers.filter(c=>c.snapshot.players.find(p=>p.id===c.snapshot.your_id).team===firstTeam);
 const opponent=peers.find(c=>!sameTeam.includes(c));
 const leaver=kind==='remake'?sameTeam.shift():null;
 if(leaver)leaver.close(); // Actual transport timeout + ninety seconds; no shortened server clock.
 const voters=sameTeam, initiator=voters[0];
 await pumpUntil(kind+' becomes server-eligible',()=>initiator.career?.match_vote?.[kind]?.available,
  kind==='remake'?160000:630000,now=>{
   assert.equal(allocationStatus(allocation.allocation_id)?.phase,'running','the game ended before the vote');
   if(!report.last_vote_progress||now-report.last_vote_progress>15000){report.last_vote_progress=now;resourceSample();console.log(wire({phase:kind,waiting_for_eligibility:true,elapsed_seconds:Math.round((now-initiator.firstRunning)/1000)}));}
  });
 const elapsed=(performance.now()-initiator.firstRunning)/1000;
 assert(elapsed>=(kind==='remake'?90:600)-1,'vote opened before its real gameplay gate');
 const request=(c,action,generation,overrides={})=>{
  const view=c.career?.match_vote;
  if(view)c.gameplay({type:'match_vote',server_epoch:view.server_epoch,match_id:view.match_id,generation,kind,action,...overrides});
 };
 let next=0;
 await pumpUntil('team vote starts',()=>initiator.career?.match_vote?.active?.kind===kind,5000,now=>{
  if(now>=next){next=now+500;request(initiator,'start',initiator.career.match_vote.next_generation);}
 });
 const active=initiator.career.match_vote.active, generation=active.generation;
 const required=kind==='remake'?3:4;
 assert.equal(active.required,required);assert.equal(active.eligible,voters.length);assert.equal(active.yes,1);
 request(opponent,'yes',generation);request(initiator,'yes',generation);request(voters[1],'yes',0);
 const match=initiator.career.match_vote.match_id;
 request(voters[1],'yes',generation,{match_id:typeof match==='bigint'?match+1n:match+1});
 await pumpFor(700);
 assert.equal(initiator.career.match_vote.active.yes,1,'opponent/duplicate/stale request changed the ballot');
 request(voters[required],'no',generation);
 await pumpUntil('explicit No is counted once',()=>initiator.career?.match_vote?.active?.no===1,5000);
 next=0;
 await pumpUntil('authoritative vote reaches saved settlement',()=>peers.some(c=>c.career?.last_result?.result_id===status.result_id&&c.career.last_result.saved),20000,now=>{
  if(now>=next){next=now+500;for(const c of voters.slice(1,required))request(c,'yes',generation);}
 });
 for(const c of peers)c.close();
 const observers=peers.map((c,i)=>new Client(9000+i,lobbyPort,c,{autoQueue:false,autoJoin:false,autoHandoff:false,preference:'humans_only'}));
 all.push(...observers);
 await pumpUntil('every vote participant reauthenticates on lobby',()=>observers.every(c=>c.auth),20000);
 let query=9000,nextRead=0;
 const profileRequests=observers.map(()=>++query);
 // Career actions share the normal 100ms account throttle. Read one query
 // kind per phase so a repeated Detail cannot starve History/Profile forever.
 await pumpUntil('all ten vote results are durable',()=>observers.every(c=>c.lastDetail?.result_id===status.result_id&&c.lastDetail.saved),20000,now=>{
  if(now>=nextRead){nextRead=now+1500;observers.forEach(c=>c.account({action:'detail',request_id:++query,result_id:status.result_id}));}
 });
 nextRead=0;
 await pumpUntil('all ten vote profiles are durable',()=>observers.every((c,i)=>{
  const p=c.profileResponses.get(String(profileRequests[i])),played=kind==='surrender'||peers[i]===leaver?1:0;
  return p?.matches_played===baselines[i].matches_played+played;
 }),20000,now=>{
  if(now>=nextRead){nextRead=now+1500;observers.forEach((c,i)=>c.account({action:'profile',request_id:profileRequests[i],profile_id:baselines[i].profile_id}));}
 });
 nextRead=0;
 await pumpUntil('all ten vote histories are durable',()=>observers.every(c=>c.lastHistory?.filter(r=>r.result_id===status.result_id).length===1),20000,now=>{
  if(now>=nextRead){nextRead=now+1500;observers.forEach(c=>c.account({action:'history',request_id:++query,before:null}));}
 });
 const saved=observers[0].lastDetail;
 assert.equal(saved.outcome,kind==='remake'?'remade':'completed');assert.equal(saved.end_reason,kind);
 assert.equal(saved.rated,kind==='surrender');assert.equal(saved.participants.length,10);
 assert.equal(saved.winner===null,kind==='remake');
 if(kind==='surrender')assert.notEqual(saved.winner,firstTeam);
 for(let i=0;i<observers.length;i++){
  const c=observers[i],before=baselines[i],fresh=c.profileResponses.get(String(profileRequests[i]));
  assert.deepEqual(c.lastDetail,saved,'participants loaded different settled receipts');
  const p=saved.participants.find(p=>p.profile_id===before.profile_id),history=c.lastHistory.find(r=>r.result_id===saved.result_id);
  const abandoned=peers[i]===leaver;
  assert.equal(p.participation.abandoned,abandoned);assert.equal(fresh.abandons??0,before.abandons+Number(abandoned));
  assert.deepEqual(history.rating,p.rating);
  if(kind==='remake'){
   assert.equal(p.progression_xp_gained,0);assert.equal(fresh.progression_xp,before.progression_xp);
   assert.equal(fresh.wins,before.wins);assert.equal(fresh.losses,before.losses+Number(abandoned));
   assert.equal(fresh.rated_matches,before.rated_matches+Number(abandoned));
   assert.equal(fresh.rating,before.rating-(abandoned?32:0));
   assert.equal(history.won,abandoned?false:null);
   assert.equal(p.rating===null,!abandoned);
  }else{
   const won=p.team===saved.winner,delta=won?24:-24;
   assert.equal(p.rating.delta,delta);assert.equal(fresh.rating,before.rating+delta);
   assert.equal(fresh.progression_xp,before.progression_xp+(won?150:100));
   assert.equal(fresh.wins,before.wins+Number(won));assert.equal(fresh.losses,before.losses+Number(!won));
   assert.equal(fresh.rated_matches,before.rated_matches+1);assert.equal(history.won,won);
  }
 }
 if(leaver){
  const c=observers[peers.indexOf(leaver)];
  assert.equal(c.career.play_cooldown?.strikes,1);
  c.queueId++;next=0;
  await pumpUntil('durable leaver cooldown rejects the next signed lobby search',()=>c.career?.match_service?.state==='failed'&&c.career.match_service.code==='play_cooldown',7000,now=>{
   if(now>=next){next=now+1500;c.account({action:'find_match',request_id:c.queueId,preference:'humans_only'});}
  });
 }
 report.team_vote={status:'PASS',kind,result_id:saved.result_id,participants:10,elapsed_to_vote_seconds:elapsed,
  outcome:saved.outcome,all_profiles_checked:true,all_histories_checked:true,explicit_no:true,stale_and_opponent_votes_rejected:true,
  durable_cooldown_checked:!!leaver,input_model:report.input_model};
 delete report.last_vote_progress;
 report.scenarios.push('signed_'+kind+'_through_durable_profiles_and_history');
}

async function recoverRoster(originals, early) {
 const original=originals[0], allocation=original.allocation;
 assert(originals.every(c=>c.allocation.allocation_id===allocation.allocation_id),'recovery needs one worker roster');
 const baselines=originals.map(c=>({client:c,progress:careerProgress(c),epoch:c.epoch}));
 const before=allocationStatus(allocation.allocation_id), workerPid=ownedWorkerPid(original.port);
 if(original.preference==='humans_only'){
  assert.equal(allocation.rated,true);assert.equal(allocation.human_count,10);assert.equal(allocation.bot_count,0);
 }
 assert(before?.phase==='running'&&before.result_id,'worker must have committed Running');
 const playingFor=Math.max(...originals.map(c=>(performance.now()-c.firstRunning)/1000));
 if(early)assert(playingFor<10,`early crash reached the ordinary checkpoint: ${playingFor}s`);
 process.kill(workerPid,'SIGKILL'); for(const c of originals)c.close();
 const crashedAt=performance.now();
 const observers=baselines.map(({client},i)=>new Client(7000+i,lobbyPort,client,{autoQueue:false,autoJoin:false,autoHandoff:false,preference:client.preference}));
 all.push(...observers);
 await pumpUntil('every recovery observer authenticates',()=>observers.every(c=>!!c.auth));
 let nextDetail=0,nextFind=0,nextProgress=0,query=700,sawRecovering=false;
 const observer=observers[0]; observer.queueId++;
 await pumpUntil('full roster interrupted recovery',()=>{
  const status=allocationStatus(allocation.allocation_id);
  if(status?.phase==='recovering')sawRecovering=true;
  return observer.lastDetail?.result_id===before.result_id&&status?.phase==='failed';
 },300000,now=>{
  const status=allocationStatus(allocation.allocation_id);
  if(status)assert.equal(status.server_epoch,before.server_epoch,'recovery lost the original epoch');
  if(status?.phase!=='failed')assert(!observer.allocationsSeen.some(a=>a.allocation_id!==allocation.allocation_id),'profile reassigned before durable settlement');
  if(now>=nextDetail){nextDetail=now+2000;observer.account({action:'detail',request_id:++query,result_id:before.result_id});}
  if(now>=nextFind&&status?.phase!=='failed'){nextFind=now+7000;observer.account({action:'find_match',request_id:observer.queueId,preference:original.preference});}
  if(now>=nextProgress){nextProgress=now+10000;resourceSample();console.log(wire({phase:'roster-recovery',elapsed_seconds:Math.round((now-crashedAt)/1000),worker_phase:status?.phase}));}
 });
 const saved=observer.lastDetail;
 assert(sawRecovering); assert.equal(saved.outcome,'interrupted'); assert(saved.saved&&!saved.rated);
 assert.notEqual(saved.gameplay_started,false,'a Running match was recovered as an unplayed draft');
 if(original.preference==='humans_only'){
  assert.equal(saved.ruleset,'verdant-default-v1');assert.equal(saved.participants.length,10);
  assert(saved.participants.every(p=>!p.is_bot&&p.profile_id),'Ranked recovery must contain ten authenticated humans');
 }
 assert.equal(saved.server_epoch,before.server_epoch);
 assert(saved.participants.every(p=>p.progression_xp_gained===0&&p.rating==null));
 assert(performance.now()-crashedAt>=120000,'recovery bypassed its real ownership/recovery gate');
 const profileRequests=observers.map(()=>++query); let nextProfiles=0;
 await pumpUntil('every participant has exactly one played match after recovery',()=>observers.every((c,i)=>c.profileResponses.get(String(profileRequests[i]))?.matches_played===baselines[i].progress.matches_played+1),15000,now=>{
  if(now>=nextProfiles){nextProfiles=now+1500;observers.forEach((c,i)=>c.account({action:'profile',request_id:profileRequests[i],profile_id:baselines[i].progress.profile_id}));}
 });
 for(let i=0;i<observers.length;i++){
  const fresh=observers[i].profileResponses.get(String(profileRequests[i])), previous=baselines[i].progress;
  for(const key of ['rating','rated_matches','progression_xp','wins','losses'])assert.equal(fresh[key],previous[key],`recovery changed ${key} for participant ${i}`);
 }
 let nextHistory=0;
 await pumpUntil('interrupted history is visible to every participant',()=>observers.every(c=>c.lastHistory?.filter(r=>r.result_id===before.result_id).length===1),15000,now=>{
  if(now>=nextHistory){nextHistory=now+1500;observers.forEach(c=>c.account({action:'history',request_id:++query,before:null}));}
 });
 const requeueAt=performance.now();
 for(const c of observers){c.queueId++;c.lastQueue=0;c.autoQueue=true;}
 await pumpUntil('all recovered participants can search and accept a fresh roster',()=>observers.every(c=>c.allocation&&c.allocation.allocation_id!==allocation.allocation_id),capacity===1?20000:45000);
 const reassignmentMs=performance.now()-requeueAt;
 if(capacity===1){
  assert(observers.every(c=>c.allocation.endpoint===allocation.endpoint),'single-capacity recovery must reuse the original physical port');
  assert(reassignmentMs<20000,'a reaped terminal child retained its port');
  assert(Date.now()-Number(allocationStatus(allocation.allocation_id).heartbeat_ms)<35000,'old heartbeat hold could explain port reuse');
 }
 if(original.preference==='humans_only')assert.equal(new Set(observers.map(c=>c.allocation.allocation_id)).size,1,'recovered Ranked roster split across workers');
 report.recovery={status:'PASS',reassignment_ms:reassignmentMs,same_port_reused:observers.every(c=>c.allocation.endpoint===allocation.endpoint),queue:original.preference,participants:originals.length,crashed_after_running_seconds:playingFor,early_before_first_checkpoint:early,result_id:before.result_id,server_epoch:before.server_epoch,outcome:saved.outcome,all_profiles_checked:true,all_histories_checked:true,no_elo_or_xp:true,elapsed_seconds:(performance.now()-crashedAt)/1000};
 report.scenarios.push('full_roster_crash_recovers_played_history_without_elo_or_xp');
}
async function physicalDatabaseRestore(original) {
 const allocation=original.allocation,epoch=original.epoch,worker=ownedWorkerPid(original.port);
 const nonce=crypto.randomBytes(16).toString('hex');
 const status=allocationStatus(allocation.allocation_id);
 assert(status?.phase==='running'&&status.result_id,'physical backup requires an acknowledged running match');
 fs.writeFileSync(path.join(databaseRestoreControl,'request.json'),wire({nonce,allocation_id:allocation.allocation_id,result_id:status.result_id,server_epoch:epoch}),{flag:'wx'});
 let receipt;
 await pumpUntil('isolated physical database replacement',()=>{
  const file=path.join(databaseRestoreControl,'response.json');if(!fs.existsSync(file))return false;
  assert(fs.statSync(file).size<=4096,'oversized physical restore receipt');
  receipt=JSON.parse(fs.readFileSync(file,'utf8'));
  assert.equal(receipt.nonce,nonce);assert.equal(receipt.status,'PASS');
  assert.equal(receipt.physical_restored,true);assert.equal(receipt.system_identifier_matched,true);
  assert.equal(receipt.career_schema,13);return true;
 },180000);
 const restoredAt=Date.now();
 assert.equal(ownedWorkerPid(original.port),worker,'database replacement changed the live worker');
 const lastTick=original.snapshot.snapshot_tick;
 original.close();
 const resumed=new Client(original.index,original.port,original,{autoQueue:false});
 resumed.allocation=allocation;all.push(resumed);
 await pumpUntil('same authenticated human resumes after physical restoration',()=>resumed.stage==='running'&&resumed.snapshot.snapshot_tick>lastTick,30000);
 assert.equal(resumed.epoch,epoch);assert.equal(resumed.profile.profile_id,original.profile.profile_id);
 assert.equal(ownedWorkerPid(resumed.port),worker);
 const readHealth=p=>{try{return parse(fs.readFileSync(p));}catch{return null;}};
 let postRestoreHealth;
 await pumpUntil('restored database authenticated worker health',()=>{
  const h=readHealth(path.join(root,'matches',allocation.allocation_id,'outbox','health','status.json'));
  if(h?.version===2&&h.sql_ready===true&&h.journal_ready===true&&h.server_epoch===epoch&&h.critical_replies===0&&h.reply_receiver_closed===false&&h.probe_checked_ms>=restoredAt&&h.updated_ms>=restoredAt){postRestoreHealth=h;return true;}return false;
 },30000);
 report.database_restore={status:'PASS',result_id:status.result_id,allocation_id:allocation.allocation_id,worker_pid:worker,server_epoch:epoch,same_worker:true,same_profile:true,resumed_running:true,authenticated_sql_ready:true,restore_observed_ms:restoredAt,probe_checked_ms:postRestoreHealth.probe_checked_ms,health_updated_ms:postRestoreHealth.updated_ms,receipt};
 report.scenarios.push('physical_database_restore_preserves_live_worker_and_authenticated_resume');
 return resumed;
}
async function restartAndRecover(original, crashWorker) {
 const allocation=original.allocation, baseline=ids(original.snapshot), progress=careerProgress(original), workerEpoch=original.epoch;
 const workerPid=ownedWorkerPid(original.port);
 const checks=report.recovery={checks:[],allocation_id:allocation.allocation_id,worker_pid:workerPid};
 console.log(wire({phase:'recovery',action:'restart_lobby_only',pid:lobby.pid}));
 const previousLobby=lobby;
 previousLobby.kill('SIGTERM'); // Never signal the group here: its worker must survive.
 const exited=performance.now()+5000;
 while(previousLobby.exitCode===null&&previousLobby.signalCode===null&&performance.now()<exited){
  for(const c of all)c.tick(performance.now());await delay(20);
 }
 assert(previousLobby.exitCode!==null||previousLobby.signalCode!==null,'old lobby did not exit');
 lobby=launchLobby();
 await pumpFor(700);
 const observer=new Client(2001,lobbyPort,original,{autoQueue:false,autoJoin:false,autoHandoff:false});
 observer.queueId=original.queueId+1;all.push(observer);
 await pumpUntil('new lobby account authentication',()=>!!observer.auth);
 let nextFind=0;
 await pumpUntil('new lobby adopts existing allocation',()=>observer.allocation?.allocation_id===allocation.allocation_id,20000,now=>{
  if(now>=nextFind){nextFind=now+1500;observer.account({action:'find_match',request_id:observer.queueId,preference:'bot_practice'});}
 });
 assert.equal(observer.allocation.endpoint,allocation.endpoint);
 assert.equal(ownedWorkerPid(original.port),workerPid,'lobby restart duplicated or replaced worker');
 assert(performance.now()-original.lastSnapshot<2000,'surviving worker stopped replication');
 assert.equal(original.epoch,workerEpoch,'lobby restart changed the worker epoch');
 assertRoster(original,baseline);
 assert.equal(allocationManifests().filter(m=>m.humans.some(h=>h.profile_id===original.profile.profile_id)).length,1);
 checks.checks.push({name:'lobby_restart_adopts_live_worker',status:'PASS',old_lobby_pid:previousLobby.pid,new_lobby_pid:lobby.pid,worker_pid:workerPid,server_epoch:workerEpoch,endpoint:allocation.endpoint,roster:baseline});
 report.scenarios.push('lobby_restart_preserves_worker_and_assignment');
 if(!crashWorker)return;
 const before=allocationStatus(allocation.allocation_id);
 assert(before?.phase==='running'&&before.result_id, 'worker needs durable started result before crash');
 assert.equal(before.server_epoch,original.epoch);
 const outbox=path.join(root,'matches',allocation.allocation_id,'outbox');
 const outboxFiles=fs.existsSync(outbox)?fs.readdirSync(outbox).filter(name=>name.endsWith('.json')):[];
 checks.before_crash={result_id:before.result_id,server_epoch:before.server_epoch,status:before.phase,outbox_json_count:outboxFiles.length};
 assert.equal(ownedWorkerPid(original.port),workerPid);
 console.log(wire({phase:'recovery',action:'crash_owned_worker',pid:workerPid,result_id:before.result_id}));
 process.kill(workerPid,'SIGKILL');original.close();
 const crashedAt=performance.now();let nextQuery=0,nextProgress=0,query=400,probeQueueAt=crashedAt+750;let sawRecovering=false,saved=null;
 // The restarted lobby observes adopted-worker death through the real stale
 // heartbeat (100s); its recovery child then waits the real recovery gate (120s).
 await pumpUntil('worker crash durable recovery',()=>{
  const status=allocationStatus(allocation.allocation_id);
  if(status?.phase==='recovering')sawRecovering=true;
  if(observer.lastDetail?.result_id===before.result_id)saved=observer.lastDetail;
  return saved&&status?.phase==='failed';
 },300000,now=>{
  const status=allocationStatus(allocation.allocation_id);
  if(status)assert.equal(status.server_epoch,before.server_epoch,'recovery lost original server epoch');
  if(status?.phase!=='failed')assert(!observer.allocationsSeen.some(a=>a.allocation_id!==allocation.allocation_id),'profile was reassigned before durable recovery');
  if(now>=probeQueueAt&&status?.phase!=='failed'){probeQueueAt=now+7000;observer.account({action:'find_match',request_id:observer.queueId,preference:'bot_practice'});}
  if(now>=nextQuery){nextQuery=now+2000;observer.account({action:'detail',request_id:++query,result_id:before.result_id});}
  if(now>=nextProgress){nextProgress=now+10000;resourceSample();console.log(wire({phase:'recovery',elapsed_seconds:Math.round((now-crashedAt)/1000),worker_phase:status?.phase,saved_result:!!saved}));}
 });
 assert(sawRecovering,'did not observe recovery child');
 assert.equal(saved.outcome,'interrupted');assert.equal(saved.saved,true);assert.equal(saved.rated,false);
 assert(saved.participants.every(p=>p.progression_xp_gained===0&&p.rating==null),'interrupted recovery awarded XP or Elo');
 assert.equal(saved.server_epoch,before.server_epoch);
 assert(performance.now()-crashedAt>=120000,'recovery bypassed its real time gate');
 nextQuery=0;
 const profileRequest=++query;
 await pumpUntil('persisted profile after crash',()=>{
  const response=observer.profileResponses.get(String(profileRequest));
  return response?.profile_id===progress.profile_id
   && response.matches_played===progress.matches_played+1
   && observer.profile?.matches_played===progress.matches_played+1;
 },10000,now=>{
  if(now>=nextQuery){nextQuery=now+1000;observer.account({action:'profile',request_id:profileRequest,profile_id:progress.profile_id});}
 });
 // The authenticated player's Profile action returns profile; visited_profile
 // is reserved for another player. Correlate the fresh SQL response explicitly.
 for(const key of ['rating','rated_matches','progression_xp','wins','losses'])assert.equal(observer.profile[key],progress[key],`interruption changed ${key}`);
 assert.equal(observer.profile.matches_played,progress.matches_played+1);
 nextQuery=0;
 await pumpUntil('interrupted game in history',()=>observer.lastHistory?.some(game=>game.result_id===before.result_id),10000,now=>{
  if(now>=nextQuery){nextQuery=now+1000;observer.account({action:'history',request_id:++query,before:null});}
 });
 checks.checks.push({name:'worker_crash_recovers_interrupted_history',status:'PASS',elapsed_seconds:(performance.now()-crashedAt)/1000,result_id:before.result_id,server_epoch:before.server_epoch,outcome:saved.outcome,profile_request_id:profileRequest,profile:observer.profile});
 report.scenarios.push('worker_crash_recovers_original_epoch_without_xp_or_elo');
 await pumpFor(150);
 observer.queueId++;nextFind=0;
 await pumpUntil('new assignment after durable cleanup',()=>observer.allocation&&observer.allocation.allocation_id!==allocation.allocation_id,20000,now=>{
  if(now>=nextFind){nextFind=now+1500;observer.account({action:'find_match',request_id:observer.queueId,preference:'bot_practice'});}
 });
 const replacement=observer.allocation;
 assert.notEqual(replacement.allocation_id,allocation.allocation_id);
 checks.checks.push({name:'reassignment_after_durable_cleanup',status:'PASS',allocation_id:replacement.allocation_id,endpoint:replacement.endpoint});
 report.scenarios.push('fresh_assignment_only_after_durable_recovery');
}
try {
 await delay(800);
 assert(lobby.exitCode===null, 'lobby failed to start');
 // A raw pre-admission ping must not receive any response or allocate an endpoint.
 const stranger=dgram.createSocket('udp4');let reflected=0;stranger.on('message',b=>reflected+=b.length);stranger.send(Buffer.from('{"type":"ping"}'),lobbyPort,'127.0.0.1');await delay(700);stranger.close();assert.equal(reflected,0);report.scenarios.push('raw_ping_no_replication');
 if(scenario==='unique-draft'){await uniqueDraftLifecycle();await casualHandoffReleaseLifecycle();}
 else if(scenario==='handoff-release')await casualHandoffReleaseLifecycle();
 else if(scenario==='party-draft')await partyDraftLifecycle();
 else {
 if(queueOnly){
  const holder=new Client(clientsCount+50,lobbyPort,null,{preference:'bot_practice',keepActive:true});all.push(holder);
  await pumpUntil('normal signed capacity-holder publishes Running and delivers gameplay',()=>Boolean(holder.firstRunning)&&allocationStatus(holder.allocation?.allocation_id)?.phase==='running',30000);
  assert.equal(allocationStatus(holder.allocation.allocation_id)?.phase,'running');
  all.pop();support.push(holder);
  report.queue_capacity_holder={disclosed_fixture:true,not_counted_in_target_clients:true,profile_id:holder.profile.profile_id,allocation_id:holder.allocation.allocation_id,endpoint:holder.allocation.endpoint,gameplay_rules:'ordinary standard map; signed bounded activity'};
 }
 const start=performance.now();let runningAt=null,nextProgress=0;
 while(performance.now()-start<240000) {
  const now=performance.now();
  if(all.length<clientsCount && now-start>all.length*50)all.push(new Client(all.length,lobbyPort,null,queueOnly?{autoHandoff:false}:{}));
  for(const c of [...all,...support])c.tick(now);
  if(queueOnly)assert(support.every(c=>allocationStatus(c.allocation.allocation_id)?.phase==='running'),'capacity-holder stopped Running during queue-only observation');
  if(now>=nextProgress){nextProgress=now+5000;resourceSample();const states={};for(const c of all){const s=queueOnly?(c.career?.match_service?.state||c.stage):c.stage;states[s]=(states[s]||0)+1;}console.log(wire({elapsed:Math.round((now-start)/1000),states}));fs.writeFileSync(path.join(root,'clients.json'),wire(all.map(c=>c.diagnostic())));}
  // Everyone is where this run wants them (in the queue, or Running): soak, then stop.
  if(all.length===clientsCount && all.every(queueOnly?queued:(c=>c.stage==='running'))){if(!runningAt){runningAt=now;resourceSample();}if(now-runningAt>=soakSecs*1000)break;}
  await delay(20);
 }
 resourceSample();
 if(queueOnly){
  assert(all.length===clientsCount && all.every(queued),'not all clients reached the queue; inspect clients.json');
  for(const c of all){assert.equal(c.errors.length,0);assert.equal(c.serverFull||0,0,'queued target received server_full');assert.equal(c.career.error??null,null,'queued target has a final account error');assert.equal(c.port,lobbyPort,'queued target handed off to a worker');assert.equal(c.worldSnapshots,0,'queued target received gameplay world data');assert.equal(c.career.match_service.state,'waiting','capacity-holder failed to keep target players waiting');assert.equal(c.career.match_service_request_id,c.queueId,'queue view belongs to a different search');}
  assert.equal(support.length,1);
  assert(support.every(c=>c.errors.length===0&&c.stage==='running'&&performance.now()-c.lastSnapshot<2000),'capacity-holder stopped delivering healthy gameplay');
  const careerAges=all.map(c=>performance.now()-c.lastCareer);
  assert(Math.max(...careerAges)<2000,'queued career views became stale');
  const careerGaps=all.flatMap(c=>c.careerGaps).sort((a,b)=>a-b);
  report.queue_view_freshness={max_age_ms:Math.max(...careerAges),gap_p95_ms:percentile(careerGaps,.95),gap_p99_ms:percentile(careerGaps,.99)};
  if(verifyPresence){
   const profiles=all.map(c=>c.profile.profile_id);
   assert.equal(new Set(profiles).size,clientsCount,'presence proof requires distinct target profiles');
   assert(profiles.every(id=>/^[a-f0-9]{64}$/.test(id)),'invalid synthetic profile ID');
   const query=`SELECT json_build_object('entries',count(*),'fresh',count(*) FILTER (WHERE expires_at>clock_timestamp()),'owners',count(DISTINCT owner_id),'min_remaining_secs',min(extract(epoch from expires_at-clock_timestamp()))) FROM career_presence WHERE profile_id=ANY(ARRAY[${profiles.map(id=>"'"+id+"'").join(',')}]::text[]);`;
   const checked=spawnSync('psql',[process.env.OMOBA_TEST_DATABASE_URL,'--no-psqlrc','-v','ON_ERROR_STOP=1','--tuples-only','--no-align'],{input:query,encoding:'utf8',timeout:10000,maxBuffer:1048576});
   assert.equal(checked.status,0,'disposable presence query failed; ensure psql is available');
   const presence=JSON.parse(checked.stdout.trim());
   assert.equal(presence.entries,clientsCount,'missing target presence');assert.equal(presence.fresh,clientsCount,'expired target presence');assert.equal(presence.owners,1,'queued targets must share the lobby owner');
   report.queue_presence={...presence,status:'PASS',observed_at:new Date().toISOString(),scope:'read-only query of target profiles in isolated loopback DB'};
  }
  report.scenarios.push('one_disclosed_normal_capacity_holder_and_no_target_worker_handoff');
  const states={};for(const c of all){const s=c.career.match_service.state;states[s]=(states[s]||0)+1;}
  const signIn=all.map(c=>(c.queuedAt-c.createdAt)/1000).sort((a,b)=>a-b);
  const soak=resources.filter(r=>r.wall_ms>=runningAt&&r.cpu_percent_interval!==undefined);
  report.queue={states,server_full_replies:all.reduce((n,c)=>n+(c.serverFull||0),0),
   sign_in_to_queue_secs:{p50:percentile(signIn,.5),p95:percentile(signIn,.95),max:signIn.at(-1)},
   resource_scope:'lobby_and_one_disclosed_capacity_holder_worker; client_generator_excluded',
   server_group_cpu_percent_mean:soak.length?soak.reduce((s,r)=>s+r.cpu_percent_interval,0)/soak.length:null,
   server_group_rss_mib_peak:Math.max(...resources.map(r=>r.rss_mib)),
   datagrams_received:all.reduce((n,c)=>n+c.packets,0),datagrams_sent:all.reduce((n,c)=>n+c.sentPackets,0)};
  report.scenarios.push('sign_in_and_queue_only');
  console.log(`QUEUE_RESULT clients=${clientsCount} capacity=${capacity} states=${JSON.stringify(states)} server_full=${report.queue.server_full_replies} sign_in_p95_s=${report.queue.sign_in_to_queue_secs.p95?.toFixed(2)} sign_in_max_s=${report.queue.sign_in_to_queue_secs.max?.toFixed(2)} cpu_mean=${report.queue.server_group_cpu_percent_mean?.toFixed(1)} rss_peak_mib=${report.queue.server_group_rss_mib_peak.toFixed(1)}`);
 } else {
 assert(all.length===clientsCount && all.every(c=>c.stage==='running'),'not all clients reached Running; inspect clients.json');
 const allocations=new Map();for(const c of all){const id=c.allocation.allocation_id;if(!allocations.has(id))allocations.set(id,[]);allocations.get(id).push(c);assert.equal(ids(c.snapshot).length,10);assert.equal(c.errors.length,0);assert(performance.now()-c.lastSnapshot<2000,'active match stopped delivering snapshots');}
 assert.equal(new Set([...allocations.values()].map(cs=>cs[0].port)).size,allocations.size,'distinct arenas require distinct ports');
 for(const cs of allocations.values()){const roster=new Set(ids(cs[0].snapshot));for(const c of cs)assert(roster.has(String(c.snapshot.your_id)));}
 report.scenarios.push('allocated_draft_countdown_loading_running','distinct_workers_and_ten_player_rosters');
 report.arenas=allocations.size;report.total_bytes=all.reduce((n,c)=>n+c.bytes,0);report.total_datagrams=all.reduce((n,c)=>n+c.packets,0);report.elapsed_seconds=(performance.now()-start)/1000;
 const gaps=all.flatMap(c=>c.gaps).sort((a,b)=>a-b);report.snapshot_gap_ms={p50:gaps[Math.floor(gaps.length*.5)],p95:gaps[Math.floor(gaps.length*.95)],p99:gaps[Math.floor(gaps.length*.99)],max:gaps.at(-1)};
 measure(allocations, runningAt);
 }
 }
 if(['remake','surrender'].includes(scenario))await teamVoteJourney(all.filter(c=>c.stage==='running'),scenario);
 if(scenario==='termination')await gracefulTermination(all[0]);
 if(scenario==='ready-check')await readyCheckLifecycle();
 if(scenario==='cross-lobby-ready')await crossLobbyReadyLifecycle();
 if(scenario==='legacy-penalty')await legacyPenaltyLifecycle();
 if(scenario==='lifecycle')await lifecycle(all[0]);
 if(scenario==='abandoned')await abandoned(all[0]);
 if(scenario==='capacity-wait')await capacityWait();
 if(scenario==='drain')await maintenanceDrain(all[0]);
 if(scenario==='early-recovery'||(scenario==='recovery'&&preference!=='bot_practice'))await recoverRoster(all.filter(c=>c.stage==='running'),scenario==='early-recovery');
 else if(scenario==='lobby-restart'||scenario==='recovery')await restartAndRecover(databaseRestoreControl?await physicalDatabaseRestore(all[0]):all[0],scenario==='recovery');
 report.total_transforms_sent=all.reduce((n,c)=>n+c.transforms,0);
 report.total_sent_datagrams=all.reduce((n,c)=>n+c.sentPackets,0);
 report.total_sent_bytes=all.reduce((n,c)=>n+c.sentBytes,0);
 if(inputHz>0&&soakSecs>0&&!queueOnly)assert(all.filter(c=>c.firstRunning).every(c=>c.transforms>0),'running clients did not exercise signed input');
 if(exerciseStaleHandoff){
  const handed=all.filter(c=>c.allocation);
  assert(handed.length>0&&handed.every(c=>c.staleHandoffChecks>0&&!c.staleHandoffFailure),'stale handoff fixture did not execute for every handed client');
  report.stale_handoff_checks=handed.reduce((n,c)=>n+c.staleHandoffChecks,0);
  assert(handed.every(c=>c.staleEndpointChecks>0),'stale endpoint fixture did not execute for every handed client');
  report.stale_endpoint_checks=handed.reduce((n,c)=>n+c.staleEndpointChecks,0);
 }
 report.resources=resources;
 report.status='PASS';
} catch(error){
 report.status='FAIL';report.error=error.stack;process.exitCode=1;
 // CI may have no artifact upload: keep bounded admission/draft evidence in
 // its log, without device keys, signatures or transport/session nonces.
 report.failed_clients=all.filter(c=>c.stage!=='running'||c.staleHandoffFailure).slice(0,25).map(c=>c.diagnostic());
 try {report.worker_statuses=allocationManifests().slice(0,25).map(m=>{
  const s=allocationStatus(m.allocation_id);return {allocation_id:m.allocation_id,preference:m.preference,phase:s?.phase,result_id:s?.result_id,joined:s?.joined_sessions?.length,released:s?.released_sessions?.length};
 });}catch{report.worker_statuses=[];}
} finally {
 report.resources=resources;
 fs.writeFileSync(path.join(root,'report.json'),wire(report));fs.writeFileSync(path.join(root,'clients.json'),wire(all.map(c=>c.diagnostic())));console.log(wire(report));cleanup();
}
