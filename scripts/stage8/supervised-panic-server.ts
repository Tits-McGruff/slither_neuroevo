/** Explicit test-addon service process; never an ordinary production entry point. */
import { existsSync } from 'node:fs';
import { registerHooks } from 'node:module';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { configurePanicFixture, startPreparedPanicRuntime } from '../../server/test/panicRuntime.ts';

/** Dedicated metadata path supplied by the supervisor acceptance runner. */
const databasePath = process.env['SLITHER_PANIC_FIXTURE_DB'];
/** Explicit disposable port; zero is allowed for process-boundary tests. */
const port = Number(process.env['SLITHER_PANIC_FIXTURE_PORT']);
if (!databasePath || !process.env['SLITHER_PANIC_FIXTURE_PORT'] ||
    !Number.isInteger(port) || port < 0 || port > 65535 || port === 5174) {
  throw new Error('set SLITHER_PANIC_FIXTURE_DB and a disposable SLITHER_PANIC_FIXTURE_PORT (never 5174)');
}

/** Exact production router dependency to replace, leaving all other imports real. */
const routerUrl = new URL('../../server/rustServer.ts', import.meta.url).href;
/** Original startup dependency, matched only when imported by the router itself. */
const startupUrl = new URL('../../server/rustEngine/experimentalStartup.ts', import.meta.url).href;
/** Test-only startup preserving the real budget helpers and persistence machinery. */
const fixtureUrl = new URL('../../server/test/supervisionStartup.ts', import.meta.url).href;
registerHooks({
  resolve(specifier, context, nextResolve) {
    const resolved = nextResolve(specifier, context);
    return context.parentURL === routerUrl && resolved.url === startupUrl
      ? { ...resolved, url: fixtureUrl } : resolved;
  }
});

/** Only the first invocation faults; subsequent invocations restore its real checkpoint. */
const fresh = !existsSync(resolve(databasePath));
configurePanicFixture(fresh);
/** Dynamically import after installing the isolated startup replacement. */
const { startRustServer } = await import(pathToFileURL(resolve('server/rustServer.ts')).href);
/** Real HTTP/WebSocket server with its actual native engine and SQLite worker. */
const server = await startRustServer({ ...DEFAULT_CONFIG, host: '127.0.0.1',
  port, dbPath: resolve(databasePath), resume: fresh ? 'fresh' : 'latest',
  ...(fresh ? { seed: 42 } : {}), rustCalculationWorkers: 2 });
if (server.startupFault) {
  await server.close();
  throw new Error(server.startupFault);
}
/** Join once on supervisor stop, also supporting an IPC stop in Windows tests. */
let closing: Promise<void> | undefined;
/** Complete real shutdown before exiting successfully. */
function close(): void {
  closing ??= server.close();
  void closing.then(() => process.exit(0), error => {
    console.error(error);
    process.exit(1);
  });
}
process.once('SIGTERM', close);
process.once('SIGINT', close);
process.on('message', message => { if (message === 'stop') close(); });
startPreparedPanicRuntime();
console.info(JSON.stringify({ fixture: 'supervised-calculation-panic', pid: process.pid,
  port: server.port, fresh }));
