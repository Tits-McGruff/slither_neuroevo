import WebSocket from 'ws';
import { once } from 'node:events';
import { writeFile } from 'node:fs/promises';
const targets=await(await fetch('http://127.0.0.1:9229/json/list')).json();
const target=targets.find(item=>item.url==='file:///srv/opt/apps/slither_neuroevo/data/codex-memory-profile-255999c/server/rustServer.ts');
if(!target)throw new Error('owned diagnostic Node target missing');
const socket=new WebSocket(target.webSocketDebuggerUrl);
await once(socket,'open');
let next=0;
const requests=new Map(),workerRequests=new Map(),workers=new Map();
socket.on('message',data=>{const message=JSON.parse(data.toString());if(message.id){const settle=requests.get(message.id);if(settle){requests.delete(message.id);message.error?settle.reject(message.error):settle.resolve(message.result);}}
if(message.method==='NodeWorker.attachedToWorker')workers.set(message.params.sessionId,message.params.workerInfo);
if(message.method==='NodeWorker.receivedMessageFromWorker'){const response=JSON.parse(message.params.message);const key=`${message.params.sessionId}/${response.id}`;const settle=workerRequests.get(key);if(settle){workerRequests.delete(key);response.error?settle.reject(response.error):settle.resolve(response.result);}}});
function call(method,params={}){const id=++next;return new Promise((resolve,reject)=>{const timer=setTimeout(()=>{requests.delete(id);reject(new Error(`inspector timeout: ${method}`));},5000);requests.set(id,{resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}});socket.send(JSON.stringify({id,method,params}));});}
const expression="JSON.stringify({threadId:process.getBuiltinModule('worker_threads').threadId,memory:process.memoryUsage(),v8:process.getBuiltinModule('v8').getHeapStatistics()})";
function workerMemory(sessionId){const id=++next,key=`${sessionId}/${id}`;return new Promise((resolve,reject)=>{const timer=setTimeout(()=>{workerRequests.delete(key);reject(new Error('worker inspector timeout'));},5000);workerRequests.set(key,{resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}});void call('NodeWorker.sendMessageToWorker',{sessionId,message:JSON.stringify({id,method:'Runtime.evaluate',params:{expression,returnByValue:true}})}).catch(reject);});}
try{const result={scope:'Read-only Node inspector memory of main and existing worker isolates; diagnostic run only',capturedAtUtc:new Date().toISOString(),main:JSON.parse((await call('Runtime.evaluate',{expression,returnByValue:true})).result.value),workers:[]};
await call('NodeWorker.enable',{waitForDebuggerOnStart:false});
await new Promise(resolve=>setTimeout(resolve,500));
for(const[sessionId,info]of workers){const response=await workerMemory(sessionId);result.workers.push({info,...JSON.parse(response.result.value)});}
await call('NodeWorker.disable');
await writeFile(process.argv[2],JSON.stringify(result,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify(result));
}finally{socket.close();}
