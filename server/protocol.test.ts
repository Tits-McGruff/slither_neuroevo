import { describe, it, expect } from 'vitest';
import { parseClientMessage } from './protocol.ts';

describe('server protocol', () => {
  it('accepts a valid hello message', () => {
    const msg = parseClientMessage({ type: 'hello', clientType: 'ui', version: 1 });
    expect(msg?.type).toBe('hello');
  });

  it('rejects hello with NaN version', () => {
    const msg = parseClientMessage({
      type: 'hello',
      clientType: 'ui',
      version: Number.NaN
    });
    expect(msg).toBeNull();
  });

  it('accepts a valid action message', () => {
    const msg = parseClientMessage({
      type: 'action',
      tick: 12,
      snakeId: 2,
      turn: -0.4,
      boost: 1
    });
    expect(msg?.type).toBe('action');
  });
});
