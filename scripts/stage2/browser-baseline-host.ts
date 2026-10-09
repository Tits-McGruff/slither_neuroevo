/** Host a finite Stage 2 browser fixture without detached shell processes. */

import { createReadStream, existsSync, statSync } from 'node:fs';
import { createServer, type ServerResponse } from 'node:http';
import path from 'node:path';
import { DEFAULT_CONFIG, type ServerConfig } from '../../server/config.ts';
import { startServer } from '../../server/index.ts';

/** Browser-fixture host options. */
interface BrowserHostOptions {
  /** Existing disposable current-format database. */
  databasePath: string;
  /** Simulation HTTP and WebSocket port. */
  serverPort: number;
  /** Static browser port. */
  uiPort: number;
  /** Display-frame publication rate used by the isolated browser fixture. */
  uiFrameRateHz: number;
  /** Maximum lifetime before automatic cleanup. */
  durationMs: number;
}

/** Small content-type map for current built browser assets. */
const CONTENT_TYPES: Readonly<Record<string, string>> = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.ico': 'image/x-icon',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml'
};

/**
 * Parse a positive bounded integer.
 * @param value - CLI text.
 * @param name - Option name for errors.
 * @param maximum - Inclusive maximum.
 * @returns Validated integer.
 */
function parsePositiveInteger(value: string | undefined, name: string, maximum: number): number {
  const parsed = Number.parseInt(value ?? '', 10);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > maximum) {
    throw new Error(`${name} must be an integer from 1 to ${maximum}`);
  }
  return parsed;
}

/**
 * Parse browser host arguments.
 * @param argv - Arguments after the script path.
 * @returns Validated host options.
 */
function parseOptions(argv: readonly string[]): BrowserHostOptions {
  let databasePath: string | null = null;
  let serverPort = 5174;
  let uiPort = 5173;
  let uiFrameRateHz = 30;
  let durationMs = 10 * 60 * 1000;
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!value) throw new Error(`${option ?? '<missing>'} requires a value`);
    switch (option) {
      case '--db':
        databasePath = path.resolve(value);
        break;
      case '--server-port':
        serverPort = parsePositiveInteger(value, '--server-port', 65_535);
        break;
      case '--ui-port':
        uiPort = parsePositiveInteger(value, '--ui-port', 65_535);
        break;
      case '--ui-rate':
        uiFrameRateHz = parsePositiveInteger(value, '--ui-rate', 240);
        break;
      case '--duration-ms':
        durationMs = parsePositiveInteger(value, '--duration-ms', 60 * 60 * 1000);
        break;
      default:
        throw new Error(`Unknown option ${option}`);
    }
    index++;
  }
  if (!databasePath) throw new Error('--db is required');
  if (!existsSync(databasePath)) throw new Error(`Database does not exist: ${databasePath}`);
  if (serverPort === uiPort) throw new Error('server and UI ports must differ');
  return { databasePath, serverPort, uiPort, uiFrameRateHz, durationMs };
}

/**
 * End an HTTP response with a small plain-text error.
 * @param response - Response to complete.
 * @param statusCode - HTTP status.
 * @param message - Public message.
 */
function sendError(response: ServerResponse, statusCode: number, message: string): void {
  response.statusCode = statusCode;
  response.setHeader('Content-Type', 'text/plain; charset=utf-8');
  response.end(`${message}\n`);
}

/**
 * Resolve a request path beneath the built-browser root.
 * @param root - Absolute dist directory.
 * @param requestUrl - Raw request URL.
 * @returns Safe existing file, or null.
 */
function resolveBuiltAsset(root: string, requestUrl: string): string | null {
  let pathname: string;
  try {
    pathname = decodeURIComponent(new URL(requestUrl, 'http://fixture.invalid').pathname);
  } catch {
    return null;
  }
  const relative = pathname === '/' ? 'index.html' : pathname.replace(/^\/+/, '');
  const candidate = path.resolve(root, relative);
  if (candidate !== root && !candidate.startsWith(`${root}${path.sep}`)) return null;
  try {
    return statSync(candidate).isFile() ? candidate : null;
  } catch {
    return null;
  }
}

/**
 * Start the static built-browser server.
 * @param port - Loopback port.
 * @returns Listening HTTP server.
 */
async function startStaticServer(port: number): Promise<ReturnType<typeof createServer>> {
  const root = path.resolve('dist');
  if (!existsSync(path.join(root, 'index.html'))) {
    throw new Error('dist/index.html is missing; run the normal Vite build first');
  }
  const server = createServer((request, response) => {
    response.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    response.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    response.setHeader('Cache-Control', 'no-store');
    const asset = resolveBuiltAsset(root, request.url ?? '/');
    if (!asset) {
      sendError(response, 404, 'Not found');
      return;
    }
    response.statusCode = 200;
    response.setHeader(
      'Content-Type',
      CONTENT_TYPES[path.extname(asset).toLowerCase()] ?? 'application/octet-stream'
    );
    const stream = createReadStream(asset);
    stream.once('error', error => {
      if (!response.headersSent) sendError(response, 500, error.message);
      else response.destroy(error);
    });
    stream.pipe(response);
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', () => {
      server.off('error', reject);
      resolve();
    });
  });
  return server;
}

/**
 * Close an HTTP server if it is listening.
 * @param server - Server to close.
 */
async function closeHttpServer(server: ReturnType<typeof createServer>): Promise<void> {
  if (!server.listening) return;
  await new Promise<void>((resolve, reject) => {
    server.close(error => {
      if (error) reject(error);
      else resolve();
    });
  });
}

/** Run the finite browser fixture. */
async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  const config: ServerConfig = {
    ...DEFAULT_CONFIG,
    host: '127.0.0.1',
    port: options.serverPort,
    uiHost: '127.0.0.1',
    uiPort: options.uiPort,
    publicWsUrl: `ws://127.0.0.1:${options.serverPort}`,
    uiFrameRateHz: options.uiFrameRateHz,
    dbPath: options.databasePath,
    checkpointEveryGenerations: 1_000_000,
    logLevel: 'warn',
    inferenceBackend: 'native',
    resume: 'latest'
  };
  const simulation = await startServer(config);
  let ui: Awaited<ReturnType<typeof startStaticServer>> | null = null;
  let closing = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const close = async (): Promise<void> => {
    if (closing) return;
    closing = true;
    if (timer) clearTimeout(timer);
    if (ui) await closeHttpServer(ui);
    await simulation.close();
  };
  const terminate = (): void => {
    void close().then(() => process.exit(0), error => {
      console.error(error);
      process.exit(1);
    });
  };
  process.once('SIGINT', terminate);
  process.once('SIGTERM', terminate);
  try {
    ui = await startStaticServer(options.uiPort);
    timer = setTimeout(terminate, options.durationMs);
    console.info(JSON.stringify({
      type: 'stage2-browser-host-ready',
      uiUrl: `http://127.0.0.1:${options.uiPort}/`,
      serverUrl: `http://127.0.0.1:${options.serverPort}/`,
      databasePath: options.databasePath,
      uiFrameRateHz: options.uiFrameRateHz,
      durationMs: options.durationMs
    }));
  } catch (error) {
    await close();
    throw error;
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
