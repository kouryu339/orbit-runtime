import { afterEach, describe, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Conversation from '../src/lib/Conversation.svelte';
import { TRANSPORT_CONTRACT, type ConversationTransport, type ConversationTransportHandlers, type SendMessageRequest } from '@agent-runtime/conversation-element/host';

class Transport implements ConversationTransport {
  contract = TRANSPORT_CONTRACT;
  id = 'test';
  handlers: ConversationTransportHandlers[] = [];
  send = vi.fn(async (_request: SendMessageRequest) => ({accepted:true}));
  pause = vi.fn(async () => ({accepted:true}));
  close = vi.fn(async () => ({accepted:true}));
  resolveToolPermission = vi.fn(async () => ({accepted:true}));
  async connect(context: {conversationId?: string | null}, handlers: ConversationTransportHandlers) {
    this.handlers.push(handlers);
    handlers.connection('connected');
    handlers.event({type:'conversation-created',conversationId:context.conversationId ?? 'one'});
    return {disconnect() {}};
  }
  snapshot(payload: Record<string, unknown> = {}, index = this.handlers.length - 1, id = 'one') {
    this.handlers[index].event({type:'state-snapshot',conversationId:id,payload:{conversation_state:'waiting',ledger_records:[],...payload}});
  }
}
const instances: ReturnType<typeof mount>[] = [];
async function setup(props: Record<string, unknown> = {}) {
  const target = document.createElement('div'); document.body.append(target);
  const transport = new Transport();
  const component = mount(Conversation, {target,props:{transport,...props}});
  instances.push(component);
  await tick(); await Promise.resolve(); await tick();
  return {component,transport,target};
}
afterEach(async () => { for (const instance of instances.splice(0)) await unmount(instance); document.body.innerHTML=''; });

describe('Svelte conversation contract', () => {
  it('defers the first send until an authoritative snapshot and preserves options', async () => {
    const {component,transport} = await setup();
    const pending = component.send('hello', {source:'test',metadata:{x:1}});
    expect(transport.send).not.toHaveBeenCalled();
    transport.snapshot(); await tick();
    expect((await pending).accepted).toBe(true);
    expect(transport.send).toHaveBeenCalledOnce();
    expect(transport.send.mock.calls[0][0].metadata).toEqual({x:1,source:'test'});
  });
  it('queues a send during pause and rejects stale snapshots after switching', async () => {
    const {component,transport,target} = await setup();
    transport.snapshot({conversation_state:'thinking'}); await tick();
    await component.pause();
    const pending = component.send('after pause');
    expect(transport.send).not.toHaveBeenCalled();
    transport.snapshot(); await pending;
    await component.openConversation('two');
    transport.snapshot({ledger_records:[{record_id:'old',role:'assistant',content:'stale response'}]},0);
    transport.snapshot({},1,'two'); await tick();
    expect(target.textContent).not.toContain('stale response');
  });
  it('supports aborting a deferred send', async () => {
    const {component,transport} = await setup(); const abort = new AbortController();
    const pending = component.send('do not send',{signal:abort.signal}); abort.abort();
    expect((await pending).accepted).toBe(false); transport.snapshot();
    expect(transport.send).not.toHaveBeenCalled();
  });
  it('renders FC bubbles, running state and safe approval targets', async () => {
    const {transport,target,component} = await setup();
    transport.snapshot({conversation_state:'executing',ledger_records:[
      {record_id:'a',role:'assistant',content:'Getting a snapshot.',metadata:{extra:{tool_calls:[{id:'call-1',function:{name:'GetSnapshot'}}]}}},
      {record_id:'t',role:'gateway_message',content:'Running',metadata:{subtype:'tool_call_started',tool_name:'GetSnapshot',extra:{call_id:'call-1'}}}
    ],pending_permissions:[{conversation_id:'one',agent_id:'boss',tool_call_id:'call-1',tool_name:'GetSnapshot',effect:'read_only',arguments:{page_id:'page-7',api_key:'do-not-show',content:{raw:true}}}]});
    await tick();
    expect(target.querySelector('details.tool')).not.toBeNull();
    expect(target.textContent).toContain('工具执行中');
    expect(target.querySelector('.approval')?.textContent).toContain('page-7');
    expect(target.querySelector('.approval')?.textContent).not.toContain('do-not-show');
    expect(target.querySelector('.approval pre')).toBeNull();
    await component.resolveToolPermission('call-1','allow'); await tick();
    expect(target.querySelector('.approval')).toBeNull();
  });
  it('anchors presentation items and progressively reveals one preset at a time', async () => {
    const {component,transport,target} = await setup();
    transport.snapshot({ledger_records:[{record_id:'a',role:'assistant',content:'original'}]});
    await tick();
    component.insertPresetMarkdown({id:'p1',markdown:'first'});
    component.insertPresetMarkdown({id:'p2',markdown:'second'});
    await tick();
    expect(target.textContent).not.toContain('second');
    await vi.waitFor(() => expect(target.textContent).toContain('second'));
    expect(target.textContent!.indexOf('first')).toBeLessThan(target.textContent!.indexOf('second'));
    component.clearPresentationItems('preset-markdown'); await tick();
    expect(target.textContent).not.toContain('first');
    expect(target.textContent).toContain('original');
  });
  it('restored assistant text is shown immediately without replaying it', async () => {
    const {transport,target} = await setup();
    transport.snapshot({ledger_records:[{record_id:'old',role:'assistant',content:'restored complete answer'}]});
    await tick(); expect(target.textContent).toContain('restored complete answer');
  });
});
