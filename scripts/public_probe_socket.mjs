// Loopback peers share the server's network namespace in process rehearsals.
// An ephemeral client must not occupy a port reserved for a future worker.
import dgram from 'node:dgram';

export function bindLoopbackPeer({isReserved,onSocket,onReady,onFailure,onRejected=()=>{},active=()=>true,maxAttempts=32}) {
 let attempts=0;
 const attempt=()=>{
  if(!active())return;
  const socket=dgram.createSocket('udp4');
  attempts++;onSocket(socket);
  const closeThen=next=>{
   try{socket.close(next);}catch(error){
    if(error.code==='ERR_SOCKET_DGRAM_NOT_RUNNING')next();else onFailure(error);
   }
  };
  const failed=error=>closeThen(()=>{if(active())onFailure(error);});
  socket.once('error',failed);
  socket.once('listening',()=>{
   socket.removeListener('error',failed);
   if(!active()){closeThen(()=>{});return;}
   const port=socket.address().port;
   if(isReserved(port)){
    onRejected(port);
    closeThen(()=>{
     if(!active())return;
     if(attempts>=maxAttempts)onFailure(new Error(`No non-reserved loopback peer port after ${attempts} attempts`));
     else attempt();
    });
   }else onReady(socket);
  });
  socket.bind(0,'127.0.0.1');
 };
 attempt();
}

// A newly bound peer may reuse another client's former lobby endpoint. Late
// lobby traffic must never replace the new worker's epoch or authentication.
export function listenForLoopbackPeer(socket, peerPort, onMessage, onForeign=()=>{}) {
 socket.on('message',(bytes,remote)=>{
  if(remote?.address==='127.0.0.1'&&remote.port===peerPort())onMessage(bytes);
  else onForeign();
 });
}
