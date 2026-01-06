import { describe, it, expect } from 'vitest';
import { parseClientMessage } from './protocol.ts';

/** Test suite label for server protocol validation. */
const SUITE = 'server protocol';

describe(SUITE, () => {
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

  it('accepts a valid view message', () => {
    const msg = parseClientMessage({
      type: 'view',
      viewW: 1280,
      viewH: 720,
      mode: 'overview'
    });
    expect(msg?.type).toBe('view');
  });

  it('accepts a valid viz message', () => {
    const msg = parseClientMessage({
      type: 'viz',
      enabled: true
    });
    expect(msg?.type).toBe('viz');
  });
});
