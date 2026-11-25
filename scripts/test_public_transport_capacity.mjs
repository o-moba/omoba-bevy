#!/usr/bin/env node
// Owned loopback transport boundary; disposable DB startup/heartbeat only, no profiles or arenas.
// Requires the isolated loopback OMOBA_TEST_DATABASE_URL used by public probes.
// HARNESS_SERVER_BIN=... node scripts/test_public_transport_capacity.mjs --smoke=8
// HARNESS_SERVER_BIN=... node scripts/test_public_transport_capacity.mjs --allow-large
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import dgram from 'node:dgram';
import fs from 'node:fs';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {performance} from 'node:perf_hooks';

const options=Object.fromEntries(process.argv.slice(2).map(arg=>{
 const [key,...value]=arg.replace(/^--/,'').split('=');return [key,value.join('=')];
}));
function configuration(opts){
 assert(!('host' in opts),'this probe launches only its own loopback lobby');
 const smoke='smoke' in opts,limit=smoke?Number(opts.smoke):2048;
 assert(Number.isInteger(limit)&&limit>=1&&limit<=(smoke?32:2048),'invalid endpoint limit');
 assert(smoke||'allow-large' in opts,'the2048+1 probe requires --allow-large');
 const port=Number(opts.port||46700);
 assert(Number.isInteger(port)&&port>=1024&&port<=65534,'invalid loopback port');
 return {limit,port,kind:smoke?'per_source_smoke_fixture':'global_and_per_source_endpoint_ceiling'};
}
function isolatedDatabase(value){
 try{const u=new URL(value);return ['postgres:','postgresql:'].includes(u.protocol)&&['localhost','127.0.0.1','::1','[::1]'].includes(u.hostname)&&u.pathname==='/omoba_test'&&!u.search&&!u.hash;}catch{return false;}
}
function epochFrom(bytes){
 const value=/"server_epoch"\s*:\s*(\d+)\s*[,}]/.exec(bytes.toString())?.[1];
 assert(value&&BigInt(value)>0n&&BigInt(value)<=18446744073709551615n,'invalid u64 epoch');return value;
}
if('self-test-policy' in options){
 assert.equal(configuration({'allow-large':''}).limit,2048);
 assert(isolatedDatabase('postgres://postgres:postgres@127.0.0.1:55581/omoba_test'));
 for(const u of ['postgres://remote/omoba_test','postgres://localhost/production','postgres://localhost/omoba_test?host=remote'])assert(!isolatedDatabase(u));
 assert.equal(configuration({smoke:'8'}).limit,8);
 for(const opts of [{},{smoke:'0'},{smoke:'33'},{smoke:'NaN'},{smoke:'8',port:'65535'},{smoke:'8',host:'remote'}])assert.throws(()=>configuration(opts));
 for(const value of ['9007199254740993','18446744073709551615'])assert.equal(epochFrom(Buffer.from(`{"server_epoch":${value}}`)),value);
 for(const value of ['0','18446744073709551616','1e9'])assert.throws(()=>epochFrom(Buffer.from(`{"server_epoch":${value}}`)));
 console.log('transport_capacity_policy: PASS (scope, bounds, explicit large flag and exact u64 epochs)');process.exit(0);
}
const config=configuration(options);
assert(isolatedDatabase(process.env.OMOBA_TEST_DATABASE_URL),'use an isolated loopback /omoba_test database without URI overrides');
const binary=process.env.HARNESS_SERVER_BIN;assert(binary&&fs.statSync(binary).isFile(),'set HARNESS_SERVER_BIN');
const source=new URL('../shared/src/protocol.rs',import.meta.url);
const protocol=Number(options['protocol-version']||(fs.existsSync(source)?/pub const PROTOCOL_VERSION: u16 = (\d+);/.exec(fs.readFileSync(source,'utf8'))?.[1]:0));
assert(Number.isInteger(protocol)&&protocol>0,'provide a valid protocol version');
const root=path.resolve(options.output||`.agent/codex-ranked-readiness/transport-capacity-${Date.now()}`);fs.mkdirSync(root,{recursive:true});
const hash=file=>crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const report={status:'RUNNING',scope:'return-path-validated raw transport only; disposable DB startup/heartbeat; no account, search or gameplay',...config,protocol,source_sha256:hash(new URL(import.meta.url)),server_sha256:hash(binary),started_at:new Date().toISOString()};
const env=Object.fromEntries(Object.entries(process.env).filter(([key])=>!key.startsWith('OMOBA_')&&!key.startsWith('GAME_')&&!key.startsWith('PG')&&!['SERVER_ADDR','DATABASE_URL'].includes(key)));
Object.assign(env,{OMOBA_DATABASE_URL:process.env.OMOBA_TEST_DATABASE_URL,SERVER_ADDR:`127.0.0.1:${config.port}`,OMOBA_SERVER_ROLE:'lobby',OMOBA_MATCH_ROOT:path.join(root,'matches'),OMOBA_CAREER_OUTBOX:path.join(root,'outbox'),OMOBA_MATCH_CAPACITY:'1',OMOBA_MATCH_FIRST_PORT:String(config.port+1),OMOBA_MATCH_PUBLIC_HOST:'127.0.0.1',OMOBA_MATCH_BIND_IP:'127.0.0.1',OMOBA_LIMIT_ENDPOINTS_PER_IP:String(config.limit),OMOBA_LIMIT_PROBES_PER_IP:'10000',OMOBA_LIMIT_PROBES_PER_NETWORK:'10000',OMOBA_LIMIT_PROBES_GLOBAL:'10000'});
report.fixture={database_scope:'isolated loopback /omoba_test startup/heartbeat only; no account requests',per_source_endpoints:Number(env.OMOBA_LIMIT_ENDPOINTS_PER_IP),probe_budgets:10000,global_endpoint_limit_unchanged:2048,boundary_scope:config.kind==='per_source_smoke_fixture'?'per-source fixture only':'global and per-source both2048; this wire proof does not distinguish them'};
const log=fs.openSync(path.join(root,'lobby.log'),'w');
const lobby=spawn(binary,[],{env,detached:true,stdio:['ignore',log,log]});
const clients=[];const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));let stopped=false;
class RawClient{
 constructor(index){
  this.index=index;this.nonce=crypto.randomBytes(16).toString('hex');this.bound=false;this.epoch=null;this.path=null;this.lastProbe=0;this.lastProof=0;this.lastSnapshot=0;this.snapshots=0;this.full=[];this.errors=[];
  this.probe=Buffer.from(JSON.stringify({type:'transport_probe',protocol_version:protocol,client_nonce:this.nonce,padding:'0'.repeat(384)}));
  this.socket=dgram.createSocket('udp4');this.socket.on('error',error=>this.errors.push(error.message));
  this.socket.on('message',bytes=>{try{
   assert(bytes.length<=65536,'oversized response');const packet=JSON.parse(bytes.toString());
   if(packet.type==='transport_challenge'){
    assert.equal(packet.client_nonce,this.nonce);assert(/^[a-f0-9]{64}$/.test(packet.path_nonce));
    this.epoch=epochFrom(bytes);this.path=packet.path_nonce;this.proof();
   }else if(packet.type==='server_full'){
    assert.equal(packet.client_nonce,this.nonce);assert(bytes.length<=this.probe.length,'server_full amplified the probe');this.full.push({client_nonce:packet.client_nonce,bytes:bytes.length});
   }else if(packet.type==='snapshot'){
    assert(['players','structures','minions','neutrals','projectiles'].every(key=>!packet[key]?.length),'raw endpoint received world data');
    assert.equal(packet.game_state?.type,'lobby');this.lastSnapshot=performance.now();this.snapshots++;
   }else assert.notEqual(packet.type,'career','no account flow belongs in the raw probe');
  }catch(error){if(this.errors.length<4)this.errors.push(error.message);}});
  this.socket.bind(0,'127.0.0.1',()=>{this.bound=true;});
 }
 send(bytes){if(!stopped)this.socket.send(bytes,config.port,'127.0.0.1');}
 proof(){if(!this.path)return;this.lastProof=performance.now();this.send(Buffer.from(`{"type":"transport_proof","server_epoch":${this.epoch},"path_nonce":${JSON.stringify(this.path)}}`));}
 tick(now){if(!this.bound)return;if(!this.path&&now-this.lastProbe>=1000){this.lastProbe=now;this.send(this.probe);}if(this.path&&now-this.lastProof>=1000)this.proof();}
 close(){try{this.socket.close();}catch{}}
}
async function pump(label,predicate,timeout,onTick=()=>{}){
 const start=performance.now();while(performance.now()-start<timeout){
  assert.equal(lobby.exitCode,null,'owned lobby exited');const now=performance.now();onTick(now);for(const client of clients)client.tick(now);
  assert(clients.every(c=>c.errors.length===0),`${label}: ${JSON.stringify(clients.filter(c=>c.errors.length).map(c=>({index:c.index,errors:c.errors})).slice(0,3))}`);
  if(predicate())return;await delay(5);
 }throw new Error(`${label}: timed out after${timeout}ms`);
}
function cleanup(){if(stopped)return;stopped=true;for(const c of clients)c.close();if(lobby.exitCode===null&&lobby.pid){try{process.kill(-lobby.pid,'SIGTERM');}catch{}}fs.closeSync(log);}
for(const signal of ['SIGINT','SIGTERM'])process.on(signal,()=>{cleanup();process.exit(130);});
try{
 await delay(1000);assert.equal(lobby.exitCode,null,'lobby startup failed');
 const started=performance.now();
 await pump('admitted endpoints reach empty lobby snapshots',()=>clients.length===config.limit&&clients.every(c=>c.lastSnapshot>0),20000,now=>{
  while(clients.length<config.limit&&now-started>=clients.length*2)clients.push(new RawClient(clients.length));
 });
 assert(clients.every(c=>c.full.length===0),'an intended admitted endpoint was refused');
 assert.equal(new Set(clients.map(c=>c.epoch)).size,1,'lobby epoch changed');
 const admitted=clients.slice();const extra=new RawClient(config.limit);clients.push(extra);
 await pump('next endpoint receives correlated server_full',()=>extra.full.length>0,5000);
 assert.equal(extra.path,null,'extra endpoint was admitted');assert.equal(extra.snapshots,0);
 const maxAge=Math.max(...admitted.map(c=>performance.now()-c.lastSnapshot));assert(maxAge<2000,'admitted endpoint snapshots became stale');
 report.status='PASS';report.admitted=admitted.length;report.refused_endpoint=config.limit+1;report.server_full=extra.full[0];report.probe_bytes=extra.probe.length;report.max_bootstrap_age_ms=maxAge;report.server_epoch=admitted[0].epoch;report.elapsed_seconds=(performance.now()-started)/1000;
 const matches=fs.existsSync(path.join(root,'matches'))?fs.readdirSync(path.join(root,'matches'),{withFileTypes:true}).filter(entry=>entry.isDirectory()):[];
 assert.equal(matches.length,0,'raw probe must not allocate an arena');report.arenas=0;report.profiles_created=0;
}catch(error){report.status='FAIL';report.error=String(error.stack||error);report.observed={created:clients.length,snapshots:clients.filter(c=>c.lastSnapshot).length,full:clients.filter(c=>c.full.length).length};process.exitCode=1;}
finally{cleanup();fs.writeFileSync(path.join(root,'report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));}
