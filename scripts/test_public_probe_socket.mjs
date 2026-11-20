import test from 'node:test';
import assert from 'node:assert/strict';
import dgram from 'node:dgram';
import {bindLoopbackPeer,listenForLoopbackPeer} from './public_probe_socket.mjs';

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

test('late lobby datagrams cannot enter the new worker context on a reused endpoint',{timeout:5000},async()=>{
 const lobby=dgram.createSocket('udp4'),worker=dgram.createSocket('udp4'),peer=dgram.createSocket('udp4');
 const sockets=[lobby,worker,peer];
 try{
  for(const socket of sockets)await new Promise((resolve,reject)=>{socket.once('error',reject);socket.bind(0,'127.0.0.1',resolve);});
  let expected=lobby.address().port,foreign=0;
  const received=[];
  let delivered;
  listenForLoopbackPeer(peer,()=>expected,bytes=>{received.push(bytes.toString());delivered?.();},()=>foreign++);
  const send=(socket,text)=>new Promise((resolve,reject)=>socket.send(Buffer.from(text),peer.address().port,'127.0.0.1',error=>error?reject(error):resolve()));
  let next=new Promise(resolve=>{delivered=resolve;});
  await send(lobby,'lobby epoch');await next;
  expected=worker.address().port;
  // Wait for the rejected datagram to arrive before sending the valid worker
  // view, so the test does not rely on ordering across two UDP senders.
  const late=new Promise(resolve=>peer.once('message',resolve));
  await send(lobby,'stale lobby auth');await late;
  next=new Promise(resolve=>{delivered=resolve;});
  await send(worker,'worker epoch');await next;
  assert.deepEqual(received,['lobby epoch','worker epoch']);assert.equal(foreign,1);
 }finally{await Promise.all(sockets.map(close));}
});
