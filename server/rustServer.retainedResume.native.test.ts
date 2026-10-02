/** Production exact-ID startup across retained runs, with no metadata-selection workaround. */
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdtemp, readdir, readFile, rm, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import Database from 'better-sqlite3';
import { expect, it, vi } from 'vitest';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from './config.ts';
import { startRustServer, type RustServer } from './rustServer.ts';
import { CheckpointPersistenceClient } from './rustEngine/checkpointPersistenceClient.ts';
import { parseManagedCheckpointDescriptor, type ManagedCheckpointDescriptor } from './rustEngine/checkpointPersistenceProtocol.ts';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { verifyRetainedAnchors } from '../scripts/stage7/verify-retained-anchors.ts';

/** Genuine durable boundary held before its coordinator receives the SQLite acknowledgement. */
interface Boundary {
  /** Immutable production descriptor. */
  descriptor: ManagedCheckpointDescriptor;
  /** Release only the already committed reply. */
  release(): void;
}

/** Private experiment containing a prior evolved run and a different active run. */
interface Fixture {
  /** Test-owned SQLite path. */
  databasePath: string;
  /** Prior-run generation-two and generation-three boundaries. */
  boundaries: Map<number, Boundary>;
  /** Complete actual exports captured at those boundaries. */
  archives: Map<number, Buffer>;
  /** Durable run superseding the source before exact selection. */
  activeRunId: string;
  /** Real subsequent branch commits, held for independent observation. */
  successors: Boundary[];
  /** Register servers before assertions so failures still join them. */
  servers: RustServer[];
  /** Release existing replies and stop holding future commits. */
  releaseAll(): void;
}

/** Observe a real reply or boundary within the existing integration deadline. */
async function observed<T>(read: () => T | undefined): Promise<T> {
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    const result = read();
    if (result !== undefined) return result;
    await new Promise<void>(done => setTimeout(done, 10));
  }
  throw new Error('retained-resume boundary was not observed within five seconds');
}

/** Read complete immutable metadata and source records, excluding the permitted new branch tables. */
function records(path: string): unknown {
  const database = new Database(path, { readonly: true });
  try { return ['rust_checkpoint_v3_metadata', 'rust_generation_history_v1', 'rust_hall_of_fame_v1']
    .map(table => database.prepare(`SELECT * FROM ${table} ORDER BY rowid`).all()); }
  finally { database.close(); }
}

/** Hash every managed file without retaining population buffers. */
async function files(path: string): Promise<Array<{ name: string; sha256: string }>> {
  const results: Array<{ name: string; sha256: string }> = [];
  for (const name of (await readdir(path)).sort()) {
    const hash = createHash('sha256');
    for await (const bytes of createReadStream(join(path, name))) hash.update(bytes);
    results.push({ name, sha256: hash.digest('hex') });
  }
  return results;
}

/** Select both preserved runs' files independently of unrelated prior-run retention pruning. */
async function retainedFiles(databasePath: string, runIds: readonly [string, string]): Promise<Array<{ name: string; sha256: string }>> {
  const database = new Database(databasePath, { readonly: true });
  let roots: Set<string>;
  try {
    roots = new Set((database.prepare(`SELECT checkpoint_id FROM rust_checkpoint_retention_v1
      JOIN rust_checkpoint_v3_metadata USING (checkpoint_id)
      WHERE retention_kind IN ('automatic', 'pinned') AND run_id IN (?, ?)`)
      .all(...runIds) as Array<{ checkpoint_id: string }>)
      .map(row => row.checkpoint_id));
  } finally { database.close(); }
  return (await files(`${databasePath}.checkpoints`)).filter(file =>
    !file.name.endsWith('.checkpoint-v3') || roots.has(file.name.slice(0, 64)));
}

/** Use the real download endpoint, with only a small test population buffered for exact comparison. */
async function archive(server: RustServer, descriptor: ManagedCheckpointDescriptor): Promise<Buffer> {
  const originalRelease = CheckpointPersistenceClient.prototype.releaseExportLease;
  let released = false;
  const release = vi.spyOn(CheckpointPersistenceClient.prototype, 'releaseExportLease').mockImplementation(async function(
    this: CheckpointPersistenceClient, ...args: Parameters<CheckpointPersistenceClient['releaseExportLease']>
  ) { await originalRelease.apply(this, args); released = true; });
  try {
    const response = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`, { signal: AbortSignal.timeout(5000) });
    expect(response.status).toBe(200);
    expect(response.headers.get('x-slither-checkpoint-id')).toBe(descriptor.logicalRootSha256);
    const bytes = Buffer.from(await response.arrayBuffer());
    expect(bytes.byteLength).toBeLessThan(4 * 1024 * 1024);
    await observed(() => released ? true : undefined);
    return bytes;
  } finally { release.mockRestore(); }
}

/** Construct production checkpoints; every server, socket and private file has one cleanup owner. */
async function experiment(action: (fixture: Fixture) => Promise<void>): Promise<void> {
  const root = await mkdtemp(join(tmpdir(), 'slither-retained-resume-'));
  const databasePath = join(root, 'experiment.sqlite');
  const servers: RustServer[] = [];
  const boundaries = new Map<number, Boundary>();
  const archives = new Map<number, Buffer>();
  const successors: Boundary[] = [];
  const held: Boundary[] = [];
  let sourceRunId = '';
  let holding = true;
  let viewer: WebSocket | undefined;
  const originalCommit = CheckpointPersistenceClient.prototype.commit;
  const commits = vi.spyOn(CheckpointPersistenceClient.prototype, 'commit').mockImplementation(async function(
    this: CheckpointPersistenceClient, ...args: Parameters<CheckpointPersistenceClient['commit']>
  ) {
    const result = await originalCommit.apply(this, args);
    const descriptor = parseManagedCheckpointDescriptor(args[0]);
    if (!holding || descriptor.boundaryKind !== 'generation') return result;
    const gate = Promise.withResolvers<void>();
    const boundary = { descriptor, release: () => gate.resolve() };
    held.push(boundary);
    if (descriptor.runId === sourceRunId) boundaries.set(Number(BigInt(`0x${descriptor.generation}`)), boundary);
    else successors.push(boundary);
    await gate.promise;
    return result;
  });
  /** Release all actual worker replies before joining any coordinator. */
  const releaseAll = (): void => { holding = false; for (const boundary of held) boundary.release(); };
  try {
    const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: databasePath, resume: 'fresh', seed: 42 });
    servers.push(source);
    expect(source.startupFault).toBeUndefined();
    const messages: Array<Record<string, unknown>> = [];
    viewer = new WebSocket(`ws://127.0.0.1:${source.port}`);
    viewer.on('message', (bytes, binary) => { if (!binary) messages.push(JSON.parse(bytes.toString()) as Record<string, unknown>); });
    await new Promise<void>((done, reject) => { viewer!.once('open', done); viewer!.once('error', reject); });
    viewer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
    await observed(() => messages.find(message => message['type'] === 'welcome'));
    viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
    viewer.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 12 },
      updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 2 },
        { path: 'pelletCountTarget', value: 100 }] }));
    const replacement = await observed(() => messages.find(message => message['type'] === 'stateReplaced'));
    sourceRunId = String((replacement['welcome'] as { runId: string }).runId);
    const second = await observed(() => boundaries.get(2));
    archives.set(2, await archive(source, second.descriptor));
    const pin = await fetch(`http://127.0.0.1:${source.port}/api/checkpoints/current/pin`, { method: 'POST' });
    expect(pin.status).toBe(200);
    second.release();
    const third = await observed(() => boundaries.get(3));
    archives.set(3, await archive(source, third.descriptor));
    third.release();
    viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
    viewer.send(JSON.stringify({ type: 'settings', requestId: 'complete-retained-boundary',
      updates: [{ path: 'simSpeed', value: 0.1 }] }));
    await observed(() => messages.find(message => message['type'] === 'settingsApplied' &&
      message['requestId'] === 'complete-retained-boundary'));
    viewer.send(JSON.stringify({ type: 'newRun', requestId: 'supersede-retained-source' }));
    const newer = await observed(() => {
      expect(messages.find(message => message['type'] === 'error')).toBeUndefined();
      const result = messages.find(message => message['type'] === 'newRunResult');
      if (result) expect(result, JSON.stringify(result)).toMatchObject({ applied: true });
      return messages.find(message => message['type'] === 'stateReplaced' && message['reason'] === 'newRun');
    });
    const activeRunId = String((newer['welcome'] as { runId: string }).runId);
    viewer.terminate();
    await source.close();
    servers.pop();
    await action({ databasePath, boundaries, archives, activeRunId, successors, servers, releaseAll });
  } finally {
    releaseAll(); commits.mockRestore(); viewer?.terminate();
    for (const server of servers.reverse()) await server.close();
    await rm(root, { recursive: true, force: true });
  }
}

describeNetworkSuite('Rust retained exact-ID resume', () => {
  it.each([2, 3])('restores prior-run generation %s, exports it, and advances without replacing either source', async generation => {
    await experiment(async fixture => {
      const selected = fixture.boundaries.get(generation)!.descriptor;
      const before = records(fixture.databasePath);
      const preservedRuns = [selected.runId, fixture.activeRunId] as const;
      const sourceFiles = await retainedFiles(fixture.databasePath, preservedRuns);
      const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: fixture.databasePath,
        resume: `sha256:${selected.logicalRootSha256}` });
      fixture.servers.push(server);
      expect(server.startupFault).toBeUndefined();
      expect(records(fixture.databasePath)).toEqual(before);
      expect(await retainedFiles(fixture.databasePath, preservedRuns)).toEqual(sourceFiles);
      const health = await (await fetch(`http://127.0.0.1:${server.port}/api/health`)).json() as { runId: string };
      expect(health).toMatchObject({ ok: true, startupCheckpointId: selected.logicalRootSha256, recovery: {
        explicitResume: true, failedRunId: selected.runId, recoveredGeneration: selected.generation } });
      expect(health.runId).not.toBe(selected.runId);
      expect(health.runId).not.toBe(fixture.activeRunId);
      expect(await archive(server, selected)).toEqual(fixture.archives.get(generation));
      const successor = await observed(() => fixture.successors.find(boundary => boundary.descriptor.runId === health.runId));
      expect(successor.descriptor.generation).toBe(BigInt(generation + 1).toString(16).padStart(16, '0'));
      const database = new Database(fixture.databasePath, { readonly: true });
      try {
        expect(database.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?').get(selected.runId))
          .toEqual({ checkpoint_id: fixture.boundaries.get(3)!.descriptor.logicalRootSha256 });
        expect(database.prepare('SELECT count(*) AS count FROM rust_generation_history_v1 WHERE run_id = ?').get(selected.runId))
          .toEqual({ count: 2 });
      } finally { database.close(); }
      fixture.releaseAll();
      await server.close(); fixture.servers.pop();
      const restarted = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: fixture.databasePath, resume: 'latest' });
      fixture.servers.push(restarted);
      expect(restarted.startupFault).toBeUndefined();
      expect(await (await fetch(`http://127.0.0.1:${restarted.port}/api/health`)).json()).toMatchObject({
        ok: true, runId: health.runId, startupCheckpointId: successor.descriptor.logicalRootSha256,
        recovery: { explicitResume: true }
      });
      await restarted.close(); fixture.servers.pop();
      if (generation === 3) {
        const verified = await verifyRetainedAnchors(fixture.databasePath) as { verified: number; retainedAnchors: number };
        expect(verified.verified).toBe(verified.retainedAnchors);
        expect(verified.verified).toBeGreaterThanOrEqual(4);
      }
    });
  }, 30_000);

  it('fails exact selection for corrupt, missing and unknown roots without switching to another valid anchor', async () => {
    await experiment(async fixture => {
      const selected = fixture.boundaries.get(2)!.descriptor;
      const path = join(`${fixture.databasePath}.checkpoints`, selected.relativeFilename);
      const original = await readFile(path);
      const priorRecords = records(fixture.databasePath);
      for (const failure of ['corrupt', 'missing', 'unknown'] as const) {
        const bytes = Buffer.from(original);
        if (failure === 'corrupt') { bytes[0] = bytes[0]! ^ 1; await writeFile(path, bytes); }
        else if (failure === 'missing') await unlink(path);
        const beforeFiles = await files(`${fixture.databasePath}.checkpoints`);
        const root = failure === 'unknown' ? 'f'.repeat(64) : selected.logicalRootSha256;
        const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: fixture.databasePath, resume: `sha256:${root}` });
        fixture.servers.push(server);
        expect(server.startupFault).toBeDefined();
        expect((await fetch(`http://127.0.0.1:${server.port}/api/health`)).status).toBe(503);
        expect(records(fixture.databasePath)).toEqual(priorRecords);
        expect(await files(`${fixture.databasePath}.checkpoints`)).toEqual(beforeFiles);
        const database = new Database(fixture.databasePath, { readonly: true });
        try { expect(database.prepare('SELECT run_id FROM rust_active_run_v1').get()).toEqual({ run_id: fixture.activeRunId }); }
        finally { database.close(); }
        await server.close(); fixture.servers.pop();
        await writeFile(path, original);
      }
    });
  }, 30_000);
});
