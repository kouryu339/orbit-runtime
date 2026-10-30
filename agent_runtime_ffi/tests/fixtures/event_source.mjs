import {createServer} from 'node:http';
import {MemoryEventSource,serveEventSource} from '../../../sdk/rpctools/node/src/index.js';
let acknowledgements=0;
class FixtureSource extends MemoryEventSource {
  acknowledge(request) { const value=super.acknowledge(request);acknowledgements++;return value; }
}
const source=new FixtureSource({sourceId:'wechat:fixture-account',replayable:true,capacity:16,eventTypes:[{name:'wechat.message.received',description:'Fixture only',schema_version:'1',data_schema:{type:'object'}}]});
for(let i=1;i<=5;i++)source.publish('wechat.message.received',{message_id:`message-${i}`,text:`fixture ${i}`});
const rpc=await serveEventSource(process.argv[2],source);
const http=createServer((req,res)=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({acknowledgements}));});
await new Promise(resolve=>http.listen(0,'127.0.0.1',resolve));
console.log(JSON.stringify({http:`http://127.0.0.1:${http.address().port}`,catalog:source.describe()}));
process.on('SIGTERM',()=>{rpc.forceShutdown();http.close();});
