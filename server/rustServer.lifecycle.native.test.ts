import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { mkdtemp,readdir,readFile,rm,writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expect,it,vi } from 'vitest';
import WebSocket from 'ws';
import { buildStackGraphSpec } from '../src/brains/stackBuilder.ts';
import { type WelcomeMsg } from '../src/net/wsClient.ts';
import { normalizeConfig } from './config.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { startRustServer } from './rustServer.ts';
import { connect,healthUntil,manualFixtureHttp,replacementUntil,until,type Peer } from './rustServerFixture.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG } from './test/rustConfig.ts';

describeNetworkSuite('Rust server real sockets', () => {

  it('finishes only the requested generation through checkpoint durability and rejects stale requests', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-manual-finish-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    let target: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      await until(viewer, () => viewer!.frames > 0);
      const welcome = viewer.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
      expect(welcome.capabilities?.finishGeneration).toBe(true);
      const healthResponse = await manualFixtureHttp('http://127.0.0.1:' + server.port + '/api/health');
      expect(healthResponse.status).toBe(200);
      const health = JSON.parse(healthResponse.bytes.toString()) as { generation: string };
      const generation = Number(BigInt('0x' + health.generation));
      const finish = { type: 'finishGeneration', requestId: 'finish-1', expectedRunId: welcome.runId, expectedGeneration: generation };
      viewer.socket.send(JSON.stringify({ ...finish, requestId: 'stale-run', expectedRunId: 'wrong-run' }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'stale-run' && packet['status'] === 'rejected'));
      viewer.socket.send(JSON.stringify(finish));
      viewer.socket.send(JSON.stringify(finish));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'finish-1' && packet['status'] === 'completed'));
      expect(viewer.packets.filter(packet => packet['requestId'] === 'finish-1' && packet['status'] === 'completed')).toHaveLength(1);
      expect(viewer.packets.some(packet => packet['requestId'] === 'finish-1' && packet['status'] === 'pending')).toBe(true);
      viewer.socket.send(JSON.stringify({ ...finish, requestId: 'stale-generation' }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'stale-generation' && packet['status'] === 'rejected'));
      const db = new Database(dbPath, { readonly: true });
      try {
        const row = db.prepare('SELECT record_blob FROM rust_generation_completion_v1 WHERE run_id = ? AND generation_hex = ?')
          .get(welcome.runId, generation.toString(16).padStart(16, '0')) as { record_blob: Buffer };
        expect(row.record_blob.length).toBe(32);
        expect(row.record_blob.readUInt32LE(0)).toBe(1);
        expect(row.record_blob.readUInt32LE(4)).toBe(0);
        expect(row.record_blob.readDoubleLE(8)).toBeGreaterThan(0);
        expect(row.record_blob.readDoubleLE(16)).toBe(240);
        expect(row.record_blob.readBigUInt64LE(24)).toBeGreaterThan(0n);
        expect(db.prepare('SELECT length(record_blob) AS bytes FROM rust_generation_history_v1 WHERE run_id = ? AND generation_hex = ?')
          .get(welcome.runId, generation.toString(16).padStart(16, '0'))).toEqual({ bytes: 56 });
      } finally { db.close(); }
      const historyResponse = await manualFixtureHttp('http://127.0.0.1:' + server.port + '/api/history');
      expect(historyResponse.status).toBe(200);
      const history = JSON.parse(historyResponse.bytes.toString()) as { version: number; runId: string; history: Array<{ gen: number; experiment: { version: number; reason: string; population: number; winner: { version: number; pointMethod: string; displayLengthKind: string } } }> };
      expect(history).toMatchObject({ version: 1, runId: welcome.runId });
      expect(history.history).toHaveLength(1);
      expect(history.history[0]).toMatchObject({ gen: generation, experiment: { version: 1, reason: 'manual', winner: { version: 2, pointMethod: 'reset', displayLengthKind: 'latest-life' } } });
      const hofResponse = await manualFixtureHttp('http://127.0.0.1:' + server.port + '/api/hof');
      expect(JSON.parse(hofResponse.bytes.toString())).toMatchObject({ hof: [{ assessment: { version: 2, pointMethod: 'reset', displayLengthKind: 'latest-life' } }] });
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'stats' && Array.isArray(packet['fitnessHistory']) && packet['fitnessHistory'].length > 0));
      const compactStats = viewer.packets.find(packet => packet['type'] === 'stats' && Array.isArray(packet['fitnessHistory']) && packet['fitnessHistory'].length > 0)!;
      expect((compactStats['fitnessHistory'] as Array<Record<string, unknown>>)[0]).not.toHaveProperty('experiment');
      const response = await manualFixtureHttp('http://127.0.0.1:' + server.port + '/api/export/latest');
      expect(response.status, response.status >= 400 ? response.bytes.toString() : undefined).toBe(200);
      const archive = new Uint8Array(response.bytes);
      const targetDbPath = join(root, 'imported.sqlite');
      target = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: targetDbPath, resume: 'fresh', seed: 99,
        rustCalculationWorkers: 1 });
      const imported = await manualFixtureHttp('http://127.0.0.1:' + target.port + '/api/import/archive', archive);
      expect(imported.status, imported.status >= 400 ? imported.bytes.toString() : undefined).toBe(200);
      await target.close(); target = undefined;
      const copied = new Database(targetDbPath, { readonly: true });
      const source = new Database(dbPath, { readonly: true });
      try {
        expect(copied.prepare('SELECT generation_hex, record_blob FROM rust_generation_completion_v1 WHERE run_id = ? ORDER BY generation_hex').all(welcome.runId))
          .toEqual(source.prepare('SELECT generation_hex, record_blob FROM rust_generation_completion_v1 WHERE run_id = ? ORDER BY generation_hex').all(welcome.runId));
        expect(copied.prepare('SELECT generation_hex, record_blob FROM rust_generation_experiment_v1 WHERE run_id = ? ORDER BY generation_hex').all(welcome.runId))
          .toEqual(source.prepare('SELECT generation_hex, record_blob FROM rust_generation_experiment_v1 WHERE run_id = ? ORDER BY generation_hex').all(welcome.runId));
      } finally { copied.close(); source.close(); }
    } finally {
      viewer?.socket.terminate();
      await target?.close();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('reports manual completion failure and preserves the durable source when persistence rejects', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-manual-failure-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    let rejectCommit: ReturnType<typeof vi.spyOn> | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 42,
        rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      viewer = await connect(server.port, 'ui');
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      await until(viewer, () => viewer!.frames > 0);
      const welcome = viewer.packets.find(packet => packet['type'] === 'welcome') as unknown as WelcomeMsg;
      const sourceDb = new Database(dbPath, { readonly: true });
      let pointer: unknown;
      try { pointer = sourceDb.prepare('SELECT * FROM rust_checkpoint_v3_current WHERE run_id = ?').get(welcome.runId); }
      finally { sourceDb.close(); }
      rejectCommit = vi.spyOn(CheckpointPersistenceClient.prototype, 'commit')
        .mockRejectedValueOnce(new Error('manual completion fixture persistence failure'));
      viewer.socket.send(JSON.stringify({ type: 'finishGeneration', requestId: 'finish-failure',
        expectedRunId: welcome.runId, expectedGeneration: 1 }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'finish-failure' && packet['status'] === 'rejected'));
      expect(viewer.packets.some(packet => packet['requestId'] === 'finish-failure' && packet['status'] === 'completed')).toBe(false);
      const db = new Database(dbPath, { readonly: true });
      try {
        expect(db.prepare('SELECT * FROM rust_checkpoint_v3_current WHERE run_id = ?').get(welcome.runId)).toEqual(pointer);
        expect(db.prepare('SELECT count(*) AS count FROM rust_generation_completion_v1 WHERE run_id = ?').get(welcome.runId)).toEqual({ count: 0 });
      } finally { db.close(); }
    } finally {
      rejectCommit?.mockRestore();
      viewer?.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  });

  it.each(['latest', `sha256:${'ab'.repeat(32)}`] as const)('faults explicit resume %s against a missing database without creating state', async resume => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-missing-resume-'));
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume,
      rustCalculationWorkers: 1, dbPath: join(root, 'missing.sqlite') });
    try {
      expect(server.startupFault).toContain('database does not exist');
      const response = await fetch(`http://127.0.0.1:${server.port}/api/health`);
      expect(response.status).toBe(503);
      expect(await response.json()).toMatchObject({ ok: false, lifecycle: 'startup-fault' });
      const socket = new WebSocket(`ws://127.0.0.1:${server.port}`);
      await new Promise<void>((done, reject) => {
        socket.once('open', () => reject(new Error('missing resume accepted a game connection')));
        socket.once('error', error => {
          try { expect(error.message).toContain('503'); done(); } catch (failure) { reject(failure); }
        });
      });
      expect(await readdir(root)).toEqual([]);
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  });

  it.each([false, true])('creates and resumes the automatic first run (pre-telemetry schema: %s)', async preTelemetry => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-first-run-'));
    const dbPath = join(root, 'slither.sqlite');
    let server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, dbPath, seed: 91, rustCalculationWorkers: 1
    });
    try {
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json();
      expect(health).toMatchObject({ ok: true, authority: 'rust', generation: '0000000000000001' });
      const firstBoundary = BigInt(`0x${(health as { commandServiceBoundaries: string }).commandServiceBoundaries}`);
      await healthUntil(server.port, sample =>
        BigInt(`0x${sample['commandServiceBoundaries'] as string}`) > firstBoundary);
      await server.close();
      const database = new Database(dbPath);
      let checkpoint: string;
      try {
        checkpoint = (database.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current').get() as { checkpoint_id: string }).checkpoint_id;
        if (preTelemetry) database.exec('DROP TABLE rust_generation_work_v1');
      } finally { database.close(); }
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: (health as { runId: string }).runId, seed: 91, startupCheckpointId: checkpoint
      });
      expect((await manualFixtureHttp(`http://127.0.0.1:${server.port}/`)).status).toBe(200);
      const history = await manualFixtureHttp(`http://127.0.0.1:${server.port}/api/history`);
      expect(history.status).toBe(200);
      expect(JSON.parse(history.bytes.toString())).toEqual({ version: 1,
        runId: (health as { runId: string }).runId, history: [] });
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('appends a seeded fresh run to a managed store and preserves prior checkpoints, history and Hall of Fame', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-fresh-append-'));
    const dbPath = join(root, 'slither.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 41, rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      expect((await fetch(`http://127.0.0.1:${server.port}/api/checkpoints/current/pin`, { method: 'POST' })).status).toBe(200);
      viewer = await connect(server.port, 'ui');
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 3, simSpeed: 0.1 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'pelletCountTarget', value: 100 },
          { path: 'baselineBots.count', value: 0 }],
        graphSpec: buildStackGraphSpec({ hiddenLayers: 1, neurons1: 2, neurons2: 2, neurons3: 2, neurons4: 2, neurons5: 2 },
          { brain: { inSize: 264, outSize: 2, useMlp: false } }) }));
      const previous = await replacementUntil(viewer, server.port, 'reset');
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      expect((await fetch(`http://127.0.0.1:${server.port}/api/checkpoints/current/pin`, { method: 'POST' })).status).toBe(200);
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'fresh-append-accelerate',
        updates: [{ path: 'simSpeed', value: 12 }] }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'fresh-append-accelerate' && packet['applied'] === true));
      await healthUntil(server.port, sample => BigInt(`0x${sample['generation'] as string}`) >= 2n);
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'fresh-append-slow',
        updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'fresh-append-slow' && packet['applied'] === true));
      viewer.socket.terminate(); viewer = undefined;
      await server.close(); server = undefined;
      /** Read exact retained rows for the old lineage without materializing its population. */
      const oldRecords = (): unknown[] => {
        const database = new Database(dbPath, { readonly: true });
        try { return ['rust_checkpoint_v3_metadata', 'rust_checkpoint_v3_current', 'rust_generation_history_v1', 'rust_hall_of_fame_v1']
          .map(table => database.prepare(`SELECT * FROM ${table} WHERE run_id = ? ORDER BY rowid`).all(previous.runId)); }
        finally { database.close(); }
      };
      const retained = oldRecords();
      expect((retained[2] as unknown[]).length).toBeGreaterThan(0);
      expect((retained[3] as unknown[]).length).toBeGreaterThan(0);
      const oldFiles = new Map<string, string>();
      const inventory = new Database(dbPath, { readonly: true });
      let filenames: string[];
      try {
        filenames = (inventory.prepare(`SELECT relative_filename AS filename FROM rust_checkpoint_v3_metadata
          UNION SELECT relative_filename AS filename FROM rust_hall_of_fame_weights_v1`)
          .all() as Array<{ filename: string }>).map(row => row.filename);
      } finally { inventory.close(); }
      for (const file of filenames) {
        oldFiles.set(file, createHash('sha256').update(await readFile(join(`${dbPath}.checkpoints`, file))).digest('hex'));
      }
      expect(oldFiles.size).toBeGreaterThan(1);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 99, rustCalculationWorkers: 1 });
      expect(server.startupFault).toBeUndefined();
      const fresh = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string; retention: { activeRunId: string };
      };
      expect(fresh).toMatchObject({ ok: true, seed: 99, generation: '0000000000000001' });
      expect(fresh.runId).not.toBe(previous.runId);
      expect(fresh.retention.activeRunId).toBe(fresh.runId);
      expect(oldRecords()).toEqual(retained);
      for (const [file, hash] of oldFiles) {
        expect(createHash('sha256').update(await readFile(join(`${dbPath}.checkpoints`, file))).digest('hex')).toBe(hash);
      }
      const database = new Database(dbPath, { readonly: true });
      try { expect(database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1').get()).toEqual({ run_id: fresh.runId }); }
      finally { database.close(); }
      await server.close(); server = undefined;
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'latest', rustCalculationWorkers: 1 });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, seed: 99, runId: fresh.runId, startupCheckpointId: fresh.startupCheckpointId
      });
    } finally {
      viewer?.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it.each(['legacy', 'unrelated'])('rejects fresh startup of an existing %s database without modifying it', async kind => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-fresh-incompatible-'));
    const dbPath = join(root, 'owner.sqlite');
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    try {
      const database = new Database(dbPath);
      try {
        if (kind === 'legacy') {
          database.exec('CREATE TABLE population_snapshots (id INTEGER PRIMARY KEY, payload_json TEXT NOT NULL)');
          database.prepare('INSERT INTO population_snapshots VALUES (1, ?)').run('retained legacy population');
        } else {
          database.exec('CREATE TABLE owner_records (value TEXT NOT NULL)');
          database.prepare('INSERT INTO owner_records VALUES (?)').run('retained owner record');
        }
      } finally { database.close(); }
      const before = await readFile(dbPath);
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 99 });
      expect(server.startupFault).toBeDefined();
      expect((await fetch(`http://127.0.0.1:${server.port}/api/health`)).status).toBe(503);
      expect(await readFile(dbPath)).toEqual(before);
      expect(await readdir(root)).toEqual(['owner.sqlite']);
    } finally {
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  });
  it('passes the selected checkpoint budget through production health and retention', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-budget-'));
    const dbPath = join(root, 'slither.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, dbPath, resume: 'fresh', seed: 42,
      checkpointBudgetMiB: 2048
    });
    try {
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true,
        retention: { automaticByteCap: '0000000080000000' }
      });
    } finally {
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 15_000);

  it('reclaims a rejected New Run candidate before resuming the unchanged old authority', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-rejected-new-run-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 42, dbPath });
    let viewer: Peer | undefined;
    const cleanup = vi.spyOn(CheckpointPersistenceClient.prototype, 'reclaimManagedOrphans');
    /** Preserve every managed byte and metadata row across a real SQLite rejection. */
    const snapshot = async (): Promise<unknown> => {
      const database = new Database(dbPath, { readonly: true });
      try {
        const tables = database.prepare(`SELECT name FROM sqlite_master
          WHERE type = 'table' AND name LIKE 'rust_%' ORDER BY name`).all() as Array<{ name: string }>;
        const names = (await readdir(`${dbPath}.checkpoints`)).sort();
        return { tables: tables.map(({ name }) => ({ name,
          rows: database.prepare(`SELECT * FROM "${name.replaceAll('"', '""')}" ORDER BY rowid`).all() })),
        files: await Promise.all(names.map(async name => ({ name,
          hash: createHash('sha256').update(await readFile(join(`${dbPath}.checkpoints`, name))).digest('hex') }))) };
      } finally { database.close(); }
    };
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      const failureDatabase = new Database(dbPath);
      try {
        failureDatabase.exec(`CREATE TRIGGER reject_new_run_activation
          BEFORE INSERT ON rust_active_run_v1 BEGIN
            SELECT RAISE(ABORT, 'injected actual New Run transaction rejection');
          END`);
      } finally { failureDatabase.close(); }
      const before = await snapshot();
      const beforeHealth = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; worldEpoch: string; seed: number;
      };
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'rejected-new-run' }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'rejected-new-run'));
      expect(viewer.packets.find(packet => packet['requestId'] === 'rejected-new-run')).toMatchObject({
        type: 'newRunResult', applied: false, reason: expect.stringContaining('injected actual New Run transaction rejection')
      });
      expect(cleanup).toHaveBeenCalledOnce();
      expect(await cleanup.mock.results[0]!.value).toMatchObject({ completed: true,
        deletedCheckpointCount: '0000000000000001', deletedHallOfFameCount: '0000000000000000' });
      expect(await snapshot()).toEqual(before);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: beforeHealth.runId, worldEpoch: beforeHealth.worldEpoch, seed: beforeHealth.seed
      });
      const recoveryDatabase = new Database(dbPath);
      try { recoveryDatabase.exec('DROP TRIGGER reject_new_run_activation'); }
      finally { recoveryDatabase.close(); }
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'new-run-valid-retry' }));
      await until(viewer, () => viewer!.packets.some(packet => packet['requestId'] === 'new-run-valid-retry'));
      expect(viewer.packets.find(packet => packet['requestId'] === 'new-run-valid-retry')).toMatchObject({
        type: 'newRunResult', applied: true
      });
      expect(cleanup).toHaveBeenCalledOnce();
    } finally {
      cleanup.mockRestore();
      viewer?.socket.close();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 20_000);

  it('keeps a committed New Run when its SQLite reply is lost', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-lost-new-run-reply-'));
    const dbPath = join(root, 'experiment.sqlite');
    let server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
      resume: 'fresh', seed: 42, dbPath });
    let viewer: Peer | undefined;
    let commitSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string;
      };
      const originalCommit = CheckpointPersistenceClient.prototype.commit;
      /** Lose one reply after the real worker has activated the new run pointer. */
      commitSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'commit')
        .mockImplementationOnce(async function (this: CheckpointPersistenceClient,
          ...args: Parameters<CheckpointPersistenceClient['commit']>) {
          await originalCommit.apply(this, args);
          throw new Error('injected lost New Run commit reply');
        });
      viewer.socket.send(JSON.stringify({ type: 'newRun', requestId: 'lost-new-run-reply' }));
      await until(viewer, () => viewer!.packets.some(packet =>
        packet['type'] === 'error' || packet['type'] === 'newRunResult'));
      const faulted = await healthUntil(server.port, health => health['ok'] === false);
      expect(faulted).toMatchObject({ runId: before.runId,
        startupCheckpointId: before.startupCheckpointId,
        interfaceFault: 'fresh replacement checkpoint outcome is unknown; restart from a valid retained checkpoint' });
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'error'));
      expect(viewer.packets.filter(packet => packet['requestId'] === 'lost-new-run-reply')).toEqual([]);
      const database = new Database(dbPath, { readonly: true });
      let committedRunId: string;
      try {
        const current = database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1')
          .get() as { run_id: string };
        committedRunId = current.run_id;
        expect(committedRunId).not.toBe(before.runId);
      } finally { database.close(); }
      viewer.socket.terminate();
      viewer = undefined;
      await server.close();
      commitSpy.mockRestore();
      commitSpy = undefined;
      const { seed: _seed, ...resumeConfig } = DEFAULT_CONFIG;
      server = await startRustServer({ ...resumeConfig, port: 0, resume: 'latest', dbPath });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: committedRunId, generation: '0000000000000001'
      });
    } finally {
      commitSpy?.mockRestore();
      viewer?.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('resumes exact managed IDs and exposes recovery or health-only failure over real HTTP/WebSocket', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-recovery-server-'));
    const dbPath = join(root, 'experiment.sqlite');
    const config = { ...DEFAULT_CONFIG, port: 0, dbPath };
    let server = await startRustServer({ ...config, resume: 'fresh', seed: 42 });
    const peers: Peer[] = [];
    try {
      const initial = await (await fetch(`http://127.0.0.1:${server.port}/health`)).json() as { runId: string; startupCheckpointId: string };
      await server.close();
      const exact = normalizeConfig({ ...config, resume: initial.startupCheckpointId });
      expect(exact.resume).toBe(`sha256:${initial.startupCheckpointId}`);
      server = await startRustServer({ ...exact, port: 0 });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/health`)).json()).toMatchObject({ ok: true, runId: initial.runId });
      await server.close();
      const db = new Database(dbPath);
      try {
        db.pragma('foreign_keys = OFF');
        db.prepare('UPDATE rust_checkpoint_v3_current SET checkpoint_id = ?').run('f'.repeat(64));
      } finally { db.close(); }
      server = await startRustServer({ ...config, resume: 'latest' });
      const health = await (await fetch(`http://127.0.0.1:${server.port}/health`)).json() as { runId: string; recovery: unknown };
      expect(health.runId).not.toBe(initial.runId);
      expect(health.recovery).toMatchObject({ failedRunId: initial.runId, branchRunId: health.runId,
        recoveredCheckpointId: initial.startupCheckpointId, lostCompletedGenerations: null });
      const viewer = await connect(server.port, 'ui'); peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ recovery: health.recovery });
      viewer.socket.terminate();
      await server.close();
      const files = await readdir(`${dbPath}.checkpoints`);
      for (const file of files.filter(name => name.endsWith('.checkpoint-v3'))) await writeFile(join(`${dbPath}.checkpoints`, file), 'corrupt');
      server = await startRustServer({ ...config, resume: 'latest' });
      expect(server.startupFault).toMatch(/no valid retained/);
      const fault = await fetch(`http://127.0.0.1:${server.port}/api/health`);
      expect(fault.status).toBe(503);
      expect(await fault.json()).toMatchObject({ ok: false, lifecycle: 'startup-fault' });
      const rejected = new WebSocket(`ws://127.0.0.1:${server.port}`);
      await new Promise<void>((done, reject) => {
        rejected.once('unexpected-response', (_request, response) => {
          expect(response.statusCode).toBe(503); response.resume(); rejected.terminate(); done();
        });
        rejected.once('open', () => reject(new Error('faulted startup accepted a game socket')));
        rejected.on('error', () => {});
      });
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);
});
