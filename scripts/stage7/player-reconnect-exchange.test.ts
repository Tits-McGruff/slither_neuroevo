import { describe, expect, it } from 'vitest';
import { PlayerReconnectExchange } from './player-reconnect-exchange.ts';

/** Actual Protocol 2 assignment shape with a distinct opaque token. */
const ASSIGN = { type: 'assign', snakeId: 7, controller: 'player', resumeToken: 'B'.repeat(32), reclaimed: true };
/** Successful reply matching the original snake. */
const RESULT = { type: 'reclaimResult', reclaimed: true, reason: 'reclaimed', snakeId: 7 };

describe('player reconnect exchange correlation', () => {
  it('requires the real result and matching rotated assignment in either delivery order', () => {
    for (const packets of [[RESULT, ASSIGN], [ASSIGN, RESULT]]) {
      const exchange = new PlayerReconnectExchange(7, 'A'.repeat(32));
      exchange.consume(packets[0]!);
      expect(exchange.ready).toBe(false);
      exchange.consume(packets[1]!);
      expect(exchange.record.outcome).toBe('sameSnakeReclaim');
      expect(exchange.record.packets).toHaveLength(2);
      expect(JSON.stringify(exchange.record)).not.toContain('A'.repeat(32));
      expect(JSON.stringify(exchange.record)).not.toContain('B'.repeat(32));
    }
  });
  it('preserves the exact offending packets when identity or token rotation is wrong', () => {
    for (const assignment of [{ ...ASSIGN, snakeId: 8 }, { ...ASSIGN, resumeToken: 'A'.repeat(32) }]) {
      const exchange = new PlayerReconnectExchange(7, 'A'.repeat(32));
      exchange.consume(RESULT);
      expect(() => exchange.consume(assignment)).toThrow();
      expect(exchange.record.failure).toBeDefined();
      expect(exchange.record.packets).toHaveLength(2);
      expect(exchange.ready).toBe(false);
    }
    const changed = new PlayerReconnectExchange(7, 'A'.repeat(32));
    changed.consume({ ...RESULT, snakeId: 8 });
    expect(() => changed.consume({ ...ASSIGN, snakeId: 8 })).toThrow(/changed the requested snake/u);
  });
  it('requests a fresh join once after rejection and never counts it as same-snake reclaim', () => {
    const exchange = new PlayerReconnectExchange(7, 'A'.repeat(32));
    expect(exchange.consume({ type: 'reclaimResult', reclaimed: false, reason: 'invalid' })).toBe(true);
    expect(exchange.ready).toBe(false);
    expect(exchange.consume({ ...ASSIGN, snakeId: 8, reclaimed: false })).toBe(false);
    expect(exchange.record).toMatchObject({ freshJoinRequested: true, outcome: 'freshAfterRejectedReclaim' });
    expect(exchange.record.packets[0]).toMatchObject({ reclaimed: false, reason: 'invalid' });
  });
  it('correlates a legacy reply only after the explicit token-free fallback', () => {
    const exchange = new PlayerReconnectExchange(7, 'A'.repeat(32));
    exchange.consume({ type: 'reclaimResult', reclaimed: false, reason: 'snake-unavailable' });
    exchange.consume({ ...RESULT, snakeId: 8 });
    exchange.consume({ ...ASSIGN, snakeId: 8 });
    expect(exchange.record.outcome).toBe('freshAfterRejectedReclaim');
    expect(exchange.record.packets).toHaveLength(3);
  });
  it('rejects duplicate replies and cannot turn a later replacement into a reclaim', () => {
    const duplicate = new PlayerReconnectExchange(7, 'A'.repeat(32));
    duplicate.consume(RESULT);
    expect(() => duplicate.consume(RESULT)).toThrow(/duplicate/u);
    const settled = new PlayerReconnectExchange(7, 'A'.repeat(32));
    settled.consume(RESULT); settled.consume(ASSIGN);
    settled.consume({ ...ASSIGN, snakeId: 8, reclaimed: false });
    expect(settled.record.outcome).toBe('sameSnakeReclaim');
    expect(settled.record.packets[1]?.snakeId).toBe(7);
    expect(() => settled.consume(RESULT)).toThrow(/unsolicited/u);
  });
});
