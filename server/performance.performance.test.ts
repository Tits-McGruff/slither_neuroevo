import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { performance } from 'node:perf_hooks';
import { expect, it } from 'vitest';
import { createExperimentalServerRuntime } from './rustEngine/experimentalStartup.ts';

it('measures committed native fixed steps without a TypeScript simulation or layer bridge', async () => {
  const root = await mkdtemp(join(tmpdir(), 'slither-native-performance-'));
  const owner = await createExperimentalServerRuntime({ databasePath: join(root, 'metadata.sqlite'),
    managedDirectory: join(root, 'checkpoints'), calculationWorkers: 1, seed: 771, onWake: () => {} });
  try {
    const started = performance.now();
    owner.runtime.start();
    while (BigInt(`0x${owner.runtime.health().completedStep}`) < 60n && performance.now() - started < 6000) {
      await new Promise<void>(resolve => setTimeout(resolve, 10));
    }
    const health = owner.runtime.health();
    const completed = Number(BigInt(`0x${health.completedStep}`));
    console.info('[performance.native.fixed-steps]', { completed, elapsedMs: performance.now() - started,
      calculationWorkers: health.calculationWorkers });
    expect(health.faultCode).toBeUndefined();
    expect(completed).toBeGreaterThanOrEqual(60);
  } finally { await owner.close(); await rm(root, { recursive: true, force: true }); }
}, 15_000);
