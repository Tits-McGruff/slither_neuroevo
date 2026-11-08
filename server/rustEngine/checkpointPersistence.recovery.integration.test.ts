import Database from 'better-sqlite3';
import { readFileSync,readdirSync,unlinkSync,writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe,expect,it } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { SUITE,clients,createDescriptor,createFixture,createGenerationCommit,createHallOfFameWeights,f64,u64 } from './checkpointPersistenceFixture.ts';
import type {
ManagedImportInventoryDescriptor
} from './checkpointPersistenceProtocol.ts';

describe(SUITE, { timeout: 30_000 }, () => {

  it('scans corrupt retained metadata newest first with bounded records and exact cursor retry', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, { operationId: '88'.repeat(16),
      generation: u64(2n), completedStep: u64(60n), boundaryKind: 'generation' });
    const third = createDescriptor(fixture.managedRoot, { operationId: '99'.repeat(16),
      generation: u64(3n), completedStep: u64(120n), boundaryKind: 'generation' });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    await fixture.client.commit(third, createGenerationCommit(2n));
    await fixture.client.close();
    const failedCheckpointId = 'f'.repeat(64);
    const db = new Database(fixture.databasePath);
    try {
      db.pragma('foreign_keys = OFF');
      db.prepare('UPDATE rust_checkpoint_v3_current SET checkpoint_id = ?').run(failedCheckpointId);
      db.prepare('UPDATE rust_checkpoint_v3_metadata SET descriptor_json = ? WHERE checkpoint_id = ?').run('x'.repeat(32_768), third.logicalRootSha256);
      db.prepare('UPDATE rust_checkpoint_v3_metadata SET descriptor_json = ? WHERE checkpoint_id = ?').run('{broken', second.logicalRootSha256);
    } finally { db.close(); }
    const client = new CheckpointPersistenceClient({ databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot, existingOnly: true });
    clients.push(client);
    const newest = await client.scanRecoveryCandidate();
    expect(newest).toMatchObject({ exhausted: false, descriptor: null, issue: 'retained checkpoint metadata is invalid',
      cursor: { generation: u64(3n), checkpointId: third.logicalRootSha256, failedCheckpointId } });
    const middle = await client.scanRecoveryCandidate(newest.cursor);
    expect(middle).toMatchObject({ descriptor: null, cursor: { generation: u64(2n) } });
    expect(await client.scanRecoveryCandidate(newest.cursor)).toEqual(middle);
    const oldest = await client.scanRecoveryCandidate(middle.cursor);
    expect(oldest.descriptor).toEqual(first);
    expect(await client.scanRecoveryCandidate(oldest.cursor)).toMatchObject({ exhausted: true, descriptor: null, issue: null });
    await client.close();
    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 3 });
      expect(inspect.prepare('SELECT length(descriptor_json) AS size FROM rust_checkpoint_v3_metadata WHERE checkpoint_id = ?').get(third.logicalRootSha256))
        .toEqual({ size: 32_768 });
    } finally { inspect.close(); }

  });

  it('rejects a recovery cursor after the pinned source pointer advances', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const scanning = fixture.client.scanRecoveryCandidate();
    await expect(fixture.client.scanRecoveryCandidate()).rejects.toThrow(/busy/);
    const initial = await scanning;
    const second = createDescriptor(fixture.managedRoot, { operationId: 'aa'.repeat(16),
      generation: u64(2n), completedStep: u64(60n), boundaryKind: 'generation' });
    await fixture.client.commit(second, createGenerationCommit(1n));
    await expect(fixture.client.scanRecoveryCandidate(initial.cursor)).rejects.toThrow(/source pointer changed/);
    expect((await fixture.client.scanRecoveryCandidate()).descriptor).toEqual(second);
  });

  it.each(['current', 'older'] as const)('selects a prior-run %s checkpoint by exact ID without rewriting either source', async boundary => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, { operationId: '41'.repeat(16),
      generation: u64(2n), completedStep: u64(60n), boundaryKind: 'generation' });
    const other = createDescriptor(fixture.managedRoot, { operationId: '42'.repeat(16), runId: 'newer-run' });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    await fixture.client.commit(other, null, true);
    const selected = boundary === 'current' ? second : first;
    const beforeFiles = readdirSync(fixture.managedRoot).sort().map(name => ({ name,
      bytes: readFileSync(join(fixture.managedRoot, name)) }));
    /** Read unchanged immutable metadata and compact source records. */
    const records = (): unknown => {
      const database = new Database(fixture.databasePath, { readonly: true });
      try { return ['rust_checkpoint_v3_metadata', 'rust_generation_history_v1', 'rust_hall_of_fame_v1',
        'rust_checkpoint_retention_v1'].map(table => database.prepare(`SELECT * FROM ${table} ORDER BY rowid`).all()); }
      finally { database.close(); }
    };
    const before = records();
    const selecting = fixture.client.selectRetainedCheckpoint(selected.logicalRootSha256);
    await expect(fixture.client.selectRetainedCheckpoint(selected.logicalRootSha256)).rejects.toThrow('busy');
    const exact = await selecting;
    expect(exact).toMatchObject({ descriptor: selected, exhausted: false, cursor: {
      sourceRunId: first.runId, failedCheckpointId: second.logicalRootSha256,
      explicitResume: { activeRunId: other.runId, activeCheckpointId: other.logicalRootSha256 }
    } });
    expect(await fixture.client.selectCurrent()).toEqual(other);
    await expect(fixture.client.scanRecoveryCandidate(exact.cursor)).rejects.toThrow('cannot continue as an automatic recovery scan');
    await expect(fixture.client.selectRetainedCheckpoint('f'.repeat(64))).rejects.toThrow('not retained');
    const request = { operationId: '43'.repeat(16), branchRunId: 'explicit-branch', sourceRunId: first.runId,
      failedCheckpointId: second.logicalRootSha256, recoveredDescriptor: selected };
    await expect(fixture.client.commitRecoveryBranch(request)).rejects.toThrow('source is no longer active');
    const explicit = { ...request, explicitResume: exact.cursor.explicitResume! };
    const branch = await fixture.client.commitRecoveryBranch(explicit);
    expect(await fixture.client.commitRecoveryBranch(explicit)).toEqual(branch);
    expect(await fixture.client.selectStartup()).toMatchObject({ descriptor: selected, runId: 'explicit-branch', recovery: branch });
    expect(await fixture.client.selectCurrent(first.runId)).toEqual(second);
    expect(await fixture.client.selectCurrent(other.runId)).toEqual(other);
    expect(records()).toEqual(before);
    expect(readdirSync(fixture.managedRoot).sort().map(name => ({ name,
      bytes: readFileSync(join(fixture.managedRoot, name)) }))).toEqual(beforeFiles);
  });

  it.each(['active advances', 'source advances', 'target pruned'] as const)(
    'rejects an explicit-resume transaction after %s', async change => {
      const fixture = createFixture();
      const source = createDescriptor(fixture.managedRoot);
      const active = createDescriptor(fixture.managedRoot, { operationId: '51'.repeat(16), runId: 'active-run' });
      await fixture.client.commit(source);
      await fixture.client.commit(active, null, true);
      const exact = await fixture.client.selectRetainedCheckpoint(source.logicalRootSha256);
      if (change === 'target pruned') {
        const database = new Database(fixture.databasePath);
        try { database.prepare("UPDATE rust_checkpoint_retention_v1 SET retention_kind = 'pruned' WHERE checkpoint_id = ?")
          .run(source.logicalRootSha256); } finally { database.close(); }
      } else {
        const prior = change === 'active advances' ? active : source;
        const successor = createDescriptor(fixture.managedRoot, { operationId: '52'.repeat(16), runId: prior.runId,
          generation: u64(2n), completedStep: u64(60n), boundaryKind: 'generation' });
        await fixture.client.commit(successor, createGenerationCommit(1n));
      }
      const pointer = await fixture.client.selectStartup();
      await expect(fixture.client.commitRecoveryBranch({ operationId: '53'.repeat(16), branchRunId: 'rejected-branch',
        sourceRunId: source.runId, failedCheckpointId: source.logicalRootSha256, recoveredDescriptor: source,
        explicitResume: exact.cursor.explicitResume! })).rejects.toThrow(
          change === 'target pruned' ? 'no longer retained' : change === 'source advances' ? 'source pointer changed' : 'active pointer changed');
      expect(await fixture.client.selectStartup()).toEqual(pointer);
      const database = new Database(fixture.databasePath, { readonly: true });
      try { expect(database.prepare('SELECT count(*) AS count FROM rust_recovery_branches_v1').get()).toEqual({ count: 0 }); }
      finally { database.close(); }
    }
  );

  it('branches from an older retained boundary without changing the failed suffix and advances independently', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, { operationId: '22'.repeat(16),
      generation: u64(2n), completedStep: u64(60n), boundaryKind: 'generation' });
    const third = createDescriptor(fixture.managedRoot, { operationId: '33'.repeat(16),
      generation: u64(3n), completedStep: u64(120n), boundaryKind: 'generation' });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    await fixture.client.commit(third, createGenerationCommit(2n));
    const originalFile = readFileSync(join(fixture.managedRoot, third.relativeFilename));
    const request = { operationId: '44'.repeat(16), branchRunId: 'recovered-lineage',
      sourceRunId: first.runId, failedCheckpointId: third.logicalRootSha256,
      recoveredDescriptor: second, compatibleBuild: true as const };
    const result = await fixture.client.commitRecoveryBranch(request);
    expect(result).toEqual({ ...request, abandonedThroughGeneration: u64(3n) });
    expect(await fixture.client.commitRecoveryBranch(request)).toEqual(result);
    await expect(fixture.client.selectCurrent(request.branchRunId)).rejects.toThrow(/provenance-aware/);
    const branchLease = await fixture.client.acquireCurrentExportLease();
    expect(branchLease).toMatchObject({ runId: request.branchRunId, descriptor: second,
      inventory: { historyCount: u64(1n), hallOfFameCount: u64(1n) } });
    await fixture.client.releaseExportLease(branchLease.operationId);
    const successor = createDescriptor(fixture.managedRoot, { operationId: '55'.repeat(16),
      runId: request.branchRunId, generation: u64(3n), completedStep: u64(121n), boundaryKind: 'generation' });
    await fixture.client.commit(successor, createGenerationCommit(2n, { bestF64Hex: f64(20) }));
    await expect(fixture.client.commitRecoveryBranch(request)).rejects.toThrow(/superseded/);
    expect(await fixture.client.selectCurrent(request.branchRunId)).toEqual(successor);
    expect(await fixture.client.selectCurrent(first.runId)).toEqual(third);
    expect(await fixture.client.readBrowserHistory(request.branchRunId)).toEqual([
      { gen: 1, best: 12.5, avg: 7.25, min: -1.5, speciesCount: 2,
        topSpeciesSize: 1, avgWeight: 0.125, weightVariance: 0.03125 },
      { gen: 2, best: 20, avg: 7.25, min: -1.5, speciesCount: 2,
        topSpeciesSize: 1, avgWeight: 0.125, weightVariance: 0.03125 }
    ]);
    expect(await fixture.client.readBrowserHistory(request.branchRunId, 1)).toEqual([
      { gen: 2, best: 20, avg: 7.25, min: -1.5, speciesCount: 2,
        topSpeciesSize: 1, avgWeight: 0.125, weightVariance: 0.03125 }
    ]);
    expect(await fixture.client.readBrowserHallOfFame(request.branchRunId)).toEqual([
      { entryId: u64(2n), gen: 2, fitness: 20, points: 6.25, length: 9, pinned: false },
      { entryId: u64(1n), gen: 1, fitness: 12.5, points: 6.25, length: 9, pinned: false }
    ]);
    expect(await fixture.client.readBrowserHallOfFame(request.branchRunId, 1)).toEqual([
      { entryId: u64(2n), gen: 2, fitness: 20, points: 6.25, length: 9, pinned: false }
    ]);
    const selectedWinner = await fixture.client.selectHallOfFameEntry(request.branchRunId, u64(2n));
    expect(selectedWinner).toMatchObject({
      runId: request.branchRunId,
      entryId: u64(2n),
      checkpoint: successor,
      reference: createGenerationCommit(2n, { bestF64Hex: f64(20) }).hallOfFame,
      weights: createHallOfFameWeights(2n)
    });
    await expect(fixture.client.selectHallOfFameEntry(request.branchRunId, u64(1n)))
      .rejects.toThrow(/busy/);
    await fixture.client.releaseHallOfFameEntry(selectedWinner.operationId);
    await fixture.client.close();
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 4 });
      expect(db.prepare('SELECT run_id FROM rust_active_run_v1').get()).toEqual({ run_id: request.branchRunId });
      expect(db.prepare('SELECT source_run_id, recovered_checkpoint_id, history_through_generation_hex FROM rust_recovery_branches_v1').get())
        .toEqual({ source_run_id: first.runId, recovered_checkpoint_id: second.logicalRootSha256, history_through_generation_hex: u64(1n) });
      expect(JSON.parse((db.prepare('SELECT provenance_json FROM rust_recovery_branches_v1').get() as { provenance_json: string }).provenance_json))
        .toMatchObject({ compatibleBuild: true });
      expect(db.prepare('SELECT count(*) AS count FROM rust_generation_history_v1 WHERE run_id = ?').get(first.runId)).toEqual({ count: 2 });
    } finally { db.close(); }
    expect(readFileSync(join(fixture.managedRoot, third.relativeFilename))).toEqual(originalFile);
  });

  it('recovers an already-branched lineage only through its inherited prefix', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, { operationId: 'b1'.repeat(16), generation: u64(2n), completedStep: u64(60n), boundaryKind: 'generation' });
    const third = createDescriptor(fixture.managedRoot, { operationId: 'b2'.repeat(16), generation: u64(3n), completedStep: u64(120n), boundaryKind: 'generation' });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    await fixture.client.commit(third, createGenerationCommit(2n));
    await fixture.client.commitRecoveryBranch({ operationId: 'b3'.repeat(16), branchRunId: 'branch-one',
      sourceRunId: first.runId, failedCheckpointId: third.logicalRootSha256, recoveredDescriptor: second });
    const newest = await fixture.client.scanRecoveryCandidate();
    expect(newest.descriptor).toEqual(second);
    const older = await fixture.client.scanRecoveryCandidate(newest.cursor);
    expect(older.descriptor).toEqual(first);
    const recovery = { operationId: 'b4'.repeat(16), branchRunId: 'branch-two', sourceRunId: 'branch-one',
      failedCheckpointId: second.logicalRootSha256, recoveredDescriptor: first };
    await expect(fixture.client.commitRecoveryBranch({ ...recovery, recoveredDescriptor: third }))
      .rejects.toThrow(/outside the inherited lineage prefix/);
    await expect(fixture.client.commitRecoveryBranch(recovery)).resolves.toMatchObject({ abandonedThroughGeneration: u64(2n) });
    const selected = await fixture.client.selectStartup();
    expect(selected).toMatchObject({ runId: 'branch-two', descriptor: first, recovery });
    expect((await fixture.client.scanRecoveryCandidate()).descriptor).toEqual(first);
  });

  it('rolls back recovery provenance when current-pointer publication fails', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    await fixture.client.close();
    const db = new Database(fixture.databasePath);
    db.exec("CREATE TRIGGER reject_recovery BEFORE INSERT ON rust_checkpoint_v3_current WHEN NEW.run_id = 'branch' BEGIN SELECT RAISE(ABORT, 'injected recovery failure'); END");
    db.close();
    const client = new CheckpointPersistenceClient({ databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot, existingOnly: true });
    clients.push(client);
    const request = { operationId: '66'.repeat(16), branchRunId: 'branch', sourceRunId: first.runId,
      failedCheckpointId: first.logicalRootSha256, recoveredDescriptor: first };
    await expect(client.commitRecoveryBranch(request)).rejects.toThrow(/injected recovery failure/);
    expect(await client.selectCurrent()).toEqual(first);
    await client.close();
    const inspect = new Database(fixture.databasePath);
    try {
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_recovery_branches_v1').get()).toEqual({ count: 0 });
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_active_run_v1').get()).toEqual({ count: 0 });
      inspect.exec('DROP TRIGGER reject_recovery');
    } finally { inspect.close(); }
    const retry = new CheckpointPersistenceClient({ databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot, existingOnly: true });
    clients.push(retry);
    await expect(retry.commitRecoveryBranch(request)).resolves.toMatchObject({ branchRunId: 'branch' });
  });

  it.each(['automatic', 'pinned', 'pruning', 'pruned'] as const)(
    'resumes an older %s checkpoint as an immediately exportable branch without replacing its future', async retentionKind => {
    const fixture = createFixture(undefined, undefined, undefined, undefined, 'before-commit');
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, { operationId: '91'.repeat(16),
      transitionEpoch: u64(2n), generation: u64(2n), completedStep: u64(3_600n), boundaryKind: 'generation' });
    const third = createDescriptor(fixture.managedRoot, { operationId: '92'.repeat(16),
      transitionEpoch: u64(3n), generation: u64(3n), completedStep: u64(7_200n), boundaryKind: 'generation' });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    const lease = await fixture.client.acquireCurrentExportLease();
    await fixture.client.commit(third, createGenerationCommit(2n), true);

    const operationId = '93'.repeat(16);
    const relativeFilename = `.${operationId}.import-inventory-v1`;
    const inventoryBytes = readFileSync(join(fixture.managedRoot, lease.inventory.relativeFilename));
    const checkpointBytes = readFileSync(join(fixture.managedRoot, second.relativeFilename));
    await fixture.client.releaseExportLease(lease.operationId);
    const classified = new Database(fixture.databasePath);
    try {
      classified.prepare(`UPDATE rust_checkpoint_retention_v1
        SET retention_kind = ? WHERE checkpoint_id = ?`).run(retentionKind, second.logicalRootSha256);
    } finally { classified.close(); }
    if (retentionKind === 'pruned') unlinkSync(join(fixture.managedRoot, second.relativeFilename));
    // The native import publisher restores the original bytes before asking the worker to commit.
    writeFileSync(join(fixture.managedRoot, second.relativeFilename), checkpointBytes);
    const copyInventory = (): ManagedImportInventoryDescriptor => {
      writeFileSync(join(fixture.managedRoot, relativeFilename), inventoryBytes);
      return { ...lease.inventory, relativeFilename };
    };
    const imported = { ...second, operationId };
    await expect(fixture.client.commitImport(imported, copyInventory())).rejects.toThrow(/resume it as a branch/);
    const rejected = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(rejected.prepare('SELECT retention_kind FROM rust_checkpoint_retention_v1 WHERE checkpoint_id = ?')
        .get(second.logicalRootSha256)).toEqual({ retention_kind: retentionKind });
    } finally { rejected.close(); }
    const branchRunId = 'owner-selected-import-branch';
    await expect(fixture.client.commitImport(imported, copyInventory(), branchRunId))
      .rejects.toThrow(/injected managed import failure before SQLite commit/);
    const rolledBack = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(rolledBack.prepare('SELECT retention_kind FROM rust_checkpoint_retention_v1 WHERE checkpoint_id = ?')
        .get(second.logicalRootSha256)).toEqual({ retention_kind: retentionKind });
      expect(rolledBack.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
        .toEqual({ run_id: third.runId });
      expect(rolledBack.prepare('SELECT 1 FROM rust_import_branches_v1 WHERE branch_run_id = ?')
        .get(branchRunId)).toBeUndefined();
    } finally { rolledBack.close(); }
    const committed = await fixture.client.commitImport(imported, copyInventory(), branchRunId);
    expect(committed).toMatchObject({ runId: branchRunId, checkpointId: second.logicalRootSha256,
      descriptor: second, importBranch: { operationId, branchRunId, sourceRunId: second.runId,
        sourceGeneration: second.generation, sourceCheckpointId: second.logicalRootSha256,
        recoveredDescriptor: second } });
    const selected = await fixture.client.selectStartup();
    expect(selected).toMatchObject({ runId: branchRunId, descriptor: second,
      recovery: null, importBranch: committed.importBranch });
    expect(await fixture.client.selectCurrent(second.runId)).toEqual(third);
    const branchExport = await fixture.client.acquireCurrentExportLease();
    expect(branchExport).toMatchObject({ runId: branchRunId, descriptor: second });
    await fixture.client.releaseExportLease(branchExport.operationId);
    await fixture.client.close();

    const reopened = new CheckpointPersistenceClient({ databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot, existingOnly: true });
    clients.push(reopened);
    await expect(reopened.selectStartup()).resolves.toMatchObject({ runId: branchRunId,
      descriptor: second, importBranch: committed.importBranch });
    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?')
        .get(second.runId)).toEqual({ checkpoint_id: third.logicalRootSha256 });
      expect(inspect.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?')
        .get(branchRunId)).toEqual({ checkpoint_id: second.logicalRootSha256 });
      expect(inspect.prepare('SELECT retention_kind FROM rust_checkpoint_retention_v1 WHERE checkpoint_id = ?')
        .get(second.logicalRootSha256)).toEqual({ retention_kind: retentionKind === 'pinned' ? 'pinned' : 'automatic' });
      expect(readFileSync(join(fixture.managedRoot, second.relativeFilename))).toEqual(checkpointBytes);
    } finally { inspect.close(); }
  });
});
