import { describe, expect, it } from 'vitest';
import type { RustBackgroundControllerMessage, RustGenerationAssignmentReceipt, RustBackgroundReclaimAssignment, RustBackgroundReclaimReceipt } from '../../src/protocol/rustBackground.ts';
import { ControllerDeliveryRouter, ReclaimDeliveryRouter, GenerationDeliveryRouter } from './controllerDelivery.ts';

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

/** Same-snake reconnect emitted by Rust; the new token is already staged. */
const RECLAIM: RustBackgroundReclaimAssignment = {
  requestSequence: '0000000000000001', connectionId: '0000000000000002', leaseId: '0000000000000003',
  completedStep: '0000000000000000', snakeId: 4, controllerKind: 'player', resumeToken: 'A'.repeat(32)
};

describe('Rust reclaim transport', () => {
  it('validates both packets before sends and retries only the retained completion', () => {
    const sent: unknown[] = [];
    const receipts: RustBackgroundReclaimReceipt[] = [];
    let capacity = false;
    const router = new ReclaimDeliveryRouter({
      send(connectionId, message) { sent.push({ connectionId, message }); return true; },
      nextSequence() { return '0000000000000009'; },
      trySubmitReceipt(sequence, receipt) {
        expect(sequence).toBe('0000000000000009');
        receipts.push({ ...receipt });
        return capacity;
      }
    });
    expect(() => router.deliver({ ...RECLAIM, resumeToken: 'invalid' })).toThrow();
    expect(sent).toHaveLength(0);
    expect(router.deliver(RECLAIM)).toBe(true);
    expect(router.deliver(RECLAIM)).toBe(false);
    expect(router.flushReceipts()).toBe(false);
    capacity = true;
    expect(router.flushReceipts()).toBe(true);
    expect(sent).toEqual([
      { connectionId: RECLAIM.connectionId, message: { type: 'reclaimResult', reclaimed: true, reason: 'reclaimed', snakeId: 4 } },
      { connectionId: RECLAIM.connectionId, message: { type: 'assign', controller: 'player', reclaimed: true, snakeId: 4, resumeToken: RECLAIM.resumeToken } }
    ]);
    expect(receipts).toHaveLength(3);
    expect(receipts.every(receipt => receipt.accepted && receipt.requestSequence === RECLAIM.requestSequence)).toBe(true);
  });

  it('reports a failed first or second send once and guards recursive delivery', () => {
    for (const failureAt of [1, 2]) {
      let sends = 0;
      let completion: RustBackgroundReclaimReceipt | undefined;
      const router = new ReclaimDeliveryRouter({
        send() {
          sends++;
          expect(router.deliver(RECLAIM)).toBe(false);
          expect(router.flushReceipts()).toBe(false);
          if (sends === failureAt) throw new Error('socket closed');
          return true;
        },
        nextSequence() { return '0000000000000009'; },
        trySubmitReceipt(_sequence, receipt) { completion = { ...receipt }; return true; }
      });
      expect(router.deliver(RECLAIM)).toBe(true);
      expect(router.blocked).toBe(false);
      expect(sends).toBe(failureAt);
      expect(completion?.accepted).toBe(false);
    }
  });
});

describe('Rust generation assignment transport', () => {
  it('validates the whole batch, preserves public IDs, and retries receipts without resending', () => {
    let capacity = false;
    const sent: unknown[] = [];
    const receipts: RustGenerationAssignmentReceipt[] = [];
    const router = new GenerationDeliveryRouter(2, {
      send(connectionId, message) { sent.push({ connectionId, message }); return true; },
      nextSequence() { return '0000000000000001'; },
      trySubmitReceipt(_sequence, receipt) { receipts.push({ ...receipt }); return capacity; }
    });
    const assignment = {
      operationEpoch: '0000000000000002', eventSequence: '0000000000000003',
      connectionId: '0000000000000004', leaseId: '0000000000000005',
      snakeId: 'ffffffffffffffff', frameV1Id: '0000000000000006',
      controllerKind: 'player' as const, resumeToken: 'a'.repeat(32)
    };
    expect(() => router.deliverAssignments([assignment, { ...assignment, frameV1Id: '0000000001000001' }])).toThrow();
    expect(sent).toHaveLength(0);
    expect(router.deliverAssignments([assignment])).toBe(true);
    expect(router.deliverAssignments([assignment])).toBe(false);
    capacity = true;
    expect(router.flushReceipts()).toBe(true);
    expect(sent).toEqual([{ connectionId: assignment.connectionId, message: {
      type: 'assign', controller: 'player', snakeId: 6, resumeToken: assignment.resumeToken
    } }]);
    expect(receipts).toHaveLength(2);
    expect(receipts[0]).toEqual(receipts[1]);
    expect(receipts[0]).toMatchObject({ operationEpoch: assignment.operationEpoch, accepted: true });
  });
});

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
