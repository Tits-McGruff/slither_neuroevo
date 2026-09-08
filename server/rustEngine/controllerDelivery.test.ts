import { describe, expect, it } from 'vitest';
import type { RustBackgroundControllerMessage, RustGenerationAssignmentReceipt } from '../../src/protocol/rustBackground.ts';
import { ControllerDeliveryRouter } from './controllerDelivery.ts';

/** Rust-owned fixture with distinct wire, snake, lease, connection, and event identities. */
function observation(eventSequence = '0000000000000009'): RustBackgroundControllerMessage {
  return {
    operationEpoch: '0000000000000007', eventSequence,
    connectionId: '0000000000000003', leaseId: '0000000000000005',
    controllerKind: 'reinforcementLearning', internalSnakeId: '0000000000001000', snakeId: 12,
    sourceCompletedStep: '0000000000000000', kind: 'observation',
    sensors: Array.from({ length: 83 }, (_, i) => i / 100), x: 5, y: 8, direction: 0.25
  };
}

describe('Rust ordinary controller transport', () => {
  it('retains exact receipts under partial input admission without repeating sends', () => {
    const sent: unknown[] = [];
    const submitted: Array<{ sequence: string; receipt: RustGenerationAssignmentReceipt }> = [];
    let next = 20n;
    let available = 1;
    const router = new ControllerDeliveryRouter(2, {
      send(connection, message) { sent.push({ connection, message }); return true; },
      nextSequence() { return (next++).toString(16).padStart(16, '0'); },
      trySubmitReceipt(sequence, receipt) {
        submitted.push({ sequence, receipt: { ...receipt } });
        if (!available) return false;
        available--;
        return true;
      }
    });
    const first = observation();
    const replacement = { ...observation('000000000000000a'), kind: 'replacementAssignment' as const,
      resumeToken: 'rust-issued-token', snakeId: 16_777_216 };
    expect(router.deliver([first, replacement])).toBe(true);
    expect(router.blocked).toBe(true);
    expect(router.deliver([first])).toBe(false);
    expect(router.flushReceipts()).toBe(false);
    expect(sent).toEqual([
      { connection: first.connectionId, message: { type: 'sensors', tick: 0, snakeId: 12,
        sensors: first.sensors, meta: { x: 5, y: 8, dir: 0.25 } } },
      { connection: first.connectionId, message: { type: 'assign', snakeId: 16_777_216,
        controller: 'bot', resumeToken: 'rust-issued-token' } }
    ]);
    available = 1;
    expect(router.flushReceipts()).toBe(true);
    expect(router.blocked).toBe(false);
    expect(sent).toHaveLength(2);
    expect(submitted.slice(1).every(item => item.sequence === '0000000000000015')).toBe(true);
    expect(next).toBe(22n);
  });

  it('rejects the whole malformed or oversized batch before sending or retaining receipts', () => {
    let sends = 0;
    const router = new ControllerDeliveryRouter(2, {
      send() { sends++; return true; }, nextSequence() { throw new Error('unexpected sequence'); },
      trySubmitReceipt() { throw new Error('unexpected receipt'); }
    });
    expect(() => router.deliver([observation(), { ...observation(), sensors: [NaN] }])).toThrow();
    expect(() => router.deliver([observation(), observation(), observation()])).toThrow();
    expect(() => router.deliver([{ ...observation(), sourceCompletedStep: '0020000000000000' }])).toThrow();
    expect(sends).toBe(0);
    expect(router.blocked).toBe(false);
  });

  it('reports failed sends once and refuses reentrant delivery or receipt flush', () => {
    const accepted: boolean[] = [];
    let sequence = 0n;
    let router: ControllerDeliveryRouter;
    router = new ControllerDeliveryRouter(2, {
      send() {
        expect(router.deliver([observation()])).toBe(false);
        expect(router.flushReceipts()).toBe(false);
        throw new Error('closed socket');
      },
      nextSequence() { return (++sequence).toString(16).padStart(16, '0'); },
      trySubmitReceipt(_sequence, receipt) { accepted.push(receipt.accepted); return true; }
    });
    expect(router.deliver([observation()])).toBe(true);
    expect(accepted).toEqual([false]);
    expect(router.flushReceipts()).toBe(true);
    expect(accepted).toHaveLength(1);
  });
});
