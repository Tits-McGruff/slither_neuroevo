import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { World } from '../src/world.ts';
import { buildGenerationCheckpoint } from '../server/checkpoint.ts';
import { createPersistence, initDb, type PopulationCheckpoint } from '../server/persistence.ts';

/** Default synthetic population used by the Phase 7 memory diagnostic. */
const DEFAULT_POPULATION_COUNT = 500;
/** Maximum population accepted by the persistence schema. */
const MAX_POPULATION_COUNT = 10_000;

/**
 * Parse and bound the optional synthetic population CLI argument.
 * @param raw - First CLI argument.
 * @returns Positive bounded population count.
 */
function parsePopulationCount(raw: string | undefined): number {
  if (raw === undefined) return DEFAULT_POPULATION_COUNT;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > MAX_POPULATION_COUNT) {
    throw new Error(`population count must be between 1 and ${MAX_POPULATION_COUNT}`);
  }
  return parsed;
}

/**
 * Install a moderately large graph for meaningful serialization measurements.
 * @returns Installed graph definition.
 */
function installMeasurementGraph(): GraphSpec {
  const inputSize = CFG.brain.inSize;
  const spec: GraphSpec = {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      {
        id: 'network',
        type: 'MLP',
        inputSize,
        outputSize: 2,
        hiddenSizes: [128, 128]
      }
    ],
    edges: [{ from: 'input', to: 'network' }],
    outputs: [{ nodeId: 'network' }],
    outputSize: 2
  };
  CFG.brain.graphSpec = spec;
  return spec;
}

/**
 * Run one disk-backed save and report measured process-memory bounds.
 * @param populationCount - Synthetic dense genome count.
 */
function measurePersistence(populationCount: number): void {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 100;
  CFG.pelletSpawnPerSecond = 5;
  installMeasurementGraph();
  let baseCheckpoint: PopulationCheckpoint | null = null;
  const world = new World(
    {
      snakeCount: 1,
      simSpeed: 1,
      hiddenLayers: 2,
      neurons1: 128,
      neurons2: 128,
      neurons3: 2,
      neurons4: 2,
      neurons5: 2
    },
    {
      seed: 0x5a17c0de,
      runId: 'phase7-memory-measurement',
      onGenerationBoundary: (boundary, candidate) => {
        baseCheckpoint = buildGenerationCheckpoint(candidate, boundary, 0);
      }
    }
  );
  if (!baseCheckpoint) throw new Error('measurement checkpoint boundary was not captured');
  const template = world.population[0];
  if (!template) throw new Error('measurement genome template is missing');
  const syntheticPopulation: Float32Array[] = new Array(populationCount);
  for (let slot = 0; slot < populationCount; slot++) {
    const weights = template.weights.slice();
    weights[0] = Math.fround(slot / Math.max(1, populationCount));
    syntheticPopulation[slot] = weights;
  }
  globalThis.gc?.();
  const baseline = process.memoryUsage();
  let peakRss = baseline.rss;
  let peakHeapUsed = baseline.heapUsed;
  let peakExternal = baseline.external;
  const sampleMemory = (): void => {
    const current = process.memoryUsage();
    peakRss = Math.max(peakRss, current.rss);
    peakHeapUsed = Math.max(peakHeapUsed, current.heapUsed);
    peakExternal = Math.max(peakExternal, current.external);
  };
  const checkpoint: PopulationCheckpoint = {
    metadata: {
      ...baseCheckpoint.metadata,
      boundaryKind: 'population-export',
      resumable: false,
      populationCount,
      settings: {
        ...baseCheckpoint.metadata.settings,
        snakeCount: populationCount
      }
    },
    genomes: {
      *[Symbol.iterator]() {
        for (let slot = 0; slot < populationCount; slot++) {
          const weights = syntheticPopulation[slot];
          if (!weights) throw new Error(`synthetic population slot ${slot} is missing`);
          sampleMemory();
          yield {
            slot,
            archKey: template.archKey,
            brainType: template.brainType,
            fitness: 0,
            weights
          };
          sampleMemory();
        }
      }
    }
  };
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-phase7-memory-'));
  const dbPath = path.join(root, 'measurement.db');
  let db: ReturnType<typeof initDb> | null = null;
  try {
    db = initDb(dbPath);
    const persistence = createPersistence(db);
    const started = performance.now();
    const snapshotId = persistence.saveCheckpoint(checkpoint);
    const durationMs = performance.now() - started;
    sampleMemory();
    const row = db.prepare(
      `SELECT COUNT(*) AS genome_count,
              SUM(LENGTH(weights_blob)) AS payload_bytes
         FROM snapshot_genomes WHERE snapshot_id = ?`
    ).get(snapshotId) as { genome_count: number; payload_bytes: number };
    console.log(JSON.stringify({
      snapshotId,
      populationCount,
      genomeWeightCount: template.weights.length,
      payloadBytes: row.payload_bytes,
      persistedGenomeCount: row.genome_count,
      durationMs: Number(durationMs.toFixed(2)),
      baselineRssBytes: baseline.rss,
      peakRssBytes: peakRss,
      rssDeltaBytes: peakRss - baseline.rss,
      baselineHeapUsedBytes: baseline.heapUsed,
      peakHeapUsedBytes: peakHeapUsed,
      heapDeltaBytes: peakHeapUsed - baseline.heapUsed,
      baselineExternalBytes: baseline.external,
      peakExternalBytes: peakExternal,
      externalDeltaBytes: peakExternal - baseline.external
    }));
  } finally {
    db?.close();
    fs.rmSync(root, { recursive: true, force: true });
    resetCFGToDefaults();
  }
}

try {
  measurePersistence(parsePopulationCount(process.argv[2]));
} catch (error) {
  console.error(error);
  process.exitCode = 1;
}
