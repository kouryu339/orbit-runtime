"""DLL + Node event-only plugin regression. No real WeChat or model requests.
Usage: python agent_runtime_ffi/tests/event_sources.py PATH_TO_DLL
"""
from pathlib import Path
import json,socket,subprocess,sys,time,urllib.request,uuid
repo=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(repo/'sdk/runtime/python'))
from corework_runtime import Runtime,RuntimeError
root=repo/'target'/('event-source-'+uuid.uuid4().hex)
(root/'skills').mkdir(parents=True)
with socket.socket() as sock:
    sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
fixture=subprocess.Popen(['node',str(Path(__file__).parent/'fixtures/event_source.mjs'),f'127.0.0.1:{port}'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
runtime=None
try:
    line=fixture.stdout.readline();assert line,fixture.stderr.read()
    info=json.loads(line)
    runtime=Runtime(Path(sys.argv[1]),{'schema':'agent-runtime-create-options/v1','data_dir':str(root/'data'),'language':'en'},required_commands=('runtime.add_event_source','runtime.poll_source_events','runtime.acknowledge_events'))
    runtime.register_resources({'schema':'agent-runtime-resource-registration/v1','id':'events-fixture','skills':{'root_dir':str(root/'skills')}})
    runtime.register_llm({'schema':'agent-runtime-llm-registration/v1','id':'unused','current_model_uid':1,'providers':[{'id':1,'name':'unused','type':'openai','base_url':'http://127.0.0.1:1/v1','api_key':'unused','api_paradigm':'openai_chat_completions','enabled_models':[{'uid':1,'model_id':'gpt-4o'}]}]})
    runtime.start()
    assert runtime.capabilities['features']['rpc_event_sources']==1
    endpoint=runtime.add_event_source({'id':'wechat-events','endpoint':f'http://127.0.0.1:{port}'})
    assert endpoint['source_id']=='wechat:fixture-account'
    assert endpoint['durability']=='memory'
    assert runtime.get_event_source_definitions()['sources'][0]==endpoint
    assert not any('wechat.message' in t['name'] for t in runtime.invoke('runtime.get_tool_definitions')['tools'])
    runtime.subscribe_events({'endpoint_id':'wechat-events','subscription_id':'messages','consumer_id':'project-a','capacity':2})
    def poll_until(count):
        for _ in range(150):
            value=runtime.poll_source_events('messages')
            if len(value['events'])==count:return value
            time.sleep(.02)
        raise AssertionError(value)
    batch=poll_until(2)
    retry=runtime.poll_source_events('messages')
    assert retry['events']==batch['events'] and retry['pending_count']==2
    assert retry['acknowledged_cursor']==''
    assert json.load(urllib.request.urlopen(info['http'],timeout=3))['acknowledgements']==0
    cursor=batch['events'][-1]['cursor']
    runtime.acknowledge_events('messages',cursor)
    runtime.acknowledge_events('messages',cursor)
    assert json.load(urllib.request.urlopen(info['http'],timeout=3))['acknowledgements']==1
    following=poll_until(2)
    assert following['events'][0]['data']['message_id']=='message-3'
    runtime.unsubscribe_events('messages')
    runtime.subscribe_events({'endpoint_id':'wechat-events','subscription_id':'messages','consumer_id':'project-a','capacity':8,'after_cursor':cursor,'stream_id':endpoint['stream_id']})
    restored=poll_until(3)
    assert restored['events'][0]['data']['message_id']=='message-3'
    try:
        runtime.acknowledge_events('messages','invented-cursor')
        raise AssertionError('accepted forged cursor')
    except RuntimeError as error:
        assert error.error['kind']=='invalid_argument'
    # Shutdown must cancel the idle subscription without host polling/draining it.
    started=time.monotonic();runtime.close();runtime=None
    assert time.monotonic()-started<5
    print(json.dumps({'passed':True,'checks':['event-only discovery','capabilities','no AI tool registration','bounded retryable poll','explicit idempotent acknowledgement','exclusive cursor resume','invalid cursor rejection','shutdown cancellation']}))
finally:
    if runtime is not None:runtime.close()
    fixture.terminate()
    try:fixture.wait(timeout=5)
    except subprocess.TimeoutExpired:fixture.kill();fixture.wait()
