import { createServer, type Server } from 'node:http';
import { pathToFileURL } from 'node:url';
import { resetCFGToDefaults } from '../src/config.ts';
import { World } from '../src/world.ts';
import { WorldSerializer } from '../src/serializer.ts';
import { normalizeSeed } from '../src/rng.ts';
import {
  getNativeAddonBuildIdentifier,
  getSimdKernelStatus,
  prepareInferenceBackend
} from '../src/brains/nativeBridge.ts';
import { parseConfig, type ServerConfig } from './config.ts';
import { createHttpHandler } from './httpApi.ts';
import { createLogger } from './logger.ts';
import { createPersistence, initDb } from './persistence.ts';
import { PROTOCOL_VERSION, SERIALIZER_VERSION, type WelcomeMsg } from './protocol.ts';
import { SimServer, applySettingsUpdates, coerceCoreSettings } from './simServer.ts';
import { WsHub } from './wsHub.ts';
import type { Logger } from './logger.ts';
import { buildSensorSpec } from './sensorSpec.ts';
import { createEntropySeed, createRunId, createSessionId } from './runIdentity.ts';
import { buildAuthoritativeConfigHash } from './configIdentity.ts';
import { buildCoreSettingsSnapshot, buildSettingsUpdatesSnapshot } from './settingsSnapshot.ts';

/** Minimal server handle returned by `startServer` for lifecycle management. */
export interface RunningServer {
  port: number;
  wsUrl: string;
  close: () => Promise<void>;
}

/**
 * Close a listening HTTP server and wait for its close callback.
 * @param server - Node HTTP server to close.
 */
async function closeHttpServer(server: Server): Promise<void> {
  if (!server.listening) return;
  await new Promise<void>((resolve, reject) => {
    server.close((error) => {
      if (error) reject(error);
      else resolve();
    });
  });
}

/**
 * Start the HTTP/WS simulation server.
 * @param config - Normalized server configuration.
 * @returns Running server handle with close method.
 */
export async function startServer(config: ServerConfig, logger?: Logger): Promise<RunningServer> {
  resetCFGToDefaults();
  const worldSeed = Number.isFinite(config.seed)
    ? normalizeSeed(config.seed as number)
    : createEntropySeed();
  const sessionId = createSessionId();
  const runId = createRunId();
  await prepareInferenceBackend(config.inferenceBackend);
  const db = initDb(config.dbPath);
  const persistence = createPersistence(db);
  const latestSnapshot = persistence.loadLatestSnapshot();
  const initialSettings = latestSnapshot?.settings
    ? coerceCoreSettings(latestSnapshot.settings)
    : {};
  if (latestSnapshot?.updates) {
    applySettingsUpdates(latestSnapshot.updates);
  }
  const sensorSpec = buildSensorSpec();
  const sampleWorld = new World(initialSettings, {
    seed: worldSeed,
    inferenceBackend: config.inferenceBackend
  });
  const cfgHash = buildAuthoritativeConfigHash(sampleWorld);
  const sampleGraphKey = typeof sampleWorld.archKey === 'string'
    ? sampleWorld.archKey
    : 'initializing';
  const sampleParameterCount = sampleWorld.population?.[0]?.weights.length ?? 0;
  const frameByteLength = WorldSerializer.serialize(sampleWorld).byteLength;
  const welcome: WelcomeMsg = {
    type: 'welcome',
    protocolVersion: PROTOCOL_VERSION,
    sessionId,
    tickRate: config.tickRateHz,
    worldSeed,
    runId,
    configRevision: 0,
    configHash: cfgHash,
    settings: {
      core: buildCoreSettingsSnapshot(sampleWorld),
      updates: buildSettingsUpdatesSnapshot()
    },
    inferenceMode: {
      requestedBackend: config.inferenceBackend,
      activeBackend: config.inferenceBackend,
      requestedMt: config.mtEnabled,
      activeWorkerCount: 0,
      poolEpoch: null,
      weightEpoch: null,
      graphKey: sampleGraphKey,
      parameterCount: sampleParameterCount,
      seed: worldSeed,
      nativeAddonStatus: getSimdKernelStatus(),
      nativeAddonBuildIdentifier: getNativeAddonBuildIdentifier()
    },
    sensorSpec,
    serializerVersion: SERIALIZER_VERSION,
    frameByteLength
  };

  let simServer: SimServer | null = null;
  let wsHub: WsHub | null = null;

  const httpHandler = createHttpHandler({
    getStatus: () => {
      if (!simServer) throw new Error('simulation server not ready');
      return {
        tick: simServer.getTickId(),
        clients: wsHub?.getClientCount() ?? 0,
        inferenceMode: simServer.getInferenceMode(),
        scheduler: simServer.getSchedulerDiagnostics(),
        fault: simServer.getFaultStatus(),
        run: simServer.getRunIdentity(),
        ...simServer.getConfigState()
      };
    },
    getWorld: () => simServer?.getWorld() ?? null,
    importPopulation: async (data) =>
      simServer
        ? simServer.importPopulation(data)
        : { ok: false, reason: 'world not ready' },
    persistence,
    getConfigHash: () => simServer?.getConfigHash() ?? cfgHash,
    getWorldSeed: () => simServer?.getRunIdentity().seed ?? worldSeed,
    logger
  });

  const httpServer = createServer((req, res) => {
    res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
    res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
    httpHandler(req, res);
  });

  let closed = false;
  /** Close every resource exactly once, preserving the first cleanup error. */
  const close = async (): Promise<void> => {
    if (closed) return;
    closed = true;
    let firstError: unknown = null;
    try {
      await simServer?.stop();
    } catch (error) {
      firstError = error;
    }
    try {
      wsHub?.closeAll();
    } catch (error) {
      firstError ??= error;
    }
    try {
      await closeHttpServer(httpServer);
    } catch (error) {
      firstError ??= error;
    }
    try {
      db.close();
    } catch (error) {
      firstError ??= error;
    }
    if (firstError) throw firstError;
  };

  try {
    await new Promise<void>((resolve, reject) => {
      const onError = (err: Error) => {
        httpServer.off('error', onError);
        reject(err);
      };
      httpServer.once('error', onError);
      try {
        httpServer.listen({ port: config.port, host: config.host }, () => {
          httpServer.off('error', onError);
          resolve();
        });
      } catch (err) {
        onError(err as Error);
      }
    });
    // Attach WebSocket and simulation resources only after the HTTP bind has
    // succeeded. The ws package forwards HTTP bind errors through its own
    // EventEmitter, which would otherwise create a second uncaught error path.
    wsHub = new WsHub(httpServer, welcome);
    simServer = new SimServer(
      config,
      wsHub,
      persistence,
      cfgHash,
      worldSeed,
      initialSettings,
      runId
    );
    wsHub.setHandlers({
      onJoin: (connId, msg, clientType) =>
        simServer?.handleJoin(connId, msg.mode, clientType, msg.name),
      onAction: (connId, msg) => simServer?.handleAction(connId, msg),
      onView: (connId, msg) => simServer?.handleView(connId, msg),
      onViz: (connId, msg) => simServer?.handleViz(connId, msg),
      onReset: (connId, msg) => simServer?.handleReset(connId, msg),
      onSettings: (connId, msg) => simServer?.handleSettings(connId, msg),
      onGodMode: (connId, msg) => simServer?.handleGodMode(connId, msg),
      onNewRun: (connId, msg) => simServer?.handleNewRun(connId, msg),
      onDisconnect: (connId) => simServer?.handleDisconnect(connId)
    });
    await simServer.start();
  } catch (error) {
    try {
      await close();
    } catch (cleanupError) {
      logger?.error('server.cleanup', String(cleanupError));
    }
    throw error;
  }

  const address = httpServer.address();
  const port = typeof address === 'object' && address ? address.port : config.port;

  logger?.info('inference-mode', JSON.stringify(simServer.getInferenceMode()));

  const wsHost =
    config.host === '0.0.0.0' || config.host === '::' ? 'localhost' : config.host;
  return {
    port,
    wsUrl: `ws://${wsHost}:${port}`,
    close
  };
}

/**
 * CLI entry point for the simulation server.
 */
export async function main(): Promise<void> {
  const config = parseConfig(process.argv.slice(2), process.env);
  const logger = createLogger(config.logLevel);
  const server = await startServer(config, logger);
  logger.info('server', `listening on :${server.port}`);

  let closing = false;
  const shutdown = async () => {
    if (closing) return;
    closing = true;
    logger.info('server', 'shutting down');
    await server.close();
    process.exit(0);
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
