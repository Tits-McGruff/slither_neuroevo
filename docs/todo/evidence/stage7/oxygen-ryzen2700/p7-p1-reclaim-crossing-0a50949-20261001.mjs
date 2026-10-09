import WebSocket from 'ws';
import {createHash} from 'node:crypto';
import {writeFile} from 'node:fs/promises';
const started=performance.now(), events=[], attempts=[];
let token, snakeId, peer, tick=0, gen=0, generationTime=0, failure, frames=0;
const digest=value=>value===undefined?undefined:createHash('sha256').update(value).digest('hex');
const log=(type,value)=>events.push({wallSeconds:(performance.now()-started)/1000,type,...value});
async function health(){const response=await fetch('http://127.0.0.1:5180/api/health',{signal:AbortSignal.timeout(15000)});const h=await response.json();return {ok:h.ok,generation:h.generation,completedStep:h.completedStep,worldEpoch:h.worldEpoch};}
async function connect(){
  const requested={snakeId,tokenHash:digest(token),generation:gen,tick};
  const requestedToken=token;
  const socket=new WebSocket('ws://127.0.0.1:5180');peer=socket;
  let assigned,result,closed=false;
  socket.on('error',error=>{failure??=String(error);});
  socket.on('close',()=>{closed=true;});
  socket.on('message',(bytes,binary)=>{
    if(binary){frames++;return;}
    const packet=JSON.parse(bytes.toString());
    if(typeof packet.tick==='number')tick=packet.tick;
    if(packet.type==='stats'){gen=packet.gen;generationTime=packet.generationTime;return;}
    if(packet.type==='assign'){
      const value={snakeId:packet.snakeId,tokenHash:digest(packet.resumeToken),reclaimed:packet.reclaimed===true};
      assigned??=value;snakeId=packet.snakeId;token=packet.resumeToken;log('assign',{...value,generation:gen,tick});
    } else if(packet.type==='reclaimResult'){result=packet;log('reclaimResult',{packet,generation:gen,tick});}
    else if(packet.type==='error'){log('error',{packet});failure??=packet.message;}
  });
  await new Promise((resolve,reject)=>{socket.once('open',resolve);socket.once('error',reject);});
  log('connect',{requested});
  socket.send(JSON.stringify({type:'hello',version:2,clientType:'ui'}));
  socket.send(JSON.stringify({type:'join',mode:'player',name:'BoundaryProbe',...(requestedToken?{resumeToken:requestedToken}:{})}));
  const deadline=performance.now()+5000;
  while((!assigned||(requestedToken&&!result))&&!closed&&performance.now()<deadline)await new Promise(r=>setTimeout(r,5));
  const attempt={requested,assigned,result,health:await health()};
  attempt.consistent=!requestedToken||result?.reclaimed!==true||assigned?.snakeId===requested.snakeId&&result.snakeId===requested.snakeId&&assigned.tokenHash!==requested.tokenHash;
  attempts.push(attempt);
  log('resolved',{attempt});
  if(!assigned||requestedToken&&!result)throw Error('missing lifecycle response');
  if(!attempt.consistent)throw Error('same-snake/token assertion mismatch captured');
}
const sender=setInterval(()=>{if(peer?.readyState===WebSocket.OPEN&&snakeId!==undefined)peer.send(JSON.stringify({type:'action',tick,snakeId,turn:0.5,boost:0}));},1000/30);
try{
  await connect();
  for(let round=0;round<2;round++){
    const deadline=performance.now()+70000;
    while(generationTime<59.7&&performance.now()<deadline){if(failure)throw Error(failure);await new Promise(r=>setTimeout(r,5));}
    log('disconnect',{snakeId,tokenHash:digest(token),generation:gen,generationTime,tick,health:await health()});
    await new Promise(resolve=>{peer.once('close',resolve);peer.close();});
    await new Promise(r=>setTimeout(r,800));
    await connect();
    await new Promise(r=>setTimeout(r,1000));
  }
}catch(error){failure??=String(error);}
finally{clearInterval(sender);peer?.terminate();}
const report={scope:'Focused generation-boundary reconnect attribution; supplements an already interrupted player soak, not full P7 acceptance',wallSeconds:(performance.now()-started)/1000,frames,attempts,events,failure};
await writeFile('p7-p1-reclaim-crossing-0a50949-20261001.json',JSON.stringify(report,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({wallSeconds:report.wallSeconds,attempts:attempts.length,failure}));
if(failure)process.exitCode=1;
