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
import { ARCHIVE_ARTIFACT_GRACE_MS } from './rustEngine/archiveScavenger.ts';

/** One scalar health response needed by the process-death contract. */
interface ProcessHealth {
  ok: boolean;
  runId: string;
  startupCheckpointId: string;
  completedStep: string;
}

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

/** Start the server or its commit-death fixture in a distinct OS process. */
function spawnRustServer(port: number, databasePath: string, resume: 'fresh' | 'latest',
  killAfterNewRunCommit = false): {
  child: ChildProcess;
  output: () => string;
} {
  const args = killAfterNewRunCommit
    ? ['--import', 'tsx', resolve('server/test/killAfterNewRunCommit.ts'), String(port), databasePath]
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

/** Poll until the child publishes healthy Rust authority and at least one step. */
async function readyHealth(
  port: number,
  child: ChildProcess,
  output: () => string
): Promise<ProcessHealth> {
  const deadline = performance.now() + 15_000;
  while (performance.now() < deadline && child.exitCode === null && child.signalCode === null) {
    try {
      const response = await fetch(`http://127.0.0.1:${port}/api/health`, { signal: AbortSignal.timeout(1000) });
      const health = await response.json() as ProcessHealth;
      if (health.ok && BigInt(`0x${health.completedStep}`) > 0n) return health;
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
async function waitForCommitDeath(child: ChildProcess, output: () => string): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return;
  let timer: NodeJS.Timeout | undefined;
  try {
    await Promise.race([
      new Promise<void>(resolveExit => child.once('exit', () => resolveExit())),
      new Promise<never>((_, rejectExit) => {
        timer = setTimeout(() => rejectExit(new Error(
          `New Run did not reach commit-death point: ${output()}`)), 15_000);
      })
    ]);
  } finally { if (timer) clearTimeout(timer); }
}

describeNetworkSuite('Rust process-death recovery', () => {
  it('resumes a New Run killed after SQLite commit and before Rust swap', async () => {
    const root = await mkdtemp(join(tmpdir(), 'slither-rust-new-run-commit-death-'));
    const databasePath = join(root, 'experiment.sqlite');
    const port = await availablePort();
    const first = spawnRustServer(port, databasePath, 'fresh', true);
    let restarted: ReturnType<typeof spawnRustServer> | undefined;
    let viewer: WebSocket | undefined;
    try {
      const before = await readyHealth(port, first.child, first.output);
      viewer = new WebSocket(`ws://127.0.0.1:${port}`);
      const welcome = new Promise<void>((resolveWelcome, rejectWelcome) => {
        viewer!.once('error', rejectWelcome);
        viewer!.on('message', (bytes, binary) => {
          if (binary) return;
          if ((JSON.parse(bytes.toString()) as { type: string }).type === 'welcome') {
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
      viewer.send(JSON.stringify({ type: 'newRun', requestId: 'kill-after-commit' }));
      await waitForCommitDeath(first.child, first.output);
      expect(first.child.signalCode === 'SIGKILL' ||
        (process.platform === 'win32' && first.child.exitCode === 1), first.output()).toBe(true);
      const database = new Database(databasePath, { readonly: true });
      let committedRunId: string;
      try {
        committedRunId = (database.prepare('SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1')
          .get() as { run_id: string }).run_id;
        expect(committedRunId).not.toBe(before.runId);
      } finally { database.close(); }
      viewer.terminate();
      viewer = undefined;
      restarted = spawnRustServer(port, databasePath, 'latest');
      const after = await readyHealth(port, restarted.child, restarted.output);
      expect(after.runId).toBe(committedRunId);
      expect(after.completedStep).not.toBe('0000000000000000');
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
