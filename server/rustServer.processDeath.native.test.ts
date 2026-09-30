import { spawn, type ChildProcess } from 'node:child_process';
import { mkdtemp, readdir, rm, utimes } from 'node:fs/promises';
import { request as httpRequest } from 'node:http';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vitest';
import WebSocket from 'ws';
import { describeNetworkSuite } from './test/networkSuites.ts';
import { ARCHIVE_ARTIFACT_GRACE_MS, isRecognizedArchiveArtifact } from './rustEngine/archiveScavenger.ts';

/** One scalar health response needed by the process-death contract. */
interface ProcessHealth {
  ok: boolean;
  runId: string;
  startupCheckpointId: string;
  completedStep: string;
  worldEpoch: string;
  generation: string;
}

/** Replacement boundaries reached by the real child process. */
type ReplacementCrashPoint = 'afterCommit' | 'afterSwap';

/** Owner-visible operations that publish a complete replacement authority. */
const REPLACEMENT_OPERATIONS = ['newRun', 'reset', 'import'] as const;
/** Both sides of the final Rust swap, tested for each replacement entry point. */
const REPLACEMENT_CRASH_CASES = REPLACEMENT_OPERATIONS.flatMap(operation => [
  { operation, point: 'afterCommit' as const, description: 'after SQLite commit and before Rust swap' },
  { operation, point: 'afterSwap' as const, description: 'after Rust swap and before public success' }
]);

/** Reserve a loopback port briefly for a child server with CLI-only startup. */
async function availablePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolveReady, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolveReady);
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('loopback test server has no port');
  await new Promise<void>((resolveClosed, reject) => server.close(error => error ? reject(error) : resolveClosed()));
  return address.port;
}

/** Start the server or its replacement-death fixture in a distinct OS process. */
function spawnRustServer(port: number, databasePath: string, resume: 'fresh' | 'latest',
  crashFixture?: { kind: 'replacement' | 'generation'; point: ReplacementCrashPoint }): {
  child: ChildProcess;
  output: () => string;
} {
  const args = crashFixture
    ? ['--import', 'tsx', resolve(crashFixture.kind === 'replacement'
      ? 'server/test/killDuringAuthorityReplacement.ts' : 'server/test/killDuringGenerationHandoff.ts'),
    String(port), databasePath, crashFixture.point]
    : [
      '--import', 'tsx', resolve('server/rustServer.ts'),
      '--host', '127.0.0.1', '--port', String(port),
      '--db-path', databasePath, '--checkpoint-every', '1', '--mt', 'false',
      '--backend', 'native', '--rust-workers', '1',
      ...(resume === 'fresh' ? ['--fresh', '--seed', '42'] : ['--resume', 'latest'])
    ];
  const child = spawn(process.execPath, args, { cwd: resolve(), stdio: ['ignore', 'pipe', 'pipe'] });
  let transcript = '';
  child.stdout?.on('data', bytes => { transcript = `${transcript}${String(bytes)}`.slice(-4096); });
  child.stderr?.on('data', bytes => { transcript = `${transcript}${String(bytes)}`.slice(-4096); });
  return { child, output: () => transcript };
}

/** Poll until healthy Rust authority advances beyond the requested saved boundary. */
async function readyHealth(
  port: number,
  child: ChildProcess,
  output: () => string,
  minimumCompletedStep = 0n
): Promise<ProcessHealth> {
  const deadline = performance.now() + 15_000;
  while (performance.now() < deadline && child.exitCode === null && child.signalCode === null) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/api/health`, { signal: AbortSignal.timeout(1000) });
      const health = await response.json() as ProcessHealth;
      if (health.ok && BigInt(`0x${health.completedStep}`) > minimumCompletedStep) return health;
    } catch { /* Child has not bound its socket yet. */ }
    await new Promise<void>(done => setTimeout(done, 25));
  }
  throw new Error(`Rust child did not start: ${output()}`);
}

/** Wait for one deliberately incomplete import to create its private spool file. */
async function pendingUpload(managedRoot: string): Promise<string> {
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    const names = await readdir(managedRoot);
    const pending = names.find(name => name.endsWith('.upload.partial'));
    if (pending) return pending;
    await new Promise<void>(done => setTimeout(done, 10));
  }
  throw new Error('archive upload never created its partial spool');
}

/** Wait for a child to terminate after an explicit signal. */
async function terminate(child: ChildProcess, signal: NodeJS.Signals): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = new Promise<void>(resolveExit => child.once('exit', () => resolveExit()));
  child.kill(signal);
  let timer: NodeJS.Timeout | undefined;
  try {
    await Promise.race([
      exited,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('Rust child did not exit')), 5000);
      })
    ]);
  } finally { if (timer) clearTimeout(timer); }
}

/** Wait for the injected child death without retaining a test timer afterward. */
async function waitForReplacementDeath(child: ChildProcess, output: () => string): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  let timer: NodeJS.Timeout | undefined;
  try {
    await Promise.race([
      new Promise<void>(resolveExit => child.once('exit', () => resolveExit())),
      new Promise<never>((_, rejectExit) => {
        timer = setTimeout(() => rejectExit(new Error(
          `Server did not reach replacement-death point: ${output()}`)), 15_000);
      })
    ]);
  } finally { if (timer) clearTimeout(timer); }
}

describeNetworkSuite('Rust process-death recovery', () => {
  it.each(REPLACEMENT_CRASH_CASES)('resumes $operation killed $description', async ({ operation, point: crashPoint }) => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-replacement-death-'));
    const databasePath = join(root, 'experiment.sqlite');
    const managedRoot = `${databasePath}.checkpoints`;
    const port = await availablePort();
    const first = spawnRustServer(port, databasePath, 'fresh', { kind: 'replacement', point: crashPoint });
    let restarted: ReturnType<typeof spawnRustServer> | undefined;
    let viewer: WebSocket | undefined;
    try {
      let archive: ArrayBuffer | undefined;
      let sourceHealth: ProcessHealth | undefined;
      if (operation === 'import') {
        const sourcePort = await availablePort();
        const source = spawnRustServer(sourcePort, join(root, 'source.sqlite'), 'fresh');
        try {
          sourceHealth = await readyHealth(sourcePort, source.child, source.output);
          const exported = await fetch(`http://127.0.0.1:${sourcePort}/api/export/latest`, {
            signal: AbortSignal.timeout(15_000)
          });
          expect(exported.status).toBe(200);
          archive = await exported.arrayBuffer();
        } finally { await terminate(source.child, 'SIGKILL'); }
      }
      const before = await readyHealth(port, first.child, first.output);
      viewer = new WebSocket(`ws://127.0.0.1:${port}`);
      const viewerClosed = new Promise<void>(resolveClosed => viewer!.once('close', () => resolveClosed()));
      const publicSuccess: string[] = [];
      const welcome = new Promise<void>((resolveWelcome, rejectWelcome) => {
        viewer!.once('error', rejectWelcome);
        viewer!.on('message', (bytes, binary) => {
          if (binary) return;
          const message = JSON.parse(bytes.toString()) as { type: string };
          if (message.type === 'stateReplaced' || message.type === 'newRunResult') {
            publicSuccess.push(message.type);
          }
          if (message.type === 'welcome') {
            viewer!.off('error', rejectWelcome);
            resolveWelcome();
          }
        });
      });
      await new Promise<void>((resolveOpen, rejectOpen) => {
        viewer!.once('open', resolveOpen);
        viewer!.once('error', rejectOpen);
      });
      viewer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
      await welcome;
      viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      let importResponse: Promise<{ status: number } | { error: unknown }> | undefined;
      if (operation === 'import') {
        importResponse = fetch(`http://127.0.0.1:${port}/api/import/archive`, {
          method: 'POST', headers: { 'Content-Type': 'application/vnd.slither-neuroevo.save' },
          body: archive!, signal: AbortSignal.timeout(15_000)
        }).then(response => ({ status: response.status }), error => ({ error }));
      } else {
        viewer.send(JSON.stringify(operation === 'newRun'
          ? { type: 'newRun', requestId: 'kill-during-replacement' }
          : { type: 'reset' }));
      }
      await waitForReplacementDeath(first.child, first.output);
      await viewerClosed;
      if (importResponse) expect(await importResponse).toHaveProperty('error');
      expect(first.child.signalCode === 'SIGKILL' ||
        (process.platform === 'win32' && first.child.exitCode === 1), first.output()).toBe(true);
      const markerLine = first.output().split(/\r?\n/u)
        .find(line => line.startsWith('{"type":"replacementCrashPoint"'));
      expect(markerLine, first.output()).toBeDefined();
      const marker = JSON.parse(markerLine!) as {
        point: ReplacementCrashPoint;
        runId: string;
        checkpointId: string;
        publication?: { worldEpoch: string; generation: string; completedStep: string };
      };
      expect(marker.point).toBe(crashPoint);
      if (sourceHealth) expect(marker).toMatchObject({ runId: sourceHealth.runId,
        checkpointId: sourceHealth.startupCheckpointId });
      expect(publicSuccess).toEqual([]);
      if (crashPoint === 'afterSwap') {
        expect(marker.publication).toMatchObject({ generation: '0000000000000001',
          completedStep: '0000000000000000' });
        expect(BigInt(`0x${marker.publication!.worldEpoch}`)).toBeGreaterThan(BigInt(`0x${before.worldEpoch}`));
      }
      const database = new Database(databasePath, { readonly: true });
      let committedRunId: string;
      try {
        committedRunId = (database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1')
          .get() as { run_id: string }).run_id;
        expect(committedRunId).not.toBe(before.runId);
        expect(committedRunId).toBe(marker.runId);
        const current = database.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?')
          .get(committedRunId) as { checkpoint_id: string };
        expect(current.checkpoint_id).toBe(marker.checkpointId);
      } finally { database.close(); }
      viewer = undefined;
      const abandonedArtifacts = (await readdir(managedRoot)).filter(isRecognizedArchiveArtifact);
      if (operation === 'import') expect(abandonedArtifacts.some(name => name.endsWith('.upload.ready'))).toBe(true);
      const stale = new Date(Date.now() - ARCHIVE_ARTIFACT_GRACE_MS - 60_000);
      for (const name of abandonedArtifacts) await utimes(join(managedRoot, name), stale, stale);
      restarted = spawnRustServer(port, databasePath, 'latest');
      const after = await readyHealth(port, restarted.child, restarted.output);
      expect(after.runId).toBe(committedRunId);
      expect(after.startupCheckpointId).toBe(marker.checkpointId);
      expect(after.completedStep).not.toBe('0000000000000000');
      expect((await readdir(managedRoot)).filter(isRecognizedArchiveArtifact)).toEqual([]);
    } finally {
      viewer?.terminate();
      if (restarted) await terminate(restarted.child, 'SIGTERM');
      await terminate(first.child, 'SIGKILL');
      await rm(root, { recursive: true, force: true });
    }
  }, 45_000);

  it.each(['afterCommit', 'afterSwap'] as const)('resumes the evolved checkpoint after generation death %s', async (point) => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-generation-death-'));
    const databasePath = join(root, 'experiment.sqlite');
    const port = await availablePort();
    const first = spawnRustServer(port, databasePath, 'fresh', { kind: 'generation', point });
    let restarted: ReturnType<typeof spawnRustServer> | undefined;
    let viewer: WebSocket | undefined;
    try {
      await readyHealth(port, first.child, first.output);
      viewer = new WebSocket(`ws://127.0.0.1:${port}`);
      const viewerClosed = new Promise<void>(done => viewer!.once('close', () => done()));
      const packets: Array<Record<string, unknown>> = [];
      const displayedGenerations: number[] = [];
      viewer.on('message', (bytes, binary) => {
        if (binary) displayedGenerations.push((bytes as Buffer).readFloatLE(0));
        else packets.push(JSON.parse(bytes.toString()) as Record<string, unknown>);
      });
      await new Promise<void>((done, reject) => { viewer!.once('open', done); viewer!.once('error', reject); });
      viewer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
      /** Wait for a specific lifecycle reply before the next control request. */
      const untilPacket = async (predicate: (packet: Record<string, unknown>) => boolean): Promise<void> => {
        const deadline = performance.now() + 5000;
        while (!packets.some(predicate) && performance.now() < deadline) {
          await new Promise<void>(done => setTimeout(done, 10));
        }
        expect(packets.some(predicate), JSON.stringify(packets)).toBe(true);
      };
      await untilPacket(packet => packet['type'] === 'welcome');
      viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      viewer.send(JSON.stringify({ type: 'reset', settings: { snakeCount: 12, simSpeed: 1 },
        updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 0 }] }));
      await untilPacket(packet => packet['type'] === 'stateReplaced' && packet['reason'] === 'reset');
      const before = await readyHealth(port, first.child, first.output);
      viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
      viewer.send(JSON.stringify({ type: 'settings', requestId: 'generation-death-speed',
        updates: [{ path: 'simSpeed', value: 4 }] }));
      await untilPacket(packet => packet['type'] === 'settingsApplied' &&
        packet['requestId'] === 'generation-death-speed' && packet['applied'] === true);
      await waitForReplacementDeath(first.child, first.output);
      await viewerClosed;
      expect(first.child.signalCode === 'SIGKILL' ||
        (process.platform === 'win32' && first.child.exitCode === 1), first.output()).toBe(true);
      const markerLine = first.output().split(/\r?\n/u)
        .find(line => line.startsWith('{"type":"generationCrashPoint"'));
      expect(markerLine, first.output()).toBeDefined();
      const marker = JSON.parse(markerLine!) as {
        point: ReplacementCrashPoint; runId: string; checkpointId: string;
        generation: string; completedStep: string;
        publication?: { publication: { worldEpoch: string; generation: string; completedStep: string } };
      };
      expect(marker).toMatchObject({ point, runId: before.runId, generation: '0000000000000002' });
      expect(marker.checkpointId).not.toBe(before.startupCheckpointId);
      expect(BigInt(`0x${marker.completedStep}`)).toBeGreaterThan(BigInt(`0x${before.completedStep}`));
      expect(displayedGenerations.length).toBeGreaterThan(0);
      expect(displayedGenerations.every(generation => generation === 1)).toBe(true);
      expect(packets.filter(packet => packet['type'] === 'stats').every(packet => packet['gen'] === 1)).toBe(true);
      if (point === 'afterSwap') {
        expect(marker.publication?.publication).toMatchObject({ generation: marker.generation,
          completedStep: marker.completedStep });
        expect(BigInt(`0x${marker.publication!.publication.worldEpoch}`)).toBeGreaterThan(BigInt(`0x${before.worldEpoch}`));
      }
      const database = new Database(databasePath, { readonly: true });
      let history: unknown;
      let winner: unknown;
      try {
        expect(database.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?')
          .get(marker.runId)).toEqual({ checkpoint_id: marker.checkpointId });
        history = database.prepare('SELECT * FROM rust_generation_history_v1 WHERE run_id = ? AND generation_hex = ?')
          .all(marker.runId, '0000000000000001');
        winner = database.prepare('SELECT * FROM rust_hall_of_fame_v1 WHERE run_id = ? AND generation_hex = ?')
          .all(marker.runId, '0000000000000001');
        expect(history).toHaveLength(1);
        expect(winner).toHaveLength(1);
      } finally { database.close(); }
      viewer = undefined;
      restarted = spawnRustServer(port, databasePath, 'latest');
      const after = await readyHealth(port, restarted.child, restarted.output, BigInt(`0x${marker.completedStep}`));
      expect(after).toMatchObject({ runId: marker.runId, startupCheckpointId: marker.checkpointId });
      expect(BigInt(`0x${after.generation}`)).toBeGreaterThanOrEqual(2n);
      expect(BigInt(`0x${after.completedStep}`)).toBeGreaterThan(BigInt(`0x${marker.completedStep}`));
      const recovered = new Database(databasePath, { readonly: true });
      try {
        expect(recovered.prepare('SELECT * FROM rust_generation_history_v1 WHERE run_id = ? AND generation_hex = ?')
          .all(marker.runId, '0000000000000001')).toEqual(history);
        expect(recovered.prepare('SELECT * FROM rust_hall_of_fame_v1 WHERE run_id = ? AND generation_hex = ?')
          .all(marker.runId, '0000000000000001')).toEqual(winner);
      } finally { recovered.close(); }
    } finally {
      viewer?.terminate();
      if (restarted) await terminate(restarted.child, 'SIGTERM');
      await terminate(first.child, 'SIGKILL');
      await rm(root, { recursive: true, force: true });
    }
  }, 45_000);

  it('resumes the last committed boundary and exports it after an OS kill during steps', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-process-death-'));
    const databasePath = join(root, 'experiment.sqlite');
    const port = await availablePort();
    const first = spawnRustServer(port, databasePath, 'fresh');
    let restarted: ReturnType<typeof spawnRustServer> | undefined;
    try {
      const before = await readyHealth(port, first.child, first.output);
      await terminate(first.child, 'SIGKILL');
      const database = new Database(databasePath, { readonly: true });
      try {
        const current = database.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?')
          .get(before.runId) as { checkpoint_id: string } | undefined;
        expect(current?.checkpoint_id).toBe(before.startupCheckpointId);
      } finally { database.close(); }

      restarted = spawnRustServer(port, databasePath, 'latest');
      const after = await readyHealth(port, restarted.child, restarted.output);
      expect(after.runId).toBe(before.runId);
      expect(after.startupCheckpointId).toBe(before.startupCheckpointId);
      const exported = await fetch(`http://127.0.0.1:${port}/api/export/latest`, {
        signal: AbortSignal.timeout(15_000)
      });
      expect(exported.status).toBe(200);
      expect(exported.headers.get('content-disposition')).toContain(before.startupCheckpointId.slice(0, 12));
      expect(Number(exported.headers.get('content-length'))).toBeGreaterThan(0);
      await exported.arrayBuffer();
    } finally {
      if (restarted) await terminate(restarted.child, 'SIGTERM');
      await terminate(first.child, 'SIGKILL');
      await rm(root, { recursive: true, force: true });
    }
  }, 45_000);

  it('cleans a partial import upload after an OS kill without changing the committed run', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-upload-death-'));
    const databasePath = join(root, 'experiment.sqlite');
    const managedRoot = `${databasePath}.checkpoints`;
    const port = await availablePort();
    const first = spawnRustServer(port, databasePath, 'fresh');
    let restarted: ReturnType<typeof spawnRustServer> | undefined;
    let upload: ReturnType<typeof httpRequest> | undefined;
    try {
      const before = await readyHealth(port, first.child, first.output);
      upload = httpRequest(`http://127.0.0.1:${port}/api/import/archive`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/vnd.slither-neuroevo.save',
          'Transfer-Encoding': 'chunked'
        }
      });
      upload.on('error', () => { /* Killing the receiving process closes this upload. */ });
      upload.write(Buffer.from('incomplete-archive-body'));
      const partial = await pendingUpload(managedRoot);
      await terminate(first.child, 'SIGKILL');
      upload.destroy();
      expect(await readdir(managedRoot)).toContain(partial);

      // Restart cleanup deliberately preserves fresh artifacts for its documented grace.
      const stale = new Date(Date.now() - ARCHIVE_ARTIFACT_GRACE_MS - 60_000);
      await utimes(join(managedRoot, partial), stale, stale);

      restarted = spawnRustServer(port, databasePath, 'latest');
      const after = await readyHealth(port, restarted.child, restarted.output);
      expect(after.runId).toBe(before.runId);
      expect(after.startupCheckpointId).toBe(before.startupCheckpointId);
      expect(await readdir(managedRoot)).not.toContain(partial);
      const database = new Database(databasePath, { readonly: true });
      try {
        const current = database.prepare('SELECT checkpoint_id FROM rust_checkpoint_v3_current WHERE run_id = ?')
          .get(before.runId) as { checkpoint_id: string } | undefined;
        expect(current?.checkpoint_id).toBe(before.startupCheckpointId);
      } finally { database.close(); }
    } finally {
      upload?.destroy();
      if (restarted) await terminate(restarted.child, 'SIGTERM');
      await terminate(first.child, 'SIGKILL');
      await rm(root, { recursive: true, force: true });
    }
  }, 45_000);
});
