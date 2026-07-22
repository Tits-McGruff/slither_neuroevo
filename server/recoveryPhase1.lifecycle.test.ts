import { createServer, type Server } from 'node:http';
import { describe, expect, it, vi } from 'vitest';
import { DEFAULT_CONFIG } from './config.ts';
import { startServer } from './index.ts';
import { SimServer } from './simServer.ts';
import { networkTest } from './test/networkSuites.ts';

/** Phase 1 server lifecycle and authoritative-failure suite label. */
const SUITE = 'recovery Phase 1 — server lifecycle';

/** Private loop surface exercised as a component boundary. */
interface LoopAccess {
  /** Run one production loop iteration. */
  loop: () => Promise<void>;
}

/** Mutable running flag used to stop a component loop without a timer. */
interface RunningAccess {
  /** Whether another loop iteration may be scheduled. */
  running: boolean;
}

/**
 * Bind an HTTP server to a loopback ephemeral port.
 * @param server - Server to bind.
 * @returns Assigned TCP port.
 */
async function listenOnLoopback(server: Server): Promise<number> {
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject);
      resolve();
    });
  });
  const address = server.address();
  if (!address || typeof address === 'string') {
    throw new Error('expected a TCP address');
  }
  return address.port;
}

/**
 * Close a test HTTP server and await the close callback.
 * @param server - Server to close.
 */
async function closeServer(server: Server): Promise<void> {
  if (!server.listening) return;
  await new Promise<void>((resolve, reject) => {
    server.close((error) => {
      if (error) reject(error);
      else resolve();
    });
  });
}

describe(SUITE, () => {
  networkTest('fails the required lifecycle when its configured address is already bound', async () => {
    const occupied = createServer();
    const port = await listenOnLoopback(occupied);
    try {
      await expect(startServer({
        ...DEFAULT_CONFIG,
        host: '127.0.0.1',
        port,
        dbPath: ':memory:',
        logLevel: 'error'
      })).rejects.toMatchObject({ code: 'EADDRINUSE' });
    } finally {
      await closeServer(occupied);
    }
  });

  networkTest('observes MT initialization failure through awaited server startup', async () => {
    const initSpy = vi.spyOn(SimServer.prototype, 'initMT')
      .mockRejectedValue(new Error('phase1 MT initialization failure'));
    try {
      await expect(startServer({
        ...DEFAULT_CONFIG,
        port: 0,
        dbPath: ':memory:',
        logLevel: 'error',
        mtEnabled: true
      })).rejects.toThrow('phase1 MT initialization failure');
    } finally {
      initSpy.mockRestore();
    }
  });

  it('does not resolve stop until in-flight worker cleanup has completed', async () => {
    let resolveShutdown!: () => void;
    const shutdownGate = new Promise<void>((resolve) => {
      resolveShutdown = resolve;
    });
    const pool = {
      shutdown: vi.fn(async () => shutdownGate)
    };
    const server = Object.assign(Object.create(SimServer.prototype), {
      running: false,
      timer: null,
      loopPromise: null,
      brainPool: pool,
      core: { brainPool: pool }
    }) as unknown as SimServer;
    let stopped = false;

    const stopPromise = server.stop().then(() => {
      stopped = true;
    });
    await Promise.resolve();

    expect(pool.shutdown).toHaveBeenCalledOnce();
    expect(stopped).toBe(false);
    resolveShutdown();
    await stopPromise;
    expect(stopped).toBe(true);
  });

  it('faults after inference rejection without a new tick, frame, stats, or checkpoint', async () => {
    const broadcastFrame = vi.fn();
    const broadcastStats = vi.fn();
    const checkpoint = vi.fn();
    const update = vi.fn(async () => {
      (server as unknown as RunningAccess).running = false;
      throw new Error('phase1 inference rejection');
    });
    const server = Object.assign(Object.create(SimServer.prototype), {
      running: true,
      timer: null,
      loopPromise: null,
      nextTickAt: 0,
      tickRateHz: 60,
      uiFrameRateHz: 30,
      lastTickAt: 0,
      lastFrameSentAt: 0,
      lastStatsSentAt: 0,
      lastSchedulerDropLogAt: 0,
      mtEnabled: false,
      mtActive: false,
      faultReason: null,
      faultedAtTick: null,
      core: { tickId: 7, update },
      controllers: {
        setTickId: vi.fn(),
        reassignDeadSnakes: vi.fn()
      },
      wsHub: {
        hasFrameRecipients: () => true,
        broadcastFrame,
        broadcastStats
      },
      persistence: { savePopulationSnapshot: checkpoint }
    }) as unknown as SimServer;
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    try {
      await (server as unknown as LoopAccess).loop();
    } finally {
      errorSpy.mockRestore();
    }

    expect(update).toHaveBeenCalledOnce();
    expect(server.getTickId()).toBe(7);
    expect(server.getFaultStatus()).toEqual({
      faulted: true,
      reason: 'phase1 inference rejection',
      tick: 7
    });
    expect(broadcastFrame).not.toHaveBeenCalled();
    expect(broadcastStats).not.toHaveBeenCalled();
    expect(checkpoint).not.toHaveBeenCalled();
  });
});
