import Database from 'better-sqlite3';
import { createHash } from 'node:crypto';
import { mkdtemp,readdir,rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { expect,it,vi } from 'vitest';
import { compileGraph } from '../src/brains/graph/compiler.ts';
import { buildStackGraphSpec } from '../src/brains/stackBuilder.ts';
import { CFG_DEFAULT } from '../src/config.ts';
import { DEFAULT_CORE_SETTINGS } from '../src/protocol/settings.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { startRustServer } from './rustServer.ts';
import { ACTIVE_WORKERS,archiveFileSha256,connect,healthUntil,legacyCompanions,legacyExportManifest,legacyRecords,migratedArchiveRoundTrip,PARALLEL_WORKERS,readOnlyLegacyConversion,replacementUntil,rewriteLegacyManifest,until,type Peer } from './rustServerFixture.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { RUST_TEST_CONFIG as DEFAULT_CONFIG } from './test/rustConfig.ts';

describeNetworkSuite('Rust server real sockets', () => {

  it('imports an old browser JSON population as a new Rust run', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-legacy-import-'));
    const dbPath = join(root, 'experiment.sqlite');
    const server = await startRustServer({
      ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 41, dbPath, rustCalculationWorkers: PARALLEL_WORKERS
    });
    const peers: Peer[] = [];
    try {
      const before = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; nativeBuildIdentifier: string; calculationWorkers: number;
      };
      expect(before.nativeBuildIdentifier).toMatch(/^slither_native\/[0-9A-Za-z.+-]+$/u);
      expect(before.calculationWorkers).toBe(PARALLEL_WORKERS);
      const viewer = await connect(server.port, 'ui');
      peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        inferenceMode: { nativeAddonBuildIdentifier: before.nativeBuildIdentifier,
          requestedMt: PARALLEL_WORKERS > 1, activeWorkerCount: ACTIVE_WORKERS }
      });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      const weights = new Array<number>(13_458).fill(0);
      const legacyFile = JSON.stringify({
        runId: 'browser-source-run',
        generation: 37,
        archKey: 'legacy-default-graph',
        worldSeed: 1_234_567,
        settings: { snakeCount: 2, simSpeed: 3, baselineBots: { count: 1 } },
        genomes: [
          { archKey: 'legacy-default-graph', brainType: 'mlp', fitness: 99, weights },
          { archKey: 'legacy-default-graph', brainType: 'mlp', fitness: 50, weights }
        ]
      });
      const response = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, body: legacyFile
      });
      const imported = await response.json() as {
        ok: boolean; runId: string; generation: string; completedStep: string; checkpointId: string;
      };
      expect({ status: response.status, imported }).toMatchObject({
        status: 200,
        imported: {
          ok: true,
          generation: '0000000000000001',
          completedStep: '0000000000000000'
        }
      });
      expect(imported.runId).not.toBe(before.runId);
      expect(imported.checkpointId).toMatch(/^[0-9a-f]{64}$/u);
      const currentHealth = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { seed: number };
      await until(viewer, () => viewer.packets.some(packet =>
        packet['type'] === 'stateReplaced' && packet['reason'] === 'import'));
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')).toMatchObject({
        checkpointId: imported.checkpointId,
        welcome: {
          runId: imported.runId,
          worldSeed: currentHealth.seed,
          legacyConversion: { version: 1, sourceFormat: 'browser-json', sourceRunId: 'browser-source-run',
            sourceGeneration: '0000000000000025', sourceSeed: 1_234_567,
            sourceSha256: createHash('sha256').update(legacyFile).digest('hex'), completeness: 'population-only', exactContinuation: false },
          inferenceMode: { activeWorkerCount: ACTIVE_WORKERS },
          settings: {
            core: { snakeCount: 2, simSpeed: 3 },
            updates: expect.arrayContaining([{ path: 'baselineBots.count', value: 1 }])
          }
        }
      });
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true,
        runId: imported.runId,
        seed: currentHealth.seed,
        generation: '0000000000000001',
        startupCheckpointId: imported.checkpointId
      });
      const rejected = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          generation: 1,
          archKey: 'legacy-default-graph',
          genomes: [{ archKey: 'legacy-default-graph', weights: [0] }]
        })
      });
      expect(rejected.status).toBe(400);
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: imported.runId, startupCheckpointId: imported.checkpointId
      });
      expect((await readdir(`${dbPath}.checkpoints`)).filter(name =>
        name.includes('upload') || name.includes('import-inventory'))).toEqual([]);
      await migratedArchiveRoundTrip(root, server, createHash('sha256')
        .update(Buffer.alloc(weights.length * 2 * Float32Array.BYTES_PER_ELEMENT)).digest('hex'));
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it.each(['reset', 'newRun'] as const)('retains legacy origin through evolution and restart, then clears it on %s', async replacement => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-origin-lineage-'));
    let server: Awaited<ReturnType<typeof startRustServer>> | undefined;
    let viewer: Peer | undefined;
    let releaseSpy: ReturnType<typeof vi.spyOn> | undefined;
    try {
      server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, resume: 'fresh', seed: 42, dbPath: join(root, 'source.sqlite') });
      const legacyFile = JSON.stringify({ generation: 37, worldSeed: 1234567, archKey: 'legacy-default-graph',
        settings: { snakeCount: 2, simSpeed: 12, baselineBots: { count: 0 } },
        updates: [{ path: 'generationSeconds', value: 8 }],
        genomes: [0, 1].map(() => ({ archKey: 'legacy-default-graph', weights: new Array<number>(13_458).fill(0) })) });
      const imported = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, { method: 'POST', body: legacyFile });
      expect(imported.status, await imported.clone().text()).toBe(200);
      const result = await imported.json() as { legacyConversion: unknown; runId: string };
      await healthUntil(server.port, health => BigInt(`0x${health['generation'] as string}`) >= 3n);
      viewer = await connect(server.port, 'ui');
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({ legacyConversion: result.legacyConversion });
      viewer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator',
        rejoinToken: viewer.rejoinToken }));
      viewer.socket.send(JSON.stringify({ type: 'settings', requestId: 'hold-origin', updates: [{ path: 'simSpeed', value: 0.1 }] }));
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'settingsApplied' && packet['requestId'] === 'hold-origin'));
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({ legacyConversion: result.legacyConversion });
      let released = false;
      const originalRelease = CheckpointPersistenceClient.prototype.releaseExportLease;
      releaseSpy = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease')
        .mockImplementationOnce(async function(this: CheckpointPersistenceClient, operationId) {
          await originalRelease.call(this, operationId);
          released = true;
        });
      const exported = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(exported.status).toBe(200);
      const archive = Buffer.from(await exported.arrayBuffer());
      await until(viewer, () => released);
      releaseSpy.mockRestore();
      releaseSpy = undefined;
      const retainedRows = legacyRecords(join(root, 'source.sqlite'));
      const managedDirectory = join(root, 'source.sqlite.checkpoints');
      /** Observe every durable file without materializing population data. */
      const retainedFiles = async (): Promise<Array<{ filename: string; sha256: string }>> =>
        Promise.all((await readdir(managedDirectory)).sort().map(async filename => ({
          filename, sha256: await archiveFileSha256(join(managedDirectory, filename))
        })));
      const durableFiles = await retainedFiles();
      for (const mutate of [
        (manifest: Record<string, unknown>) => { manifest['archiveKind'] = 'exact-generation-boundary-v1'; },
        (manifest: Record<string, unknown>) => { delete manifest['legacyConversion']; manifest['archiveKind'] = 'exact-generation-boundary-v1'; },
        (manifest: Record<string, unknown>) => { (manifest['legacyConversion'] as Record<string, unknown>)['sourceSeed'] = 1234568; }
      ]) {
        const rejected = await fetch(`http://127.0.0.1:${server.port}/api/import/archive`, {
          method: 'POST', body: new Uint8Array(rewriteLegacyManifest(archive, mutate)) });
        expect(rejected.status, await rejected.clone().text()).toBe(400);
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, runId: result.runId, legacyConversion: result.legacyConversion });
        expect(legacyRecords(join(root, 'source.sqlite'))).toEqual(retainedRows);
        expect(await retainedFiles()).toEqual(durableFiles);
      }
      await migratedArchiveRoundTrip(root, server);
      viewer.socket.send(JSON.stringify({ type: replacement, ...(replacement === 'newRun' ? { requestId: 'clear-origin' } : {}) }));
      await until(viewer, () => viewer!.packets.some(packet => packet['type'] === 'stateReplaced' && packet['reason'] === replacement));
      expect(viewer.packets.findLast(packet => packet['type'] === 'stateReplaced')?.['welcome']).not.toHaveProperty('legacyConversion');
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).not.toHaveProperty('legacyConversion');
      const freshExport = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`);
      expect(legacyExportManifest(Buffer.from(await freshExport.arrayBuffer()))).toMatchObject({ archiveKind: 'exact-generation-boundary-v1' });
    } finally {
      releaseSpy?.mockRestore();
      viewer?.socket.terminate();
      await server?.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it('converts the newest TypeScript v2 checkpoint and repeatedly resets without changing its source rows', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-v2-startup-'));
    const dbPath = join(root, 'experiment.sqlite');
    const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: 2, simSpeed: 2, neurons1: 96, neurons2: 96 };
    const graphSpec = buildStackGraphSpec(core, {...CFG_DEFAULT, brain: {...CFG_DEFAULT.brain, inSize: 83}});
    const graph = compileGraph(graphSpec);
    expect(graph.totalParams).toBeGreaterThan(64 * 1024 / Float32Array.BYTES_PER_ELEMENT);
    const firstWeights = Buffer.alloc(graph.totalParams * Float32Array.BYTES_PER_ELEMENT);
    const secondWeights = Buffer.from(firstWeights);
    secondWeights.writeFloatLE(0.25, 0);
    const metadata = JSON.stringify({
      formatVersion: 2,
      boundaryVersion: 1,
      boundaryKind: 'generation',
      resumable: true,
      generation: 19,
      simulationStep: 72_000,
      runId: 'typescript-source-run',
      worldSeed: 7_654_321,
      configHash: 'legacy-config',
      configRevision: 4,
      archKey: graph.key,
      graphSpec,
      populationCount: 2,
      settings: core,
      updates: [
        { path: 'baselineBots.count', value: 0 },
        { path: 'foodSpawn.edgeFalloffEnabled', value: 1 }
      ],
      rng: {},
      allocators: {},
      bestFitnessEver: 0,
      fitnessHistory: [],
      lastHofEntry: null
    });
    const sourceDigest = createHash('sha256')
      .update(metadata)
      .update(firstWeights)
      .update(secondWeights)
      .digest('hex');
    const database = new Database(dbPath);
    try {
      database.exec(`
        CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, payload_json TEXT, format_version INTEGER,
          boundary_kind TEXT, population_count INTEGER
        );
        CREATE TABLE snapshot_genomes (
          snapshot_id INTEGER NOT NULL, slot INTEGER NOT NULL, arch_key TEXT NOT NULL,
          brain_type TEXT NOT NULL, fitness REAL NOT NULL, weight_count INTEGER NOT NULL,
          weights_blob BLOB NOT NULL, weights_checksum TEXT NOT NULL,
          PRIMARY KEY (snapshot_id, slot)
        );
      `);
      database.prepare(`INSERT INTO population_snapshots
        (id, payload_json, format_version, boundary_kind, population_count)
        VALUES (1, ?, 2, 'generation', 2)`).run(metadata);
      const insert = database.prepare(`INSERT INTO snapshot_genomes
        (snapshot_id, slot, arch_key, brain_type, fitness, weight_count, weights_blob, weights_checksum)
        VALUES (1, ?, ?, 'mlp', ?, ?, ?, ?)`);
      for (const [slot, weights] of [firstWeights, secondWeights].entries()) {
        insert.run(slot, graph.key, 10 - slot, graph.totalParams, weights,
          createHash('sha256').update(weights).digest('hex'));
      }
      legacyCompanions(database, JSON.stringify(graphSpec), graph.key, graph.totalParams);
    } finally { database.close(); }
    const originalRecords = legacyRecords(dbPath);
    await readOnlyLegacyConversion(root, dbPath, 'typescript-v2').catch(async error => {
      await rm(root, { recursive: true, force: true });
      throw error;
    });
    const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
    let server = await startRustServer({
      ...resumeConfig, port: 0, resume: 'latest', dbPath
    });
    const peers: Peer[] = [];
    try {
      expect(server.startupFault).toBeUndefined();
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as {
        runId: string; startupCheckpointId: string; seed: number;
      };
      const legacyConversion = {
        version: 1, sourceRunId: 'typescript-source-run', sourceGeneration: '0000000000000013', sourceSeed: 7_654_321,
        sourceSnapshotId: 1,
        sourceFormat: 'typescript-v2',
        completeness: 'population-only',
        exactContinuation: false
      };
      expect(health).toMatchObject({
        ok: true, seed: expect.any(Number), generation: '0000000000000001', legacyConversion
      });
      expect(health.runId).not.toBe('typescript-source-run');
      const viewer = await connect(server.port, 'ui');
      peers.push(viewer);
      await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
      expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
        worldSeed: health.seed,
        settings: { core: { snakeCount: 2, simSpeed: 2 } },
        legacyConversion
      });
      viewer.socket.terminate();
      await server.close();
      server = await startRustServer({
        ...resumeConfig, port: 0, resume: 'latest', dbPath
      });
      expect(server.startupFault).toBeUndefined();
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: health.runId, startupCheckpointId: health.startupCheckpointId,
        legacyConversion
      });
      await migratedArchiveRoundTrip(root, server, createHash('sha256')
        .update(firstWeights).update(secondWeights).digest('hex'));
      expect(legacyRecords(dbPath, originalRecords.tables)).toEqual(originalRecords);
      const retained = new Database(dbPath, { readonly: true });
      try {
        const row = retained.prepare('SELECT payload_json FROM population_snapshots WHERE id = 1')
          .get() as { payload_json: string };
        const blobs = retained.prepare(`SELECT weights_blob FROM snapshot_genomes
          WHERE snapshot_id = 1 ORDER BY slot`).all() as Array<{ weights_blob: Buffer }>;
        expect(createHash('sha256').update(row.payload_json)
          .update(blobs[0]!.weights_blob).update(blobs[1]!.weights_blob).digest('hex')).toBe(sourceDigest);
      } finally { retained.close(); }
      const resetPeer = await connect(server.port, 'ui');
      peers.push(resetPeer);
      resetPeer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      let activeRunId = health.runId;
      for (let index = 0; index < 5; index++) {
        resetPeer.packets.length = 0;
        resetPeer.socket.send(JSON.stringify({ type: 'reset', settings: { simSpeed: 0.1 },
          updates: [{ path: 'worldRadius', value: 4200 + index * 50 }] }));
        const replaced = await replacementUntil(resetPeer, server.port, 'reset');
        expect(replaced.worldSeed).toBe(health.seed);
        expect(replaced.runId).not.toBe(activeRunId);
        expect(replaced).not.toHaveProperty('legacyConversion');
        expect(replaced.settings.updates).toContainEqual({ path: 'worldRadius', value: 4200 + index * 50 });
        expect(resetPeer.packets.filter(packet => packet['type'] === 'error')).toEqual([]);
        activeRunId = replaced.runId;
        resetPeer.socket.send(JSON.stringify({ type: 'join', mode: 'spectator', rejoinToken: resetPeer.rejoinToken }));
      }
      resetPeer.socket.terminate();
      await server.close();
      server = await startRustServer({ ...resumeConfig, port: 0, resume: 'latest', dbPath });
      expect(server.startupFault).toBeUndefined();
      expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json())
        .toMatchObject({ ok: true, runId: activeRunId, seed: health.seed });
      expect(legacyRecords(dbPath, originalRecords.tables)).toEqual(originalRecords);
      const afterResets = new Database(dbPath, { readonly: true });
      try {
        expect(afterResets.prepare('PRAGMA foreign_key_check').all()).toEqual([]);
        expect(afterResets.prepare('SELECT run_id FROM rust_checkpoint_v3_current WHERE run_id = ?').get(health.runId))
          .toBeUndefined();
        expect(afterResets.prepare('SELECT source_snapshot_id, source_format, completeness FROM rust_legacy_conversions_v1 WHERE run_id = ?')
          .get(health.runId)).toEqual({ source_snapshot_id: 1, source_format: 'typescript-v2', completeness: 'population-only' });
      } finally { afterResets.close(); }
    } finally {
      for (const peer of peers) peer.socket.terminate();
      await server.close();
      await rm(root, { recursive: true, force: true });
    }
  }, 30_000);

  it.each((['gzip', 'embedded'] as const).flatMap(storage =>
    (['absent', 'null', 'zero'] as const).map(format => ({ storage, format }))))(
    'converts a $storage population with $format format column without changing any source row',
    async ({ storage, format }) => {
      const root = await mkdtemp(join(tmpdir(), `slither-rust-v0-${storage}-`));
      const dbPath = join(root, 'experiment.sqlite');
      const core = { ...DEFAULT_CORE_SETTINGS, snakeCount: 2, simSpeed: 3 };
      const graphSpec = buildStackGraphSpec(core, {...CFG_DEFAULT, brain: {...CFG_DEFAULT.brain, inSize: 83}});
      const graph = compileGraph(graphSpec);
      const first = new Array<number>(graph.totalParams).fill(0);
      const second = new Array<number>(graph.totalParams).fill(0);
      second[0] = 0.25;
      const genomes = [first, second].map((weights, slot) => ({
        archKey: graph.key, brainType: 'mlp', fitness: 10 - slot, weights
      }));
      const payload = JSON.stringify({
        generation: 7,
        archKey: graph.key,
        genomes: storage === 'embedded' ? genomes : [],
        cfgHash: 'legacy-config',
        worldSeed: 1_234_567,
        graphSpec,
        ...(storage === 'embedded' ? {
          settings: core,
          updates: [{ path: 'baselineBots.count', value: 0 }]
        } : {})
      });
      const framed = storage === 'gzip' ? gzipSync(Buffer.concat(genomes.flatMap(genome => {
        const encoded = Buffer.from(JSON.stringify(genome));
        const prefix = Buffer.alloc(4);
        prefix.writeUInt32LE(encoded.byteLength);
        return [prefix, encoded];
      }))) : null;
      const sourceDigest = createHash('sha256').update(payload).update(framed ?? Buffer.alloc(0)).digest('hex');
      const database = new Database(dbPath);
      try {
        database.exec(storage === 'gzip' ? `CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, created_at INTEGER, gen INTEGER, payload_json TEXT,
          settings_json TEXT, updates_json TEXT, genomes_blob BLOB
        )` : `CREATE TABLE population_snapshots (
          id INTEGER PRIMARY KEY, created_at INTEGER, gen INTEGER, payload_json TEXT
        )`);
        if (format !== 'absent') database.exec('ALTER TABLE population_snapshots ADD COLUMN format_version INTEGER');
        if (storage === 'gzip') {
          database.prepare(`INSERT INTO population_snapshots
            (id, created_at, gen, payload_json, settings_json, updates_json, genomes_blob)
            VALUES (1, ?, 7, ?, ?, ?, ?)`).run(Date.now(), payload, JSON.stringify(core),
              JSON.stringify([{ path: 'baselineBots.count', value: 0 }]), framed);
        } else {
          database.prepare(`INSERT INTO population_snapshots
            (id, created_at, gen, payload_json) VALUES (1, ?, 7, ?)`).run(Date.now(), payload);
        }
        if (format !== 'absent') database.prepare('UPDATE population_snapshots SET format_version = ? WHERE id = 1')
          .run(format === 'zero' ? 0 : null);
        legacyCompanions(database, JSON.stringify(graphSpec), graph.key, graph.totalParams);
      } finally { database.close(); }
      const originalRecords = legacyRecords(dbPath);
      await readOnlyLegacyConversion(root, dbPath, storage === 'gzip' ? 'legacy-gzip' : 'legacy-json').catch(async error => {
        await rm(root, { recursive: true, force: true });
        throw error;
      });

      const { seed: _defaultSeed, ...resumeConfig } = DEFAULT_CONFIG;
      const server = await startRustServer({
        ...resumeConfig, port: 0, resume: 'latest', dbPath
      });
      const peers: Peer[] = [];
      try {
        expect(server.startupFault).toBeUndefined();
        const legacyConversion = {
          version: 1, sourceGeneration: '0000000000000007', sourceSeed: 1_234_567,
          sourceSnapshotId: 1,
          sourceFormat: storage === 'gzip' ? 'legacy-gzip' : 'legacy-json',
          completeness: 'population-only',
          exactContinuation: false
        };
        expect(await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json()).toMatchObject({
          ok: true, seed: expect.any(Number), generation: '0000000000000001', legacyConversion
        });
        const viewer = await connect(server.port, 'ui');
        peers.push(viewer);
        await until(viewer, () => viewer.packets.some(packet => packet['type'] === 'welcome'));
        expect(viewer.packets.find(packet => packet['type'] === 'welcome')).toMatchObject({
          worldSeed: expect.any(Number), settings: { core: { snakeCount: 2, simSpeed: 3 } },
          legacyConversion
        });
        const retained = new Database(dbPath, { readonly: true });
        try {
          const row = retained.prepare(`SELECT payload_json${storage === 'gzip' ? ', genomes_blob' : ''}
            FROM population_snapshots WHERE id = 1`).get() as { payload_json: string; genomes_blob?: Buffer };
          expect(createHash('sha256').update(row.payload_json)
            .update(row.genomes_blob ?? Buffer.alloc(0)).digest('hex')).toBe(sourceDigest);
          expect((retained.prepare('SELECT count(*) AS count FROM rust_checkpoint_v3_current').get() as
            { count: number }).count).toBe(1);
          expect(retained.prepare(`SELECT source_snapshot_id, source_format, completeness
            FROM rust_legacy_conversions_v1`).get()).toEqual({
            source_snapshot_id: 1,
            source_format: legacyConversion.sourceFormat,
            completeness: 'population-only'
          });
        } finally { retained.close(); }
        const packedWeights = Buffer.alloc(graph.totalParams * 2 * Float32Array.BYTES_PER_ELEMENT);
        packedWeights.writeFloatLE(0.25, graph.totalParams * Float32Array.BYTES_PER_ELEMENT);
        await migratedArchiveRoundTrip(root, server, createHash('sha256').update(packedWeights).digest('hex'));
        expect(legacyRecords(dbPath, originalRecords.tables)).toEqual(originalRecords);
      } finally {
        for (const peer of peers) peer.socket.terminate();
        await server.close();
        await rm(root, { recursive: true, force: true });
      }
    },
    30_000
  );
});
