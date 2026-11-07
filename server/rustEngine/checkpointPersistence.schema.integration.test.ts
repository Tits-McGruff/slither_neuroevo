import Database from 'better-sqlite3';
import { existsSync,readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe,expect,it } from 'vitest';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import { SUITE,clients,createDescriptor,createFixture,createGenerationCommit,readCurrentPointer,u64 } from './checkpointPersistenceFixture.ts';

describe(SUITE, { timeout: 30_000 }, () => {
  it('reports bounded SQLite file and free-page storage counters', async () => {
    const fixture = createFixture();
    const diagnostics = await fixture.client.inspectStorage();
    const pageSize = BigInt(`0x${diagnostics.pageSizeByteCount}`);
    const pageCount = BigInt(`0x${diagnostics.pageCount}`);
    const freePages = BigInt(`0x${diagnostics.freelistPageCount}`);
    expect(diagnostics.schemaVersion).toBe(1);
    expect(BigInt(`0x${diagnostics.databaseByteCount}`)).toBeGreaterThan(0n);
    expect(BigInt(`0x${diagnostics.walByteCount}`)).toBeGreaterThanOrEqual(0n);
    expect(BigInt(`0x${diagnostics.shmByteCount}`)).toBeGreaterThanOrEqual(0n);
    expect(pageSize).toBeGreaterThanOrEqual(512n);
    expect(pageCount).toBeGreaterThan(0n);
    expect(freePages).toBeLessThanOrEqual(pageCount);
    expect(BigInt(`0x${diagnostics.usedPageByteCount}`)).toBe((pageCount - freePages) * pageSize);
  });

  it('keeps validated graph presets across metadata-worker restarts', async () => {
    const fixture = createFixture();
    const spec: GraphSpec = {
      type: 'graph',
      nodes: [
        { id: 'input', type: 'Input', outputSize: 2 },
        { id: 'head', type: 'Dense', inputSize: 2, outputSize: 2 }
      ],
      edges: [{ from: 'input', to: 'head' }],
      outputs: [{ nodeId: 'head' }],
      outputSize: 2
    };
    const presetId = await fixture.client.saveGraphPreset('  Small graph  ', spec);
    expect(await fixture.client.listGraphPresets()).toEqual([
      { id: presetId, name: 'Small graph', createdAt: expect.any(Number) }
    ]);
    await fixture.client.close();
    const reopened = new CheckpointPersistenceClient({ databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot, existingOnly: true });
    clients.push(reopened);
    await expect(reopened.loadGraphPreset(presetId)).resolves.toMatchObject({
      id: presetId, name: 'Small graph', spec
    });
    await expect(reopened.loadGraphPreset(presetId + 1)).resolves.toBeNull();
  });

  it('rejects legacy schema in managed-only mode before journal or schema writes', async () => {
    const fixture = createFixture();
    await fixture.client.close();
    const path = join(fixture.root, 'legacy.sqlite');
    const database = new Database(path);
    try {
      database.exec('CREATE TABLE population_snapshots (id INTEGER PRIMARY KEY, payload_json TEXT)');
      database.prepare('INSERT INTO population_snapshots VALUES (1, ?)').run('preserved population');
    } finally { database.close(); }
    const before = readFileSync(path);
    const client = new CheckpointPersistenceClient({ databasePath: path,
      managedRootPath: fixture.managedRoot, existingOnly: 'managed' });
    clients.push(client);
    await expect(client.selectCurrent()).rejects.toThrow(/fresh startup requires a compatible managed/u);
    await expect(client.close()).rejects.toThrow();
    expect(readFileSync(path)).toEqual(before);
    expect(existsSync(`${path}-wal`)).toBe(false);
  });

  it('opens an existing managed store in managed-only mode without changing its current checkpoint', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(descriptor);
    await fixture.client.close();
    const client = new CheckpointPersistenceClient({ databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot, existingOnly: 'managed' });
    clients.push(client);
    expect(await client.selectCurrent()).toEqual(descriptor);
    await client.close();
    expect(readCurrentPointer(fixture.databasePath, descriptor.runId)).toMatchObject({
      checkpoint_id: descriptor.logicalRootSha256, operation_id: descriptor.operationId
    });
  });
  it.each([true, 'managed'] as const)('upgrades a pre-telemetry save without changing its population/history (%s)', async existingOnly => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, { operationId: '22'.repeat(16),
      generation: u64(2n), completedStep: u64(60n), boundaryKind: 'generation' });
    await fixture.client.commit(first);
    await fixture.client.commit(second, createGenerationCommit(1n));
    await fixture.client.close();
    const database = new Database(fixture.databasePath);
    let preserved: unknown[];
    try {
      database.exec('DROP TABLE rust_generation_work_v1');
      preserved = ['rust_checkpoint_v3_metadata', 'rust_checkpoint_v3_current',
        'rust_generation_history_v1', 'rust_hall_of_fame_v1']
        .map(table => database.prepare(`SELECT * FROM ${table} ORDER BY rowid`).all());
    } finally { database.close(); }
    const checkpointBytes = readFileSync(join(fixture.managedRoot, second.relativeFilename));
    const reopened = new CheckpointPersistenceClient({ databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot, existingOnly });
    clients.push(reopened);
    expect(await reopened.selectCurrent()).toEqual(second);
    const history = await reopened.readBrowserHistory(second.runId);
    expect(history).toHaveLength(1);
    expect(history[0]).toMatchObject({ gen: 1, best: 12.5 });
    expect(history[0]).not.toHaveProperty('workload');
    await reopened.close();
    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(['rust_checkpoint_v3_metadata', 'rust_checkpoint_v3_current',
        'rust_generation_history_v1', 'rust_hall_of_fame_v1']
        .map(table => inspect.prepare(`SELECT * FROM ${table} ORDER BY rowid`).all())).toEqual(preserved);
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_generation_work_v1').get()).toEqual({ count: 0 });
    } finally { inspect.close(); }
    expect(readFileSync(join(fixture.managedRoot, second.relativeFilename))).toEqual(checkpointBytes);
  });

  it('refuses missing and unrelated resume databases without creating or changing them', async () => {
    const fixture = createFixture();
    await fixture.client.close();
    const missing = join(fixture.root, 'missing.sqlite');
    const unrelated = join(fixture.root, 'reference.sqlite');
    const db = new Database(unrelated);
    db.exec("CREATE TABLE snapshots (id INTEGER PRIMARY KEY, value TEXT); INSERT INTO snapshots VALUES (1, 'retained')");
    db.close();
    const before = readFileSync(unrelated);
    for (const databasePath of [missing, unrelated]) {
      const client = new CheckpointPersistenceClient({ databasePath,
        managedRootPath: fixture.managedRoot, existingOnly: true });
      clients.push(client);
      await expect(client.selectCurrent()).rejects.toThrow();
      await expect(client.close()).rejects.toThrow();
    }
    expect(existsSync(missing)).toBe(false);
    expect(readFileSync(unrelated)).toEqual(before);
    expect(existsSync(`${unrelated}-wal`)).toBe(false);
  });

  it('selects one current descriptor after reopening without changing publication identity', async () => {
    const fixture = createFixture();
    await expect(fixture.client.selectCurrent()).resolves.toBeNull();
    const descriptor = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(descriptor);
    await fixture.client.close();
    const reopened = new CheckpointPersistenceClient({ databasePath: fixture.databasePath, managedRootPath: fixture.managedRoot, existingOnly: true });
    clients.push(reopened);
    const selecting = reopened.selectCurrent();
    await expect(reopened.selectCurrent()).rejects.toThrow('busy');
    await expect(selecting).resolves.toEqual(descriptor);
    await expect(reopened.selectCurrent('missing-run')).resolves.toBeNull();
    const finalRead = reopened.selectCurrent(descriptor.runId);
    const closing = reopened.close();
    await expect(finalRead).resolves.toEqual(descriptor);
    await closing;
    expect(readCurrentPointer(fixture.databasePath, descriptor.runId)).toEqual({
      checkpoint_id: descriptor.logicalRootSha256, transition_epoch: descriptor.transitionEpoch, operation_id: descriptor.operationId
    });
  });

  it('persists population-only conversion provenance and requires an exact run-start replay', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    const legacyConversion = {
      snapshotId: 17,
      sourceFormat: 'legacy-gzip' as const,
      completeness: 'population-only' as const
    };
    await expect(fixture.client.commit(descriptor, null, false, legacyConversion)).resolves.toMatchObject({
      checkpointId: descriptor.logicalRootSha256
    });
    await expect(fixture.client.selectStartup()).resolves.toMatchObject({
      descriptor,
      runId: descriptor.runId,
      legacyConversion
    });
    await expect(fixture.client.commit(descriptor, null, false, legacyConversion)).resolves.toMatchObject({
      checkpointId: descriptor.logicalRootSha256
    });
    await expect(fixture.client.commit(descriptor, null, false, {
      ...legacyConversion,
      snapshotId: 18
    })).rejects.toThrow(/different legacy conversion provenance/);
  });

  it.each([false, true])('preserves converted-run provenance across repeated resets and expired pointers (old schema=%s)', async oldSchema => {
    const fixture = createFixture();
    const converted = createDescriptor(fixture.managedRoot);
    const legacyConversion = { snapshotId: 17, sourceFormat: 'typescript-v2' as const, completeness: 'population-only' as const };
    await fixture.client.commit(converted, null, true, legacyConversion);
    await fixture.client.close();
    const historical = new Database(fixture.databasePath);
    let originalProvenance: unknown;
    try {
      originalProvenance = historical.prepare('SELECT * FROM rust_legacy_conversions_v1').get();
      if (oldSchema) {
        historical.exec(`
          ALTER TABLE rust_legacy_conversions_v1 RENAME TO conversion_fixture;
          CREATE TABLE rust_legacy_conversions_v1 (
            run_id TEXT PRIMARY KEY NOT NULL REFERENCES rust_checkpoint_v3_current(run_id),
            source_snapshot_id INTEGER NOT NULL CHECK(source_snapshot_id > 0),
            source_format TEXT NOT NULL CHECK(source_format IN ('typescript-v2', 'legacy-gzip', 'legacy-json')),
            completeness TEXT NOT NULL CHECK(completeness = 'population-only'),
            created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
          );
          INSERT INTO rust_legacy_conversions_v1 SELECT * FROM conversion_fixture;
          DROP TABLE conversion_fixture;
        `);
      }
    } finally { historical.close(); }
    const client = new CheckpointPersistenceClient({ databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot, existingOnly: true });
    clients.push(client);
    expect(await client.selectStartup()).toMatchObject({ descriptor: converted, legacyConversion });
    let latest = converted;
    for (let index = 1; index <= 4; index++) {
      latest = createDescriptor(fixture.managedRoot, { runId: `reset-run-${index}`,
        operationId: index.toString(16).padStart(32, '0') });
      await client.commit(latest, null, true);
      await client.applyRetention();
    }
    expect(await client.selectStartup()).toMatchObject({ descriptor: latest, legacyConversion: null });
    expect(await client.selectCurrent(converted.runId)).toBeNull();
    expect(existsSync(join(fixture.managedRoot, converted.relativeFilename))).toBe(false);
    await client.close();
    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT * FROM rust_legacy_conversions_v1').get()).toEqual(originalProvenance);
      expect(inspect.prepare('PRAGMA foreign_key_check').all()).toEqual([]);
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_metadata').get()).toEqual({ count: 5 });
    } finally { inspect.close(); }
  });

  it('migrates Hall-of-Fame history beyond one startup page without loading it all at once', async () => {
    const fixture = createFixture();
    const current = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(current);
    await fixture.client.close();
    const historical = new Database(fixture.databasePath);
    try {
      const insert = historical.prepare(`INSERT INTO rust_hall_of_fame_v1
        (run_id, generation_hex, checkpoint_id, record_version, record_blob,
          weights_sha256, genome_sha256, fitness_value, pinned, weight_state, created_at_ms)
        VALUES (?, ?, NULL, 1, ?, NULL, NULL, 0, 0, 'legacy', 0)`);
      historical.transaction(() => {
        for (let index = 1; index <= 300; index++) {
          const record = Buffer.alloc(56);
          record.writeBigUInt64LE(BigInt(index), 0);
          record.writeDoubleLE(index + 0.5, 32);
          insert.run(index <= 150 ? 'legacy-page-a' : 'legacy-page-b', u64(BigInt(index)), record);
        }
      })();
    } finally { historical.close(); }
    const restarted = new CheckpointPersistenceClient({
      databasePath: fixture.databasePath,
      managedRootPath: fixture.managedRoot,
      existingOnly: true
    });
    clients.push(restarted);
    await expect(restarted.selectStartup()).resolves.toMatchObject({ descriptor: current });
    await restarted.close();
    const inspect = new Database(fixture.databasePath, { readonly: true });
    try {
      expect(inspect.prepare('SELECT count(*) AS count FROM rust_hall_of_fame_v1').get()).toEqual({ count: 300 });
      expect(inspect.prepare(`SELECT fitness_value FROM rust_hall_of_fame_v1
        WHERE run_id = ? AND generation_hex = ?`).get('legacy-page-b', u64(300n)))
        .toEqual({ fitness_value: 300.5 });
    } finally { inspect.close(); }
  });

  it('rejects ambiguous runs until one durable checkpoint explicitly activates its lineage', async () => {
    const fixture = createFixture();
    const first = createDescriptor(fixture.managedRoot);
    const second = createDescriptor(fixture.managedRoot, { runId: 'another-run', operationId: 'c'.repeat(32) });
    await fixture.client.commit(first);
    await fixture.client.commit(second);
    await expect(fixture.client.selectCurrent()).rejects.toThrow('multiple current runs');
    await expect(fixture.client.selectCurrent(first.runId)).resolves.toEqual(first);
    await expect(fixture.client.selectCurrent(second.runId)).resolves.toEqual(second);
    await fixture.client.commit(second, null, true);
    await expect(fixture.client.selectCurrent()).resolves.toEqual(second);
  });

  it('rejects oversized stored metadata without materializing it or rewriting the source', async () => {
    const fixture = createFixture();
    const descriptor = createDescriptor(fixture.managedRoot);
    await fixture.client.commit(descriptor);
    const database = new Database(fixture.databasePath);
    try {
      database.prepare('UPDATE rust_checkpoint_v3_metadata SET descriptor_json = ? WHERE checkpoint_id = ?')
        .run(' '.repeat(32 * 1024), descriptor.logicalRootSha256);
      await expect(fixture.client.selectCurrent()).rejects.toThrow('missing immutable metadata');
      expect(database.prepare('SELECT length(descriptor_json) AS length FROM rust_checkpoint_v3_metadata').get()).toEqual({ length: 32 * 1024 });
    } finally { database.close(); }
  });
});
