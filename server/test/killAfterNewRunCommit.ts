/** Child process used only to kill a real Rust server after a New Run's SQLite commit. */
import { DEFAULT_CONFIG } from '../config.ts';
import { startRustServer } from '../rustServer.ts';
import { CheckpointPersistenceClient } from '../rustEngine/checkpointPersistenceClient.ts';

const port = Number(process.argv[2]);
const dbPath = process.argv[3];
if (!Number.isInteger(port) || port <= 0 || port > 65535 || !dbPath) {
  throw new Error('expected loopback port and disposable database path');
}

await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port, dbPath,
  resume: 'fresh', seed: 42, rustCalculationWorkers: 1 });

const originalCommit = CheckpointPersistenceClient.prototype.commit;
CheckpointPersistenceClient.prototype.commit = async function (
  this: CheckpointPersistenceClient,
  ...args: Parameters<CheckpointPersistenceClient['commit']>
) {
  const result = await originalCommit.apply(this, args);
  if (args[2] === true) process.kill(process.pid, 'SIGKILL');
  return result;
};
