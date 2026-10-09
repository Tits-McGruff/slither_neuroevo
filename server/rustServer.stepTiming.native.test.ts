import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { expect, it, vi } from 'vitest';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer } from './rustServer.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { summarizeStepWindow, type StepHistogram } from '../scripts/stage7/step-window-summary.ts';

/** Actual source-identified native class; production startup independently enforces its identity. */
const addon = createRequire(import.meta.url)('../native/index.js') as typeof import('../native/index.js');

/** Production health scalars required to establish one unchanged native owner. */
interface Health extends StepHistogram {
  /** True only while the authority is usable. */
  ok: boolean;
  /** Source-identified addon selected by production startup. */
  nativeBuildIdentifier: string;
  /** Durable active lineage. */
  runId: string;
}

/** Obtain a complete real HTTP histogram prefix within the existing five-second integration bound. */
async function observed(port: number, minimumSamples: bigint): Promise<Health> {
  const deadline = performance.now() + 5000;
  do {
    const response = await fetch(`http://127.0.0.1:${port}/api/health`, { signal: AbortSignal.timeout(1000) });
    expect(response.status).toBe(200);
    const health = await response.json() as Health;
    expect(health.ok).toBe(true);
    if (health.stepTimingHistogramConsistent && BigInt(`0x${health.stepTimingSamples}`) >= minimumSamples) return health;
    await new Promise<void>(done => setTimeout(done, 10));
  } while (performance.now() < deadline);
  throw new Error('complete native HTTP histogram prefix was not observed within five seconds');
}

describeNetworkSuite('Rust production step timing window', () => {
  it('publishes exact bounded prefixes through the real HTTP bridge and subtracts only new computations', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-step-window-'));
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    const originalHealth = addon.ExperimentalRunningAuthority.prototype.health;
    const healthSpy = vi.spyOn(addon.ExperimentalRunningAuthority.prototype, 'health')
      .mockImplementation(function(this: InstanceType<typeof addon.ExperimentalRunningAuthority>, include) {
        const result = originalHealth.call(this, include);
        if (include !== true) {
          expect(result.stepTimingBucketCounts).toBeUndefined();
          expect(result.stepTimingBucketUpperMicros).toBeUndefined();
          expect(result.stepTimingHistogramConsistent).toBe(false);
        }
        return result;
      });
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42,
        dbPath: join(root, 'fixture.sqlite') });
      expect(server.startupFault).toBeUndefined();
      const initial = await observed(server.port, 1n);
      const initialSamples = BigInt(`0x${initial.stepTimingSamples}`);
      const final = await observed(server.port, initialSamples + 16n);
      expect(final.runId).toBe(initial.runId);
      expect(final.nativeBuildIdentifier).toBe(initial.nativeBuildIdentifier);
      expect(final.stepTimingBucketCounts).toHaveLength(25);
      expect(final.stepTimingBucketUpperMicros).toContain('000000000000411b');
      const window = summarizeStepWindow(initial, final);
      expect(BigInt(window.samples)).toBe(BigInt(`0x${final.stepTimingSamples}`) - initialSamples);
      expect(BigInt(window.samples)).toBeGreaterThanOrEqual(16n);
      expect(window.bucketCounts.reduce((sum, count) => sum + BigInt(count), 0n)).toBe(BigInt(window.samples));
      expect(window.p99UpperMs).not.toBeNull();
      expect(healthSpy.mock.calls.some(([include]) => include === true)).toBe(true);
      expect(healthSpy.mock.calls.some(([include]) => include === undefined)).toBe(true);
    } finally {
      await server?.close();
      healthSpy.mockRestore();
      await rm(root, { recursive: true, force: true });
    }
  }, 10_000);
});
