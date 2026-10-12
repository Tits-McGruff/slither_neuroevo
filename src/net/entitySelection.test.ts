import { describe, expect, it, vi } from 'vitest';
import { createEntitySelectionClient } from './entitySelection.ts';
import type { EntitySelectedMsg, SelectEntityMsg } from '../protocol/entitySelection.ts';

/** Native scalar reply with explicit full-width life identity. */
function reply(request: SelectEntityMsg, id = 7, token = '1'.repeat(64)): EntitySelectedMsg {
  return { type: 'entitySelected', requestId: request.requestId, found: true,
    entity: { token, snakeId: id, populationSlot: 0, lifeOrdinal: '0000000000000001', x: 2, y: 3, radius: 4, skin: 0 } };
}
/** Minimal isolated selection transport with no native or socket fallback. */
function fixture() {
  const requests: SelectEntityMsg[] = [];
  const kill = vi.fn(); const changed = vi.fn();
  let follow = true;
  const client = createEntitySelectionClient({ send: request => requests.push(request), kill, changed,
    followIndividual: () => follow });
  client.setScope({ sessionId: 'session', runId: 'run', generation: 1 });
  return { client, requests, kill, changed, disableFollow: () => { follow = false; } };
}

describe('native physical selection presentation', () => {
  it('waits for the exact native token before killing and ignores superseded clicks', () => {
    const f = fixture(); f.client.kill(7);
    expect(f.kill).not.toHaveBeenCalled();
    f.client.select(8); f.client.select(9);
    expect(f.requests).toHaveLength(1);
    f.client.receive(reply(f.requests[0]!));
    expect(f.kill).not.toHaveBeenCalled();
    expect(f.requests).toHaveLength(2);
    expect(f.requests[1]).toMatchObject({ snakeId: 9 });
    f.client.receive(reply(f.requests[1]!, 9));
    f.client.kill(9);
    expect(f.kill).toHaveBeenCalledExactlyOnceWith(9, '1'.repeat(64));
  });
  it('reacquires by population slot even when a replacement reuses its frame ID', () => {
    const f = fixture(); f.client.select(7); f.client.receive(reply(f.requests[0]!));
    f.client.observeFrame(1, () => true, 0);
    expect(f.requests[1]).toMatchObject({ populationSlot: 0 });
    expect(f.requests[1]).not.toHaveProperty('snakeId');
    f.client.receive(reply(f.requests[1]!, 7, '2'.repeat(64)));
    f.client.kill(7);
    expect(f.kill).toHaveBeenCalledExactlyOnceWith(7, '2'.repeat(64));
    f.client.observeFrame(1, () => true, 249);
    expect(f.requests).toHaveLength(2);
    f.client.observeFrame(1, () => true, 250);
    expect(f.requests).toHaveLength(3);
  });
  it('retains one followed owner while waiting, then accepts its next physical life', () => {
    const f = fixture(); f.client.select(7); f.client.receive(reply(f.requests[0]!));
    f.client.observeFrame(1, () => false, 0);
    f.client.receive({ type: 'entitySelected', requestId: f.requests[1]!.requestId, found: false });
    expect(f.changed).toHaveBeenLastCalledWith(null);
    expect(f.client.followedPopulationSlot()).toBe(0);
    f.client.observeFrame(1, () => false, 250);
    f.client.receive(reply(f.requests[2]!, 11, '2'.repeat(64)));
    expect(f.changed).toHaveBeenLastCalledWith(expect.objectContaining({ snakeId: 11, populationSlot: 0 }));
  });
  it('retires physical selection when a reused frame ID resolves to another life', () => {
    const f = fixture(); f.disableFollow(); f.client.select(7); f.client.receive(reply(f.requests[0]!));
    f.client.observeFrame(1, () => true, 0);
    expect(f.requests[1]).toMatchObject({ snakeId: 7 });
    f.client.receive(reply(f.requests[1]!, 7, '2'.repeat(64)));
    expect(f.changed).toHaveBeenLastCalledWith(null);
    expect(f.client.followedPopulationSlot()).toBeNull();
  });

  it('clears physical following when the preference is off and the entity disappears', () => {
    const f = fixture(); f.client.select(7); f.client.receive(reply(f.requests[0]!)); f.disableFollow();
    f.client.observeFrame(1, () => false, 0);
    expect(f.changed).toHaveBeenLastCalledWith(null);
    expect(f.requests).toHaveLength(1);
    f.client.observeFrame(1, () => false, 250);
    expect(f.requests).toHaveLength(1);
  });
  it.each(['session', 'run', 'generation', 'disconnect'])('rejects pending replies after %s replacement', change => {
    const f = fixture(); f.client.kill(7);
    if (change === 'disconnect') f.client.reset();
    else f.client.setScope({ sessionId: change === 'session' ? 'next' : 'session',
      runId: change === 'run' ? 'next' : 'run', generation: change === 'generation' ? 2 : 1 });
    f.client.receive(reply(f.requests[0]!));
    expect(f.kill).not.toHaveBeenCalled();
  });
  it('rejects invalid tokens and mismatched native selectors', () => {
    for (const invalid of ['0'.repeat(64), 'A'.repeat(64), 'f'.repeat(63)]) {
      const f = fixture(); f.client.kill(7); f.client.receive(reply(f.requests[0]!, 7, invalid));
      expect(f.kill).not.toHaveBeenCalled();
    }
    const f = fixture(); f.client.kill(7); f.client.receive(reply(f.requests[0]!, 8));
    expect(f.kill).not.toHaveBeenCalled();
  });
});
