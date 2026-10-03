/** Exact next-generation comparisons across the real production archive boundary. */
import { mkdtemp, rm } from 'node:fs/promises';
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
import type { GraphSpec } from '../src/brains/graph/schema.ts';

/** Small graph exercising dense math and all three recurrent kernels. */
const GRAPH: GraphSpec = {
  type: 'graph', outputSize: 2,
  nodes: [
    { id: 'input', type: 'Input', outputSize: 83 },
    { id: 'dense', type: 'Dense', inputSize: 83, outputSize: 8 },
    { id: 'gru', type: 'GRU', inputSize: 8, hiddenSize: 4 },
    { id: 'lstm', type: 'LSTM', inputSize: 4, hiddenSize: 4 },
    { id: 'rru', type: 'RRU', inputSize: 4, hiddenSize: 4 },
    { id: 'head', type: 'Dense', inputSize: 4, outputSize: 2 }
  ],
  edges: [{ from: 'input', to: 'dense' }, { from: 'dense', to: 'gru' },
    { from: 'gru', to: 'lstm' }, { from: 'lstm', to: 'rru' }, { from: 'rru', to: 'head' }],
  outputs: [{ nodeId: 'head' }]
};

/** A genuine FULL commit whose reply is held while its exact archive is examined. */
interface HeldBoundary {
  /** Rust's actual published descriptor, with no reconstructed population. */
  descriptor: ManagedCheckpointDescriptor;
  /** Release only the already durable reply to the running coordinator. */
  release(): void;
}

/** Wait for one explicit observation within the existing integration deadline. */
async function observed<T>(read: () => T | undefined): Promise<T> {
  const deadline = performance.now() + 5000;
  while (performance.now() < deadline) {
    const value = read();
    if (value !== undefined) return value;
    await new Promise<void>(done => setTimeout(done, 10));
  }
  throw new Error('continuation boundary was not observed within five seconds');
}

/** Read complete history and the archive's retained winners, excluding superseded duplicate tombstones. */
function records(databasePath: string, runId: string): { history: unknown[]; hallOfFame: unknown[] } {
  const database = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    return {
      history: database.prepare(`SELECT generation_hex, record_version, record_blob FROM rust_generation_history_v1
        WHERE run_id = ? ORDER BY generation_hex`).all(runId),
      hallOfFame: database.prepare(`SELECT generation_hex, record_version, record_blob, weights_sha256,
        genome_sha256, fitness_value, pinned FROM rust_hall_of_fame_v1
        WHERE run_id = ? AND weight_state = 'selected' ORDER BY generation_hex`).all(runId)
    };
  } finally { database.close(); }
}

/** Download the actual selected checkpoint while the source coordinator awaits its durable reply. */
async function archive(server: RustServer, boundary: HeldBoundary): Promise<Buffer> {
  const response = await fetch(`http://127.0.0.1:${server.port}/api/export/latest`, { signal: AbortSignal.timeout(5000) });
  expect(response.status, response.status === 200 ? undefined : await response.text()).toBe(200);
  expect(response.headers.get('x-slither-checkpoint-id')).toBe(boundary.descriptor.logicalRootSha256);
  const bytes = Buffer.from(await response.arrayBuffer());
  expect(bytes.byteLength).toBeLessThan(4 * 1024 * 1024);
  return bytes;
}

describeNetworkSuite('Rust archive exact continuation', () => {
  it.each([{ generation: 2, workers: 1 }, { generation: 4, workers: 4 },
    { generation: 4, workers: 5 }, { generation: 4, workers: 6 }])(
    'matches direct successors from generation $generation through import and restart with $workers workers', async scenario => {
      const root = await mkdtemp(join(tmpdir(), 'slither-rust-archive-continuation-'));
      const sourcePath = join(root, 'direct.sqlite');
      const targetPath = join(root, 'imported.sqlite');
      const servers: RustServer[] = [];
      let viewer: WebSocket | undefined;
      const sourceClients = new Set<CheckpointPersistenceClient>();
      let selected: HeldBoundary | undefined;
      let directSuccessor: HeldBoundary | undefined;
      let restoredSuccessor: HeldBoundary | undefined;
      let directAfterRestart: HeldBoundary | undefined;
      let restoredAfterRestart: HeldBoundary | undefined;
      const held: HeldBoundary[] = [];
      const originalCommit = CheckpointPersistenceClient.prototype.commit;
      /** Commit through the real SQLite worker before withholding any response. */
      const commits = vi.spyOn(CheckpointPersistenceClient.prototype, 'commit').mockImplementation(async function(
        this: CheckpointPersistenceClient, ...args: Parameters<CheckpointPersistenceClient['commit']>
      ) {
        const result = await originalCommit.apply(this, args);
        const descriptor = parseManagedCheckpointDescriptor(args[0]);
        if (descriptor.boundaryKind !== 'generation') return result;
        const generation = Number(BigInt(`0x${descriptor.generation}`));
        if (generation < scenario.generation || generation > scenario.generation + 2) return result;
        const gate = Promise.withResolvers<void>();
        const boundary: HeldBoundary = { descriptor, release: () => gate.resolve() };
        held.push(boundary);
        if (generation === scenario.generation) {
          expect(sourceClients.size).toBe(0);
          sourceClients.add(this);
          selected = boundary;
        } else if (generation === scenario.generation + 1) {
          if (sourceClients.has(this)) directSuccessor = boundary;
          else restoredSuccessor = boundary;
        } else if (sourceClients.has(this)) directAfterRestart = boundary;
        else restoredAfterRestart = boundary;
        await gate.promise;
        return result;
      });
      try {
        const source = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: sourcePath,
          resume: 'fresh', seed: 0x5a17_c0de, rustCalculationWorkers: 1 });
        servers.push(source);
        expect(source.startupFault).toBeUndefined();
        const messages: Array<Record<string, unknown>> = [];
        viewer = new WebSocket(`ws://127.0.0.1:${source.port}`);
        viewer.on('message', (bytes, binary) => {
          if (!binary && messages.length < 128) messages.push(JSON.parse(bytes.toString()) as Record<string, unknown>);
        });
        await new Promise<void>((done, reject) => { viewer!.once('open', done); viewer!.once('error', reject); });
        viewer.send(JSON.stringify({ type: 'hello', version: 2, clientType: 'ui' }));
        await observed(() => messages.find(message => message['type'] === 'welcome'));
        viewer.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
        viewer.send(JSON.stringify({ type: 'reset', graphSpec: GRAPH, settings: { snakeCount: 12, simSpeed: 12 },
          updates: [{ path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 2 },
            { path: 'baselineBots.respawnDelay', value: 1 }, { path: 'pelletCountTarget', value: 100 }] }));
        const reset = await observed(() => {
          const rejected = messages.find(message => message['type'] === 'error');
          expect(rejected, JSON.stringify(messages)).toBeUndefined();
          return messages.find(message => message['type'] === 'stateReplaced');
        });
        expect(reset['welcome']).toMatchObject({ graphSpec: GRAPH,
          settings: { core: { snakeCount: 12, simSpeed: 12 }, updates: expect.arrayContaining([
            { path: 'generationSeconds', value: 8 }, { path: 'baselineBots.count', value: 2 }
          ]) } });
        const original = await observed(() => selected);
        const healthResponse = await fetch(`http://127.0.0.1:${source.port}/api/health`, {
          signal: AbortSignal.timeout(5000)
        });
        expect(healthResponse.status).toBe(200);
        const terminalHealth = await healthResponse.json() as Record<string, unknown>;
        /** Decode diagnostic counters while the real durable reply holds this terminal boundary. */
        const terminalCounter = (field: string): bigint => {
          const value = terminalHealth[field];
          expect(value).toMatch(/^[0-9a-f]{16}$/);
          return BigInt(`0x${value as string}`);
        };
        expect(terminalCounter('terminalStepSamples')).toBeGreaterThanOrEqual(1n);
        const terminalTotal = terminalCounter('terminalStepTotalMicros');
        expect(terminalTotal).toBeGreaterThanOrEqual(terminalCounter('terminalStepMaxMicros'));
        expect(terminalCounter('terminalStepEvolutionMicros')).toBeLessThanOrEqual(
          terminalCounter('terminalStepPreparationMicros')
        );
        expect(terminalCounter('terminalStepEvolutionSummaryMicros') +
          terminalCounter('terminalStepEvolutionReproductionMicros')).toBeLessThanOrEqual(
          terminalCounter('terminalStepEvolutionMicros')
        );
        expect(['Control', 'World', 'Preparation', 'Admission', 'Other'].reduce(
          (sum, phase) => sum + terminalCounter(`terminalStep${phase}Micros`), 0n
        )).toBe(terminalTotal);
        expect(original.descriptor.recurrentStateCount).not.toBe('0000000000000000');
        const exported = await archive(source, original);
        const target = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: targetPath,
          resume: 'fresh', seed: 99, rustCalculationWorkers: scenario.workers });
        servers.push(target);
        expect(target.startupFault).toBeUndefined();
        const imported = await fetch(`http://127.0.0.1:${target.port}/api/import/archive`, {
          method: 'POST', body: new Uint8Array(exported), signal: AbortSignal.timeout(5000) });
        expect(imported.status, imported.status === 200 ? undefined : await imported.text()).toBe(200);
        expect(await imported.json()).toMatchObject({ ok: true, runId: original.descriptor.runId,
          generation: original.descriptor.generation, completedStep: original.descriptor.completedStep,
          checkpointId: original.descriptor.logicalRootSha256 });
        const restored = await observed(() => restoredSuccessor);
        original.release();
        const direct = await observed(() => directSuccessor);
        expect(restored.descriptor.logicalRootSha256).toBe(direct.descriptor.logicalRootSha256);
        expect(restored.descriptor.completedStep).toBe(direct.descriptor.completedStep);
        expect(restored.descriptor.graphLayoutSha256).toBe(direct.descriptor.graphLayoutSha256);
        const sourceRecords = records(sourcePath, original.descriptor.runId);
        expect(sourceRecords.history).toHaveLength(scenario.generation);
        expect(sourceRecords.hallOfFame.length).toBeGreaterThan(0);
        expect(records(targetPath, original.descriptor.runId)).toEqual(sourceRecords);
        const directArchive = await archive(source, direct);
        const restoredArchive = await archive(target, restored);
        expect(restoredArchive).toEqual(directArchive);

        restored.release();
        await target.close();
        servers.pop();
        const restarted = await startRustServer({ ...DEFAULT_CONFIG, port: 0, dbPath: targetPath,
          resume: 'latest', rustCalculationWorkers: scenario.workers });
        servers.push(restarted);
        expect(restarted.startupFault).toBeUndefined();
        const afterRestart = await observed(() => restoredAfterRestart);
        direct.release();
        const uninterrupted = await observed(() => directAfterRestart);
        expect(afterRestart.descriptor.logicalRootSha256).toBe(uninterrupted.descriptor.logicalRootSha256);
        expect(afterRestart.descriptor.completedStep).toBe(uninterrupted.descriptor.completedStep);
        expect(records(targetPath, original.descriptor.runId)).toEqual(records(sourcePath, original.descriptor.runId));
        expect(await archive(restarted, afterRestart)).toEqual(await archive(source, uninterrupted));
      } finally {
        for (const boundary of held) boundary.release();
        commits.mockRestore();
        viewer?.terminate();
        for (const server of servers.reverse()) await server.close();
        await rm(root, { recursive: true, force: true });
      }
    }, 30_000
  );
});
