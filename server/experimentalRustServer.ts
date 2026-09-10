import type { RustRecoveryNotice } from '../src/protocol/rustBackground.ts';
import type { ExperimentalServerRuntime } from './rustEngine/experimentalStartup.ts';
import { createServer } from 'node:http';
import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import { dirname, extname, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { networkInterfaces } from 'node:os';
import { isIP } from 'node:net';
import { DEFAULT_CONFIG, parseConfig, type ServerConfig } from './config.ts';
import { WsHub } from './wsHub.ts';
import { createExperimentalServerRuntime } from './rustEngine/experimentalStartup.ts';
import { BackgroundOutputPump } from './rustEngine/backgroundOutput.ts';
import { ExternalControllerRouting } from './rustEngine/externalRouting.ts';
import { createRustStats, createRustWelcome } from './rustEngine/browserMetadata.ts';

/** Repository-owned built browser assets. */
const CLIENT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../dist');
/** P0 external routing cap, within the native admitted controller capacity. */
const MAX_CONTROLLERS = 16;
/** Browser asset MIME types emitted by Vite. */
const CONTENT_TYPES: Readonly<Record<string, string>> = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon' };

/** Explicit experimental process ownership returned to tests and the CLI. */
export interface ExperimentalRustServer {
  /** Actual bound port, including an OS-selected test port. */
  port: number;
  /** Explicit health-only startup failure, without an active simulation. */
  startupFault?: string;
  /** Stop sockets, join native execution, then close the metadata worker. */
  close(): Promise<void>;
}

/** Keep bounded diagnostics reachable after failed restore without starting any game or socket authority. */
async function startFaultedServer(config: ServerConfig, error: unknown): Promise<ExperimentalRustServer> {
  const reason = (error instanceof Error ? error.message : String(error)).slice(0, 512);
  const server = createServer((_request, response) => {
    response.writeHead(503, { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': '*' });
    response.end(JSON.stringify({ ok: false, authority: 'rust', lifecycle: 'startup-fault', interfaceFault: reason }));
  });
  server.on('upgrade', (_request, socket) => { socket.end('HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n'); });
  await new Promise<void>((done, reject) => { server.once('error', reject); server.listen(config.port, config.host, () => { server.off('error', reject); done(); }); });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('faulted server has no TCP address');
  let closing: Promise<void> | undefined;
  console.error('[rust.startup-fault]', reason);
  return { port: address.port, startupFault: reason, close() {
    closing ??= new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done()));
    return closing;
  } };
}

/** Project durable provenance without exposing native population or metadata-worker internals. */
function recoveryNotice(owner: ExperimentalServerRuntime): RustRecoveryNotice | undefined {
  const recovery = owner.recovery;
  if (!recovery) return undefined;
  const recovered = BigInt(`0x${recovery.recoveredDescriptor.generation}`);
  const through = BigInt(`0x${recovery.abandonedThroughGeneration}`) - 1n;
  return { failedRunId: recovery.sourceRunId, branchRunId: recovery.branchRunId,
    failedCheckpointId: recovery.failedCheckpointId, recoveredCheckpointId: recovery.recoveredDescriptor.logicalRootSha256,
    recoveredGeneration: recovery.recoveredDescriptor.generation,
    lostCompletedGenerations: through >= recovered ? { from: recovery.recoveredDescriptor.generation,
      through: through.toString(16).padStart(16, '0') } : null };
}

/** Start the fixed native P0 profile from fresh or retained managed authority. */
export async function startExperimentalRustServer(config: ServerConfig): Promise<ExperimentalRustServer> {
  if (resolve(config.dbPath) === resolve(DEFAULT_CONFIG.dbPath)) {
    throw new Error('experimental Rust startup requires a dedicated managed --db-path');
  }
  if (config.inferenceBackend !== 'native' || config.mtEnabled || config.controllerInputHoldMs !== 500 ||
      config.controllerDisconnectGraceMs !== 30_000 || config.checkpointEveryGenerations !== 1) {
    throw new Error('experimental Rust startup supports native scalar P0, default controller timing, and every-generation checkpoints');
  }
  let schedule = (): void => {};
  let owner: ExperimentalServerRuntime;
  try {
    if (typeof config.resume === 'number') throw new Error('numeric reference snapshot IDs are not managed checkpoint IDs');
    owner = await createExperimentalServerRuntime({ databasePath: config.dbPath,
      managedDirectory: `${resolve(config.dbPath)}.checkpoints`,
      ...(config.resume === 'latest' ? { restoreLatest: true } : {}),
      ...(config.resume.startsWith('sha256:') ? { restoreCheckpointId: config.resume.slice(7) } : {}),
      ...(config.seed === undefined ? {} : { seed: config.seed }), onWake: () => schedule() });
  } catch (error) { return startFaultedServer(config, error); }
  const recovery = recoveryNotice(owner);
  if (recovery) console.warn('[rust.recovery]', recovery);
  let fault: string | undefined;
  let stopping = false;
  let scheduled: NodeJS.Immediate | undefined;
  let timer: NodeJS.Timeout | undefined;
  let draining: Promise<boolean> | undefined;
  let lastFrame = 0;
  let lastStats = 0;
  let pumpStart = performance.now();
  let pumps = 0;
  let pumpsPerSecond = 0;
  const server = createServer((request, response) => {
    response.setHeader('Access-Control-Allow-Origin', '*');
    response.setHeader('Access-Control-Allow-Methods', 'GET, OPTIONS');
    response.setHeader('Access-Control-Allow-Headers', 'Content-Type');
    if (request.method === 'OPTIONS') { response.writeHead(204); response.end(); return; }
    const pathname = new URL(request.url ?? '/', 'http://localhost').pathname;
    if (pathname === '/api/health' || pathname === '/health') {
      response.writeHead(fault ? 503 : 200, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ ok: !fault, authority: 'rust', runId: owner.metadata.runId,
        seed: owner.metadata.seed, startupCheckpointId: owner.runStart.checkpointId, ...owner.runtime.health(), ...(recovery ? { recovery } : {}), ...(fault ? { interfaceFault: fault } : {}) }));
      return;
    }
    if (request.method !== 'GET' || pathname.startsWith('/api/')) {
      response.writeHead(501, { 'Content-Type': 'application/json' });
      response.end(JSON.stringify({ error: 'not available in the experimental P0 runtime' })); return;
    }
    void (async () => {
      const path = resolve(CLIENT_ROOT, `.${decodeURIComponent(pathname === '/' ? '/index.html' : pathname)}`);
      if (!path.startsWith(`${CLIENT_ROOT}${sep}`) || !(await stat(path)).isFile()) {
        response.writeHead(404); response.end(); return;
      }
      response.writeHead(200, { 'Content-Type': CONTENT_TYPES[extname(path)] ?? 'application/octet-stream' });
      const stream = createReadStream(path);
      stream.on('error', () => response.destroy());
      response.on('close', () => stream.destroy());
      stream.pipe(response);
    })().catch(() => { if (!response.headersSent) response.writeHead(404); response.end(); });
  });
  let hub: WsHub | undefined;
  let closePromise: Promise<void> | undefined;
  /** Join every owner exactly once, including startup failure. */
  const close = (): Promise<void> => {
    closePromise ??= (async () => {
      stopping = true;
      if (scheduled) clearImmediate(scheduled);
      if (timer) clearInterval(timer);
      hub?.closeAll();
      owner.runtime.requestStop();
      await draining?.catch(() => {});
      try { await owner.close(); }
      finally { if (server.listening) await new Promise<void>((done, reject) => server.close(error => error ? reject(error) : done())); }
    })();
    return closePromise;
  };
  try {
    hub = new WsHub(server, { ...createRustWelcome(owner.metadata), ...(recovery ? { recovery } : {}) }, { maxConnections: 64 });
    const sockets = hub;
    let routing!: ExternalControllerRouting;
    const output = new BackgroundOutputPump({
      owner, maxControllers: MAX_CONTROLLERS,
      send: (id, message) => sockets.sendJsonTo(Number(BigInt(`0x${id}`)), message),
      event(event) {
        routing.event(event);
        const now = performance.now();
        if (event.display) {
          sockets.updateWelcome({ frameByteLength: event.display.frameByteLength });
          if (now - lastStats >= 1000 / config.uiFrameRateHz) {
            lastStats = now;
            sockets.broadcastStats(createRustStats(event.display, owner.metadata, pumpsPerSecond));
          }
        }
      },
      hasFrameRecipients: () => sockets.hasFrameRecipients() && performance.now() - lastFrame >= 1000 / config.uiFrameRateHz,
      frame(lease) { lastFrame = performance.now(); sockets.broadcastFrame(lease.bytes, lease.release); }
    });
    routing = new ExternalControllerRouting({ native: owner.runtime, admission: output.admission,
      maxControllers: MAX_CONTROLLERS, maxActionsPerSecond: config.maxActionsPerSecond, maxActionsPerTick: config.maxActionsPerTick,
      send: (connection, message) => sockets.sendJsonTo(connection, message) });
    /** Keep health available after a terminal native/interface failure. */
    const fail = (error: unknown): void => {
      if (!fault) sockets.broadcastError(error instanceof Error ? error.message : String(error));
      fault ??= error instanceof Error ? error.message : String(error);
      owner.runtime.requestStop();
    };
    /** Run one bounded drain without overlapping asynchronous persistence. */
    schedule = (): void => {
      if (scheduled || draining || stopping || fault) return;
      scheduled = setImmediate(() => {
        scheduled = undefined;
        if (stopping || fault) return;
        pumps++;
        const now = performance.now();
        if (now - pumpStart >= 1000) { pumpsPerSecond = pumps * 1000 / (now - pumpStart); pumpStart = now; pumps = 0; }
        draining = output.drain();
        void draining.then(() => routing.flush()).catch(fail).finally(() => { draining = undefined; });
      });
    };
    /** Reject secondary commands explicitly until their planned migration slice. */
    const unsupported = (connection: number): void => { sockets.sendJsonTo(connection, { type: 'error', message: fault ?? 'command unavailable in experimental P0; restart for a fresh run' }); };
    /** Convert unexpected admission failures into a terminal interface fault. */
    const route = (action: () => void): void => { try { action(); } catch (error) { fail(error); } schedule(); };
    sockets.setHandlers({
      onJoin(connection, message, client) { route(() => { if (fault || stopping) unsupported(connection); else routing.join(connection, message, client); }); },
      onAction(connection, message) { route(() => { if (!fault && !stopping) routing.action(connection, message); }); },
      onDisconnect(connection) { route(() => { if (!stopping && !fault) routing.disconnect(connection); }); },
      onReset: unsupported, onSettings: unsupported, onGodMode: unsupported, onNewRun: unsupported,
      onViz: unsupported
    });
    await new Promise<void>((done, reject) => { server.once('error', reject); server.listen(config.port, config.host, () => { server.off('error', reject); done(); }); });
    const address = server.address();
    if (!address || typeof address === 'string') throw new Error('experimental server has no TCP address');
    owner.runtime.start();
    timer = setInterval(schedule, 16);
    schedule();
    return { port: address.port, close };
  } catch (error) { await close(); throw error; }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const config = parseConfig(process.argv.slice(2), process.env);
  void startExperimentalRustServer(config).then(server => {
    const hosts = config.host === '0.0.0.0'
      ? ['127.0.0.1', ...Object.values(networkInterfaces()).flatMap(addresses => addresses?.filter(address => address.family === 'IPv4' && !address.internal).map(address => address.address) ?? [])]
      : [config.host];
    for (const host of new Set(hosts)) {
      const authorityHost = isIP(host) === 6 ? `[${host}]` : host;
      const ws = config.publicWsUrl || `ws://${authorityHost}:${server.port}`;
      if (server.startupFault) console.error(`Rust startup fault: ${server.startupFault}. Health: http://${authorityHost}:${server.port}/api/health`);
      else console.info(`Rust P0: http://${authorityHost}:${server.port}/?server=${encodeURIComponent(ws)} (WebSocket ${ws})`);
    }
    process.once('SIGINT', () => { void server.close(); });
    process.once('SIGTERM', () => { void server.close(); });
  }).catch(error => { console.error(error); process.exitCode = 1; });
}
