import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { existsSync,mkdirSync,readFileSync,readdirSync,rmSync,statSync,unlinkSync,writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { Worker } from 'node:worker_threads';
import { describe,expect,it,vi } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { SUITE,clients,createDescriptor,createFixture,createGenerationCommit,createHallOfFameWeights,readCurrentPointer,u64 } from './checkpointPersistenceFixture.ts';
import type {
ManagedCheckpointDescriptor
} from './checkpointPersistenceProtocol.ts';

describe(SUITE, { timeout: 30_000 }, () => {

  it('verifies referenced objects before removing only unreferenced final managed files', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, {
      operationId: '51'.repeat(16),
      transitionEpoch: u64(2n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    const generationCommit = createGenerationCommit(1n);
    await fixture.client.commit(first);
    await fixture.client.commit(second, generationCommit);
    await fixture.client.close();

    const orphanCheckpoint = join(fixture.managedRoot, `${'a'.repeat(64)}.checkpoint-v3`);
    const orphanWinner = join(fixture.managedRoot, `${'b'.repeat(64)}.hof-weights-v1`);
    const unknownFile = join(fixture.managedRoot, 'owner-notes.checkpoint-v3');
    writeFileSync(orphanCheckpoint, 'orphan checkpoint');
    writeFileSync(orphanWinner, 'orphan winner');
    writeFileSync(unknownFile, 'leave me alone');
    const unreferenced = new Database(fixture.databasePath);
    const unusedFiles: string[] = [];
    try {
      const insert = unreferenced.prepare(`INSERT INTO rust_hall_of_fame_weights_v1
        (logical_sha256, relative_filename, encoding, stored_byte_count_hex,
          decoded_byte_count_hex, weight_count_hex, created_at_ms)
        VALUES (?, ?, ?, ?, ?, ?, 0)`);
      unreferenced.transaction(() => {
        for (let generation = 100n; generation < 170n; generation++) {
          const weights = createHallOfFameWeights(generation);
          unusedFiles.push(weights.relativeFilename);
          insert.run(weights.logicalSha256, weights.relativeFilename, weights.encoding,
            weights.storedByteCount, weights.decodedByteCount, weights.weightCount);
        }
      })();
    } finally { unreferenced.close(); }

    const reopened = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(reopened);
    await expect(reopened.selectStartup()).resolves.toMatchObject({ descriptor: second });
    expect(existsSync(orphanCheckpoint)).toBe(false);
    expect(existsSync(orphanWinner)).toBe(false);
    expect(unusedFiles.every(file => !existsSync(join(fixture.managedRoot, file)))).toBe(true);
    expect(existsSync(unknownFile)).toBe(true);
    const cleaned = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(cleaned.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_weights_v1').get())
        .toEqual({ count: 1 });
    } finally { cleaned.close(); }
    expect(existsSync(join(fixture.managedRoot, first.relativeFilename))).toBe(true);
    expect(existsSync(join(fixture.managedRoot, second.relativeFilename))).toBe(true);
    expect(existsSync(join(
      fixture.managedRoot,
      generationCommit.hallOfFameWeights.relativeFilename
    ))).toBe(true);
    await reopened.close();

    const protectedOrphan = join(fixture.managedRoot, `${'c'.repeat(64)}.checkpoint-v3`);
    writeFileSync(protectedOrphan, 'do not delete after failed verification');
    rmSync(join(fixture.managedRoot, second.relativeFilename));
    const damaged = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(damaged);
    await expect(damaged.selectStartup()).resolves.toMatchObject({ descriptor: second });
    expect(existsSync(protectedOrphan)).toBe(true);
  });

  it('reclaims only unreferenced finals after leases release and preserves files when a retained reference is invalid', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(descriptor);
    const lease = await fixture.client.acquireCurrentExportLease();
    const orphanCheckpoint = join(fixture.managedRoot, `${'a'.repeat(64)}.checkpoint-v3`);
    const orphanElite = join(fixture.managedRoot, `${'b'.repeat(64)}.hof-weights-v1`);
    const unknown = join(fixture.managedRoot, 'owner-notes.txt');
    writeFileSync(orphanCheckpoint, 'abandoned checkpoint');
    writeFileSync(orphanElite, 'abandoned elite');
    writeFileSync(unknown, 'preserve owner notes');
    await expect(fixture.client.reclaimManagedOrphans()).resolves.toEqual({ completed: false,
      deletedCheckpointCount: u64(0n), deletedHallOfFameCount: u64(0n), deletedStoredByteCount: u64(0n) });
    expect(existsSync(orphanCheckpoint) && existsSync(orphanElite)).toBe(true);
    await fixture.client.releaseExportLease(lease.operationId);
    const deletedBytes = BigInt(statSync(orphanCheckpoint).size + statSync(orphanElite).size);
    await expect(fixture.client.reclaimManagedOrphans()).resolves.toEqual({ completed: true,
      deletedCheckpointCount: u64(1n), deletedHallOfFameCount: u64(1n), deletedStoredByteCount: u64(deletedBytes) });
    expect(existsSync(orphanCheckpoint) || existsSync(orphanElite)).toBe(false);
    expect(existsSync(unknown)).toBe(true);
    expect(await fixture.client.selectCurrent()).toEqual(descriptor);
    writeFileSync(orphanCheckpoint, 'preserve after failed reference check');
    rmSync(join(fixture.managedRoot, descriptor.relativeFilename));
    await expect(fixture.client.reclaimManagedOrphans()).resolves.toMatchObject({ completed: false,
      deletedCheckpointCount: u64(0n), deletedHallOfFameCount: u64(0n), deletedStoredByteCount: u64(0n) });
    expect(existsSync(orphanCheckpoint)).toBe(true);
  });

  it('classifies old managed files and returns bounded owner-policy retention accounting', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, {
      operationId: 'abababababababababababababababab',
      transitionEpoch: u64(2n),
      generation: u64(2n),
      completedStep: u64(3_600n),
      boundaryKind: 'generation'
    });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    const initial = await fixture.client.inspectRetention();
    expect(initial).toMatchObject({
      schemaVersion: 1,
      activeRunId: first.runId,
      retained: {
        latest: { checkpointCount: 1 },
        recent: { checkpointCount: 1 },
        milestone: { checkpointCount: 0 },
        priorRunAnchor: { checkpointCount: 0 },
        pinned: { checkpointCount: 0 }
      },
      plannedPrune: { checkpointCount: 0 }
    });
    expect(initial.retained.latest.encodings.rawWeights).toBe(1);
    expect(initial.retained.recent.encodings.rawRecurrent).toBe(1);
    await fixture.client.close();

    const oldSchema = new Database(fixture.databasePath);
    try { oldSchema.exec(`
      PRAGMA foreign_keys = OFF;
      CREATE TABLE rust_hall_of_fame_v1_old (
        run_id TEXT NOT NULL,
        generation_hex TEXT NOT NULL,
        checkpoint_id TEXT NOT NULL UNIQUE REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
        record_version INTEGER NOT NULL,
        record_blob BLOB NOT NULL CHECK(length(record_blob) = 56),
        created_at_ms INTEGER NOT NULL,
        PRIMARY KEY (run_id, generation_hex)
      );
      INSERT INTO rust_hall_of_fame_v1_old
        SELECT run_id, generation_hex, checkpoint_id, record_version, record_blob, created_at_ms
        FROM rust_hall_of_fame_v1;
      DROP TABLE rust_hall_of_fame_v1;
      ALTER TABLE rust_hall_of_fame_v1_old RENAME TO rust_hall_of_fame_v1;
      DROP TABLE rust_hall_of_fame_weights_v1;
      DROP TABLE rust_checkpoint_retention_v1;
      CREATE TABLE rust_checkpoint_retention_v1 (
        checkpoint_id TEXT PRIMARY KEY NOT NULL REFERENCES rust_checkpoint_v3_metadata(checkpoint_id),
        retention_kind TEXT NOT NULL CHECK(retention_kind IN ('automatic', 'pinned')),
        classified_at_ms INTEGER NOT NULL
      );
      INSERT INTO rust_checkpoint_retention_v1
        SELECT checkpoint_id, 'automatic', 0 FROM rust_checkpoint_v3_metadata;
    `); }
    finally { oldSchema.close(); }
    const reopened = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(reopened);
    const backfilled = await reopened.inspectRetention();
    expect(backfilled.automaticStoredByteCount).toBe(initial.automaticStoredByteCount);
    expect(backfilled.retained.latest.checkpointCount + backfilled.retained.recent.checkpointCount).toBe(2);
    const migrated = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(migrated.prepare('SELECT weights_sha256 FROM rust_hall_of_fame_v1').get()).toEqual({
        weights_sha256: null
      });
      expect(migrated.prepare(`SELECT count(*) AS count FROM sqlite_schema
        WHERE type = 'table' AND name = 'rust_hall_of_fame_weights_v1'`).get()).toEqual({ count: 1 });
      expect(migrated.prepare(`SELECT count(*) AS count FROM sqlite_schema
        WHERE type = 'index' AND name = 'rust_hof_weights_reference_v1'`).get()).toEqual({ count: 1 });
    } finally { migrated.close(); }
    const legacyLease = await reopened.acquireCurrentExportLease();
    expect(legacyLease.inventory).toMatchObject({ historyCount: u64(1n), hallOfFameCount: u64(0n) });
    await reopened.releaseExportLease(legacyLease.operationId);
    await expect(reopened.pinCurrentCheckpoint()).resolves.toEqual({
      checkpointId: second.logicalRootSha256,
      generation: second.generation
    });
    const pinned = await reopened.inspectRetention();
    expect(pinned.retained.pinned.checkpointCount).toBe(1);
    expect(pinned.retained.latest.checkpointCount).toBe(1);
    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare(`SELECT retention_kind, count(*) AS count
        FROM rust_checkpoint_retention_v1 GROUP BY retention_kind ORDER BY retention_kind`).all()).toEqual([
        { retention_kind: 'automatic', count: 1 },
        { retention_kind: 'pinned', count: 1 }
      ]);
    } finally { inspect.close(); }
  });

  it('resumes two-phase pruning, preserves pins, and keeps compact history', async () => {
    const fixture = createFixture();
    const descriptors = [createDescriptor(fixture.managedRoot)];
    await fixture.client.commit(descriptors[0]!);
    for (let generation = 2n; generation <= 11n; generation++) {
      const descriptor = createDescriptor(fixture.managedRoot, {
        operationId: generation.toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation),
        generation: u64(generation),
        completedStep: u64((generation - 1n) * 3_600n),
        boundaryKind: 'generation'
      });
      descriptors.push(descriptor);
      await fixture.client.commit(descriptor, createGenerationCommit(generation - 1n));
      if (generation === 2n) await fixture.client.pinCurrentCheckpoint();
    }
    const before = await fixture.client.inspectRetention();
    expect(before.retained.pinned.checkpointCount).toBe(1);
    expect(before.plannedPrune.checkpointCount).toBe(2);

    await fixture.client.close();
    const interrupted = new Database(fixture.databasePath);
    try {
      interrupted.prepare(`UPDATE rust_checkpoint_retention_v1
        SET retention_kind = 'pruning' WHERE checkpoint_id = ?`).run(descriptors[0]!.logicalRootSha256);
    } finally { interrupted.close(); }
    rmSync(join(fixture.managedRoot, descriptors[0]!.relativeFilename));
    const reopened = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(reopened);
    const result = await reopened.applyRetention();
    expect(result.deletedCheckpointCount).toBe(2);
    expect(result.inventory).toMatchObject({
      retained: {
        latest: { checkpointCount: 1 },
        recent: { checkpointCount: 7 },
        pinned: { checkpointCount: 1 }
      },
      plannedPrune: { checkpointCount: 0 }
    });
    expect(existsSync(join(fixture.managedRoot, descriptors[0]!.relativeFilename))).toBe(false);
    expect(existsSync(join(fixture.managedRoot, descriptors[1]!.relativeFilename))).toBe(true);
    expect(existsSync(join(fixture.managedRoot, descriptors[2]!.relativeFilename))).toBe(false);
    expect(await reopened.selectCurrent()).toEqual(descriptors.at(-1));
    await reopened.close();

    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 11 });
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_generation_history_v1').get()).toEqual({ count: 10 });
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_v1').get()).toEqual({ count: 10 });
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_weights_v1').get()).toEqual({ count: 10 });
      expect(inspect.prepare(`SELECT count(*) AS count FROM rust_hall_of_fame_v1
        WHERE weights_sha256 IS NOT NULL`).get()).toEqual({ count: 10 });
      const weightFiles = inspect.prepare(`SELECT relative_filename FROM rust_hall_of_fame_weights_v1`)
        .all() as Array<{ relative_filename: string }>;
      expect(weightFiles.every(row => existsSync(join(fixture.managedRoot, row.relative_filename)))).toBe(true);
      expect(inspect.prepare(`SELECT retention_kind, count(*) AS count
        FROM rust_checkpoint_retention_v1 GROUP BY retention_kind ORDER BY retention_kind`).all()).toEqual([
        { retention_kind: 'automatic', count: 8 },
        { retention_kind: 'pinned', count: 1 },
        { retention_kind: 'pruned', count: 2 }
      ]);
    } finally { inspect.close(); }

    const historical = new Database(fixture.databasePath);
    try {
      historical.prepare('UPDATE rust_checkpoint_v3_metadata SET descriptor_json = ? WHERE checkpoint_id = ?')
        .run('invalid old descriptor', descriptors[0]!.logicalRootSha256);
    } finally { historical.close(); }
    const stalePrunedFile = join(fixture.managedRoot, descriptors[0]!.relativeFilename);
    writeFileSync(stalePrunedFile, 'stale pruned file');
    const restarted = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(restarted);
    await expect(restarted.selectStartup()).resolves.toMatchObject({ descriptor: descriptors.at(-1) });
    expect(existsSync(stalePrunedFile)).toBe(false);
  });

  it('preprunes measured physical storage before a bounded checkpoint publication', async () => {
    const fixture = createFixture();
    const fileBytes = 1024 * 1024;
    const descriptors: ManagedCheckpointDescriptor[] = [];
    for (let generation = 1n; generation <= 10n; generation++) {
      const bytes = Buffer.alloc(fileBytes, Number(generation));
      const digest = createHash('sha256').update(bytes).digest('hex');
      const filename = `${digest}.checkpoint-v3`;
      writeFileSync(join(fixture.managedRoot, filename), bytes);
      const descriptor = createDescriptor(fixture.managedRoot, {
        operationId: generation.toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation),
        generation: u64(generation),
        completedStep: u64((generation - 1n) * 3_600n),
        boundaryKind: generation === 1n ? 'run-start' : 'generation',
        logicalRootSha256: digest,
        relativeFilename: filename,
        storedByteCount: u64(BigInt(fileBytes)),
        decodedByteCount: u64(BigInt(fileBytes))
      });
      descriptors.push(descriptor);
      await fixture.client.commit(descriptor,
        generation === 1n ? null : createGenerationCommit(generation - 1n));
      if (generation === 2n) await fixture.client.pinCurrentCheckpoint();
    }
    await fixture.client.applyRetention();
    const before = await fixture.client.inspectRetention();
    /** Count the same managed files and SQLite sidecars charged by production. */
    const physicalBytes = (): bigint => {
      let total = BigInt(statSync(fixture.databasePath).size);
      for (const suffix of ['-wal', '-shm']) {
        const path = `${fixture.databasePath}${suffix}`;
        if (existsSync(path)) total += BigInt(statSync(path).size);
      }
      for (const name of readdirSync(fixture.managedRoot)) {
        total += BigInt(statSync(join(fixture.managedRoot, name)).size);
      }
      return total;
    };
    const automatic = BigInt(`0x${before.automaticStoredByteCount}`);
    const pinned = BigInt(`0x${before.pinnedStoredByteCount}`);
    const other = physicalBytes() - automatic - pinned;
    const cap = BigInt(`0x${before.automaticByteCap}`);
    const reserve = cap - other - BigInt(3 * fileBytes + fileBytes / 2);
    const result = await fixture.client.applyRetention(reserve);
    expect(result.inventory.retained.pinned.checkpointCount).toBe(1);
    expect(result.inventory.automaticStoredByteCount).toBe(u64(BigInt(3 * fileBytes)));
    expect(await fixture.client.selectCurrent()).toEqual(descriptors.at(-1));
    expect(physicalBytes() - pinned + reserve).toBeLessThanOrEqual(cap);
    expect(existsSync(join(fixture.managedRoot, descriptors[1]!.relativeFilename))).toBe(true);
    expect(existsSync(join(fixture.managedRoot, descriptors[6]!.relativeFilename))).toBe(false);
    expect(existsSync(join(fixture.managedRoot, descriptors[7]!.relativeFilename))).toBe(true);
    const afterOther = physicalBytes() - BigInt(3 * fileBytes) - pinned;
    const unsafeReserve = cap - afterOther - BigInt(2 * fileBytes) + 1n;
    await expect(fixture.client.applyRetention(unsafeReserve)).rejects.toThrow(/protected automatic checkpoints/);
    expect(await fixture.client.selectCurrent()).toEqual(descriptors.at(-1));
    expect(existsSync(join(fixture.managedRoot, descriptors[1]!.relativeFilename))).toBe(true);
    expect(existsSync(join(fixture.managedRoot, descriptors[8]!.relativeFilename))).toBe(true);
  });

  it('charges an in-flight export validation directory without faulting the checkpoint', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(descriptor, null);
    const validationDirectory = join(fixture.managedRoot,
      `.${'a'.repeat(32)}.import-validation`);
    mkdirSync(validationDirectory);
    writeFileSync(join(validationDirectory, `${'b'.repeat(64)}.checkpoint-v3`), Buffer.alloc(1024 * 1024));
    const before = await fixture.client.inspectRetention();
    let physicalBytes = BigInt(statSync(fixture.databasePath).size);
    for (const suffix of ['-wal', '-shm']) {
      const path = `${fixture.databasePath}${suffix}`;
      if (existsSync(path)) physicalBytes += BigInt(statSync(path).size);
    }
    physicalBytes += BigInt(statSync(join(fixture.managedRoot, descriptor.relativeFilename)).size) +
      BigInt(statSync(validationDirectory).size) +
      BigInt(statSync(join(validationDirectory, `${'b'.repeat(64)}.checkpoint-v3`)).size);
    const automatic = BigInt(`0x${before.automaticStoredByteCount}`);
    const pinned = BigInt(`0x${before.pinnedStoredByteCount}`);
    const cap = BigInt(`0x${before.automaticByteCap}`);
    const reserve = cap - (physicalBytes - automatic - pinned) - automatic;
    expect(reserve).toBeGreaterThan(0n);
    await expect(fixture.client.applyRetention(reserve)).resolves.toMatchObject({
      inventory: { automaticStoredByteCount: before.automaticStoredByteCount }
    });
    await expect(fixture.client.applyRetention(reserve + 1n))
      .rejects.toThrow(/protected automatic checkpoints/);
    expect(await fixture.client.selectCurrent()).toEqual(descriptor);
    mkdirSync(join(fixture.managedRoot, 'unknown-directory'));
    await expect(fixture.client.applyRetention(0n))
      .rejects.toThrow(/non-file during budget admission: unknown-directory/);
  });

  it('uses the selected automatic byte cap in the worker inventory and pruning decision', async () => {
    const fixture = createFixture();
    await fixture.client.close();
    const budgeted = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath, managedRootPath: fixture.managedRoot,
      existingOnly: true, automaticByteCapBytes: 96n
    });
    clients.push(budgeted);
    const descriptors: ManagedCheckpointDescriptor[] = [];
    for (let generation = 1n; generation <= 4n; generation++) {
      const descriptor = createDescriptor(fixture.managedRoot, {
        operationId: generation.toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation), generation: u64(generation),
        completedStep: u64((generation - 1n) * 3_600n),
        boundaryKind: generation === 1n ? 'run-start' : 'generation'
      });
      descriptors.push(descriptor);
      await budgeted.commit(descriptor,
        generation === 1n ? null : createGenerationCommit(generation - 1n));
    }
    const before = await budgeted.inspectRetention();
    expect(before.automaticByteCap).toBe(u64(96n));
    expect(before.plannedPrune.checkpointCount).toBe(1);
    const result = await budgeted.applyRetention();
    expect(result.inventory.automaticStoredByteCount).toBe(u64(96n));
    expect(existsSync(join(fixture.managedRoot, descriptors[0]!.relativeFilename))).toBe(false);
    expect(await budgeted.selectCurrent()).toEqual(descriptors.at(-1));
  });

  it('detaches only expired prior-run pointers when pruning their checkpoint files', async () => {
    const fixture = createFixture();
    const descriptors: ManagedCheckpointDescriptor[] = [];
    for (let index = 1; index <= 4; index++) {
      const descriptor = createDescriptor(fixture.managedRoot, {
        runId: `retention-run-${index}`,
        operationId: index.toString(16).padStart(32, '0')
      });
      descriptors.push(descriptor);
      await fixture.client.commit(descriptor, null, true);
    }
    const before = await fixture.client.inspectRetention();
    expect(before.retained.priorRunAnchor.checkpointCount).toBe(2);
    expect(before.plannedPrune.checkpointCount).toBe(1);

    const result = await fixture.client.applyRetention();
    expect(result.deletedCheckpointCount).toBe(1);
    expect(result.inventory.plannedPrune.checkpointCount).toBe(0);
    expect(await fixture.client.selectCurrent()).toEqual(descriptors[3]);
    expect(existsSync(join(fixture.managedRoot, descriptors[0]!.relativeFilename))).toBe(false);
    for (const descriptor of descriptors.slice(1)) {
      expect(existsSync(join(fixture.managedRoot, descriptor.relativeFilename))).toBe(true);
    }

    await fixture.client.close();
    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT run_id FROM rust_checkpoint_v3_current ORDER BY run_id').all()).toEqual(
        descriptors.slice(1).map(descriptor => ({ run_id: descriptor.runId }))
      );
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 4 });
      expect(inspect.prepare(`SELECT retention_kind FROM rust_checkpoint_retention_v1 WHERE checkpoint_id = ?`)
        .get(descriptors[0]!.logicalRootSha256)).toEqual({ retention_kind: 'pruned' });
    } finally { inspect.close(); }
  });

  it.each(['partial', 'final'] as const)('preserves a pre-existing export inventory %s and permits a fresh lease', async suffix => {
    const fixture = createFixture();
    await fixture.client.commit(createDescriptor(fixture.managedRoot));
    const descriptor = createDescriptor(fixture.managedRoot, {
      operationId: '22'.repeat(16), transitionEpoch: u64(2n), generation: u64(2n),
      completedStep: u64(3_600n), boundaryKind: 'generation'
    });
    await fixture.client.commit(descriptor, createGenerationCommit(1n));
    /** Read every compact Rust metadata row without changing the worker-owned database. */
    const metadataRows = (): unknown => {
      const database = new Database(fixture.databasePath, { readonly: true });
      try {
        const tables = database.prepare(`SELECT name FROM sqlite_master
          WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
        return tables.map(({ name }) => ({ name, rows: database.prepare(
          `SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`
        ).all() }));
      } finally { database.close(); }
    };
    const beforeMetadata = metadataRows();
    /** Record every small managed fixture's bytes without including operation scratch. */
    const managedFiles = () => readdirSync(fixture.managedRoot).sort().map(filename => ({
      filename, bytes: readFileSync(join(fixture.managedRoot, filename))
    }));
    const before = managedFiles();
    const pointer = readCurrentPointer(fixture.databasePath, descriptor.runId);
    const earlierBytes = Buffer.from('pre-existing inventory evidence must survive exclusive-create failure');
    let collisionPath: string | undefined;
    const originalPost = Worker.prototype.postMessage;
    /** Insert the collision before the real worker receives its unpredictable operation ID. */
    const posting = vi.spyOn(Worker.prototype, 'postMessage').mockImplementation(function(this: Worker, ...args) {
      const message = args[0] as { type?: string; operationId?: string };
      if (message.type === 'acquireCurrentExportLease' && collisionPath === undefined) {
        expect(message.operationId).toMatch(/^[0-9a-f]{32}$/u);
        collisionPath = join(fixture.managedRoot,
          `.${message.operationId}.export-inventory-v1${suffix === 'partial' ? '.partial' : ''}`);
        writeFileSync(collisionPath, earlierBytes, { flag: 'wx' });
      }
      return Reflect.apply(originalPost, this, args);
    });
    try {
      await expect(fixture.client.acquireCurrentExportLease()).rejects.toThrow(/EEXIST|exist/u);
      expect(readFileSync(collisionPath!)).toEqual(earlierBytes);
      unlinkSync(collisionPath!);
      collisionPath = undefined;
      posting.mockRestore();
      expect(managedFiles()).toEqual(before);
      expect(metadataRows()).toEqual(beforeMetadata);
      expect(readCurrentPointer(fixture.databasePath, descriptor.runId)).toEqual(pointer);
      const lease = await fixture.client.acquireCurrentExportLease();
      expect(lease.descriptor).toEqual(descriptor);
      const inventoryPath = join(fixture.managedRoot, lease.inventory.relativeFilename);
      const bytes = readFileSync(inventoryPath);
      expect(createHash('sha256').update(bytes).digest('hex')).toBe(lease.inventory.sha256);
      expect(bytes.subarray(0, 13).toString('ascii')).toBe('SLITHER-EXPV1');
      expect(lease.inventory).toMatchObject({ historyCount: u64(1n), hallOfFameCount: u64(1n),
        storedByteCount: u64(208n) });
      await fixture.client.releaseExportLease(lease.operationId);
      expect(managedFiles()).toEqual(before);
      expect(readCurrentPointer(fixture.databasePath, descriptor.runId)).toEqual(pointer);
      expect(metadataRows()).toEqual(beforeMetadata);
    } finally {
      posting.mockRestore();
      if (collisionPath && existsSync(collisionPath)) unlinkSync(collisionPath);
    }
  });

  it('keeps one exact export lease alive across later checkpoints and pruning', async () => {
    const fixture = createFixture();
    const descriptors = [createDescriptor(fixture.managedRoot)];
    await fixture.client.commit(descriptors[0]!);
    let lease: Awaited<ReturnType<CheckpointPersistenceClient['acquireCurrentExportLease']>> | undefined;
    for (let generation = 2n; generation <= 11n; generation++) {
      const descriptor = createDescriptor(fixture.managedRoot, {
        operationId: (generation + 32n).toString(16).padStart(32, '0'),
        transitionEpoch: u64(generation),
        generation: u64(generation),
        completedStep: u64((generation - 1n) * 3_600n),
        boundaryKind: 'generation'
      });
      descriptors.push(descriptor);
      await fixture.client.commit(descriptor, createGenerationCommit(generation - 1n));
      if (generation === 2n) lease = await fixture.client.acquireCurrentExportLease();
    }
    expect(lease).toMatchObject({ runId: descriptors[1]!.runId, descriptor: descriptors[1] });
    expect(lease?.inventory).toMatchObject({
      version: 1,
      historyCount: u64(1n),
      hallOfFameCount: u64(1n),
      storedByteCount: u64(208n)
    });
    const inventoryPath = join(fixture.managedRoot, lease!.inventory.relativeFilename);
    const inventoryBytes = readFileSync(inventoryPath);
    expect(inventoryBytes.subarray(0, 13).toString('ascii')).toBe('SLITHER-EXPV1');
    expect(createHash('sha256').update(inventoryBytes).digest('hex')).toBe(lease!.inventory.sha256);
    await expect(fixture.client.acquireCurrentExportLease()).rejects.toThrow(/busy/);

    const protectedCleanup = await fixture.client.applyRetention();
    expect(protectedCleanup.deletedCheckpointCount).toBe(2);
    expect(protectedCleanup.inventory.plannedPrune.checkpointCount).toBe(1);
    expect(existsSync(join(fixture.managedRoot, descriptors[1]!.relativeFilename))).toBe(true);
    await fixture.client.releaseExportLease(lease!.operationId);
    expect(existsSync(inventoryPath)).toBe(false);

    const releasedCleanup = await fixture.client.applyRetention();
    expect(releasedCleanup.deletedCheckpointCount).toBe(1);
    expect(releasedCleanup.inventory.plannedPrune.checkpointCount).toBe(0);
    expect(existsSync(join(fixture.managedRoot, descriptors[1]!.relativeFilename))).toBe(false);
    await expect(fixture.client.releaseExportLease(lease!.operationId)).rejects.toThrow(/not active/);
  });
});
