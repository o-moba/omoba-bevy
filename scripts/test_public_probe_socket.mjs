import test from 'node:test';
import assert from 'node:assert/strict';
import dgram from 'node:dgram';
import {bindLoopbackPeer} from './public_probe_socket.mjs';

const close=socket=>new Promise(resolve=>{try{socket.close(resolve);}catch{resolve();}});
test('a client binding a future worker port releases it and resamples before sending',{timeout:5000},async()=>{
 const reserved=new Set(),rejected=[],sockets=[];
 let ready;
 try{
  ready=await new Promise((resolve,reject)=>bindLoopbackPeer({
   isReserved:port=>{if(!reserved.size)reserved.add(port);return reserved.has(port);},
   onSocket:socket=>{sockets.push(socket);socket.on('error',()=>{});},
   onRejected:port=>rejected.push(port),onReady:resolve,onFailure:reject,
  }));
  assert.ok(rejected.length>=1);assert.ok(!reserved.has(ready.address().port));
  // The rejected endpoint is now actually bindable by the worker.
  const worker=dgram.createSocket('udp4');sockets.push(worker);
  await new Promise((resolve,reject)=>{worker.once('error',reject);worker.bind(rejected[0],'127.0.0.1',resolve);});
  const message=new Promise(resolve=>worker.once('message',resolve));
  ready.send(Buffer.from('peer'),rejected[0],'127.0.0.1');
  assert.equal((await message).toString(),'peer');
 }finally{await Promise.all(sockets.map(close));}
});
test('an exhausted reserved range is bounded and leaves no bound client sockets',{timeout:5000},async()=>{
 const sockets=[],ports=[];
 const error=await new Promise(resolve=>bindLoopbackPeer({
  isReserved:()=>true,maxAttempts:3,onSocket:s=>{sockets.push(s);s.on('error',()=>{});},
  onReady:()=>resolve(new Error('unexpected ready')),onRejected:p=>ports.push(p),onFailure:resolve,
 }));
 assert.match(error.message,/after 3 attempts/);assert.equal(ports.length,3);
 for(const s of sockets)assert.throws(()=>s.address(),{code:'ERR_SOCKET_DGRAM_NOT_RUNNING'});
});
