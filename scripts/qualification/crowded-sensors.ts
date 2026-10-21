/** Run the native dense-fixture matrix; TypeScript only routes processes and checks scalar receipts. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';

/** Bounded native witnesses used to reject mismatched or partial matrix cells. */
interface Receipt {
  gate: string; schemaVersion: number; sourceSha256: string; target: string;
  buildProfile: string; buildClass: string; scenario: string; workers: number;
  warmupPasses: number; measuredPasses: number; worldUnchanged: boolean; scratchStable: boolean;
  worldSha256: string; sensorSha256: string; observationsSha256: string;
  rayMinimum: number; rayCap: number; rayBatch: number; grantedWork: number;
  geometry: {bodyPoints: number; inArenaCollisionSegments: number; pellets: number;
    outsideArenaPoints: number; outsideArenaPellets: number; headSkipPoints: number};
  proofPassWork: {observations: number; charged: number; incompleteObservations: number};
  allocationOperationsPerPass: number[];
}

/** Require exact matching observations/work across native worker counts; timings remain descriptive. */
function main(): void {
  const args = process.argv.slice(2);
  assert(args.length === 4 && args[0] === '--runner' && args[2] === '--directory', '--runner PATH --directory NEW_DIRECTORY');
  const runner = resolve(args[1]!); const directory = resolve(args[3]!);
  assert(statSync(runner).isFile()); mkdirSync(directory); // Exclusive task-owned evidence directory.
  const records: Array<{filename: string; sha256: string}> = [];
  let source: string | undefined; let target: string | undefined;
  for (const scenario of ['dense-body', 'dense-pellet']) {
    let world: string | undefined;
    for (const [profile, bins] of [['basic', 8], ['motion', 32]] as const) {
      for (const cap of [1, 512]) {
        let reference: Receipt | undefined;
        for (const workers of [1, 3]) {
          const filename = `${scenario}-${profile}-B${bins}-cap${cap}-w${workers}.json`;
          const output = resolve(directory, filename);
          const child = spawnSync(runner, ['--scenario', scenario, '--profile', profile, '--bins', String(bins),
            '--minimum', '1', '--cap', String(cap), '--batch', '131072', '--workers', String(workers),
            '--warmup', '2', '--passes', '10', '--output', output], {stdio: 'inherit', timeout: 120_000, windowsHide: true});
          if (child.error) throw child.error;
          assert.equal(child.status, 0, `native crowded cell failed: ${filename}`);
          assert(statSync(output).size <= 1024 * 1024);
          const bytes = readFileSync(output); const report = JSON.parse(bytes.toString('utf8')) as Receipt;
          assert.equal(report.gate, 'crowded-v4-sensing-v1'); assert.equal(report.schemaVersion, 1);
          assert.match(report.sourceSha256, /^[0-9a-f]{64}$/u); source ??= report.sourceSha256;
          assert.equal(report.sourceSha256, source); target ??= report.target; assert.equal(report.target, target);
          assert(['x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu'].includes(report.target));
          assert.equal(report.buildProfile, 'release'); assert.equal(report.buildClass, 'test-hooks');
          assert.equal(report.scenario, scenario); assert.equal(report.workers, workers);
          assert.equal(report.warmupPasses, 2); assert.equal(report.measuredPasses, 10);
          assert.equal(report.worldUnchanged, true); assert.equal(report.scratchStable, true);
          assert.equal(report.geometry.outsideArenaPoints, 0); assert.equal(report.geometry.outsideArenaPellets, 0);
          assert.equal(report.geometry.headSkipPoints, 30);
          if (scenario === 'dense-body') assert(report.geometry.inArenaCollisionSegments > 200_000);
          else assert.equal(report.geometry.pellets, 25_000);
          world ??= report.worldSha256; assert.equal(report.worldSha256, world);
          for (const value of [report.worldSha256, report.sensorSha256, report.observationsSha256]) assert.match(value, /^[0-9a-f]{64}$/u);
          assert.equal(report.rayMinimum, 1); assert.equal(report.rayCap, cap); assert.equal(report.rayBatch, 131072);
          assert(report.proofPassWork.charged <= report.grantedWork && report.grantedWork <= report.rayBatch);
          assert.equal(report.proofPassWork.observations, scenario === 'dense-body' ? 310 : 65);
          if (cap === 1) assert(report.proofPassWork.incompleteObservations > 0);
          assert.equal(report.allocationOperationsPerPass.length, 10);
          for (const count of report.allocationOperationsPerPass) assert(Number.isSafeInteger(count) && count >= 0);
          if (reference) {
            for (const key of ['geometry', 'sensorSha256', 'grantedWork', 'proofPassWork', 'observationsSha256'] as const) {
              assert.deepEqual(report[key], reference[key], `worker count changed ${key}`);
            }
          }
          reference = report;
          records.push({filename, sha256: createHash('sha256').update(bytes).digest('hex')});
        }
      }
    }
  }
  assert.equal(records.length, 16);
  writeFileSync(resolve(directory, 'matrix.json'), JSON.stringify({schemaVersion: 1, sourceSha256: source,
    target, cells: records, releaseQualified: false,
    scope: 'Synthetic immutable in-arena native v4 sensing stress and exact worker comparison. Complete gameplay, mature policies, capture/trainer contention, wall scheduling and production defaults require separate qualification.'}, null, 2) + '\n', {flag: 'wx'});
  console.info(JSON.stringify({gate: 'crowded-v4-sensing-v1', cells: records.length, sourceSha256: source, target}));
}

main();
