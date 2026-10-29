import {createServer} from 'node:http';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
const sdk=process.env.ORBIT_RPC_FIXTURE_SDK_ROOT?pathToFileURL(path.join(process.env.ORBIT_RPC_FIXTURE_SDK_ROOT,'sdk/rpctools/node/src/index.js')):new URL('../../../sdk/rpctools/node/src/index.js',import.meta.url);
const {registerTool,serve,AIOutput}=await import(sdk.href);
const requests=[], executions=[];
for(const name of ['BrowserOpenPage','PlainRpcProbe'])registerTool({name,description:'Return a local regression fixture result.',readonly:true,idempotent:true,parameters:[{name:'url',param_type:'String',required:true}],outputs:[{name:'page_id',field_type:'String'}]},async(ctx,args)=>{executions.push({name,args,conversation_id:ctx.conversationId,agent_id:ctx.agentId,tool_call_id:ctx.toolCallId});return new AIOutput({result:{page_id:'fixture-page'},to_ai:'RPC fixture completed'});});
const rpc=await serve(process.argv[2]);
const http=createServer(async(req,res)=>{
  if(req.url==='/stats'){res.setHeader('Content-Type','application/json');res.end(JSON.stringify({requests,executions}));return;}
  if(req.url!='/v1/chat/completions'){res.writeHead(404);res.end();return;}
  const chunks=[];for await(const chunk of req)chunks.push(chunk);
  const body=JSON.parse(Buffer.concat(chunks));requests.push(body);
  const toolResult=body.messages.findLast(message=>message.role==='tool');
  if(toolResult){const assistant=body.messages.findLast(message=>message.role==='assistant'&&message.tool_calls?.some(call=>call.id===toolResult.tool_call_id));if(!assistant){res.writeHead(400);res.end(JSON.stringify({error:{message:'orphan tool result'}}));return;}}
  const wanted=body.messages.findLast(message=>message.role==='user'&&String(message.content).startsWith('Use '))?.content?.includes('PlainRpcProbe')?'PlainRpcProbe':'BrowserOpenPage';
  const delta=toolResult?{content:'RPC_AI_SUCCESS_'+wanted}:{tool_calls:[{index:0,id:'call_'+wanted,type:'function',function:{name:wanted,arguments:'{"url":"https://fixture.invalid/"}'}}]};
  if(body.stream){res.writeHead(200,{'Content-Type':'text/event-stream','Cache-Control':'no-cache'});res.end(`data: ${JSON.stringify({id:'fixture',choices:[{index:0,delta,finish_reason:null}]})}\n\ndata: ${JSON.stringify({id:'fixture',choices:[{index:0,delta:{},finish_reason:toolResult?'stop':'tool_calls'}]})}\n\ndata: [DONE]\n\n`);}
  else{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({id:'fixture',choices:[{index:0,message:{role:'assistant',...delta},finish_reason:toolResult?'stop':'tool_calls'}]}));}
});
await new Promise(resolve=>http.listen(0,'127.0.0.1',resolve));
console.log(JSON.stringify({ready:true,http:`http://127.0.0.1:${http.address().port}`}));
process.on('SIGTERM',()=>{rpc.forceShutdown();http.close();});
