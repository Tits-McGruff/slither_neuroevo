import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { copyFileSync,existsSync,rmSync } from 'node:fs';
import { join } from 'node:path';
import { describe,expect,it } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { SUITE,clients,createDescriptor,createFixture,createGenerationCommit,createGenerationSummary,createSymlinkOrSkip,decodeGenerationSummary,decodeHallOfFameReference,f64,readCurrentPointer,u64 } from './checkpointPersistenceFixture.ts';
import type {
ManagedImportInventoryDescriptor
} from './checkpointPersistenceProtocol.ts';

describe(SUITE, { timeout: 30_000 }, () => {

  it('rolls back every metadata row when the real checkpoint transaction fails before commit', async () => {
    const fixture = createFixture(undefined, undefined, undefined, 'before-commit');
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/injected checkpoint failure before SQLite commit/u);
    expect(readCurrentPointer(fixture.databasePath, descriptor.runId)).toBeUndefined();
    const database = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(database.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 0 });
    } finally { database.close(); }
    await expect(fixture.client.commit(descriptor)).resolves.toMatchObject({
      operationId: descriptor.operationId,
      checkpointId: descriptor.logicalRootSha256
    });
  });

  it('replays the exact committed checkpoint after its worker exits before acknowledgement', async () => {
    const fixture = createFixture(undefined, undefined, undefined, 'after-commit-before-reply');
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit(descriptor)).rejects.toThrow(/worker|exit|stopped/u);
    expect(readCurrentPointer(fixture.databasePath, descriptor.runId)).toMatchObject({
      checkpoint_id: descriptor.logicalRootSha256,
      operation_id: descriptor.operationId
    });
    const reopened = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(reopened);
    await expect(reopened.commit(descriptor)).resolves.toMatchObject({
      operationId: descriptor.operationId,
      checkpointId: descriptor.logicalRootSha256
    });
    const database = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(database.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 1 });
    } finally { database.close(); }
  });

  it('imports complete history and shared weight objects in one current-pointer transaction', async () => {
    const source = createFixture();
    await source.client.commit(createDescriptor(source.managedRoot));
    for (let generation = 2n; generation <= 3n; generation++) {
      await source.client.commit(createDescriptor(source.managedRoot, {
        operationId: (generation + 160n).toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation), generation: u64(generation),
        completedStep: u64((generation - 1n) * 3_600n), boundaryKind: 'generation'
      }), createGenerationCommit(generation - 1n, { bestF64Hex: f64(Number(generation)) }));
    }
    const lease = await source.client.acquireCurrentExportLease();
    expect(lease.inventory).toMatchObject({ historyCount: u64(2n), hallOfFameCount: u64(2n) });

    const target = createFixture();
    copyFileSync(join(source.managedRoot, lease.descriptor.relativeFilename),
      join(target.managedRoot, lease.descriptor.relativeFilename));
    const sourceDatabase = new Database(source.databasePath, { readonly: true });
    try {
      const objects = sourceDatabase.prepare('SELECT relative_filename FROM rust_hall_of_fame_weights_v1')
        .all() as Array<{ relative_filename: string }>;
      for (const object of objects) copyFileSync(join(source.managedRoot, object.relative_filename),
        join(target.managedRoot, object.relative_filename));
    } finally { sourceDatabase.close(); }
    const operationId = 'ef'.repeat(16);
    const relativeFilename = `.${operationId}.import-inventory-v1`;
    copyFileSync(join(source.managedRoot, lease.inventory.relativeFilename), join(target.managedRoot, relativeFilename));
    const descriptor = { ...lease.descriptor, operationId };
    const inventory: ManagedImportInventoryDescriptor = {
      ...lease.inventory,
      relativeFilename
    };
    const committed = await target.client.commitImport(descriptor, inventory);
    expect(committed).toMatchObject({ checkpointId: descriptor.logicalRootSha256, descriptor });
    expect(await target.client.selectCurrent()).toEqual(descriptor);
    expect(existsSync(join(target.managedRoot, relativeFilename))).toBe(false);

    const inspect = new Database(target.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_generation_history_v1').get()).toEqual({ count: 2 });
      expect(inspect.prepare(`SELECT count(*) AS count FROM rust_hall_of_fame_v1
        WHERE weight_state = 'selected'`).get()).toEqual({ count: 2 });
      expect(inspect.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
        .toEqual({ run_id: descriptor.runId });
    } finally { inspect.close(); }
    await source.client.releaseExportLease(lease.operationId);
  });

  it.each(['before-commit', 'after-commit-before-reply'] as const)(
    'reconciles a managed import worker failure %s', async phase => {
      const source = createFixture();
      await source.client.commit(createDescriptor(source.managedRoot));
      const lease = await source.client.acquireCurrentExportLease();
      const target = createFixture(undefined, undefined, undefined, undefined, phase);
      copyFileSync(join(source.managedRoot, lease.descriptor.relativeFilename),
        join(target.managedRoot, lease.descriptor.relativeFilename));
      const operationId = 'f0'.repeat(16);
      const relativeFilename = `.${operationId}.import-inventory-v1`;
      const descriptor = { ...lease.descriptor, operationId };
      /** Recreate the exact import inventory after an attempted transaction. */
      const copyInventory = (): ManagedImportInventoryDescriptor => {
        copyFileSync(join(source.managedRoot, lease.inventory.relativeFilename),
          join(target.managedRoot, relativeFilename));
        return { ...lease.inventory, relativeFilename };
      };
      await expect(target.client.commitImport(descriptor, copyInventory())).rejects.toThrow(
        phase === 'before-commit' ? /injected managed import failure before SQLite commit/u : /worker|exit|stopped/u
      );
      if (phase === 'before-commit') {
        expect(readCurrentPointer(target.databasePath, descriptor.runId)).toBeUndefined();
        await expect(target.client.commitImport(descriptor, copyInventory())).resolves.toMatchObject({
          checkpointId: descriptor.logicalRootSha256
        });
      } else {
        expect(readCurrentPointer(target.databasePath, descriptor.runId)).toMatchObject({
          checkpoint_id: descriptor.logicalRootSha256
        });
        const reopened = new CheckpointPersistenceClient({ databasePath: target.databasePath,
          managedRootPath: target.managedRoot, existingOnly: true });
        clients.push(reopened);
        await expect(reopened.selectStartup()).resolves.toMatchObject({
          runId: descriptor.runId, descriptor
        });
        await expect(reopened.commitImport(descriptor, copyInventory())).resolves.toMatchObject({
          checkpointId: descriptor.logicalRootSha256
        });
      }
      const database = new Database(target.databasePath, { readonly: true });
      try {
        expect(database.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_metadata').get())
          .toEqual({ count: 1 });
        expect(database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get())
          .toEqual({ run_id: descriptor.runId });
      } finally { database.close(); }
      await source.client.releaseExportLease(lease.operationId);
    }
  );

  it.each([null, 'conflicting-import-branch'] as const)(
    'rejects a different root at an existing run/generation with branch=%s', async branchRunId => {
      const fixture = createFixture();
      const first = createDescriptor(fixture.managedRoot);
      await fixture.client.commit(first);
      const lease = await fixture.client.acquireCurrentExportLease();
      const second = createDescriptor(fixture.managedRoot, { operationId: '96'.repeat(16),
        transitionEpoch: u64(2n), generation: u64(2n), completedStep: u64(3_600n), boundaryKind: 'generation' });
      await fixture.client.commit(second, createGenerationCommit(1n));
      const operationId = '97'.repeat(16);
      const conflicting = createDescriptor(fixture.managedRoot, { operationId });
      expect(conflicting.logicalRootSha256).not.toBe(first.logicalRootSha256);
      const relativeFilename = `.${operationId}.import-inventory-v1`;
      /** Recreate the trusted empty generation-one inventory after each terminal attempt. */
      const inventory = (): ManagedImportInventoryDescriptor => {
        copyFileSync(join(fixture.managedRoot, lease.inventory.relativeFilename), join(fixture.managedRoot, relativeFilename));
        return { ...lease.inventory, relativeFilename };
      };
      /** Observe every persisted application row independently of the current-pointer check. */
      const rows = (): unknown => {
        const database = new Database(fixture.databasePath, { readonly: true });
        try {
          const tables = database.prepare(`SELECT name FROM sqlite_master
            WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
          return tables.map(({ name }) => ({ name,
            rows: database.prepare(`SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`).all() }));
        } finally { database.close(); }
      };
      const before = rows();
      await expect(fixture.client.commitImport(conflicting, inventory(), branchRunId))
        .rejects.toThrow('generation identity conflicts with different immutable content');
      expect(rows()).toEqual(before);
      expect(await fixture.client.selectCurrent(first.runId)).toEqual(second);
      await expect(fixture.client.commitImport({ ...first, operationId }, inventory(), 'valid-original-branch'))
        .resolves.toMatchObject({ runId: 'valid-original-branch', checkpointId: first.logicalRootSha256 });
      expect(await fixture.client.selectCurrent(first.runId)).toEqual(second);
      await fixture.client.releaseExportLease(lease.operationId);
    }
  );
  it('uses boundary identity rather than resettable Rust operation epochs to advance current', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const committed = await fixture.client.commit(first);
    expect(committed).toEqual({
      operationId: first.operationId,
      transitionEpoch: first.transitionEpoch,
      runId: first.runId,
      checkpointId: first.logicalRootSha256,
      descriptor: first
    });

    const second = createDescriptor(fixture.managedRoot, {
      operationId: 'fedcba9876543210fedcba9876543210',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(second, createGenerationCommit(1n));
    const third = createDescriptor(fixture.managedRoot, {
      operationId: '11111111111111111111111111111111',
      transitionEpoch: u64(3_600n),
      generation: u64(3n),
      completedStep: u64(7_200n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(third, createGenerationCommit(2n));
    await fixture.client.close();

    expect(readCurrentPointer(fixture.databasePath, first.runId)).toEqual({
      checkpoint_id: third.logicalRootSha256,
      transition_epoch: third.transitionEpoch,
      operation_id: third.operationId
    });
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      const rows = db.prepare(`
        SELECT generation_hex, checkpoint_id, record_version, record_blob
        FROM rust_generation_history_v1 ORDER BY generation_hex
      `).all() as Array<{
        generation_hex: string;
        checkpoint_id: string;
        record_version: number;
        record_blob: Buffer;
      }>;
      expect(rows.map(row => ({
        generation_hex: row.generation_hex,
        checkpoint_id: row.checkpoint_id,
        record_version: row.record_version,
        summary: decodeGenerationSummary(row.record_blob)
      }))).toEqual([
        {
          generation_hex: u64(1n),
          checkpoint_id: second.logicalRootSha256,
          record_version: 1,
          summary: createGenerationSummary(1n)
        },
        {
          generation_hex: u64(2n),
          checkpoint_id: third.logicalRootSha256,
          record_version: 1,
          summary: createGenerationSummary(2n)
        }
      ]);
      const hallOfFameRows = db.prepare(`
        SELECT generation_hex, checkpoint_id, record_version, record_blob
        FROM rust_hall_of_fame_v1 ORDER BY generation_hex
      `).all() as Array<{
        generation_hex: string;
        checkpoint_id: string;
        record_version: number;
        record_blob: Buffer;
      }>;
      expect(hallOfFameRows.map(row => ({
        generation_hex: row.generation_hex,
        checkpoint_id: row.checkpoint_id,
        record_version: row.record_version,
        reference: decodeHallOfFameReference(row.record_blob)
      }))).toEqual([
        {
          generation_hex: u64(1n),
          checkpoint_id: second.logicalRootSha256,
          record_version: 1,
          reference: createGenerationCommit(1n).hallOfFame
        },
        {
          generation_hex: u64(2n),
          checkpoint_id: third.logicalRootSha256,
          record_version: 1,
          reference: createGenerationCommit(2n).hallOfFame
        }
      ]);
    } finally {
      db.close();
    }
  });

  it('commits a zero-weight descriptor for a valid parameterless population graph', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot, {
      weightCount: u64(0n)
    });
    const committed = await fixture.client.commit(descriptor);
    expect(committed.checkpointId).toBe(descriptor.logicalRootSha256);
    await fixture.client.close();

    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare(
        'SELECT weight_count_hex FROM rust_checkpoint_v3_metadata WHERE checkpoint_id = ?'
      ).get(descriptor.logicalRootSha256)).toEqual({ weight_count_hex: u64(0n) });
    } finally {
      db.close();
    }
    expect(readCurrentPointer(fixture.databasePath, descriptor.runId)?.checkpoint_id).toBe(
      descriptor.logicalRootSha256
    );
  });

  it('is idempotent for an exact replay without duplicating metadata or regressing the pointer', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(descriptor);
    await fixture.client.commit({ ...descriptor });
    await fixture.client.close();

    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 1 });
    } finally {
      db.close();
    }
    expect(readCurrentPointer(fixture.databasePath, descriptor.runId)?.checkpoint_id).toBe(
      descriptor.logicalRootSha256
    );
  });

  it('rejects a replay that was superseded by a later current pointer', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, {
      operationId: 'fedcba9876543210fedcba9876543210',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    await expect(fixture.client.commit(first)).rejects.toThrow(/superseded/);
  });

  it('rejects immutable conflicts and stale or skipped generations without changing current', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const sameOperationDifferentDescriptor = createDescriptor(fixture.managedRoot, {
      operationId: first.operationId,
      stateVersion: u64(2n)
    });
    await expect(fixture.client.commit(sameOperationDifferentDescriptor)).rejects.toThrow(/operationId conflicts/);
    const sameRootDifferentOperation = createDescriptor(fixture.managedRoot, {
      operationId: '44444444444444444444444444444444',
      logicalRootSha256: first.logicalRootSha256,
      relativeFilename: first.relativeFilename,
      storedByteCount: first.storedByteCount,
      decodedByteCount: first.decodedByteCount
    });
    await expect(fixture.client.commit(sameRootDifferentOperation)).rejects.toThrow(/logical checkpoint root/);

    const second = createDescriptor(fixture.managedRoot, {
      operationId: '55555555555555555555555555555555',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(second, createGenerationCommit(1n));
    const stale = createDescriptor(fixture.managedRoot, {
      operationId: '66666666666666666666666666666666',
      transitionEpoch: u64(1n),
      generation: u64(2n),
      completedStep: u64(3_000n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(stale, createGenerationCommit(1n))).rejects.toThrow(/stale/);
    const gapped = createDescriptor(fixture.managedRoot, {
      operationId: '77777777777777777777777777777777',
      transitionEpoch: u64(4n),
      generation: u64(4n),
      completedStep: u64(10_800n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(gapped, createGenerationCommit(3n))).rejects.toThrow(/exactly one/);
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)).toEqual({
      checkpoint_id: second.logicalRootSha256,
      transition_epoch: second.transitionEpoch,
      operation_id: second.operationId
    });
  });

  it('rejects and preserves a current pointer whose operation identity no longer matches metadata', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const forgedOperationId = 'ffffffffffffffffffffffffffffffff';
    const corrupt = new Database(fixture.databasePath);
    try {
      corrupt.prepare(`
        UPDATE rust_checkpoint_v3_current
        SET operation_id = ?
        WHERE run_id = ?
      `).run(forgedOperationId, first.runId);
    } finally {
      corrupt.close();
    }
    const second = createDescriptor(fixture.managedRoot, {
      operationId: '12121212121212121212121212121212',
      transitionEpoch: u64(3_600n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await expect(fixture.client.commit(second, createGenerationCommit(1n))).rejects.toThrow(
      /pointer identity does not match immutable metadata/
    );
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)).toEqual({
      checkpoint_id: first.logicalRootSha256,
      transition_epoch: first.transitionEpoch,
      operation_id: forgedOperationId
    });
    const check = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(check.prepare(
        'SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata WHERE checkpoint_id = ?'
      ).get(second.logicalRootSha256)).toEqual({ count: 0 });
      expect(check.prepare('SELECT COUNT(*) AS count FROM rust_generation_history_v1').get()).toEqual({ count: 0 });
      expect(check.prepare('SELECT COUNT(*) AS count FROM rust_hall_of_fame_v1').get()).toEqual({ count: 0 });
    } finally {
      check.close();
    }
  });

  it('accepts any positive first operation epoch because it is an acknowledgement token', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot, { transitionEpoch: u64(2_741n) });
    await expect(fixture.client.commit(descriptor)).resolves.toMatchObject({
      transitionEpoch: u64(2_741n)
    });
  });

  it('rejects zero-step generation boundaries and first pointers without branch provenance', async () => {
    const fixture = createFixture();
    const runStart = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(runStart);
    const zeroStep = createDescriptor(fixture.managedRoot, {
      operationId: 'abababababababababababababababab',
      boundaryKind: 'generation',
      generation: u64(2n),
      completedStep: u64(0n),
      transitionEpoch: u64(1n)
    });
    await expect(fixture.client.commit(zeroStep, createGenerationCommit(1n))).rejects.toThrow(
      /completedStep must be nonzero/
    );
    const branchWithoutProvenance = createDescriptor(fixture.managedRoot, {
      operationId: 'cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd',
      runId: 'branch-without-provenance',
      boundaryKind: 'generation',
      generation: u64(8n),
      completedStep: u64(25_200n),
      transitionEpoch: u64(1n)
    });
    await expect(fixture.client.commit(
      branchWithoutProvenance,
      createGenerationCommit(7n)
    )).rejects.toThrow(/explicit branch provenance/);
  });

  it('rejects traversal, binary/population fields, invalid digest, and out-of-range count before publication', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit({
      ...descriptor,
      relativeFilename: '../escape.checkpoint-v3'
    })).rejects.toThrow(/digest-derived/);
    await expect(fixture.client.commit({
      ...descriptor,
      logicalRootSha256: 'A'.repeat(64)
    })).rejects.toThrow(/SHA-256/);
    const populationPayload = new Uint8Array(2 * 1024 * 1024);
    await expect(fixture.client.commit({ ...descriptor, population: populationPayload })).rejects.toThrow(
      /prohibited payload field/
    );
    await expect(fixture.client.commit({
      ...descriptor,
      populationCount: 'ffffffffffffffff'
    })).rejects.toThrow(/populationCount/);
  });

  it('rejects distinct lone-surrogate run IDs before UTF-8 encoding or SQLite key insertion', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await expect(fixture.client.commit({
      ...descriptor,
      runId: `run-${String.fromCharCode(0xd800)}`
    })).rejects.toThrow(/well-formed UTF-16/);
    await expect(fixture.client.commit({
      ...descriptor,
      operationId: '88888888888888888888888888888888',
      runId: `run-${String.fromCharCode(0xd801)}`
    })).rejects.toThrow(/well-formed UTF-16/);
    await fixture.client.close();
    const db = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(db.prepare('SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 0 });
    } finally {
      db.close();
    }
  });

  it('rejects a symlinked final file without changing the existing pointer', async context => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const second = createDescriptor(fixture.managedRoot, {
      operationId: '11111111111111111111111111111111',
      transitionEpoch: u64(2n)
    });
    const actualPath = join(fixture.managedRoot, second.relativeFilename);
    rmSync(actualPath);
    if (!createSymlinkOrSkip(context, join(fixture.managedRoot, first.relativeFilename), actualPath)) return;

    await expect(fixture.client.commit(second)).rejects.toThrow(/never a symlink/);
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)?.checkpoint_id).toBe(first.logicalRootSha256);
  });

  it('preserves the old pointer when a final file is missing or its length disagrees with the descriptor', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(first);
    const missingRoot = createHash('sha256').update('missing').digest('hex');
    const missing = createDescriptor(fixture.managedRoot, {
      operationId: '22222222222222222222222222222222',
      transitionEpoch: u64(2n),
      logicalRootSha256: missingRoot,
      relativeFilename: `${missingRoot}.checkpoint-v3`
    });
    await expect(fixture.client.commit(missing)).rejects.toThrow(/ENOENT|no such file/i);

    const mismatch = createDescriptor(fixture.managedRoot, {
      operationId: '33333333333333333333333333333333',
      transitionEpoch: u64(2n),
      storedByteCount: u64(1n)
    });
    await expect(fixture.client.commit(mismatch)).rejects.toThrow(/size does not match/);
    await fixture.client.close();
    expect(readCurrentPointer(fixture.databasePath, first.runId)).toMatchObject({
      checkpoint_id: first.logicalRootSha256,
      transition_epoch: first.transitionEpoch
    });
  });
});
