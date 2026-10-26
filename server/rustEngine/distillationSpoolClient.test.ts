import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DistillationSpoolClient } from './distillationSpoolClient.ts';
import { distillationRecoveryTimeoutMs, distillationSpoolLimitsForGiB } from './distillationProtocol.ts';

/** Only the private worker lifecycle is controlled; real disk/worker coverage remains in integration. */
interface ControlledWorker {
  /** Deliver an exact worker event to the real client owner. */
  emit(event: string, ...args: unknown[]): boolean;
  /** Observe finite shutdown after timeout or cancellation. */
  terminated: boolean;
  /** Requests actually dispatched after readiness. */
  sent: unknown[];
}
/** One controlled worker per client, isolated by test cleanup. */
const controlled = vi.hoisted(() => ({workers: [] as ControlledWorker[]}));
vi.mock('node:worker_threads', async () => {
  const {EventEmitter} = await import('node:events');
  /** No filesystem or native authority is constructed by these lifecycle unit tests. */
  class Worker extends EventEmitter {
    /** Terminal lifecycle observation. */
    terminated = false;
    /** Exact cold requests received from the client. */
    sent: unknown[] = [];
    /** Retain the test's explicit event source. */
    constructor() { super(); controlled.workers.push(this); }
    /** Only graceful close completes automatically; other requests test their real deadline. */
    postMessage(message: unknown): void {
      this.sent.push(message);
      const request = message as {id: number; operation: string};
      if (request.operation === 'close') {
        this.emit('message', {kind: 'result', id: request.id, ok: true, result: null});
        queueMicrotask(() => this.emit('exit', 0));
      }
    }
    /** Simulate observed exit synchronously so shutdown cannot depend on a timer. */
    terminate(): Promise<number> { this.terminated = true; this.emit('exit', 1); return Promise.resolve(1); }
  }
  return {Worker};
});
/** Every owner is closed even after an assertion or initialization rejection. */
const clients: DistillationSpoolClient[] = [];
/** Construct a selected maximum quota without allocating its capacity. */
function fixture(): {client: DistillationSpoolClient; worker: ControlledWorker; outcome: Promise<void | Error>} {
  const client = new DistillationSpoolClient('controlled-spool', distillationSpoolLimitsForGiB(64));
  clients.push(client);
  return {client, worker: controlled.workers.at(-1)!, outcome: client.ready.catch(error => error as Error)};
}

beforeEach(() => vi.useFakeTimers({toFake: ['setTimeout', 'clearTimeout', 'performance']}));
afterEach(async () => {
  await Promise.all(clients.splice(0).map(client => client.close()));
  controlled.workers.length = 0; vi.useRealTimers();
});

describe('bounded spool recovery lifecycle', () => {
  it.each([{chunks: 0, bytes: 0}, {chunks: 65536, bytes: 4 * 1024 ** 3}])(
    'keeps the default deadline absolute for %j despite selecting a larger quota', async inventory => {
      const {client, worker, outcome} = fixture();
      await vi.advanceTimersByTimeAsync(1000);
      worker.emit('message', {kind: 'recoveryWork', inventory});
      expect(client.recoveryBudgetMs).toBe(60000);
      await vi.advanceTimersByTimeAsync(58999); expect(worker.terminated).toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(await outcome).toMatchObject({message: 'distillation spool startup timed out'});
      expect(worker.terminated).toBe(true);
    });

  it('prices the actual 64-GiB inventory once and stops at the absolute eleven-minute bound', async () => {
    const {client, worker, outcome} = fixture();
    await vi.advanceTimersByTimeAsync(1000);
    worker.emit('message', {kind: 'recoveryWork', inventory: {chunks: 65536, bytes: 64 * 1024 ** 3}});
    expect(client.recoveryBudgetMs).toBe(660000);
    await vi.advanceTimersByTimeAsync(58999); expect(worker.terminated).toBe(false);
    await vi.advanceTimersByTimeAsync(600000); expect(worker.terminated).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(await outcome).toMatchObject({message: 'distillation spool startup timed out'});
    expect(worker.terminated).toBe(true);
  });

  it.each([{chunks: 65537, bytes: 0}, {chunks: 65536, bytes: 64 * 1024 ** 3 + 1},
    {chunks: 1, bytes: 1024 ** 2 + 1}, {chunks: 1.5, bytes: 1}, {chunks: 1, bytes: 1, unknown: 1}])(
    'rejects malformed or impossible work without extending initialization: %j', async inventory => {
      const {client, worker, outcome} = fixture();
      worker.emit('message', {kind: 'recoveryWork', inventory});
      expect(await outcome).toBeInstanceOf(Error); expect(worker.terminated).toBe(true);
      expect(client.recoveryBudgetMs).toBe(60000);
    });

  it('rejects repeated work announcements instead of resetting the deadline', async () => {
    const {worker, outcome} = fixture();
    const message = {kind: 'recoveryWork', inventory: {chunks: 65536, bytes: 64 * 1024 ** 3}};
    worker.emit('message', message); await vi.advanceTimersByTimeAsync(300000);
    worker.emit('message', message);
    expect(await outcome).toMatchObject({message: 'invalid spool recovery inventory sequence'});
    expect(worker.terminated).toBe(true);
  });

  it('rejects a late inventory after readiness', async () => {
    const {client, worker} = fixture();
    worker.emit('message', {kind: 'ready'}); await client.ready;
    worker.emit('message', {kind: 'recoveryWork', inventory: {chunks: 0, bytes: 0}});
    await expect(client.storagePolicy()).rejects.toThrow('invalid spool recovery inventory sequence');
    expect(worker.terminated).toBe(true);
  });

  it('retains the separate sixty-second request deadline after a large recovery', async () => {
    const {client, worker} = fixture();
    worker.emit('message', {kind: 'recoveryWork', inventory: {chunks: 65536, bytes: 64 * 1024 ** 3}});
    await vi.advanceTimersByTimeAsync(100000);
    worker.emit('message', {kind: 'ready'}); await client.ready;
    const request = client.storagePolicy().catch(error => error as Error);
    await vi.advanceTimersByTimeAsync(59999); expect(worker.terminated).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(await request).toMatchObject({message: 'distillation spool request timed out'});
    expect(worker.terminated).toBe(true);
  });

  it('cancels initialization immediately rather than waiting for its extended budget', async () => {
    const {client, worker, outcome} = fixture();
    worker.emit('message', {kind: 'recoveryWork', inventory: {chunks: 65536, bytes: 64 * 1024 ** 3}});
    await client.close(); await client.close();
    expect(await outcome).toMatchObject({message: 'distillation spool closed during initialization'});
    expect(worker.terminated).toBe(true); expect(vi.getTimerCount()).toBe(0);
  });

  it('bounds metadata waiters before inventory recovery becomes ready', async () => {
    const {client, worker} = fixture();
    worker.emit('message', {kind: 'recoveryWork', inventory: {chunks: 65536, bytes: 64 * 1024 ** 3}});
    const waiting = Array.from({length: 32}, () => client.storagePolicy().catch(error => error as Error));
    await expect(client.storagePolicy()).rejects.toThrow('request admission exhausted');
    expect(worker.sent).toHaveLength(0);
    await client.close();
    for (const result of await Promise.all(waiting)) {
      expect(result).toMatchObject({message: 'distillation spool closed during initialization'});
    }
    expect(worker.terminated).toBe(true); expect(vi.getTimerCount()).toBe(0);
  });

  it('reserves empty graceful shutdown admission even when all data request slots are occupied', async () => {
    const {client, worker} = fixture();
    worker.emit('message', {kind: 'ready'}); await client.ready;
    const requests = Array.from({length: 32}, () => client.datasets('0'.repeat(32), 1));
    await Promise.resolve(); expect(worker.sent).toHaveLength(32);
    const closing = client.close();
    for (const request of worker.sent as Array<{id: number}>) {
      worker.emit('message', {kind: 'result', id: request.id, ok: true, result: {datasets: []}});
    }
    await closing; await Promise.all(requests);
    expect(worker.sent).toHaveLength(33); expect(vi.getTimerCount()).toBe(0);
  });

  it('bounds every admitted size and rejects negative or oversized inventories', () => {
    for (let gib = 1; gib <= 64; gib++) {
      expect(distillationRecoveryTimeoutMs({chunks: 65536, bytes: gib * 1024 ** 3}))
        .toBe(60000 + Math.max(0, gib - 4) * 10000);
    }
    expect(() => distillationRecoveryTimeoutMs({chunks: 1, bytes: -1})).toThrow();
  });
});
