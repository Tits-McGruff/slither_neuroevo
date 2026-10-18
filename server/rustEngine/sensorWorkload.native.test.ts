import { expect, it } from 'vitest';
import { randomBytes } from 'node:crypto';
import { loadPanicBinding } from '../test/panicRuntime.ts';
import { parseRustStartupMetadata } from './startupMetadata.ts';
import type { ExperimentalFreshRunNativeHandle } from './experimentalFreshRunSession.ts';

/** Test-only admission remains a complete coarse Rust construction request. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Explicit P0/P1 select the same modest native graph as the sensor benchmark. */
  initializeBaselineV2ForTest(request: {profile: string; bins: number; mode: number; cadence: number;
    rayMinimum: number; rayCap: number; rayBatch: number; workload?: string}): Promise<unknown>;
}
/** Known explicit ray admission; these are fixture choices, not production defaults. */
const request = {profile: 'motion', bins: 16, mode: 1, cadence: 1 / 60, rayMinimum: 1, rayCap: 256, rayBatch: 65536};

for (const [workload, population, workers] of [['P0', 55, 1], ['P1', 300, 3]] as const) {
  it('admits the complete ' + workload + ' modest native workload with explicit ray admission', async () => {
    const binding = loadPanicBinding();
    const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
    const session = new Constructor('wall-fixture-' + randomBytes(8).toString('hex'), '0000002a', '0000000100000000', workers);
    expect(() => session.initializeBaselineV2ForTest({...request, workload: 'P2'})).toThrow(/workload/);
    expect(() => session.initializeBaselineV2ForTest({...request, workload, unknown: 1} as typeof request)).toThrow(/fields/);
    await session.initializeBaselineV2ForTest({...request, workload});
    const metadata = parseRustStartupMetadata(session.startupMetadata());
    const settings = new Map(metadata.settings.map(entry => [entry.path, entry.value]));
    expect(settings.get('snakeCount')).toBe(population);
    expect(settings.get('baselineBots.count')).toBe(10);
    expect(settings.get('evolvedRespawn.enabled')).toBe(true);
    expect(settings.get('generationSeconds')).toBe(604800);
    expect(settings.get('brain.controlDt')).toBe(1 / 60);
    expect(metadata).toMatchObject({parameterCount: 25042, sensorVersion: 4, fixedStepSeconds: 1 / 60, mathBackend: 'rust-sse2-v1'});
    expect(session.snapshot()).toMatchObject({completedStep: '0000000000000000', checkpointPublished: false, authorityPublished: false});
  }, 15000);
}
