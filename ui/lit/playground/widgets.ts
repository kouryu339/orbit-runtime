import { TRANSPORT_CONTRACT, type AgentRuntimeConversationElement, type ConversationTransport, type ConversationTransportHandlers } from '../src/index.js';
const samples: Record<string,string> = {
single: '这份报告你想用什么格式？\n[select:single | label="导出格式" | options="PDF,DOCX,Markdown"]',
multi: '选择需要包含的内容，也可以补充其他内容。\n[select:multi | label="报告内容" | options="执行摘要,详细数据,趋势分析"]',
path: '选择要处理的文件，或者直接输入完整路径。\n[input:path | label="源文件" | accept=".xlsx,.csv"]',
text: '给这份报告起一个名字。\n[input:text | label="报告名称"]',
confirm: '已准备好生成报告。请确认是否继续。\n[confirm | label="生成报告"]',
all: '请填写报告配置：\n[select:single | label="导出格式" | options="PDF,DOCX"]\n[select:multi | label="报告内容" | options="摘要,数据,图表"]\n[input:path | label="源文件" | accept=".xlsx,.csv"]\n[input:text | label="报告名称"]\n[confirm | label="确认生成"]',
};
const chat=document.querySelector<AgentRuntimeConversationElement>('#chat')!;
let handlers: ConversationTransportHandlers;
let revision=0;
let records: Array<{record_id:string;role:string;content:string}>=[];
const scene=document.querySelector<HTMLSelectElement>('#scene')!;
const result=document.querySelector<HTMLPreElement>('#result')!;
function publish(){handlers.event({type:'state-snapshot',conversationId:'widget-preview',payload:{revision:++revision,conversation_state:'waiting',active_agent_id:'preview-agent',ledger_records:records}});}
function reset(){records=[{record_id:'question-'+Date.now(),role:'assistant',content:samples[scene.value]}];result.textContent='尚未提交';publish();}
const transport: ConversationTransport={
contract:TRANSPORT_CONTRACT,id:'widget-preview',
async connect(_context,next){handlers=next;next.connection('connected');next.event({type:'conversation-created',conversationId:'widget-preview'});reset();return {conversationId:'widget-preview',disconnect(){}};},
async send(request){const content=request.content??'';result.textContent=content;records.push({record_id:'reply-'+Date.now(),role:'user',content});publish();return {accepted:true};},
};
chat.transport=transport;
chat.progressiveReveal=false;
chat.capabilities={pickPath:async()=>({paths:['D:/示例数据/月度报告.xlsx']})};
void chat.connect();
scene.addEventListener('change',reset);
document.querySelector('#reset')!.addEventListener('click',reset);
document.querySelector('#scheme')!.addEventListener('change',event=>{chat.colorScheme=(event.target as HTMLSelectElement).value as 'dark'|'light';});
document.querySelector('#width')!.addEventListener('click',()=>document.querySelector('main')!.classList.toggle('narrow'));
