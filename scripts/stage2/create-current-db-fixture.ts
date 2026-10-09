/** Create a disposable current-format database for browser/persistence tests. */

import fs from 'node:fs';
import path from 'node:path';
import { buildGenerationCheckpoint } from '../../server/checkpoint.ts';
import {
  createPersistence,
  initDb,
  type PopulationCheckpoint
} from '../../server/persistence.ts';
import { resetCFGToDefaults } from '../../src/config.ts';
import { World } from '../../src/world.ts';
import {
  installStage2Scenario,
  STAGE2_WORLD_SEED,
  type Stage2ScenarioName
} from './fixtures.ts';

/** Fixture creation options. */
interface FixtureOptions {
  /** P0–P3 population/brain shape. */
  scenario: Extract<Stage2ScenarioName, 'P0' | 'P1' | 'P2' | 'P3'>;
  /** New database path. */
  outputPath: string;
}

/**
 * Parse command-line options.
 * @param argv - Arguments after script path.
 * @returns Validated scenario and output.
 */
function parseOptions(argv: readonly string[]): FixtureOptions {
  let scenario: FixtureOptions['scenario'] = 'P1';
  let outputPath: string | null = null;
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!value) throw new Error(`${option ?? '<missing>'} requires a value`);
    if (option === '--scenario') {
      if (value !== 'P0' && value !== 'P1' && value !== 'P2' && value !== 'P3') {
        throw new Error('--scenario must be P0, P1, P2, or P3');
      }
      scenario = value;
    } else if (option === '--output') {
      outputPath = path.resolve(value);
    } else {
      throw new Error(`Unknown option ${option}`);
    }
    index++;
  }
  if (!outputPath) throw new Error('--output is required');
  return { scenario, outputPath };
}

/**
 * Create one exact run-start checkpoint in a new disposable database.
 * @param options - Scenario and destination.
 * @returns Snapshot and byte counts.
 */
function createFixture(options: FixtureOptions): Record<string, unknown> {
  if (fs.existsSync(options.outputPath)) {
    throw new Error(`Refusing to overwrite existing fixture: ${options.outputPath}`);
  }
  fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
  const scenario = installStage2Scenario(options.scenario);
  let checkpoint: PopulationCheckpoint | null = null;
  new World(scenario.settings, {
    seed: STAGE2_WORLD_SEED,
    runId: `stage2-browser-${options.scenario.toLowerCase()}`,
    onGenerationBoundary: (boundary, candidate) => {
      checkpoint = buildGenerationCheckpoint(candidate, boundary, 0);
    }
  });
  if (!checkpoint) throw new Error('run-start checkpoint was not captured');
  const db = initDb(options.outputPath);
  try {
    const persistence = createPersistence(db);
    const snapshotId = persistence.saveCheckpoint(checkpoint as PopulationCheckpoint);
    const payload = db.prepare(
      `SELECT COUNT(*) AS genomes,
              SUM(weight_count) AS weights,
              SUM(LENGTH(weights_blob)) AS weight_bytes
         FROM snapshot_genomes
        WHERE snapshot_id = ?`
    ).get(snapshotId) as { genomes: number; weights: number; weight_bytes: number };
    db.pragma('wal_checkpoint(TRUNCATE)');
    return {
      schema: 'slither-stage2-current-db-fixture',
      version: 1,
      scenario,
      seed: STAGE2_WORLD_SEED,
      snapshotId,
      databasePath: options.outputPath,
      databaseBytes: fs.statSync(options.outputPath).size,
      ...payload
    };
  } finally {
    db.close();
  }
}

/** Execute the CLI. */
function main(): void {
  const options = parseOptions(process.argv.slice(2));
  try {
    process.stdout.write(`${JSON.stringify(createFixture(options), null, 2)}\n`);
  } finally {
    resetCFGToDefaults();
  }
}

try {
  main();
} catch (error) {
  console.error(error);
  process.exitCode = 1;
}
