from pathlib import Path
import argparse,json,socket,subprocess,sys,time,urllib.request,uuid,hashlib,os
repo=Path(__file__).resolve().parents[2]
sdk_repo=Path(os.environ.get('ORBIT_RPC_FIXTURE_SDK_ROOT',str(repo)))
sys.path.insert(0,str(sdk_repo/'sdk/runtime/python'))
from corework_runtime import Runtime

parser=argparse.ArgumentParser();parser.add_argument('dll');parser.add_argument('--expect-bug',action='store_true');parser.add_argument('--output',required=True);options=parser.parse_args()
root=Path(os.environ.get('ORBIT_RPC_FIXTURE_DATA_ROOT',str(repo/'target')))/('rpc-ai-'+uuid.uuid4().hex)
with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
fixture=subprocess.Popen(['node',str(Path(__file__).resolve().parent/'fixtures/rpc_ai_metadata.mjs'),f'127.0.0.1:{port}'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
runtime=None
try:
    line=fixture.stdout.readline();assert line,fixture.stderr.read()
    endpoint=json.loads(line)['http']
    for tool in ['BrowserOpenPage','PlainRpcProbe']:
        folder=root/'skills'/'role'/tool.lower();folder.mkdir(parents=True)
        (folder/'SKILL.md').write_text(f'---\nname: {tool.lower()}\ndescription: RPC metadata fixture\nkind: role\ntools: [{tool}]\n---\nUse {tool} once and report the result.\n',encoding='utf-8')
    runtime=Runtime(Path(options.dll),{'schema':'agent-runtime-create-options/v1','data_dir':str(root/'data'),'language':'en','restore_policy':'strict'})
    runtime.register_resources({'schema':'agent-runtime-resource-registration/v1','id':'rpc-ai-fixture','skills':{'root_dir':str(root/'skills')}})
    runtime.register_llm({'schema':'agent-runtime-llm-registration/v1','id':'mock','current_model_uid':1001,'providers':[{'id':1,'name':'fixture','type':'openai','base_url':endpoint+'/v1','api_key':'unused-fixture','strict_tool_schema':True,'api_paradigm':'openai_chat_completions','enabled_models':[{'uid':1001,'model_id':'gpt-4o'}]}]})
    runtime.start()
    runtime.add_rpc_endpoint({'id':'rpc-ai','protocol':'grpc','endpoint':f'http://127.0.0.1:{port}','timeout_ms':3000})
    catalog=runtime.invoke('runtime.get_tool_definitions')
    report={'dll':str(Path(options.dll).resolve()),'sha256':hashlib.sha256(Path(options.dll).read_bytes()).hexdigest(),'version':runtime.version,'cases':[]}
    def stats():return json.load(urllib.request.urlopen(endpoint+'/stats',timeout=5))
    for tool in ['BrowserOpenPage','PlainRpcProbe']:
        assert any(item['name']==tool for item in catalog['tools'])
        runtime.add_agent_cluster({'schema':'agent-runtime-agent-cluster-registration/v1','id':tool.lower(),'description':'RPC AI metadata regression','focus_agent_id':'primary','agents':[{'id':'primary','role':tool.lower(),'model_uid':1001,'tool_protocol':'native_fc'}]})
        info=runtime.spawn_conversation(cluster_id=tool.lower());conversation_id=info['conversation_id'];before=stats()
        events=[]
        try:runtime.send_message(conversation_id,'Use '+tool+' for https://fixture.invalid/')
        except Exception as error:
            if not options.expect_bug:raise
            events.append({'send_error':str(error)})
        deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            if 'has no registered metadata' in json.dumps(events):break
            event=runtime.next_event(100)
            if event is not None and event.get('conversation_id')==conversation_id:events.append(event)
            text=json.dumps(events)
            completed=any(event.get('type')=='conversation.ledger_delta' and event.get('payload',{}).get('record',{}).get('role')=='assistant' and 'RPC_AI_SUCCESS_'+tool in event.get('payload',{}).get('record',{}).get('content','') for event in events)
            waiting=bool(events and events[-1].get('type')=='frontend:state_snapshot' and events[-1].get('payload',{}).get('conversation_state')=='waiting')
            if 'has no registered metadata' in text or (completed and waiting):break
        after=stats();sent=after['requests'][len(before['requests']):];calls=after['executions'][len(before['executions']):]
        (root/(tool+'-diagnostic.json')).write_text(json.dumps({'requests':sent,'executions':calls,'events':events},ensure_ascii=False,indent=2),encoding='utf-8')
        case={'tool':tool,'catalog_visible':True,'model_requests':len(sent),'rpc_executions':len(calls),'error':next((event['send_error'] for event in events if isinstance(event,dict) and 'send_error' in event),None),'missing_metadata':'has no registered metadata' in json.dumps(events),'completed':'RPC_AI_SUCCESS_'+tool in json.dumps(events)}
        if sent:
            assert all(any(definition['function']['name']==tool for definition in request['tools']) for request in sent)
            assert calls and all(call['conversation_id']==conversation_id and call['agent_id']=='primary' and call['tool_call_id'] for call in calls),(case,calls,str(root/(tool+'-diagnostic.json')))
        report['cases'].append(case)
        runtime.invoke('conversation.close',{'conversation_id':conversation_id})
    Path(options.output).write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8');print(json.dumps(report,ensure_ascii=False))
    if options.expect_bug:assert all(case['missing_metadata'] and case['model_requests']==0 and case['rpc_executions']==0 for case in report['cases']),report
    else:assert all(case['completed'] and not case['missing_metadata'] and case['model_requests']==2 and case['rpc_executions']==1 for case in report['cases']),report
finally:
    if runtime is not None:runtime.close()
    fixture.terminate()
    try:fixture.wait(timeout=5)
    except subprocess.TimeoutExpired:fixture.kill();fixture.wait()
