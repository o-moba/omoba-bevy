#!/usr/bin/env node
// Live public-protocol integration/load probe. Uses Node built-ins only.
// Run against a disposable local database, never production.
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import dgram from 'node:dgram';
import fs from 'node:fs';
import path from 'node:path';
import {spawn, spawnSync} from 'node:child_process';
import {performance} from 'node:perf_hooks';
const opts = Object.fromEntries(process.argv.slice(2).map(s => s.replace(/^--/, '').split('=')));
const clientsCount = Number(opts.clients || 1);
const preference = opts.preference || 'bot_practice';
const scenario = opts.scenario || 'load';
assert(['load','lifecycle','lobby-restart','recovery'].includes(scenario), 'unknown scenario');
if(scenario!=='load') assert(clientsCount===1 && preference==='bot_practice', `${scenario} uses --clients=1 --preference=bot_practice`);
const soakSecs = Number(opts.seconds || 15);
const inputHz = Number(opts['input-hz'] ?? 20);
assert(Number.isFinite(inputHz)&&inputHz>=0&&inputHz<=60, 'input-hz must be between 0 and 60');
const lobbyPort = Number(opts.port || 45500);
const workerPort = Number(opts['worker-port'] || 45600);
const root = path.resolve(opts.output || `.agent/tasks/TASK-PUBLIC-MVP-2026-09-22/raw/live-${Date.now()}`);
fs.mkdirSync(root, {recursive:true});
assert(process.env.OMOBA_TEST_DATABASE_URL, 'Use a disposable OMOBA_TEST_DATABASE_URL');
assert(process.env.HARNESS_SERVER_BIN, 'Build server and set HARNESS_SERVER_BIN');
assert(clientsCount >= 1 && clientsCount <= 100);
const wire = obj => JSON.stringify(obj, (_,v) => typeof v === 'bigint' ? `@u64:${v}` : v).replace(/"@u64:(\d+)"/g, '$1');
const parse = bytes => JSON.parse(bytes.toString().replace(/([:\[,])(\d{16,})(?=[,}\]])/g, '$1"@u64:$2"'), (_,v) => typeof v==='string' && v.startsWith('@u64:') ? BigInt(v.slice(5)) : v);
const sign = (key, data) => crypto.sign(null, Buffer.from(wire(data)), key).toString('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const logfile = fs.openSync(path.join(root,'lobby.log'),'a');
const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith('OMOBA_') && !k.startsWith('GAME_') && k !== 'SERVER_ADDR'));
Object.assign(env, {SERVER_ADDR:`127.0.0.1:${lobbyPort}`, OMOBA_SERVER_ROLE:'lobby', OMOBA_DATABASE_URL:process.env.OMOBA_TEST_DATABASE_URL,
 OMOBA_MATCH_ROOT:path.join(root,'matches'), OMOBA_CAREER_OUTBOX:path.join(root,'lobby-outbox'),
 OMOBA_MATCH_CAPACITY:String(Math.max(16,clientsCount)), OMOBA_MATCH_FIRST_PORT:String(workerPort),
 OMOBA_MATCH_PUBLIC_HOST:'127.0.0.1', OMOBA_MATCH_BIND_IP:'127.0.0.1'});
const ownedGroups=new Set();
function launchLobby(){const child=spawn(process.env.HARNESS_SERVER_BIN, [], {env, detached:true, stdio:['ignore',logfile,logfile]});if(child.pid)ownedGroups.add(child.pid);return child;}
let lobby=launchLobby();
const all=[]; const report={scenario,clients:clientsCount,preference,input_hz_requested:inputHz,input_model:'signed_current_pose_transform',soak_seconds:soakSecs,started_at:new Date().toISOString(),scenarios:[],status:'RUNNING'};
const resources=[];
let stopping=false;
function resourceSample(){
 const lines=spawnSync('ps',['-axo','pid=,pgid=,%cpu=,rss='],{encoding:'utf8'}).stdout || '';
 const owned=lines.trim().split('\n').map(l=>l.trim().split(/\s+/).map(Number)).filter(r=>ownedGroups.has(r[1]));
 resources.push({at:new Date().toISOString(),processes:owned.length,cpu_percent:owned.reduce((s,r)=>s+r[2],0),rss_mib:owned.reduce((s,r)=>s+r[3],0)/1024});
}
function cleanup() {if(stopping)return;stopping=true;for(const c of all)c.close();for(const group of ownedGroups){try{process.kill(-group,'SIGTERM')}catch{}}fs.closeSync(logfile);}
process.on('SIGINT',()=>{cleanup();process.exit(130)});
process.on('SIGTERM',()=>{cleanup();process.exit(143)});
class Client {
 constructor(index, target=lobbyPort, identity=null, mode={}) {
  this.index=index;this.port=target;this.autoHandoff=mode.autoHandoff!==false;this.autoQueue=mode.autoQueue!==false;this.autoJoin=mode.autoJoin!==false;this.preference=mode.preference || preference;this.active=true;this.worldSnapshots=0;this.bootstrapSnapshots=0;this.joinAttempts=0;this.allocationsSeen=[];this.profileResponses=new Map();this.key=identity?.key || crypto.generateKeyPairSync('ed25519').privateKey;
  this.publicKey=crypto.createPublicKey(this.key).export({type:'spki',format:'der'}).subarray(-32).toString('hex');
  this.session=identity?.session || `mvp-${Date.now()}-${index}`;this.nickname=identity?.nickname || `Probe-${index}`;
  this.packets=0;this.bytes=0;this.sentPackets=0;this.sentBytes=0;this.transforms=0;this.lastInput=0;this.firstTransform=null;this.lastTransform=null;this.snapshots=0;this.gaps=[];this.frames=new Map();this.errors=[];this.stage='probing';this.next=0;this.queueId=1;
  this.bind();
 }
 bind() {this.socket=dgram.createSocket('udp4');this.socket.on('message',b=>this.receive(b));this.socket.on('error',e=>this.errors.push(e.message));this.socket.bind(0,'127.0.0.1');this.nonce=crypto.randomBytes(16).toString('hex');this.epoch=0n;this.match=0;this.path=null;this.auth=null;this.authChallenge=null;this.lastProbe=0;this.accountSequence=0;this.sequence=0;this.prematchSequence=0;this.lastSend=0;this.lastQueue=0;this.next=0;}
 send(value) {if(!stopping){const bytes=Buffer.from(wire(value));this.sentPackets++;this.sentBytes+=bytes.length;this.socket.send(bytes,this.port,'127.0.0.1');}}
 bootstrap(payload) {this.send({type:'transport_bootstrap',path_nonce:this.path,payload:wire(payload)});}
 account(action) {if(!this.auth)return;const sequence=++this.accountSequence;this.bootstrap({type:'career',request:{action:'authorized',session_nonce:this.auth,sequence,request:action,signature:sign(this.key,['omoba.career.request.v1',this.epoch,this.auth,sequence,action])}});}
 gameplay(payload) {if(!this.auth)return;const command={server_epoch:this.epoch,match_id:this.match,session_id:this.session,session_nonce:this.auth,path_nonce:this.path,sequence:++this.sequence,payload:wire(payload),signature:''};command.signature=sign(this.key,['omoba.gameplay.command.v1',command.server_epoch,command.match_id,command.session_id,command.session_nonce,command.path_nonce,command.sequence,command.payload]);this.send({type:'signed_command',command});}
 receive(bytes) {
  this.packets++;this.bytes+=bytes.length;
  if(bytes.subarray(0,4).toString()==='OMB1') {
   const id=`${bytes.readBigUInt64LE(6)}:${bytes.readBigUInt64LE(14)}`;const index=bytes.readUInt16LE(22),count=bytes.readUInt16LE(24);
   if(!this.frames.has(id))this.frames.set(id,{parts:new Array(count),received:0});
   const f=this.frames.get(id);if(!f.parts[index]){f.parts[index]=bytes.subarray(30);f.received++;}
   if(this.frames.size>8)this.frames.delete(this.frames.keys().next().value);
   if(f.received<count)return;this.frames.delete(id);bytes=Buffer.concat(f.parts);
  }
  let packet;try{packet=parse(bytes)}catch(e){this.errors.push('decode:'+e.message);return;}
  if(packet.type==='transport_challenge') {if(packet.client_nonce!==this.nonce)return;this.path=packet.path_nonce;this.epoch=packet.server_epoch;this.send({type:'transport_proof',server_epoch:this.epoch,path_nonce:this.path});if(!this.auth)this.stage='authenticating';return;}
  if(packet.type==='career') {
   this.epoch=packet.server_epoch;const v=packet.career;this.career=v;if(v.response_id!=null&&v.profile){this.profileResponses.set(String(v.response_id),v.profile);if(this.profileResponses.size>16)this.profileResponses.delete(this.profileResponses.keys().next().value);}if(v.history_loaded)this.lastHistory=v.history;if(v.detail)this.lastDetail=v.detail;if(v.visited_profile)this.lastVisited=v.visited_profile;
   if(v.challenge && !v.auth_nonce && !this.auth && this.authChallenge?.nonce!==v.challenge.nonce) {this.authChallenge=v.challenge;this.next=0;}
   if(v.auth_nonce && v.profile){this.auth=v.auth_nonce;this.profile=v.profile;if(this.stage==='authenticating')this.stage=this.port===lobbyPort?(this.autoQueue?'queued':'authenticated'):(this.autoJoin?'joining':'authenticated');}
   if(this.port===lobbyPort && v.match_service?.state==='assigned' && v.match_service_request_id===this.queueId) {
    this.allocation=v.match_service.allocation;this.allocationsSeen.push(this.allocation);if(!this.autoHandoff)return;this.snapshot=null;this.socket.close();this.port=Number(this.allocation.endpoint.split(':').at(-1));this.stage='probing';this.bind();
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
  if(!this.active)return;
  // Retry admission until authentication completes, just like the native Hello
  // loop. A lost proof must not strand a synthetic client forever. Authenticate
  // retries are timer-driven, never echoed for every repeated server view.
  if(!this.auth&&now-this.lastProbe>=1000){this.lastProbe=now;this.send({type:'transport_probe',protocol_version:2,client_nonce:this.nonce,padding:'0'.repeat(384)});}
  if(!this.path)return;
  if(!this.auth){if(now>=this.next){this.next=now+2000;const c=this.authChallenge;this.bootstrap({type:'career',request:c?{action:'authenticate',challenge:c,signature:sign(this.key,['omoba.career.auth.v1',c.public_key,c.nonce,c.server_epoch,c.session_id,c.nickname])}:{action:'challenge',public_key:this.publicKey,nickname:this.nickname,session_id:this.session}});}return;}
  // Representative signed movement-command verification/dispatch load. Current
  // authoritative pose avoids inventing movement or bypassing collision rules.
  if(inputHz>0&&this.stage==='running'&&this.port!==lobbyPort&&now-this.lastInput>=1000/inputHz){
   const me=this.snapshot?.players?.find(p=>p.id===this.snapshot.your_id&&!p.is_bot);
   if(me){
    this.lastInput=this.lastInput?this.lastInput+1000/inputHz:now;
    if(now-this.lastInput>1000/inputHz)this.lastInput=now;
    this.gameplay({type:'transform',x:me.x,y:me.y,z:me.z,yaw:me.yaw,dash_sequence:me.utility?.dash_sequence??0});
    this.transforms++;this.firstTransform??=now;this.lastTransform=now;
   }
  }
  if(now-this.lastSend<300)return;this.lastSend=now;this.gameplay({type:'ping'});
  if(this.port===lobbyPort){if(this.autoQueue&&now-this.lastQueue>2000){this.lastQueue=now;this.account({action:'find_match',request_id:this.queueId,preference:this.preference});}return;}
  if(!this.autoJoin)return;
  const s=this.snapshot;
  if(!s?.players?.some(p=>p.id===s.your_id)) {this.join();return;}
  if(s.prematch){const me=s.prematch.players.find(p=>p.player_id===s.your_id);const p=s.prematch;
   if(p.phase==='draft'&&!me?.locked)this.gameplay({type:'prematch',request:{server_epoch:this.epoch,match_id:this.match,generation:p.generation,request_id:++this.prematchSequence,action:{kind:'lock',locked:true}}});
   if(p.phase==='loading'&&!me?.loaded)this.gameplay({type:'prematch',request:{server_epoch:this.epoch,match_id:this.match,generation:p.generation,request_id:++this.prematchSequence,action:{kind:'loaded'}}});
  }
 }
 join(){this.joinAttempts++;this.gameplay({type:'join',prematch:true,team:'green',character:'ipfs',hero_class:['warrior','mage','ranger','cleric'][this.index%4],avatar:null,sprite_character:null,session_id:this.session,passport_ticket:null});}
 close(){this.active=false;try{this.socket.close()}catch{}}
 diagnostic(){return{index:this.index,stage:this.stage,active:this.active,port:this.port,world_snapshots:this.worldSnapshots,bootstrap_snapshots:this.bootstrapSnapshots,join_attempts:this.joinAttempts,profile:this.profile?.profile_id,allocation:this.allocation,join_error:this.lastJoinError,career_error:this.career?.error,match_service:this.career?.match_service,game_state:this.snapshot?.game_state,prematch:this.snapshot?.prematch,packets:this.packets,bytes:this.bytes,sent_packets:this.sentPackets,sent_bytes:this.sentBytes,transforms_sent:this.transforms,input_hz_observed:this.transforms>1?(this.transforms-1)*1000/(this.lastTransform-this.firstTransform):0,snapshots:this.snapshots,errors:this.errors.slice(-5)};}
}
// Keep every owned client alive while awaiting an observable protocol condition.
async function pumpUntil(label, predicate, timeout=20000, onTick=()=>{}) {
 const started=performance.now();let nextEvidence=0;
 while(performance.now()-started<timeout) {
  assert(lobby.exitCode===null, `${label}: lobby exited`);
  const now=performance.now();for(const c of all)c.tick(now);onTick(now);
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
const ids = snapshot => snapshot.players.map(p=>String(p.id)).sort();
const careerProgress = client => Object.fromEntries(['profile_id','rating','rated_matches','matches_played','wins','losses','progression_xp'].map(k=>[k,client.profile[k]]));
function assertRoster(client, expected) {
 assert.equal(client.snapshot.players.length,10,'running match must retain ten participants');
 assert.deepEqual(ids(client.snapshot),expected,'a fresh identity entered or replaced an allocated participant');
 assert.equal(new Set(ids(client.snapshot)).size,10,'duplicate player identity');
}
async function lifecycle(original) {
 console.log(wire({phase:'lifecycle',action:'humans_only_cancel'}));
 const baseline=ids(original.snapshot);
 const allocation=original.allocation;
 const checks=report.lifecycle={checks:[],allocation_id:allocation.allocation_id,endpoint:allocation.endpoint};
 const waiting=new Client(1001,lobbyPort,null,{autoQueue:false,autoJoin:false});all.push(waiting);
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
 await pumpUntil('abandoned queue expires despite authenticated heartbeat',()=>waiting.career?.match_service?.state==='idle',22000);
 assert.equal(waiting.allocationsSeen.length,0);
 checks.checks.push({name:'lost_cancel_expires_with_home_heartbeat',status:'PASS',elapsed_ms:performance.now()-abandonedAt,allocations:0});
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
 assert.equal(resumed.port,oldPort,'reconnect allocated a second worker');
 assert.equal(resumed.allocation.allocation_id,allocation.allocation_id,'reconnect duplicated active allocation');
 assert.equal(resumed.snapshot.your_id,oldId,'reconnect replaced the player identity');
 assert.deepEqual(careerProgress(resumed),oldProgress,'career progress changed during reconnect');
 assertRoster(resumed,baseline);
 const after=resumed.snapshot.players.find(p=>p.id===oldId);
 assert(after.level>=before.level,'reconnect reset in-match level');
 if(after.level===before.level)assert(after.xp>=before.xp,'reconnect reset in-match experience');
 assert(after.gold>=before.gold,'reconnect reset earned gold');
 assert.deepEqual(after.inventory,before.inventory,'reconnect lost inventory');
 assert.equal(resumed.snapshot.players.filter(p=>!p.is_bot).length,1,'reconnect created a duplicate human');
 await pumpFor(1000,()=>assertRoster(resumed,baseline));
 const manifests=fs.readdirSync(path.join(root,'matches'),{withFileTypes:true})
  .filter(entry=>entry.isDirectory()&&fs.existsSync(path.join(root,'matches',entry.name,'manifest.json')))
  .map(entry=>JSON.parse(fs.readFileSync(path.join(root,'matches',entry.name,'manifest.json'),'utf8')));
 assert.equal(manifests.filter(m=>m.humans.some(h=>h.profile_id===resumed.profile.profile_id)).length,1,'profile has duplicate worker manifests');
 assert.equal(manifests.filter(m=>m.humans.some(h=>h.profile_id===waiting.profile.profile_id)).length,0,'cancelled waiter was allocated');
 checks.checks.push({name:'same_participant_reconnect',status:'PASS',player_id:oldId,port:oldPort,allocation_id:allocation.allocation_id,progress:careerProgress(resumed),roster:baseline,in_match_before:{level:before.level,xp:before.xp,gold:before.gold},in_match_after:{level:after.level,xp:after.xp,gold:after.gold}});
 report.scenarios.push('same_profile_session_reconnect_preserves_player_progress_and_worker');
 for(const client of [waiting,stranger,resumed])assert.equal(client.errors.length,0,`client ${client.index} transport errors`);
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
 const start=performance.now();let runningAt=null,nextProgress=0;
 while(performance.now()-start<240000) {
  const now=performance.now();
  if(all.length<clientsCount && now-start>all.length*50)all.push(new Client(all.length));
  for(const c of all)c.tick(now);
  if(now>=nextProgress){nextProgress=now+5000;resourceSample();const states={};for(const c of all)states[c.stage]=(states[c.stage]||0)+1;console.log(wire({elapsed:Math.round((now-start)/1000),states}));fs.writeFileSync(path.join(root,'clients.json'),wire(all.map(c=>c.diagnostic())));}
  if(all.length===clientsCount && all.every(c=>c.stage==='running')){runningAt ||= now;if(now-runningAt>=soakSecs*1000)break;}
  await delay(20);
 }
 assert(all.length===clientsCount && all.every(c=>c.stage==='running'),'not all clients reached Running; inspect clients.json');
 const allocations=new Map();for(const c of all){const id=c.allocation.allocation_id;if(!allocations.has(id))allocations.set(id,[]);allocations.get(id).push(c);assert.equal(c.snapshot.players.length,10);assert.equal(c.errors.length,0);assert(performance.now()-c.lastSnapshot<2000,'active match stopped delivering snapshots');}
 assert.equal(new Set([...allocations.values()].map(cs=>cs[0].port)).size,allocations.size,'distinct arenas require distinct ports');
 for(const cs of allocations.values()){const roster=new Set(cs[0].snapshot.players.map(p=>p.id));for(const c of cs)assert(roster.has(c.snapshot.your_id));}
 report.scenarios.push('allocated_draft_countdown_loading_running','distinct_workers_and_ten_player_rosters');
 report.arenas=allocations.size;report.total_bytes=all.reduce((n,c)=>n+c.bytes,0);report.total_datagrams=all.reduce((n,c)=>n+c.packets,0);report.elapsed_seconds=(performance.now()-start)/1000;
 const gaps=all.flatMap(c=>c.gaps).sort((a,b)=>a-b);report.snapshot_gap_ms={p50:gaps[Math.floor(gaps.length*.5)],p95:gaps[Math.floor(gaps.length*.95)],p99:gaps[Math.floor(gaps.length*.99)],max:gaps.at(-1)};
 if(scenario==='lifecycle')await lifecycle(all[0]);
 if(scenario==='lobby-restart'||scenario==='recovery')await restartAndRecover(all[0],scenario==='recovery');
 report.total_transforms_sent=all.reduce((n,c)=>n+c.transforms,0);
 report.total_sent_datagrams=all.reduce((n,c)=>n+c.sentPackets,0);
 report.total_sent_bytes=all.reduce((n,c)=>n+c.sentBytes,0);
 if(inputHz>0&&soakSecs>0)assert(all.filter(c=>c.firstRunning).every(c=>c.transforms>0),'running clients did not exercise signed input');
 report.resources=resources;
 report.status='PASS';
} catch(error){report.status='FAIL';report.error=error.stack;process.exitCode=1;} finally {
 report.resources=resources;
 fs.writeFileSync(path.join(root,'report.json'),wire(report));fs.writeFileSync(path.join(root,'clients.json'),wire(all.map(c=>c.diagnostic())));console.log(wire(report));cleanup();
}
