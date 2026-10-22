import { expect, it } from 'vitest';
import { createHash, randomBytes } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { loadPanicBinding } from '../test/panicRuntime.ts';
import { parseRustStartupMetadata } from './startupMetadata.ts';
import type { ExperimentalFreshRunNativeHandle } from './experimentalFreshRunSession.ts';

/** Test-only admission remains a complete coarse Rust construction request. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Explicit P0/P1 select the same modest native graph as the sensor benchmark. */
  initializeBaselineV2ForTest(request: {profile: string; bins: number; mode: number; cadence: number;
    rayMinimum: number; rayCap: number; rayBatch: number; workload?: string; trainedCohort?: string}): Promise<{trainedCohortJson?: string}>;
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


/** Restore two real learned policies as opaque bytes, with bounded checksummed decompression. */
function learnedFamilies(root: string): {file: string; sha256: string}[] {
  const study = resolve('docs/performance/sensor-v4/replicated-ablation-windows-v1');
  const evidence = JSON.parse(readFileSync(join(study, 'evidence.json'), 'utf8')) as {
    files: {path: string; archive: string; bytes: number; sha256: string; gzipSha256: string}[];
  };
  return [1, 2].map(seed => {
    const receipt = evidence.files.find(file => file.path === 'seed-' + seed + '-fold-2/full/trained.slither-brain')!;
    expect(receipt.archive).toBe('objects/' + receipt.sha256 + '.gz');
    const encoded = readFileSync(join(study, receipt.archive));
    expect(encoded.byteLength).toBeLessThan(256 * 1024);
    expect(createHash('sha256').update(encoded).digest('hex')).toBe(receipt.gzipSha256);
    const bytes = gunzipSync(encoded, {maxOutputLength: 256 * 1024});
    expect(bytes.byteLength).toBe(receipt.bytes);
    expect(createHash('sha256').update(bytes).digest('hex')).toBe(receipt.sha256);
    const file = 'learned-' + seed + '.slither-brain';
    writeFileSync(join(root, file), bytes, {flag: 'wx'});
    return {file, sha256: receipt.sha256};
  });
}

/** Verify the exact task-owned temporary root before recursive removal. */
function cleanupCohort(root: string): void {
  if (dirname(resolve(root)) !== resolve(tmpdir()) || !root.startsWith(join(tmpdir(), 'slither-live-cohort-'))) throw new Error('unsafe cohort cleanup');
  rmSync(root, {recursive: true, force: true});
}

it('cold constructs a real learned cohort, rejects corrupt sources and retries without publishing an authority', async () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-live-cohort-'));
  try {
    const artifacts = learnedFamilies(root); const index = join(root, 'cohort.json');
    const valid = JSON.stringify({schemaVersion: 1, artifacts});
    writeFileSync(index, JSON.stringify({schemaVersion: 1, artifacts: [
      {...artifacts[0], sha256: '0'.repeat(64)}, artifacts[1],
    ]}), {flag: 'wx'});
    const binding = loadPanicBinding();
    const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
    const session = new Constructor('learned-cohort-' + randomBytes(8).toString('hex'), '0000002a', '0000000100000000', 1);
    expect(() => session.initializeBaselineV2ForTest({...request, trainedCohort: index})).toThrow(/workload/u);
    await expect(session.initializeBaselineV2ForTest({...request, workload: 'P0', trainedCohort: index})).rejects.toThrow(/digest/u);
    expect(session.snapshot()).toMatchObject({phase: 'created'});
    writeFileSync(index, valid);
    const initialized = await session.initializeBaselineV2ForTest({...request, workload: 'P0', trainedCohort: index});
    const cohort = JSON.parse(initialized.trainedCohortJson!) as {
      indexSha256: string; sourceReservationBytes: number;
      families: {populationSlots: number; artifact: {artifactSha256: string; weightsSha256: string; parameterCount: number}}[];
    };
    expect(cohort.indexSha256).toBe(createHash('sha256').update(valid).digest('hex'));
    expect(cohort.sourceReservationBytes).toBeGreaterThan(0);
    expect(cohort.sourceReservationBytes).toBeLessThanOrEqual(512 * 1024 ** 2);
    expect(cohort.families.map(family => family.populationSlots)).toEqual([28, 27]);
    expect(cohort.families.map(family => family.artifact.artifactSha256)).toEqual(artifacts.map(artifact => artifact.sha256));
    expect(new Set(cohort.families.map(family => family.artifact.weightsSha256)).size).toBe(2);
    expect(cohort.families.every(family => family.artifact.parameterCount === 25042)).toBe(true);
    expect(parseRustStartupMetadata(session.startupMetadata()).parameterCount).toBe(25042);
    expect(session.snapshot()).toMatchObject({completedStep: '0000000000000000', checkpointPublished: false, authorityPublished: false});
  } finally {
    cleanupCohort(root);
  }
}, 15000);
