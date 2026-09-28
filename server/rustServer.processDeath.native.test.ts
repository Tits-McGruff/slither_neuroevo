import { spawn, type ChildProcess } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import Database from 'better-sqlite3';
import { expect, it } from 'vitest';
import { describeNetworkSuite } from './test/networkSuites.ts';

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

/** Start the actual CLI program in a distinct OS process. */
function spawnRustServer(port: number, databasePath: string, resume: 'fresh' | 'latest'): {
  child: ChildProcess;
  output: () => string;
} {
  const args = [
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

describeNetworkSuite('Rust process-death recovery', () => {
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
});
