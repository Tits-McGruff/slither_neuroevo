import { afterEach, expect, it, vi } from 'vitest';
import { createRequestId } from './requestId.ts';

afterEach(() => vi.unstubAllGlobals());

it('encodes every random byte as a fixed-width lowercase wire identity without randomUUID', () => {
  const getRandomValues = vi.fn((bytes: Uint8Array) => {
    bytes.set([0, 1, 2, 3, 4, 5, 6, 7, 248, 249, 250, 251, 252, 253, 254, 255]);
    return bytes;
  });
  vi.stubGlobal('crypto', {getRandomValues});
  expect(createRequestId()).toBe('0001020304050607f8f9fafbfcfdfeff');
  expect(getRandomValues).toHaveBeenCalledTimes(1);
  expect(getRandomValues.mock.calls[0]?.[0]).toHaveLength(16);
});

it('fails rather than issuing a weak identity when secure randomness is unavailable', () => {
  vi.stubGlobal('crypto', undefined);
  expect(() => createRequestId()).toThrow();
});
