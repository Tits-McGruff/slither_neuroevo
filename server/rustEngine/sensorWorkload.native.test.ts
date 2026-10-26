import { expect, it } from 'vitest';
import { createHash, randomBytes } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { loadPanicBinding } from '../test/panicRuntime.ts';
import { parseRustStartupMetadata } from './startupMetadata.ts';
import type { ExperimentalFreshRunNativeHandle } from './experimentalFreshRunSession.ts';
import { parseManagedCheckpointDescriptor, type ManagedCheckpointDescriptor } from './checkpointPersistenceProtocol.ts';
import { admitDiskOperation, checkpointDiskAdmissionRequest } from './diskAdmission.ts';

/** Test-only admission remains a complete coarse Rust construction request. */
interface QualifiedSession extends ExperimentalFreshRunNativeHandle {
  /** Explicit candidate envelope accompanies one complete native population admission. */
  initializeCapacityForTest(request: CapacityRequest): Promise<{capacityAuditJson: string}>;
  /** Restore a candidate boundary with exact source, sensor, backend and ray admission. */
  initializeSensorCheckpointForTest(directory: string, descriptor: ManagedCheckpointDescriptor,
    admission: {profile: string; bins: number; mathBackend: string; rayMinimum: number; rayCap: number;
      rayBatch: number; envelopeMiB?: number}): Promise<unknown>;
  /** Explicit P0/P1 select the same modest native graph as the sensor benchmark. */
  initializeBaselineV2ForTest(request: {profile: string; bins: number; mode: number; cadence: number;
    rayMinimum: number; rayCap: number; rayBatch: number; workload?: string; trainedCohort?: string;
    /** Optional paired food-detail bound within the existing setting range. */
    pelletChecks?: number;
    /** Optional paired body-detail bound within the existing setting range. */
    segmentChecks?: number}): Promise<{trainedCohortJson?: string}>;
}
/** Known explicit ray admission; these are fixture choices, not production defaults. */
const request = {profile: 'motion', bins: 16, mode: 1, cadence: 1 / 60, rayMinimum: 1, rayCap: 256, rayBatch: 65536};

it('admits explicit bounded query detail limits in actual native settings after rejected requests', async () => {
  const binding = loadPanicBinding();
  const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
  const session = new Constructor('query-limits-' + randomBytes(8).toString('hex'), '0000002a', '0000000100000000', 1);
  for (const limits of [{pelletChecks: 3000}, {segmentChecks: 4000}, {pelletChecks: 99, segmentChecks: 4000},
    {pelletChecks: 3001, segmentChecks: 4000}, {pelletChecks: 3000, segmentChecks: 199},
    {pelletChecks: 3000, segmentChecks: 4001}, {pelletChecks: NaN, segmentChecks: 4000},
    {pelletChecks: 900.5, segmentChecks: 2200}]) {
    expect(() => session.initializeBaselineV2ForTest({...request, workload: 'P0', ...limits})).toThrow(/query|integer/u);
    expect(session.snapshot()).toMatchObject({phase: 'created'});
  }
  expect(() => session.initializeBaselineV2ForTest({...request, pelletChecks: 3000, segmentChecks: 4000})).toThrow(/workload/u);
  await session.initializeBaselineV2ForTest({...request, profile: 'basic', bins: 8, workload: 'P0', pelletChecks: 3000, segmentChecks: 4000});
  const settings = new Map(parseRustStartupMetadata(session.startupMetadata()).settings.map(entry => [entry.path, entry.value]));
  expect(settings.get('sense.maxPelletChecks')).toBe(3000);
  expect(settings.get('sense.maxSegmentChecks')).toBe(4000);
  expect(session.snapshot()).toMatchObject({completedStep: '0000000000000000', checkpointPublished: false, authorityPublished: false});
}, 15000);

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
  const fixture = resolve('native/fixtures/learned-cohorts-v1/replicated');
  const evidence = JSON.parse(readFileSync(join(fixture, 'fixture.json'), 'utf8')) as {
    files: Record<string, {bytes: number; sha256: string}>;
  };
  return [1, 2].map(seed => {
    const file = 'full-' + seed + '.slither-brain'; const receipt = evidence.files[file]!;
    const bytes = readFileSync(join(fixture, file));
    expect(bytes.byteLength).toBe(receipt.bytes); expect(bytes.byteLength).toBeLessThan(256 * 1024);
    expect(createHash('sha256').update(bytes).digest('hex')).toBe(receipt.sha256);
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

/** Fixed offline candidates; this surface cannot alter production startup or scratch limits. */
interface CapacityRequest {
  /** Only the two declared workload population sizes are admitted. */
  population: number;
  /** Named native graph fixture, never JavaScript graph execution. */
  graph: string;
  /** Selected total checkpoint envelope; both scratch limits remain 512 MiB. */
  envelopeMiB: number;
  /** Exercise raw checkpoint fallback without changing runtime scratch. */
  rawFallback: boolean;
  /** Explicit minimum ray work per due controller. */
  rayMinimum: number;
  /** Explicit maximum ray work per due controller. */
  rayCap: number;
  /** Explicit aggregate ray work per complete step. */
  rayBatch: number;
}

/** Fixed qualification choice, with a full 512-controller maximum-due admission. */
const capacityRequest: CapacityRequest = {population: 300, graph: 'large', envelopeMiB: 512,
  rawFallback: false, rayMinimum: 1, rayCap: 256, rayBatch: 65536};

it('rejects a live capacity request before population allocation and allows an admitted retry', async () => {
  const binding = loadPanicBinding();
  const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
  const session = new Constructor('live-capacity-retry-' + randomBytes(8).toString('hex'), '0000002a', '0000000100000000', 1);
  for (const invalid of [{population: 56}, {graph: 'other'}, {envelopeMiB: 640}, {rayCap: 0},
    {rawFallback: 1}, {unexpected: true}]) {
    expect(() => session.initializeCapacityForTest({...capacityRequest, ...invalid} as CapacityRequest)).toThrow();
  }
  await expect(session.initializeCapacityForTest(capacityRequest)).rejects.toThrow(/checkpoint envelope/u);
  expect(session.snapshot()).toMatchObject({phase: 'created'});
  const initialized = await session.initializeCapacityForTest({...capacityRequest, population: 55, graph: 'modest'});
  const audit = JSON.parse(initialized.capacityAuditJson) as {decodedEnvelopeFits: boolean; twoStateBudgetFits: boolean};
  expect(audit).toMatchObject({population: 55, decodedEnvelopeFits: true, twoStateBudgetFits: true,
    workerScratchBytes: 512 * 1024 ** 2, checkpointScratchBytes: 512 * 1024 ** 2});
  expect(session.snapshot()).toMatchObject({completedStep: '0000000000000000', checkpointPublished: false, authorityPublished: false});
}, 15000);

it('restores a fitting large Motion/B32 SSE2 checkpoint through qualified and ordinary admission', async () => {
  const root = mkdtempSync(join(tmpdir(), 'slither-live-cohort-'));
  try {
    await admitDiskOperation(root, checkpointDiskAdmissionRequest(768n * 1024n ** 2n));
    const binding = loadPanicBinding();
    const Constructor = binding.ExperimentalStage6aFreshRunSession as new (run: string, seed: string, memory: string, workers: number) => QualifiedSession;
    const runId = 'live-capacity-large-' + randomBytes(8).toString('hex');
    const session = new Constructor(runId, '0000002a', '0000000100000000', 1);
    const initialized = await session.initializeCapacityForTest({...capacityRequest, population: 55, envelopeMiB: 768, rawFallback: true});
    const audit = JSON.parse(initialized.capacityAuditJson) as {decodedBytes: number; limits: {maxTotalDecodedBytes: number}};
    expect(audit).toMatchObject({population: 55, parametersPerGenome: 494306,
      decodedEnvelopeFits: true, twoStateBudgetFits: true, numericPayloadAllocated: false,
      workerScratchBytes: 512 * 1024 ** 2, checkpointScratchBytes: 512 * 1024 ** 2});
    expect(audit.limits.maxTotalDecodedBytes).toBe(768 * 1024 ** 2);
    const metadata = parseRustStartupMetadata(session.startupMetadata());
    expect(metadata).toMatchObject({parameterCount: 494306, sensorVersion: 4, mathBackend: 'rust-sse2-v1'});
    expect(metadata.graphSpec.nodes.find(node => node.type === 'Input')?.outputSize).toBe(504);
    expect(metadata.settings).toEqual(expect.arrayContaining([
      {path: 'brain.controlDt', value: 1 / 60}, {path: 'baselineBots.controlDt', value: 1 / 60}
    ]));
    const descriptor = parseManagedCheckpointDescriptor(await session.publishRunStartCheckpoint({
      managedDirectory: root, operationId: randomBytes(16).toString('hex'),
    }));
    const admission = {profile: 'motion', bins: 32, mathBackend: 'sse2',
      rayMinimum: 1, rayCap: 256, rayBatch: 65536, envelopeMiB: 768};
    const restored = new Constructor(runId, '0000002a', '0000000100000000', 1);
    for (const invalid of [{bins: 16}, {profile: 'basic'}, {envelopeMiB: 640}, {envelopeMiB: 768.5}]) {
      expect(() => restored.initializeSensorCheckpointForTest(root, descriptor, {...admission, ...invalid})).toThrow();
    }
    expect(restored.snapshot()).toMatchObject({phase: 'created'});
    await restored.initializeSensorCheckpointForTest(root, descriptor, admission);
    expect(parseRustStartupMetadata(restored.startupMetadata())).toEqual(metadata);
    expect(restored.snapshot()).toMatchObject({generation: descriptor.generation, completedStep: descriptor.completedStep});
    const ordinary = new Constructor(runId, '0000002a', '0000000100000000', 1);
    // SSE2 is now production-supported. This 55-brain boundary also fits the
    // unchanged 512-MiB production envelope; the 300-brain rejection above remains.
    expect(audit.decodedBytes).toBeLessThan(512 * 1024 ** 2);
    await ordinary.initializeFromCheckpoint(root, descriptor);
    expect(parseRustStartupMetadata(ordinary.startupMetadata())).toEqual(metadata);
    expect(ordinary.snapshot()).toMatchObject({phase: 'durableBoundary', authorityPublished: false});
  } finally {
    cleanupCohort(root);
  }
}, 30000);
