/** Kill a real server after archive publication or during its first body write. */
import { existsSync, statSync, writeSync } from 'node:fs';
import { ServerResponse } from 'node:http';
import { join } from 'node:path';
import { ExperimentalRunningAuthority } from '../../native/index.js';
import { DEFAULT_CONFIG } from '../config.ts';
import { startRustServer } from '../rustServer.ts';

/** Loopback listener owned by the disposable child. */
const port = Number(process.argv[2]);
/** Parent-owned database removed after all child processes exit. */
const dbPath = process.argv[3];
/** Exact export boundary at which to stop without cleanup. */
const point = process.argv[4];
if (!Number.isInteger(port) || port <= 0 || port > 65535 || !dbPath ||
    (point !== 'afterReady' && point !== 'duringDownload' && point !== 'duringEncoding')) {
  throw new Error('expected loopback port, disposable database path and export crash point');
}

await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1', port, dbPath,
  resume: 'fresh', seed: 42, rustCalculationWorkers: 1 });

/** Exact ready archive returned by the actual Rust encoder. */
let ready: { checkpointId: string; relativeFilename: string; storedByteCount: string } | undefined;
/** Actual worker operation; the fixture never replaces the archive encoder. */
const originalPrepare = ExperimentalRunningAuthority.prototype.prepareExportArchive;
/** Actual socket write used only after the ready-file checks pass. */
const originalWrite = ServerResponse.prototype.write;

/** Record the real published archive and stop without releasing its lease or file. */
function killAtBoundary(offeredBytes = 0): never {
  if (!ready) throw new Error('download began before its recorded Rust archive publication');
  writeSync(1, `${JSON.stringify({ type: 'exportCrashPoint', point, ...ready, offeredBytes })}\n`);
  process.kill(process.pid, 'SIGKILL');
  throw new Error('SIGKILL unexpectedly returned');
}

ExperimentalRunningAuthority.prototype.prepareExportArchive = async function (
  this: ExperimentalRunningAuthority,
  ...args: Parameters<ExperimentalRunningAuthority['prepareExportArchive']>
) {
  if (point === 'duringEncoding') {
    const checkpoint = args[2] as { logicalRootSha256: string };
    const filename = `.${args[1]}.slither-save.partial`;
    const partial = join(args[0], filename);
    const pending = originalPrepare.apply(this, args);
    const watch = setInterval(() => {
      const progress = this.archiveWorkProgress();
      if (!progress || progress.operationId !== args[1] || !progress.started || progress.finished ||
          BigInt(`0x${progress.completedBytes}`) === 0n || !existsSync(partial)) return;
      const bytes = statSync(partial).size;
      if (bytes === 0) return;
      writeSync(1, `${JSON.stringify({ type: 'exportCrashPoint', point,
        checkpointId: checkpoint.logicalRootSha256, relativeFilename: filename,
        storedByteCount: BigInt(bytes).toString(16).padStart(16, '0'), offeredBytes: 0,
        progress })}\n`);
      process.kill(process.pid, 'SIGKILL');
      throw new Error('SIGKILL unexpectedly returned');
    }, 1);
    try {
      await pending;
      throw new Error('encoder completed before the fixture observed a nonempty unfinished archive');
    } finally { clearInterval(watch); }
  }
  const result = await originalPrepare.apply(this, args);
  ready = result as NonNullable<typeof ready>;
  if (point === 'afterReady') killAtBoundary();
  return result;
};

if (point === 'duringDownload') {
  ServerResponse.prototype.write = function (
    this: ServerResponse,
    ...args: Parameters<ServerResponse['write']>
  ) {
    const accepted = originalWrite.apply(this, args);
    if (this.statusCode === 200 && this.getHeader('Content-Type') === 'application/vnd.slither-neuroevo.save') {
      const chunk: unknown = args[0];
      if (!(chunk instanceof Uint8Array)) throw new Error('archive stream did not write binary bytes');
      killAtBoundary(chunk.byteLength);
    }
    return accepted;
  } as ServerResponse['write'];
}
