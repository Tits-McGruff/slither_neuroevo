import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { copyFileSync,existsSync,readFileSync,rmSync,writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe,expect,it } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { SUITE,clients,createDescriptor,createExperimentPayload,createFixture,createGenerationCommit,createHallOfFameWeights,f64,readCurrentPointer,u64 } from './checkpointPersistenceFixture.ts';
import type {
ManagedImportInventoryDescriptor
} from './checkpointPersistenceProtocol.ts';
import { decodeExperimentSummary,parseExperimentSummary,parseGenerationExperimentRecord,parseHallOfFameAssessment } from './experimentRecords.ts';

describe(SUITE, { timeout: 30_000 }, () => {

  it.each([10, 12.5, 20])('retains the boundary assessment when the same genome later scores %s', async laterFitness => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, { operationId: '22'.repeat(16),
      generation: u64(2n), completedStep: u64(60n), boundaryKind: 'generation' });
    const third = createDescriptor(fixture.managedRoot, { operationId: '33'.repeat(16),
      generation: u64(3n), completedStep: u64(120n), boundaryKind: 'generation' });
    const originalWinner = createGenerationCommit(1n);
    await fixture.client.commit(first);
    await fixture.client.commit(second, originalWinner);
    const originalLease = await fixture.client.acquireCurrentExportLease();
    const originalInventory = readFileSync(join(fixture.managedRoot, originalLease.inventory.relativeFilename));
    await fixture.client.releaseExportLease(originalLease.operationId);

    await fixture.client.commit(third, { ...createGenerationCommit(2n, { bestF64Hex: f64(laterFitness) }),
      hallOfFameWeights: originalWinner.hallOfFameWeights });
    const selectedGeneration = laterFitness > 12.5 ? 2 : 1;
    expect(await fixture.client.readBrowserHallOfFame(first.runId)).toEqual([
      { entryId: u64(BigInt(selectedGeneration)), gen: selectedGeneration,
        fitness: Math.max(12.5, laterFitness), points: 6.25, length: 9, pinned: false }
    ]);
    const database = new Database(fixture.databasePath, { readonly: true });
    const sourceRows = database.prepare('SELECT * FROM rust_hall_of_fame_v1 ORDER BY generation_hex').all();
    try {
      await fixture.client.commitRecoveryBranch({ operationId: '44'.repeat(16), branchRunId: 'prefix',
        sourceRunId: first.runId, failedCheckpointId: third.logicalRootSha256, recoveredDescriptor: second });
      expect(await fixture.client.readBrowserHallOfFame('prefix')).toEqual([
        { entryId: u64(1n), gen: 1, fitness: 12.5, points: 6.25, length: 9, pinned: false }
      ]);
      await expect(fixture.client.selectHallOfFameEntry('prefix', u64(2n))).rejects.toThrow(/not retained/);
      const selected = await fixture.client.selectHallOfFameEntry('prefix', u64(1n));
      expect(selected).toMatchObject({ checkpoint: second, reference: originalWinner.hallOfFame,
        weights: originalWinner.hallOfFameWeights });
      await fixture.client.releaseHallOfFameEntry(selected.operationId);
      const recoveredLease = await fixture.client.acquireCurrentExportLease();
      expect(recoveredLease.inventory).toMatchObject({ historyCount: u64(1n), hallOfFameCount: u64(1n) });
      expect(readFileSync(join(fixture.managedRoot, recoveredLease.inventory.relativeFilename))).toEqual(originalInventory);
      await fixture.client.releaseExportLease(recoveredLease.operationId);
      expect(database.prepare('SELECT * FROM rust_hall_of_fame_v1 ORDER BY generation_hex').all()).toEqual(sourceRows);
    } finally { database.close(); }
  });

  it('keeps compact history with the best 50 unique winner objects plus pins', async () => {
    const fixture = createFixture();
    await fixture.client.commit(createDescriptor(fixture.managedRoot));
    await fixture.client.commit(createDescriptor(fixture.managedRoot, {
      operationId: (2n + 96n).toString(16).padStart(32, '0'),
      transitionEpoch: u64(2n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    }), createGenerationCommit(1n, { bestF64Hex: f64(1) }));
    await fixture.client.close();
    const pinDatabase = new Database(fixture.databasePath);
    try {
      pinDatabase.prepare(`UPDATE rust_hall_of_fame_v1 SET pinned = 1
        WHERE generation_hex = ?`).run(u64(1n));
    } finally { pinDatabase.close(); }
    const reopened = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(reopened);
    for (let generation = 2n; generation <= 53n; generation++) {
      if (generation === 2n) continue;
      await reopened.commit(createDescriptor(fixture.managedRoot, {
        operationId: (generation + 96n).toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation),
        generation: u64(generation),
        completedStep: u64((generation - 1n) * 3_600n),
        boundaryKind: 'generation'
      }), createGenerationCommit(generation - 1n, {
        bestF64Hex: f64(Number(generation - 1n))
      }));
    }
    const lease = await reopened.acquireCurrentExportLease();
    expect(lease.inventory).toMatchObject({ historyCount: u64(52n), hallOfFameCount: u64(51n) });
    await reopened.releaseExportLease(lease.operationId);
    await reopened.applyRetention();

    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_generation_history_v1').get())
        .toEqual({ count: 52 });
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_v1').get())
        .toEqual({ count: 52 });
      expect(inspect.prepare(`SELECT weight_state, count(*) AS count FROM rust_hall_of_fame_v1
        GROUP BY weight_state ORDER BY weight_state`).all()).toEqual([
        { weight_state: 'selected', count: 51 },
        { weight_state: 'unselected', count: 1 }
      ]);
      expect(inspect.prepare(`SELECT generation_hex, pinned FROM rust_hall_of_fame_v1
        WHERE weight_state = 'unselected'`).all()).toEqual([{ generation_hex: u64(2n), pinned: 0 }]);
      expect(inspect.prepare(`SELECT count(*) AS count FROM rust_hall_of_fame_v1
        WHERE genome_sha256 IS NOT NULL`).get()).toEqual({ count: 52 });
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_weights_v1').get())
        .toEqual({ count: 51 });
    } finally { inspect.close(); }

    await reopened.close();
    const guard = new Database(fixture.databasePath);
    try {
      guard.exec(`CREATE TRIGGER reject_historical_hof_rewrite
        BEFORE UPDATE ON rust_hall_of_fame_v1
        WHEN OLD.generation_hex = '${u64(2n)}' AND OLD.weight_state = 'unselected'
        BEGIN SELECT RAISE(ABORT, 'rewrote historical unselected Hall-of-Fame row'); END`);
    } finally { guard.close(); }
    const incremental = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(incremental);
    await expect(incremental.commit(createDescriptor(fixture.managedRoot, {
      operationId: '97'.repeat(16), transitionEpoch: u64(54n), generation: u64(54n),
      completedStep: u64(53n * 3_600n), boundaryKind: 'generation'
    }), createGenerationCommit(53n, { bestF64Hex: f64(53) }))).resolves.toMatchObject({
      runId: expect.any(String), checkpointId: expect.any(String)
    });
  });

  it('does not restore a pruned assessment merely because the same genome has a pinned assessment', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const originalWinner = createGenerationCommit(1n, { bestF64Hex: f64(1) });
    for (let generation = 1n; generation <= 53n; generation++) {
      const commit = generation === 1n ? originalWinner
        : createGenerationCommit(generation, { bestF64Hex: f64(Number(generation)) });
      if (generation === 2n) commit.hallOfFameWeights = originalWinner.hallOfFameWeights;
      await fixture.client.commit(createDescriptor(fixture.managedRoot, {
        operationId: (generation + 200n).toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation + 1n), generation: u64(generation + 1n),
        completedStep: u64(generation * 3_600n), boundaryKind: 'generation'
      }), commit);
      if (generation === 1n) {
        const database = new Database(fixture.databasePath);
        try { database.prepare('UPDATE rust_hall_of_fame_v1 SET pinned = 1 WHERE generation_hex = ?').run(u64(1n)); }
        finally { database.close(); }
      }
    }
    const hall = await fixture.client.readBrowserHallOfFame(first.runId);
    expect(hall).toHaveLength(51);
    expect(hall.at(-1)).toMatchObject({ entryId: u64(1n), pinned: true });
    expect(hall.some(entry => entry.entryId === u64(2n))).toBe(false);
    await expect(fixture.client.selectHallOfFameEntry(first.runId, u64(2n))).rejects.toThrow(/not retained/);
    const lease = await fixture.client.acquireCurrentExportLease();
    expect(lease.inventory).toMatchObject({ historyCount: u64(53n), hallOfFameCount: u64(51n) });
    await fixture.client.releaseExportLease(lease.operationId);
  });

  it('exports one weight segment when multiple generations select the same genome', async () => {
    const fixture = createFixture();
    await fixture.client.commit(createDescriptor(fixture.managedRoot));
    const firstCommit = createGenerationCommit(1n, { bestF64Hex: f64(10) });
    await fixture.client.commit(createDescriptor(fixture.managedRoot, {
      operationId: '71'.repeat(16), transitionEpoch: u64(2n), generation: u64(2n),
      completedStep: u64(3_600n), boundaryKind: 'generation'
    }), firstCommit);
    const repeatedCommit = createGenerationCommit(2n, { bestF64Hex: f64(20) });
    rmSync(join(fixture.managedRoot, repeatedCommit.hallOfFameWeights.relativeFilename));
    repeatedCommit.hallOfFameWeights = firstCommit.hallOfFameWeights;
    await fixture.client.commit(createDescriptor(fixture.managedRoot, {
      operationId: '72'.repeat(16), transitionEpoch: u64(3n), generation: u64(3n),
      completedStep: u64(7_200n), boundaryKind: 'generation'
    }), repeatedCommit);
    await fixture.client.close();
    const pinDatabase = new Database(fixture.databasePath);
    try {
      pinDatabase.prepare('UPDATE rust_hall_of_fame_v1 SET pinned = 1').run();
    } finally { pinDatabase.close(); }
    const reopened = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(reopened);
    const lease = await reopened.acquireCurrentExportLease();
    expect(lease.inventory).toMatchObject({ historyCount: u64(2n), hallOfFameCount: u64(2n) });
    await reopened.releaseExportLease(lease.operationId);

    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_weights_v1').get())
        .toEqual({ count: 1 });
      expect(inspect.prepare(`SELECT generation_hex, pinned, weight_state FROM rust_hall_of_fame_v1
        ORDER BY generation_hex`).all()).toEqual([
        { generation_hex: u64(1n), pinned: 1, weight_state: 'selected' },
        { generation_hex: u64(2n), pinned: 1, weight_state: 'selected' }
      ]);
    } finally { inspect.close(); }
  });

  it('does not collect a republished winner before its generation commit', async () => {
    const fixture = createFixture();
    await fixture.client.commit(createDescriptor(fixture.managedRoot));
    for (let generation = 2n; generation <= 52n; generation++) {
      await fixture.client.commit(createDescriptor(fixture.managedRoot, {
        operationId: (generation + 200n).toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation), generation: u64(generation),
        completedStep: u64((generation - 1n) * 3_600n), boundaryKind: 'generation'
      }), createGenerationCommit(generation - 1n, { bestF64Hex: f64(Number(generation)) }));
    }
    const repeated = createGenerationCommit(52n, { bestF64Hex: f64(1_000) });
    repeated.hallOfFameWeights = createHallOfFameWeights(1n);
    const repeatedFile = join(fixture.managedRoot, repeated.hallOfFameWeights.relativeFilename);
    await expect(fixture.client.commit(createDescriptor(fixture.managedRoot, {
      operationId: 'e3'.repeat(16), transitionEpoch: u64(53n), generation: u64(53n),
      completedStep: u64(52n * 3_600n), boundaryKind: 'generation'
    }), repeated)).resolves.toMatchObject({ checkpointId: expect.any(String) });
    expect(existsSync(repeatedFile)).toBe(true);
    await fixture.client.applyRetention();
    expect(existsSync(repeatedFile)).toBe(true);
  });

  it('pinning a top-50 winner does not promote a deleted rank-51 object', async () => {
    const fixture = createFixture();
    await fixture.client.commit(createDescriptor(fixture.managedRoot));
    for (let generation = 2n; generation <= 52n; generation++) {
      await fixture.client.commit(createDescriptor(fixture.managedRoot, {
        operationId: (generation + 300n).toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation), generation: u64(generation),
        completedStep: u64((generation - 1n) * 3_600n), boundaryKind: 'generation'
      }), createGenerationCommit(generation - 1n, { bestF64Hex: f64(Number(generation)) }));
    }
    await fixture.client.applyRetention();
    await fixture.client.close();
    const pinDatabase = new Database(fixture.databasePath);
    try {
      pinDatabase.prepare('UPDATE rust_hall_of_fame_v1 SET pinned = 1 WHERE generation_hex = ?')
        .run(u64(51n));
    } finally { pinDatabase.close(); }
    const reopened = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath, managedRootPath: fixture.managedRoot, existingOnly: true
    });
    clients.push(reopened);
    await reopened.commit(createDescriptor(fixture.managedRoot, {
      operationId: 'e4'.repeat(16), transitionEpoch: u64(53n), generation: u64(53n),
      completedStep: u64(52n * 3_600n), boundaryKind: 'generation'
    }), createGenerationCommit(52n, { bestF64Hex: f64(0) }));
    await reopened.applyRetention();
    const lease = await reopened.acquireCurrentExportLease();
    expect(lease.inventory.hallOfFameCount).toBe(u64(50n));
    await reopened.releaseExportLease(lease.operationId);
    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_weights_v1').get())
        .toEqual({ count: 50 });
    } finally { inspect.close(); }
  });

  it('requires exact finite compact history only for generation checkpoints', async () => {
    const fixture = createFixture();
    const runStart = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(runStart, createGenerationCommit(1n))).rejects.toThrow(
      /run-start checkpoints must not include/
    );
    await fixture.client.commit(runStart);
    const generation = createDescriptor(fixture.managedRoot, {
      operationId: '99999999999999999999999999999999',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(generation)).rejects.toThrow(/generationCommit must be a plain object/);
    await expect(fixture.client.commit(generation, createGenerationCommit(2n))).rejects.toThrow(
      /exactly the generation preceding/
    );
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {
      bestF64Hex: '7ff0000000000000'
    }))).rejects.toThrow(/finite Float64/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {
      speciesCount: u64(3n)
    }))).rejects.toThrow(/speciesCount exceeds/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {}, {
      completedGeneration: u64(2n)
    }))).rejects.toThrow(/Hall-of-Fame generation/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {}, {
      fitnessF64Hex: f64(11.5)
    }))).rejects.toThrow(/fitness does not match/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {}, {
      sourcePopulationSlot: u64(2n)
    }))).rejects.toThrow(/outside the checkpoint population/);
    await expect(fixture.client.commit(generation, createGenerationCommit(1n, {}, {
      successorGenomeId: u64(0n)
    }))).rejects.toThrow(/identities must be nonzero/);
  });

  it('commits completion chronology atomically and rejects changed or omitted retry records', async () => {
    const fixture = createFixture();
    await fixture.client.commit(createDescriptor(fixture.managedRoot));
    const descriptor = createDescriptor(fixture.managedRoot, { operationId: 'abababababababababababababababab',
      boundaryKind: 'generation', generation: u64(2n), completedStep: u64(10n), transitionEpoch: u64(10n) });
    const commit = { ...createGenerationCommit(1n), completion: { version: 1 as const, reason: 'manual' as const,
      actualElapsedF64Hex: f64(10 / 60), configuredDurationF64Hex: f64(604800), completedStep: u64(10n) } };
    await fixture.client.commit(descriptor, commit);
    await fixture.client.commit(descriptor, structuredClone(commit));
    await expect(fixture.client.commit(descriptor, { ...commit,
      completion: { ...commit.completion, reason: 'duration' } })).rejects.toThrow(/completion chronology/);
    await expect(fixture.client.commit(descriptor, createGenerationCommit(1n))).rejects.toThrow(/completion chronology/);
    await expect(fixture.client.commit(descriptor, { ...commit,
      completion: { ...commit.completion, completedStep: u64(11n) } })).rejects.toThrow(/invalid chronology/);
    await fixture.client.close();
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      const row = db.prepare('SELECT record_blob FROM rust_generation_completion_v1').get() as { record_blob: Buffer };
      expect(row.record_blob.length).toBe(32);
      expect(row.record_blob.readUInt32LE(4)).toBe(0);
      expect(row.record_blob.readDoubleLE(8)).toBe(10 / 60);
      expect(row.record_blob.readDoubleLE(16)).toBe(604800);
      expect(row.record_blob.readBigUInt64LE(24)).toBe(10n);
    } finally { db.close(); }
  });

  it('commits fixed experiment bytes atomically and rejects omitted, changed and mismatched retries', async () => {
    const fixture = createFixture(); await fixture.client.commit(createDescriptor(fixture.managedRoot));
    const descriptor = createDescriptor(fixture.managedRoot, { operationId: 'acacacacacacacacacacacacacacacac',
      boundaryKind: 'generation', generation: u64(2n), completedStep: u64(10n), transitionEpoch: u64(10n) });
    const base = { ...createGenerationCommit(1n), completion: { version: 1 as const, reason: 'manual' as const,
      actualElapsedF64Hex: f64(10 / 60), configuredDurationF64Hex: f64(604800), completedStep: u64(10n) } };
    const bytes = createExperimentPayload(base);
    const experiment = { version: 1 as const, historyHex: bytes.subarray(0, 512).toString('hex'), hallOfFameHex: bytes.subarray(512).toString('hex') };
    const commit = { ...base, experiment };
    await fixture.client.commit(descriptor, commit);
    await fixture.client.commit(descriptor, structuredClone(commit));
    await expect(fixture.client.commit(descriptor, base)).rejects.toThrow(/experiment summary/);
    const changed = Buffer.from(bytes); changed.writeBigUInt64LE(1n, 63 * 8);
    await expect(fixture.client.commit(descriptor, { ...commit, experiment: { ...experiment, historyHex: changed.subarray(0, 512).toString('hex') } })).rejects.toThrow(/experiment summary/);
    const mismatch = Buffer.from(bytes); mismatch.writeBigUInt64LE(2n, 2 * 8); mismatch.writeBigUInt64LE(2n, 65 * 8);
    await expect(fixture.client.commit(descriptor, { ...commit, experiment: { ...experiment,
      historyHex: mismatch.subarray(0, 512).toString('hex'), hallOfFameHex: mismatch.subarray(512).toString('hex') } })).rejects.toThrow(/disagrees/);
    expect((await fixture.client.readBrowserHistory(descriptor.runId))[0]!.experiment).toMatchObject({
      reason: 'manual', lives: u64(2n), population: 2, winner: { version: 2, pointMethod: 'reset', displayLengthKind: 'latest-life' }
    });
    expect((await fixture.client.readBrowserHallOfFame(descriptor.runId))[0]).toMatchObject({
      points: 6.25, length: 9, assessment: { version: 2, fitnessBundle: 'legacy-terminal', economics: { physicalWallet: 6.25 } }
    });
    const lease = await fixture.client.acquireCurrentExportLease();
    expect(lease.inventory).toMatchObject({ version: 3, storedByteCount: u64(1024n) });
    const inventoryBytes = readFileSync(join(fixture.managedRoot, lease.inventory.relativeFilename));
    expect(inventoryBytes.subarray(0, 13).toString('ascii')).toBe('SLITHER-EXPV3');
    expect(inventoryBytes.subarray(256).equals(bytes)).toBe(true);
    const target = createFixture();
    copyFileSync(join(fixture.managedRoot, descriptor.relativeFilename), join(target.managedRoot, descriptor.relativeFilename));
    copyFileSync(join(fixture.managedRoot, commit.hallOfFameWeights.relativeFilename), join(target.managedRoot, commit.hallOfFameWeights.relativeFilename));
    const importOperation = 'ad'.repeat(16);
    const relativeFilename = '.' + importOperation + '.import-inventory-v3';
    copyFileSync(join(fixture.managedRoot, lease.inventory.relativeFilename), join(target.managedRoot, relativeFilename));
    const imported = { ...descriptor, operationId: importOperation };
    const inventory = { ...lease.inventory, relativeFilename };
    await target.client.commitImport(imported, inventory);
    expect(await target.client.readBrowserHistory(descriptor.runId)).toEqual(await fixture.client.readBrowserHistory(descriptor.runId));
    expect(await target.client.readBrowserHallOfFame(descriptor.runId)).toEqual(await fixture.client.readBrowserHallOfFame(descriptor.runId));
    copyFileSync(join(fixture.managedRoot, lease.inventory.relativeFilename), join(target.managedRoot, relativeFilename));
    await target.client.commitImport(imported, inventory);
    const badBytes = Buffer.from(inventoryBytes);
    badBytes.writeBigUInt64LE(1n, 256 + 63 * 8);
    writeFileSync(join(target.managedRoot, relativeFilename), badBytes);
    await expect(target.client.commitImport(imported, { ...inventory, sha256: createHash('sha256').update(badBytes).digest('hex') })).rejects.toThrow(/experiment identity conflicts/);
    expect((await target.client.readBrowserHistory(descriptor.runId))[0]!.experiment?.ledgerOperations).toBe(u64(0n));
    const corrupt = new Database(target.databasePath);
    try {
      const wrongGeneration = Buffer.from(bytes);
      wrongGeneration.writeBigUInt64LE(2n, 16); wrongGeneration.writeBigUInt64LE(2n, 520);
      corrupt.prepare('UPDATE rust_generation_experiment_v1 SET record_blob = ?').run(wrongGeneration);
      await expect(target.client.readBrowserHistory(descriptor.runId)).rejects.toThrow(/experiment generation mismatch/);
      await expect(target.client.readBrowserHallOfFame(descriptor.runId)).rejects.toThrow(/experiment source mismatch/);
      const wrongPhysical = Buffer.from(bytes); wrongPhysical.writeBigUInt64LE(18n, 536);
      corrupt.prepare('UPDATE rust_generation_experiment_v1 SET record_blob = ?').run(wrongPhysical);
      await expect(target.client.readBrowserHallOfFame(descriptor.runId)).rejects.toThrow(/experiment source mismatch/);
      corrupt.prepare('UPDATE rust_generation_experiment_v1 SET record_blob = ?').run(bytes);
    } finally { corrupt.close(); }
    await fixture.client.releaseExportLease(lease.operationId);
    await fixture.client.close();
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      const row = db.prepare('SELECT record_blob FROM rust_generation_experiment_v1').get() as { record_blob: Buffer };
      expect(row.record_blob.equals(bytes)).toBe(true);
      expect((db.prepare('SELECT record_blob FROM rust_generation_history_v1').get() as { record_blob: Buffer }).record_blob.length).toBe(56);
    } finally { db.close(); }
  });

  it('preserves explicit point/length methods and full-width counters in strict experiment projections', async () => {
    createFixture();
    const base = { ...createGenerationCommit(1n), completion: { version: 1 as const, reason: 'manual' as const, actualElapsedF64Hex: f64(10 / 60), configuredDurationF64Hex: f64(604800), completedStep: u64(10n) } };
    for (const [mode, method, kind, length] of [[0, 'reset', 'latest-life', 9], [1, 'cumulative-net', 'generation-peak', 12], [2, 'cumulative-gross', 'generation-peak', 12], [3, 'best-life', 'selected-life-peak', 9]] as const) {
      const bytes = createExperimentPayload(base);
      /** Set exact schema words without executing or duplicating a fitness formula. */
      const word = (index: number, value: bigint): void => { bytes.writeBigUInt64LE(value, index * 8); };
      word(1, BigInt(mode)); word(60, mode === 2 ? 1n : 0n); word(64 + 4, BigInt(mode)); word(64 + 6, mode === 2 ? 1n : 0n); word(64 + 31, BigInt(mode));
      word(64 + 27, 12n); word(64 + 28, BigInt(length)); word(64 + 29, mode === 0 ? 0n : mode === 3 ? 2n : 1n);
      if (mode === 1 || mode === 2) { word(59, 0n); word(64 + 7, 0n); word(64 + 26, 0n); }
      const summary = decodeExperimentSummary(bytes);
      expect(summary.winner).toMatchObject({ pointMethod: method, displayLengthKind: kind, displayLength: length, latestLength: 9, generationPeakLength: 12 });
      expect(parseExperimentSummary(summary)).toEqual(summary);
      expect(parseHallOfFameAssessment(summary.winner)).toEqual(summary.winner);
      expect(() => parseExperimentSummary({ ...summary, unknown: true })).toThrow();
      expect(() => parseHallOfFameAssessment({ ...summary.winner, displayLength: 1 })).toThrow();
      expect(() => parseExperimentSummary({ ...summary, winner: { ...summary.winner, pointMethod: 'unknown' } })).toThrow();
    }
    const bytes = createExperimentPayload(base);
    for (const [index, value] of [[0, 2n], [7, 301n], [11, BigInt('0x' + f64(Number.NaN))], [20, BigInt('0x' + f64(-1))], [37, 1n], [64, 1n], [64 + 3, 0n], [64 + 29, 1n]] as const) {
      const bad = Buffer.from(bytes); bad.writeBigUInt64LE(value, index * 8);
      expect(() => parseGenerationExperimentRecord({ version: 1, historyHex: bad.subarray(0, 512).toString('hex'), hallOfFameHex: bad.subarray(512).toString('hex') })).toThrow();
    }
    expect(() => parseGenerationExperimentRecord({ version: 1, historyHex: bytes.subarray(0, 511).toString('hex'), hallOfFameHex: bytes.subarray(512).toString('hex') })).toThrow();
    bytes.writeBigUInt64LE(0xffffffffffffffffn, 63 * 8);
    expect(decodeExperimentSummary(bytes).ledgerOperations).toBe('ffffffffffffffff');
  });

  it.each(['before-commit', 'after-commit-before-reply'] as const)(
    'keeps experiment generation publication atomic across failure %s', async phase => {
      const fixture = createFixture();
      const start = createDescriptor(fixture.managedRoot);
      await fixture.client.commit(start); await fixture.client.close();
      const writer = new CheckpointPersistenceClient({ databasePath: fixture.databasePath, managedRootPath: fixture.managedRoot, existingOnly: true, checkpointCommitFailpointForTesting: phase });
      clients.push(writer);
      const descriptor = createDescriptor(fixture.managedRoot, { operationId: 'b1'.repeat(16), boundaryKind: 'generation', generation: u64(2n), completedStep: u64(10n), transitionEpoch: u64(10n) });
      const base = { ...createGenerationCommit(1n), completion: { version: 1 as const, reason: 'manual' as const, actualElapsedF64Hex: f64(10 / 60), configuredDurationF64Hex: f64(604800), completedStep: u64(10n) } };
      const bytes = createExperimentPayload(base);
      const commit = { ...base, experiment: { version: 1 as const, historyHex: bytes.subarray(0, 512).toString('hex'), hallOfFameHex: bytes.subarray(512).toString('hex') } };
      await expect(writer.commit(descriptor, commit)).rejects.toThrow(phase === 'before-commit' ? /injected.*failure/ : /worker|exit|stopped/);
      const database = new Database(fixture.databasePath, { readonly: true });
      try {
        expect(database.prepare('SELECT COUNT(*) AS count FROM rust_generation_experiment_v1').get()).toEqual({ count: phase === 'before-commit' ? 0 : 1 });
        expect(readCurrentPointer(fixture.databasePath, descriptor.runId)?.checkpoint_id).toBe(phase === 'before-commit' ? start.logicalRootSha256 : descriptor.logicalRootSha256);
      } finally { database.close(); }
      const retry = phase === 'before-commit' ? writer : new CheckpointPersistenceClient({ databasePath: fixture.databasePath, managedRootPath: fixture.managedRoot, existingOnly: true });
      if (retry !== writer) clients.push(retry);
      await retry.commit(descriptor, commit);
      expect((await retry.readBrowserHistory(descriptor.runId))[0]!.experiment?.winner.version).toBe(2);
    }
  );

  it.each(['before-commit', 'after-commit-before-reply'] as const)(
    'keeps version-three experiment import atomic across failure %s', async phase => {
      const source = createFixture();
      await source.client.commit(createDescriptor(source.managedRoot));
      const descriptor = createDescriptor(source.managedRoot, { operationId: 'ae'.repeat(16), boundaryKind: 'generation', generation: u64(2n), completedStep: u64(10n), transitionEpoch: u64(10n) });
      const base = { ...createGenerationCommit(1n), completion: { version: 1 as const, reason: 'manual' as const, actualElapsedF64Hex: f64(10 / 60), configuredDurationF64Hex: f64(604800), completedStep: u64(10n) } };
      const bytes = createExperimentPayload(base);
      await source.client.commit(descriptor, { ...base, experiment: { version: 1, historyHex: bytes.subarray(0, 512).toString('hex'), hallOfFameHex: bytes.subarray(512).toString('hex') } });
      const lease = await source.client.acquireCurrentExportLease();
      const target = createFixture(undefined, undefined, undefined, undefined, phase);
      copyFileSync(join(source.managedRoot, descriptor.relativeFilename), join(target.managedRoot, descriptor.relativeFilename));
      copyFileSync(join(source.managedRoot, base.hallOfFameWeights.relativeFilename), join(target.managedRoot, base.hallOfFameWeights.relativeFilename));
      const operationId = 'af'.repeat(16);
      const relativeFilename = '.' + operationId + '.import-inventory-v3';
      const imported = { ...descriptor, operationId };
      /** Recreate the exact immutable transfer for each retry after a worker failure. */
      const inventory = (): ManagedImportInventoryDescriptor => {
        copyFileSync(join(source.managedRoot, lease.inventory.relativeFilename), join(target.managedRoot, relativeFilename));
        return { ...lease.inventory, relativeFilename };
      };
      await expect(target.client.commitImport(imported, inventory())).rejects.toThrow(phase === 'before-commit' ? /injected managed import failure/ : /worker|exit|stopped/);
      const database = new Database(target.databasePath, { readonly: true });
      try {
        expect(database.prepare('SELECT COUNT(*) AS count FROM rust_generation_experiment_v1').get()).toEqual({ count: phase === 'before-commit' ? 0 : 1 });
        expect(database.prepare('SELECT COUNT(*) AS count FROM rust_generation_completion_v1').get()).toEqual({ count: phase === 'before-commit' ? 0 : 1 });
      } finally { database.close(); }
      const retryClient = phase === 'before-commit' ? target.client : new CheckpointPersistenceClient({ databasePath: target.databasePath, managedRootPath: target.managedRoot, existingOnly: true });
      if (retryClient !== target.client) clients.push(retryClient);
      await retryClient.commitImport(imported, inventory());
      expect((await retryClient.readBrowserHistory(descriptor.runId))[0]!.experiment).toMatchObject({ winner: { physicalId: u64(17n), version: 2 } });
      await source.client.releaseExportLease(lease.operationId);
    }
  );

  it('replays all generation metadata exactly and rejects changed records', async () => {
    const fixture = createFixture();
    const runStart = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(runStart);
    const descriptor = createDescriptor(fixture.managedRoot, {
      operationId: 'edededededededededededededededed',
      boundaryKind: 'generation',
      generation: u64(2n),
      completedStep: u64(3_600n),
      transitionEpoch: u64(3_600n)
    });
    const generationCommit = createGenerationCommit(1n);
    await fixture.client.commit(descriptor, generationCommit);
    await fixture.client.commit({ ...descriptor }, {
      summary: { ...generationCommit.summary },
      hallOfFame: { ...generationCommit.hallOfFame },
      hallOfFameWeights: { ...generationCommit.hallOfFameWeights }
    });
    await expect(fixture.client.commit(
      descriptor,
      createGenerationCommit(1n, { averageF64Hex: f64(7.5) })
    )).rejects.toThrow(/different compact generation history/);
    await expect(fixture.client.commit(
      descriptor,
      createGenerationCommit(1n, {}, { pointsF64Hex: f64(6.5) })
    )).rejects.toThrow(/different Hall-of-Fame reference/);
    const corrupt = new Database(fixture.databasePath);
    try {
      corrupt.prepare(
        'UPDATE rust_generation_history_v1 SET record_version = 2 WHERE checkpoint_id = ?'
      ).run(descriptor.logicalRootSha256);
    } finally {
      corrupt.close();
    }
    await expect(fixture.client.commit(descriptor, generationCommit)).rejects.toThrow(
      /different compact generation history/
    );
    await fixture.client.close();
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 2 });
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_generation_history_v1').get()).toEqual({ count: 1 });
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_hall_of_fame_v1').get()).toEqual({ count: 1 });
    } finally {
      db.close();
    }
  });

  it('rolls back history, metadata, and pointer when the Hall-of-Fame reference cannot insert', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const forged = createGenerationCommit(1n).hallOfFame;
    const db = new Database(fixture.databasePath);
    try {
      db.pragma('foreign_keys = ON');
      db.prepare(`
        INSERT INTO rust_hall_of_fame_v1 (
          run_id, generation_hex, checkpoint_id, record_version, record_blob,
          fitness_value, pinned, weight_state, created_at_ms
        ) VALUES (?, ?, ?, ?, ?, ?, 0, 'legacy', ?)
      `).run(
        first.runId, forged.completedGeneration, first.logicalRootSha256,
        1, Buffer.alloc(56), 0, Date.now()
      );
    } finally {
      db.close();
    }
    const second = createDescriptor(fixture.managedRoot, {
      operationId: 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(second, createGenerationCommit(1n))).rejects.toThrow(
      /UNIQUE constraint failed/
    );
    await fixture.client.close();
    const check = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(check.prepare(
        'SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata WHERE checkpoint_id = ?'
      ).get(second.logicalRootSha256)).toEqual({ count: 0 });
      expect(check.prepare(
        'SELECT COUNT(*) AS count FROM rust_generation_history_v1 WHERE checkpoint_id = ?'
      ).get(second.logicalRootSha256)).toEqual({ count: 0 });
      expect(readCurrentPointer(fixture.databasePath, first.runId)?.checkpoint_id).toBe(
        first.logicalRootSha256
      );
    } finally {
      check.close();
    }
  });
});
