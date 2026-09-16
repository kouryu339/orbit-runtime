// @vitest-environment happy-dom
import { afterEach, expect, it } from 'vitest';
import { conversationReducer, createConversationState } from '../src/protocol/reducer.js';
import { defaultEnvelopeEvents } from '../src/transport/http-sse.js';
import { AgentRuntimePlanCard } from '../src/components/plan-card.js';
import type { ExecutionPlan, FrontendSnapshotPayload } from '../src/protocol/types.js';

const plan: ExecutionPlan = {
  plan_id: 'p1', revision: 1, title: 'Ship runtime', status: 'active',
  steps: [{ id: 's1', text: 'Verify', status: 'in_progress' }, { id: 's2', text: 'Publish', status: 'pending' }],
};
afterEach(() => document.body.replaceChildren());

it('routes actual plan deltas by agent, rejects stale revisions, and clears null snapshots', () => {
  let state = createConversationState('c1');
  const apply = (payload: FrontendSnapshotPayload) => {
    state = conversationReducer(state, { type: 'snapshot', payload });
  };
  apply({ active_agent_id: 'boss', plan });
  const events = defaultEnvelopeEvents({
    type: 'conversation.state_delta', conversation_id: 'c1', event_seq: 2,
    payload: { op: 'agent_plan.set', agent_id: 'worker', plan: { ...plan, plan_id: 'worker-plan' } },
  });
  for (const event of events) if (event.type === 'state-snapshot') apply(event.payload);
  expect(state.plan?.plan_id).toBe('p1');
  apply({ active_agent_id: 'worker' });
  expect(state.plan?.plan_id).toBe('worker-plan');
  apply({ active_agent_id: 'boss', plan: { ...plan, revision: 3 } });
  apply({ plan_agent_id: 'boss', plan: { ...plan, revision: 2 } });
  expect(state.plan?.revision).toBe(3);
  apply({ plan: null });
  expect(state.plan).toBeUndefined();
  state = conversationReducer(state, { type: 'reset', conversationId: 'c2' });
  expect(state.plansByAgent).toEqual({});
});

it('updates a single accessible card and preserves user collapse', async () => {
  const card = new AgentRuntimePlanCard();
  card.plan = plan;
  document.body.append(card);
  await card.updateComplete;
  expect(card.shadowRoot!.querySelectorAll('li')).toHaveLength(2);
  const details = card.shadowRoot!.querySelector('details')!;
  expect(details.open).toBe(true);
  details.open = false;
  details.dispatchEvent(new Event('toggle'));
  card.plan = { ...plan, revision: 2, steps: plan.steps!.map(s => ({ ...s, status: 'completed' })), status: 'finished' };
  await card.updateComplete;
  expect(card.shadowRoot!.querySelectorAll('details')).toHaveLength(1);
  expect(details.open).toBe(false);
  expect(card.shadowRoot!.textContent).toContain('2/2');
  expect(card.shadowRoot!.querySelector('progress')!.value).toBe(2);
});

it('renders legacy Markdown and escapes structured step text', async () => {
  const card = new AgentRuntimePlanCard();
  card.plan = { ...plan, steps: [{ id: 'x', text: '<img src=x onerror=alert(1)>', status: 'pending' }] };
  document.body.append(card);
  await card.updateComplete;
  expect(card.shadowRoot!.querySelector('img')).toBeNull();
  card.plan = { title: 'Legacy', status: 'finished', content: '- old plan' };
  await card.updateComplete;
  expect(card.shadowRoot!.querySelector('agent-conversation-rich-content')).not.toBeNull();
});
