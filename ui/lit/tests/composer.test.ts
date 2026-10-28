import { describe, expect, it, vi } from 'vitest';
import { ComposerSession } from '../src/content/composer.js';
import { createConversationState } from '../src/protocol/reducer.js';
import {
  TRANSPORT_CONTRACT,
  type CommandResult,
  type ConversationComposerController,
  type ConversationComposerSettings,
} from '../src/host/types.js';
import type { ConversationState } from '../src/protocol/types.js';

function fixture(controller: ConversationComposerController | null = null) {
  const transport = {
    contract: TRANSPORT_CONTRACT,
    id: 'test',
    connect: vi.fn(),
    send: vi.fn(),
    pause: vi.fn(async () => ({ accepted: true })),
    resume: vi.fn(async () => ({ accepted: true })),
  };
  const error = vi.fn();
  const machine = new ComposerSession(vi.fn(), error);
  let state: ConversationState = {
    ...createConversationState('one'),
    connection: 'connected',
    initialized: true,
    snapshotReceived: true,
    modelUid: 1,
  };
  const update = (patch: Partial<ConversationState> = {}) => {
    state = { ...state, ...patch };
    machine.sync(state, transport, controller, state.modelUid);
  };
  update();
  return { machine, transport, error, update };
}
describe('composer state machine', () => {
  it('blocks offline, uninitialized, sending, compacting and stopping states', () => {
    const { machine, update } = fixture();
    expect(machine.canCompose).toBe(true);
    for (const connection of [
      'disconnected',
      'connecting',
      'reconnecting',
    ] as const) {
      update({ connection });
      expect(machine.canCompose).toBe(false);
      expect(machine.canPause).toBe(false);
      expect(machine.canResume).toBe(false);
    }
    update({ connection: 'connected', snapshotReceived: false });
    expect(machine.phase).toBe('initializing');
    update({ snapshotReceived: true, awaitingAssistantResponse: true });
    expect(machine.canCompose).toBe(false);
    for (const runtimeState of ['compacting', 'stopping'] as const) {
      update({ runtimeState });
      expect(machine.canPause).toBe(false);
      expect(machine.canCompose).toBe(false);
    }
  });
  it('admits pause once and waits for the Runtime waiting snapshot before enabling continue', async () => {
    const { machine, transport, update } = fixture();
    update({ runtimeState: 'thinking', awaitingAssistantResponse: true });
    expect(machine.canPause).toBe(true);
    expect((await machine.command('pause')).accepted).toBe(true);
    expect(machine.phase).toBe('pausing');
    expect(machine.canCompose).toBe(false);
    expect((await machine.command('pause')).accepted).toBe(false);
    expect(transport.pause).toHaveBeenCalledOnce();
    update({ runtimeState: 'stopping' });
    expect(machine.phase).toBe('pausing');
    update({ runtimeState: 'waiting', awaitingAssistantResponse: false });
    expect(machine.phase).toBe('paused');
    expect(machine.canResume).toBe(true);
    await machine.command('resume');
    expect(machine.phase).toBe('resuming');
    expect(machine.canCompose).toBe(false);
    expect((await machine.command('resume')).accepted).toBe(false);
    update({ runtimeState: 'thinking', awaitingAssistantResponse: true });
    expect(machine.phase).toBe('thinking');
    update({ runtimeState: 'waiting', awaitingAssistantResponse: false });
    expect(machine.phase).toBe('ready');
  });
  it('allows retry when pause is rejected or throws', async () => {
    const { machine, transport, update, error } = fixture();
    update({ runtimeState: 'executing' });
    transport.pause.mockResolvedValueOnce({ accepted: false });
    await machine.command('pause');
    expect(machine.canPause).toBe(true);
    transport.pause.mockRejectedValueOnce(new Error('unavailable'));
    expect((await machine.command('pause')).accepted).toBe(false);
    expect(error).toHaveBeenCalledWith('unavailable');
    expect(machine.canPause).toBe(true);
  });
  it('keeps waiting approvals out of send and never resolves them implicitly', () => {
    const { machine, transport, update } = fixture();
    update({
      pendingPermissions: [
        {
          conversation_id: 'one',
          agent_id: 'agent',
          tool_call_id: 'call',
          tool_name: 'Write',
          display_name: 'Write',
          effect: 'controlled_change',
        },
      ],
    });
    expect(machine.phase).toBe('approval');
    expect(machine.canCompose).toBe(false);
    expect(machine.canConfigure).toBe(false);
    expect(machine.canPause).toBe(true);
    expect(transport.send).not.toHaveBeenCalled();
  });
  it('rejects duplicate commands and ignores their completion after switching conversation', async () => {
    const { machine, transport, update } = fixture();
    let finish!: (r: CommandResult) => void;
    transport.pause.mockImplementationOnce(
      () => new Promise((resolve) => (finish = resolve)),
    );
    update({ runtimeState: 'thinking' });
    const operation = machine.command('pause');
    expect((await machine.command('pause')).accepted).toBe(false);
    update({ conversationId: 'two', runtimeState: 'waiting' });
    finish({ accepted: true });
    await operation;
    expect(machine.phase).toBe('ready');
    expect(machine.canCompose).toBe(true);
  });
  it('applies only host-supported effort and permission values to the current conversation', async () => {
    let settings: ConversationComposerSettings = {
      supportedEfforts: ['low', 'high'],
      effort: 'low',
      permissionMode: 'ask',
      permissionModes: [
        { id: 'ask', label: 'Ask' },
        { id: 'read', label: 'Read only' },
      ],
    };
    const controller = {
      getSettings: vi.fn(async () => ({ ...settings })),
      setEffort: vi.fn(async (request: any) => {
        settings = { ...settings, effort: request.effort };
        return { accepted: true };
      }),
      setPermissionMode: vi.fn(async (request: any) => {
        settings = { ...settings, permissionMode: request.mode };
        return { accepted: true };
      }),
    };
    const { machine, update } = fixture(controller);
    await machine.load();
    await machine.configure('effort', 'max');
    expect(controller.setEffort).not.toHaveBeenCalled();
    await machine.configure('effort', 'high');
    expect(controller.setEffort).toHaveBeenCalledWith({
      conversationId: 'one',
      modelUid: 1,
      effort: 'high',
    });
    expect(machine.settings.effort).toBe('high');
    await machine.configure('permission', 'read');
    expect(machine.settings.permissionMode).toBe('read');
    update({ runtimeState: 'executing' });
    await machine.configure('effort', 'low');
    expect(controller.setEffort).toHaveBeenCalledOnce();
  });
  it('retains confirmed settings on rejected updates and ignores stale model readbacks', async () => {
    const pending = new Map<
      number,
      (s: ConversationComposerSettings) => void
    >();
    const controller = {
      getSettings: vi.fn(
        ({ modelUid }: any) =>
          new Promise<ConversationComposerSettings>((resolve) =>
            pending.set(modelUid, resolve),
          ),
      ),
      setEffort: vi.fn(async () => ({ accepted: false, rejectReason: 'busy' })),
    };
    const { machine, update, error } = fixture(controller);
    update({ modelUid: 2 });
    pending.get(2)!({ supportedEfforts: ['low', 'high'], effort: 'high' });
    await Promise.resolve();
    pending.get(1)!({ effort: 'low' });
    await Promise.resolve();
    expect(machine.settings.effort).toBe('high');
    await machine.configure('effort', 'low');
    expect(machine.settings.effort).toBe('high');
    expect(error).toHaveBeenCalledWith('busy');
  });
  it('disables resume when the host has no resume capability', () => {
    const { machine } = fixture();
    const state = {
      ...createConversationState('one'),
      connection: 'connected' as const,
      initialized: true,
      snapshotReceived: true,
    };
    machine.sync(
      state,
      {
        contract: TRANSPORT_CONTRACT,
        id: 'missing',
        connect: vi.fn(),
        send: vi.fn(),
      },
      null,
      1,
    );
    machine.paused = true;
    expect(machine.phase).toBe('paused');
    expect(machine.canResume).toBe(false);
  });
});
