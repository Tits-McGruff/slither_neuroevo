/** Prove a budget-rejected P3 Reset leaves the original live Rust game usable. */

import { existsSync } from 'node:fs';
import { mkdir, readdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import WebSocket from 'ws';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';
import { buildLargeBrainGraph } from '../fixtures/largeBrainGraph.ts';

/** Health fields needed to prove the original authority resumes stepping. */
interface Health {
  ok: boolean;
  runId: string;
  completedStep: string;
  startupCheckpointId: string;
  faultCode?: string;
}

/** Read one live health response with a bounded network deadline. */
async function health(port: number): Promise<Health> {
  const response = await fetch(`http://127.0.0.1:${port}/api/health`, {
    signal: AbortSignal.timeout(5_000)
  });
  const value = await response.json() as Health;
  if (!response.ok || !value.ok) throw new Error(`health fault: ${value.faultCode ?? response.status}`);
  return { ok: value.ok, runId: value.runId, completedStep: value.completedStep,
    startupCheckpointId: value.startupCheckpointId };
}

/** Submit the approved P3 workload as Reset and require its budget error. */
async function rejectedReset(port: number): Promise<string> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`);
  return new Promise<string>((resolveMessage, reject) => {
    let done = false;
    const timer = setTimeout(() => finish(new Error('P3 Reset did not answer within 120 seconds')), 120_000);
    /** Settle the one-command exchange and close its temporary socket. */
    const finish = (error?: Error, message?: string): void => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      socket.terminate();
      if (error) reject(error);
      else resolveMessage(message ?? '');
    };
    socket.on('open', () => socket.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' })));
    socket.on('error', error => finish(error));
    socket.on('close', () => finish(new Error('P3 Reset socket closed before the result')));
    socket.on('message', (data, binary) => {
      if (binary || done) return;
      try {
        const value = JSON.parse(data.toString()) as Record<string, unknown>;
        if (value['type'] === 'welcome') {
          socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
          socket.send(JSON.stringify({
            type: 'reset',
            settings: { snakeCount: 300, simSpeed: 1, hiddenLayers: 5,
              neurons1: 256, neurons2: 256, neurons3: 256, neurons4: 256, neurons5: 256 },
            updates: [
              { path: 'generationSeconds', value: 8 },
              { path: 'sense.bubbleBins', value: 32 },
              { path: 'baselineBots.count', value: 10 },
              { path: 'pelletCountTarget', value: 3500 }
            ],
            graphSpec: buildLargeBrainGraph(147)
          }));
        } else if (value['type'] === 'stateReplaced') {
          finish(new Error('P3 Reset unexpectedly replaced the live game'));
        } else if (value['type'] === 'error') {
          const message = String(value['message']);
          if (!message.includes('reset failed: checkpoint budget')) {
            finish(new Error(`P3 Reset failed for another reason: ${message}`));
          } else finish(undefined, message);
        }
      } catch (error) { finish(error instanceof Error ? error : new Error(String(error))); }
    });
  });
}

/** Confirm the old game advances again in the same process after Reset rejects. */
async function resumedHealth(port: number, before: Health): Promise<Health> {
  const deadline = performance.now() + 10_000;
  let current = await health(port);
  while (BigInt(`0x${current.completedStep}`) <= BigInt(`0x${before.completedStep}`) &&
      performance.now() < deadline) {
    await new Promise<void>(done => setTimeout(done, 100));
    current = await health(port);
  }
  if (current.runId !== before.runId || current.startupCheckpointId !== before.startupCheckpointId ||
      BigInt(`0x${current.completedStep}`) <= BigInt(`0x${before.completedStep}`)) {
    throw new Error(`original authority did not resume: ${JSON.stringify({ before, current })}`);
  }
  return current;
}

/** Read compact durable state and count actual managed files after server close. */
async function durableState(databasePath: string): Promise<Record<string, unknown>> {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    const pointers = database.prepare('SELECT run_id AS runId, checkpoint_id AS checkpointId FROM rust_checkpoint_v3_current').all();
    const metadataRows = (database.prepare('SELECT count(*) AS n FROM rust_checkpoint_v3_metadata').get() as { n: number }).n;
    const managedFiles = (await readdir(`${databasePath}.checkpoints`)).length;
    if (pointers.length !== 1 || metadataRows !== 1 || managedFiles !== 1) {
      throw new Error(`rejected Reset changed durable state: ${JSON.stringify({ pointers, metadataRows, managedFiles })}`);
    }
    return { pointers, metadataRows, managedFiles };
  } finally { database.close(); }
}

/** Run one disposable minimum-budget production server and report the outcome. */
export async function run(databasePath: string): Promise<Record<string, unknown>> {
  if (existsSync(databasePath) || existsSync(`${databasePath}.checkpoints`)) {
    throw new Error(`destination already exists: ${databasePath}`);
  }
  await mkdir(dirname(databasePath), { recursive: true });
  const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: databasePath,
    resume: 'fresh', seed: 1511506142, checkpointBudgetMiB: 1280,
    rustCalculationWorkers: 5, logLevel: 'error' });
  if (server.startupFault) {
    await server.close();
    throw new Error(`server startup failed: ${server.startupFault}`);
  }
  let before: Health;
  let atRejection: Health;
  let after: Health;
  let rejection: string;
  try {
    before = await health(server.port);
    rejection = await rejectedReset(server.port);
    atRejection = await health(server.port);
    if (atRejection.runId !== before.runId ||
        atRejection.startupCheckpointId !== before.startupCheckpointId) {
      throw new Error('P3 Reset changed the original authority before its rejection reply');
    }
    after = await resumedHealth(server.port, atRejection);
  } finally { await server.close(); }
  return { rejection, before, atRejection, after, durable: await durableState(databasePath) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  if (args.length !== 2 || args[0] !== '--db-path') throw new Error('usage: --db-path PATH');
  void run(resolve(args[1]!)).then(result => process.stdout.write(`${JSON.stringify(result)}\n`)).catch(error => {
    process.stderr.write(`${String(error)}\n`);
    process.exitCode = 1;
  });
}
