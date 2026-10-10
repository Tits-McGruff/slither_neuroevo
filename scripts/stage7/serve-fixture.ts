/** Serve one existing disposable database for real-browser Stage 7 acceptance. */

import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { DEFAULT_CONFIG, normalizeConfig } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';

/** Parse a fixed QA port and an existing managed checkpoint database. */
function options(argv: readonly string[]): { databasePath: string; port: number } {
  if (argv.length !== 4 || argv[0] !== '--db-path' || argv[2] !== '--port' ||
      !argv[1] || !/^[1-9][0-9]*$/u.test(argv[3] ?? '')) {
    throw new Error('usage: --db-path EXISTING_DB --port PORT');
  }
  const databasePath = resolve(argv[1]);
  const port = Number(argv[3]);
  if (!existsSync(databasePath) || !existsSync(`${databasePath}.checkpoints`)) {
    throw new Error('fixture database and managed directory must exist');
  }
  if (!Number.isSafeInteger(port) || port < 1024 || port > 65535) {
    throw new RangeError('QA port must be from 1024 to 65535');
  }
  return { databasePath, port };
}

/** Start only the required Rust production path and stop cleanly on a signal. */
export async function serve(databasePath: string, port: number): Promise<void> {
  const server = await startRustServer({ ...normalizeConfig(DEFAULT_CONFIG), host: '0.0.0.0', port,
    dbPath: databasePath, resume: 'latest', logLevel: 'error' });
  if (server.startupFault) {
    await server.close();
    throw new Error(`fixture startup failed: ${server.startupFault}`);
  }
  process.stdout.write(`fixture UI http://127.0.0.1:${server.port}/ (Rust WebSocket port ${server.port})\n`);
  /** Stop one test authority without leaving its database open. */
  const stop = (): void => { void server.close(); };
  process.once('SIGINT', stop);
  process.once('SIGTERM', stop);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const parsed = options(process.argv.slice(2));
  void serve(parsed.databasePath, parsed.port).catch(error => {
    process.stderr.write(`${String(error)}\n`);
    process.exitCode = 1;
  });
}
