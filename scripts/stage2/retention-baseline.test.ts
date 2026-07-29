/** Focused contracts for the disposable compact-history SQLite measurement. */

import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import {
  encodeHistoryRecord,
  HISTORY_RECORD_BYTES,
  runHistoryStorageMeasurement
} from './retention-baseline.ts';

/** Typed subset of the JSON-shaped history measurement used by these tests. */
interface HistoryMeasurementResult {
  fixture: { generations: number; recordBytes: number };
  growthSamples: Array<{
    records: number;
    logicalBytes: number;
    storage: {
      dbstat: {
        available: boolean;
        table: {
          name: string;
          pages: number;
          pageBytes: number;
          payloadBytes: number;
          unusedBytes: number;
        } | null;
        primaryKeyIndex: {
          name: string;
          pages: number;
          pageBytes: number;
          payloadBytes: number;
          unusedBytes: number;
        } | null;
      };
    };
  }>;
  appendTransactionMs: { count: number };
  wal: { peakObservedBytes: number };
  accountingAssertions: {
    exactRecordCount: boolean;
    exactFixedWidthLogicalBytes: boolean;
    requiredSamplesPresent: boolean;
  };
}

/** Temporary roots owned by the CLI publication test. */
const temporaryRoots: string[] = [];

afterEach(() => {
  const systemTemp = path.resolve(os.tmpdir());
  for (const root of temporaryRoots.splice(0)) {
    const resolved = path.resolve(root);
    if (
      path.dirname(resolved) !== systemTemp ||
      !path.basename(resolved).startsWith('slither-stage2-history-test-')
    ) {
      throw new Error(`refusing to clean unexpected history test root ${resolved}`);
    }
    fs.rmSync(resolved, { recursive: true, force: true });
  }
});

/**
 * Invoke the history-only evidence CLI in a fresh process.
 * @param argumentsAfterScript - CLI arguments following the runner path.
 * @returns Completed child-process result.
 */
function runHistoryCli(argumentsAfterScript: readonly string[]): ReturnType<typeof spawnSync> {
  const runner = fileURLToPath(new URL('./retention-baseline.ts', import.meta.url));
  const tsxCli = path.resolve('node_modules', 'tsx', 'dist', 'cli.mjs');
  return spawnSync(process.execPath, [tsxCli, runner, ...argumentsAfterScript], {
    encoding: 'utf8',
    timeout: 30_000,
    maxBuffer: 1024 * 1024
  });
}

describe('Stage 2 isolated compact-history SQLite measurement', () => {
  it('encodes the approved eight fields at their exact fixed offsets', () => {
    const actual = encodeHistoryRecord(23);

    expect(actual).toHaveLength(HISTORY_RECORD_BYTES);
    expect(actual.toString('hex')).toBe(
      '17000000000000000000000000c03c4000000000004031400000000000001740' +
      '07000000180000001c357bfdf911e53f5e87bd3eabf4ed3f'
    );
  });

  it('reports exact logical bytes and all required growth samples', () => {
    const result = runHistoryStorageMeasurement({ generations: 480 }) as unknown as HistoryMeasurementResult;
    const sampleByRecordCount = new Map(
      result.growthSamples.map(sample => [sample.records, sample])
    );

    expect(result.fixture).toMatchObject({ generations: 480, recordBytes: HISTORY_RECORD_BYTES });
    expect([...sampleByRecordCount.keys()]).toEqual(expect.arrayContaining([0, 1, 8, 64, 480]));
    for (const records of [0, 1, 8, 64, 480]) {
      expect(sampleByRecordCount.get(records)?.logicalBytes).toBe(records * HISTORY_RECORD_BYTES);
    }
    expect(result.appendTransactionMs.count).toBe(480);
    expect(result.wal.peakObservedBytes).toBeGreaterThanOrEqual(0);
    expect(result.accountingAssertions).toEqual({
      exactRecordCount: true,
      exactFixedWidthLogicalBytes: true,
      requiredSamplesPresent: true
    });
    for (const sample of result.growthSamples) {
      expect(typeof sample.storage.dbstat.available).toBe('boolean');
      if (sample.storage.dbstat.available && sample.records > 0) {
        expect(sample.storage.dbstat.table?.name).toBe('generation_history');
        expect(sample.storage.dbstat.primaryKeyIndex?.name).toContain('generation_history');
        for (const object of [
          sample.storage.dbstat.table,
          sample.storage.dbstat.primaryKeyIndex
        ]) {
          expect(object?.pages).toBeGreaterThan(0);
          expect(object?.pageBytes).toBeGreaterThan(0);
          expect(object?.payloadBytes).toBeGreaterThanOrEqual(0);
          expect(object?.unusedBytes).toBeGreaterThanOrEqual(0);
        }
      }
    }
  });

  it('rejects history fixture counts outside its bounded safe range', () => {
    expect(() => runHistoryStorageMeasurement({ generations: 0 })).toThrow(/history generations/);
    expect(() => runHistoryStorageMeasurement({ generations: 1_000_001 })).toThrow(
      /history generations/
    );
  });

  it('publishes one complete artifact and refuses to overwrite it', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-history-test-'));
    temporaryRoots.push(root);
    const output = path.join(root, 'history.json');
    const argumentsAfterScript = [
      '--history-only',
      '--history-generations',
      '8',
      '--output',
      output
    ];

    const first = runHistoryCli(argumentsAfterScript);
    expect(first.status).toBe(0);
    expect(first.error).toBeUndefined();
    const artifact = JSON.parse(fs.readFileSync(output, 'utf8')) as {
      schema: string;
      source: { commit: string; dirty: boolean };
      measurement: {
        fixture: { generations: number };
        accountingAssertions: Record<string, boolean>;
      };
    };
    expect(artifact.schema).toBe('slither-stage2-history-sqlite-overhead');
    expect(artifact.source.commit).not.toBe('');
    expect(typeof artifact.source.dirty).toBe('boolean');
    expect(artifact.measurement.fixture.generations).toBe(8);
    expect(Object.values(artifact.measurement.accountingAssertions).every(Boolean)).toBe(true);

    const second = runHistoryCli(argumentsAfterScript);
    expect(second.status).toBe(1);
    expect(second.stderr).toContain('EEXIST');
  });
});
