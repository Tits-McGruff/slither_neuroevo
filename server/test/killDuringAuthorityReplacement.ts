/** Child process used only to kill a real Rust server at a replacement durability boundary. */
import { writeSync } from 'node:fs';
import { DEFAULT_CONFIG } from '../config.ts';
import { startRustServer } from '../rustServer.ts';
import { BackgroundOutputPump } from '../rustEngine/backgroundOutput.ts';
import { CheckpointPersistenceClient } from '../rustEngine/checkpointPersistenceClient.ts';

/** Loopback listener used by this disposable child. */
const port = Number(process.argv[2]);
/** Database owned by the parent test and removed after the child exits. */
const dbPath = process.argv[3];
/** Exact point at which the parent requests process death. */
const point = process.argv[4];
if (!Number.isInteger(port) || port <= 0 || port > 65535 || !dbPath ||
    (point !== 'afterCommit' && point !== 'afterSwap')) {
  throw new Error('expected loopback port, disposable database path and replacement crash point');
}

await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port, dbPath,
  resume: 'fresh', seed: 42, rustCalculationWorkers: 1 });

/** Record which real boundary was reached before killing the process without shutdown. */
function killAtBoundary(evidence: Record<string, unknown>): never {
  writeSync(1, `${JSON.stringify({ type: 'replacementCrashPoint', point, ...evidence })}\n`);
  process.kill(process.pid, 'SIGKILL');
  throw new Error('SIGKILL unexpectedly returned');
}

/** Real SQLite operation, patched only after initial startup has committed. */
const originalCommit = CheckpointPersistenceClient.prototype.commit;
/** Real import transaction, including the imported current pointer and inventory. */
const originalImportCommit = CheckpointPersistenceClient.prototype.commitImport;
/** Real Rust publication, resolving only after its matching importPublished event. */
const originalPublish = BackgroundOutputPump.prototype.publishPreparedImport;
if (point === 'afterCommit') {
  CheckpointPersistenceClient.prototype.commit = async function (
    this: CheckpointPersistenceClient,
    ...args: Parameters<CheckpointPersistenceClient['commit']>
  ) {
    const result = await originalCommit.apply(this, args);
    if (args[2] === true) killAtBoundary({ runId: result.descriptor.runId,
      checkpointId: result.checkpointId });
    return result;
  };
  CheckpointPersistenceClient.prototype.commitImport = async function (
    this: CheckpointPersistenceClient,
    ...args: Parameters<CheckpointPersistenceClient['commitImport']>
  ) {
    const result = await originalImportCommit.apply(this, args);
    killAtBoundary({ runId: result.descriptor.runId, checkpointId: result.checkpointId });
  };
} else {
  BackgroundOutputPump.prototype.publishPreparedImport = async function (
    this: BackgroundOutputPump,
    ...args: Parameters<BackgroundOutputPump['publishPreparedImport']>
  ) {
    const publication = await originalPublish.apply(this, args);
    killAtBoundary({ runId: args[0].runId, checkpointId: args[0].logicalRootSha256,
      publication });
  };
}
