import { createServer } from 'node:http';
import { pathToFileURL } from 'node:url';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import { World } from '../src/world.ts';
import { WorldSerializer } from '../src/serializer.ts';
import { parseConfig, type ServerConfig } from './config.ts';
import { hashConfig } from './hash.ts';
import { createHttpHandler } from './httpApi.ts';
import { createLogger } from './logger.ts';
import { createPersistence, initDb } from './persistence.ts';
import { SERIALIZER_VERSION, type SensorSpec, type WelcomeMsg } from './protocol.ts';
import { SimServer } from './simServer.ts';
import { WsHub } from './wsHub.ts';

export interface RunningServer {
  port: number;
  wsUrl: string;
  close: () => Promise<void>;
}

function buildSensorSpec(): SensorSpec {
  const bins = Math.max(8, Math.floor(CFG.sense?.bubbleBins ?? 12));
  const order: string[] = [
    'heading_sin',
    'heading_cos',
    'size_norm',
    'boost_margin',
    'points_pct'
  ];
  for (let i = 0; i < bins; i++) order.push(`food_${i}`);
  for (let i = 0; i < bins; i++) order.push(`hazard_${i}`);
  for (let i = 0; i < bins; i++) order.push(`wall_${i}`);
  if (order.length !== CFG.brain.inSize) {
    if (order.length > CFG.brain.inSize) {
      order.length = CFG.brain.inSize;
    } else {
      for (let i = order.length; i < CFG.brain.inSize; i++) {
        order.push(`extra_${i}`);
      }
    }
  }
  return { sensorCount: order.length, order };
}

export async function startServer(config: ServerConfig): Promise<RunningServer> {
  resetCFGToDefaults();
  const worldSeed = Number.isFinite(config.seed)
    ? (config.seed as number)
    : Math.floor(Math.random() * 1e9);
  const sessionId = Math.random().toString(36).slice(2, 10);
  const cfgHash = hashConfig(CFG);
  const sensorSpec = buildSensorSpec();
  const sampleWorld = new World({});
  const frameByteLength = WorldSerializer.serialize(sampleWorld).byteLength;
  const welcome: WelcomeMsg = {
    type: 'welcome',
    sessionId,
    tickRate: config.tickRateHz,
    worldSeed,
    cfgHash,
    sensorSpec,
    serializerVersion: SERIALIZER_VERSION,
    frameByteLength
  };

  let simServer: SimServer | null = null;
  let wsHub: WsHub | null = null;
  const db = initDb(config.dbPath);
  const persistence = createPersistence(db);

  const httpHandler = createHttpHandler({
    getStatus: () => ({
      tick: simServer?.getTickId() ?? 0,
      clients: wsHub?.getClientCount() ?? 0
    }),
    getWorld: () => simServer?.getWorld() ?? null,
    importPopulation: (data) =>
      simServer?.importPopulation(data) ?? { ok: false, reason: 'world not ready' },
    persistence,
    cfgHash,
    worldSeed
  });

  const httpServer = createServer((req, res) => {
    httpHandler(req, res);
  });

  wsHub = new WsHub(httpServer, welcome);
  simServer = new SimServer(config, wsHub, persistence, cfgHash, worldSeed);
  wsHub.setHandlers({
    onJoin: (connId, msg, clientType) =>
      simServer?.handleJoin(connId, msg.mode, clientType, msg.name),
    onAction: (connId, msg) => simServer?.handleAction(connId, msg),
    onView: (connId, msg) => simServer?.handleView(connId, msg),
    onViz: (connId, msg) => simServer?.handleViz(connId, msg),
    onDisconnect: (connId) => simServer?.handleDisconnect(connId)
  });

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

  const address = httpServer.address();
  const port = typeof address === 'object' && address ? address.port : config.port;

  simServer.start();

  const close = async () => {
    simServer?.stop();
    wsHub?.closeAll();
    db.close();
    await new Promise<void>((resolve) => httpServer.close(() => resolve()));
  };

  const wsHost =
    config.host === '0.0.0.0' || config.host === '::' ? 'localhost' : config.host;
  return {
    port,
    wsUrl: `ws://${wsHost}:${port}`,
    close
  };
}

export async function main(): Promise<void> {
  const config = parseConfig(process.argv.slice(2), process.env);
  const logger = createLogger(config.logLevel);
  const server = await startServer(config);
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
