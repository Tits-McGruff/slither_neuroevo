/** Kill a real Rust server at the first completed-generation durability boundary. */
import { writeSync } from 'node:fs';
import { DEFAULT_CONFIG } from '../config.ts';
import { startRustServer } from '../rustServer.ts';
import { BackgroundGenerationRouter } from '../rustEngine/backgroundGeneration.ts';
import { CheckpointPersistenceClient, type ManagedCheckpointCommitResult } from '../rustEngine/checkpointPersistenceClient.ts';

/** Loopback listener used by this disposable child. */
const port = Number(process.argv[2]);
/** Parent-owned database, removed after the child exits. */
const dbPath = process.argv[3];
/** Exact handoff point to kill without graceful shutdown. */
const point = process.argv[4];
if (!Number.isInteger(port) || port <= 0 || port > 65535 || !dbPath ||
    (point !== 'afterCommit' && point !== 'afterSwap')) {
  throw new Error('expected loopback port, disposable database path and generation crash point');
}

await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port, dbPath,
  resume: 'fresh', seed: 42, rustCalculationWorkers: 1 });

/** First real generation transaction, retained until the requested crash point. */
let committed: ManagedCheckpointCommitResult | undefined;
/** Real commit implementation; Reset run-start commits must pass through normally. */
const originalCommit = CheckpointPersistenceClient.prototype.commit;
/** Real generation router, validating the exact final Rust publication event. */
const originalHandle = BackgroundGenerationRouter.prototype.handle;

/** Write a synchronous boundary record, then kill without any cleanup callback. */
function killAtBoundary(publication?: unknown): never {
  if (!committed) throw new Error('generation swap occurred without the recorded real SQLite commit');
  writeSync(1, `${JSON.stringify({ type: 'generationCrashPoint', point,
    runId: committed.runId, checkpointId: committed.checkpointId,
    generation: committed.descriptor.generation, completedStep: committed.descriptor.completedStep,
    publication })}\n`);
  process.kill(process.pid, 'SIGKILL');
  throw new Error('SIGKILL unexpectedly returned');
}

CheckpointPersistenceClient.prototype.commit = async function (
  this: CheckpointPersistenceClient,
  ...args: Parameters<CheckpointPersistenceClient['commit']>
) {
  const result = await originalCommit.apply(this, args);
  if (result.descriptor.boundaryKind === 'generation') {
    committed = result;
    if (point === 'afterCommit') killAtBoundary();
  }
  return result;
};

if (point === 'afterSwap') {
  BackgroundGenerationRouter.prototype.handle = async function (
    this: BackgroundGenerationRouter,
    ...args: Parameters<BackgroundGenerationRouter['handle']>
  ) {
    const handled = await originalHandle.apply(this, args);
    if (args[0].kind === 'generationStartPublished') killAtBoundary(args[0].generationStart);
    return handled;
  };
}
