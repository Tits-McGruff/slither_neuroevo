/** Preload in a disposable server and its persistence worker to kill at a real prune boundary. */
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { basename, resolve } from 'node:path';
import { isMainThread, workerData } from 'node:worker_threads';
import Database from 'better-sqlite3';

if (!isMainThread && process.env['SLITHER_PRUNE_CRASH_POINT']) {
  /** Parent-selected deletion boundary, inherited by this worker only. */
  const point = process.env['SLITHER_PRUNE_CRASH_POINT'];
  if (point !== 'afterIntent' && point !== 'afterDelete') throw new Error('invalid pruning crash point');
  /** Controlled paths already admitted by the production persistence client. */
  const bootstrap = workerData as { databasePath: string; managedRootPath: string };
  /** Real filesystem deletion, delegated at the requested side of the boundary. */
  const originalUnlink = fs.unlinkSync;
  fs.unlinkSync = function (path: fs.PathLike): void {
    const filename = String(path);
    const name = basename(filename);
    if (!/^[0-9a-f]{64}\.checkpoint-v3$/u.test(name) ||
        resolve(filename) !== resolve(bootstrap.managedRootPath, name)) {
      originalUnlink(path);
      return;
    }
    const database = new Database(bootstrap.databasePath, { readonly: true });
    /** Exact durable intent inspected before any injected process death. */
    let intent: { retention_kind: string } | undefined;
    try {
      intent = database.prepare('SELECT retention_kind FROM rust_checkpoint_retention_v1 WHERE checkpoint_id = ?')
        .get(name.slice(0, 64)) as typeof intent;
      if (intent?.retention_kind !== 'pruning') throw new Error('unlink did not follow committed prune intent');
      if (database.prepare('SELECT 1 FROM rust_checkpoint_v3_current WHERE checkpoint_id = ?').get(name.slice(0, 64))) {
        throw new Error('pruning selected a current checkpoint');
      }
    } finally { database.close(); }
    if (point === 'afterDelete') originalUnlink(path);
    fs.writeSync(1, `${JSON.stringify({ type: 'pruningCrashPoint', point,
      checkpointId: name.slice(0, 64), filename, retentionKind: intent!.retention_kind,
      fileExists: fs.existsSync(path) })}\n`);
    process.kill(process.pid, 'SIGKILL');
    throw new Error('SIGKILL unexpectedly returned');
  };
  syncBuiltinESMExports();
}
