/** Real release-addon worker panic through the production HTTP/WebSocket router. */
import { createRequire } from 'node:module';
import { once } from 'node:events';
import { mkdtemp, readdir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { expect, it, vi } from 'vitest';
import WebSocket from 'ws';
import Database from 'better-sqlite3';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer, type RustServer } from './rustServer.ts';
import { validateExperimentalFreshRunBinding } from './rustEngine/experimentalFreshRunSession.ts';
import { computeNativeSourceIdentity } from './rustEngine/nativeSourceIdentity.ts';
import { configurePanicFixture, loadPanicBinding, panicFixtureFrameSteps, startPreparedPanicRuntime } from './test/panicRuntime.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import type { RustBackgroundHealth } from '../src/protocol/rustBackground.ts';

vi.mock('./rustEngine/experimentalStartup.ts', async importOriginal => {
  const original = await importOriginal<typeof import('./rustEngine/experimentalStartup.ts')>();
  const fixture = await import('./test/panicRuntime.ts');
  return { ...original, createExperimentalServerRuntime: fixture.createPanicTestRuntime };
});

/** CommonJS production addon loader, independent of the feature addon fixture. */
const require = createRequire(import.meta.url);
/** Health fields added by the actual HTTP router to the native protocol. */
interface Health extends RustBackgroundHealth {
  /** Honest success/fault status. */
  ok: boolean;
  /** Active durable lineage. */
  runId: string;
  /** Checkpoint selected before any failed step. */
  startupCheckpointId: string;
  /** Native or interface fault reported to the browser. */
  interfaceFault?: string;
}

/** Poll a real HTTP outcome within the existing five-second integration deadline. */
async function healthUntil(port: number, predicate: (health: Health) => boolean): Promise<Health> {
  const deadline = performance.now() + 5000;
  let health: Health | undefined;
  do {
    health = await (await fetch(`http://127.0.0.1:${port}/api/health`)).json() as Health;
    if (predicate(health)) return health;
    await new Promise<void>(done => setTimeout(done, 10));
  } while (performance.now() < deadline);
  throw new Error(`HTTP condition not reached: ${JSON.stringify(health)}`);
}

/** Inspect the real current pointer and row counts independently from the persistence worker. */
function durableState(databasePath: string): unknown {
  const db = new Database(databasePath, { readonly: true });
  try {
    return {
      current: db.prepare('SELECT * FROM rust_checkpoint_v3_current ORDER BY run_id').all(),
      checkpoints: db.prepare('SELECT * FROM rust_checkpoint_v3_metadata ORDER BY checkpoint_id').all()
    };
  } finally { db.close(); }
}

describeNetworkSuite('Rust server caught calculation panic', () => {
  it('keeps production provenance strict and exposes no production panic trigger', () => {
    const production = require(resolve('native/index.js')) as {
      nativeAddonBuildClass(): string;
      ExperimentalRunningAuthority: { prototype: Record<string, unknown> };
    };
    expect(production.nativeAddonBuildClass()).toBe('production');
    expect(production.ExperimentalRunningAuthority.prototype['armCalculationPanicForTest']).toBeUndefined();
    const hooks = loadPanicBinding();
    expect(() => validateExperimentalFreshRunBinding(hooks, computeNativeSourceIdentity(resolve('native'))))
      .toThrow(/production build class/);
  });

  it('contains a real Rayon panic, preserves health and its committed save, and restarts exactly', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-server-panic-'));
    const dbPath = join(root, 'metadata.sqlite');
    let server: RustServer | undefined;
    let peer: WebSocket | undefined;
    try {
      configurePanicFixture(true);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        dbPath, rustCalculationWorkers: 2 });
      expect(server.startupFault).toBeUndefined();
      const before = await healthUntil(server.port, value => value.ok && value.completedStep === '0000000000000000');
      const databaseBefore = durableState(dbPath);
      const filesBefore = (await readdir(`${dbPath}.checkpoints`)).sort();
      const checkpointPath = join(`${dbPath}.checkpoints`, `${before.startupCheckpointId}.checkpoint-v3`);
      const checkpointBefore = await readFile(checkpointPath);
      const messages: Array<Record<string, unknown>> = [];
      let frames = 0;
      peer = new WebSocket(`ws://127.0.0.1:${server.port}`);
      peer.on('message', (data, binary) => {
        if (binary) frames++;
        else messages.push(JSON.parse(data.toString()) as Record<string, unknown>);
      });
      await new Promise<void>((done, reject) => { peer?.once('open', done); peer?.once('error', reject); });
      peer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
      const welcomeDeadline = performance.now() + 5000;
      while (!messages.some(message => message['type'] === 'welcome') && performance.now() < welcomeDeadline) {
        await new Promise<void>(done => setTimeout(done, 10));
      }
      expect(messages.some(message => message['type'] === 'welcome')).toBe(true);
      peer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      // WebSocket messages and ping are parsed in order. Its pong proves that
      // the real hub has processed join before the native failure can broadcast.
      const joined = once(peer, 'pong', { signal: AbortSignal.timeout(5000) });
      peer.ping();
      await joined;
      messages.length = 0;
      frames = 0;
      const actual = startPreparedPanicRuntime();
      const faulted = await healthUntil(server.port, value => !value.ok);
      expect(faulted.interfaceFault).toMatch(/inference.*partition 1|partition 1.*inference/);
      expect(faulted).toMatchObject({ runId: before.runId,
        startupCheckpointId: before.startupCheckpointId, completedStep: before.completedStep,
        generation: before.generation, schedulerCompletedSteps: '0000000000000000' });
      expect((await fetch(`http://127.0.0.1:${server.port}/api/health`)).status).toBe(503);
      expect(() => actual.armCalculationPanicForTest()).toThrow(/before coordinator start/);
      expect(() => actual.submitVisualization('0000000000000001', true)).toThrow();
      peer.send(JSON.stringify({ type: 'settings', requestId: 'after-panic',
        updates: [{ path: 'simSpeed', value: 2 }] }));
      const rejectionDeadline = performance.now() + 5000;
      while (!messages.some(message => message['type'] === 'settingsApplied' &&
          message['requestId'] === 'after-panic') &&
          performance.now() < rejectionDeadline) {
        await new Promise<void>(done => setTimeout(done, 10));
      }
      expect(messages.find(message => message['type'] === 'settingsApplied' &&
        message['requestId'] === 'after-panic')).toMatchObject({
        applied: false, updates: [], reason: faulted.interfaceFault
      });
      await new Promise<void>(done => setTimeout(done, 100));
      // The valid step-zero startup frame may precede the failed calculation.
      // Every frame copied by the real router must still describe that boundary.
      expect(panicFixtureFrameSteps().length).toBeGreaterThanOrEqual(frames);
      expect(panicFixtureFrameSteps().every(step => step === before.completedStep)).toBe(true);
      expect(actual.latestDisplay()?.completedStep).toBe(before.completedStep);
      expect(messages.some(message => message['type'] === 'error')).toBe(true);
      expect(messages.filter(message => message['type'] === 'stats')
        .every(message => message['tick'] === 0)).toBe(true);
      expect(messages.some(message => (message['type'] === 'settingsApplied' && message['applied'] === true) ||
        (message['type'] === 'newRunResult' && message['applied'] === true))).toBe(false);
      expect(durableState(dbPath)).toEqual(databaseBefore);
      expect((await readdir(`${dbPath}.checkpoints`)).sort()).toEqual(filesBefore);
      expect(await readFile(checkpointPath)).toEqual(checkpointBefore);
      peer.terminate(); peer = undefined;
      await server.close(); server = undefined;
      configurePanicFixture(false);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'latest',
        dbPath, rustCalculationWorkers: 2 });
      expect(server.startupFault).toBeUndefined();
      startPreparedPanicRuntime();
      const resumed = await healthUntil(server.port, value => value.ok && BigInt(`0x${value.completedStep}`) >= 2n);
      expect(resumed).toMatchObject({ runId: before.runId, startupCheckpointId: before.startupCheckpointId,
        generation: before.generation });
      expect(durableState(dbPath)).toEqual(databaseBefore);
    } finally {
      peer?.terminate();
      try { await server?.close(); }
      finally {
        configurePanicFixture(false);
        await rm(root, { recursive: true, force: true });
      }
    }
  }, 15_000);
});
