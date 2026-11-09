import { describe, expect, it, vi } from 'vitest';
import { HttpSseConversationTransport } from '../src/transport/http-sse.js';

describe('HTTP resume transport', () => {
  it('keeps resume opt-in and preserves Runtime rejection for retry', async () => {
    const fetch = vi.fn(async () => Response.json({
      decision: 'rejected', reason: 'not_paused', command_id: 'resume-1',
    }));
    expect(new HttpSseConversationTransport({ fetch }).resume).toBeUndefined();
    const transport = new HttpSseConversationTransport({
      baseUrl: 'http://fixture.invalid', resumePath: '/api/resume', fetch,
    });
    expect(await transport.resume!({ conversationId: 'one' })).toMatchObject({
      accepted: false, rejectReason: 'not_paused', commandId: 'resume-1',
    });
    expect(fetch).toHaveBeenCalledWith('http://fixture.invalid/api/resume', expect.objectContaining({
      method: 'POST', body: JSON.stringify({ conversation_id: 'one' }),
    }));
  });
});
