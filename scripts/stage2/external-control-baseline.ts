/** Real-server Stage 2 Protocol 2 external-control baseline. */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { monitorEventLoopDelay, performance } from 'node:perf_hooks';
import { spawnSync } from 'node:child_process';
import WebSocket, { type RawData } from 'ws';
import { buildGenerationCheckpoint } from '../../server/checkpoint.ts';
import { DEFAULT_CONFIG, type ServerConfig } from '../../server/config.ts';
import { startServer, type RunningServer } from '../../server/index.ts';
import {
  createPersistence,
  initDb,
  type PopulationCheckpoint
} from '../../server/persistence.ts';
import { resetCFGToDefaults } from '../../src/config.ts';
import { World } from '../../src/world.ts';
import {
  installStage2Scenario,
  STAGE2_WORLD_SEED,
  type Stage2ScenarioName
} from './fixtures.ts';

/** Supported external-control workloads. */
type ExternalScenario = Extract<Stage2ScenarioName, 'P0' | 'P1' | 'P2'>;

/** Parsed benchmark options. */
interface ExternalControlOptions {
  /** Named population/brain workload. */
  scenario: ExternalScenario;
  /** Browser-player periodic action cadence. */
  playerHz: 30 | 60;
  /** Warm-up after all clients are ready. */
  warmupMs: number;
  /** Measured wall duration. */
  durationMs: number;
  /** Canonical Node inference workers. */
  workers: number;
  /** Optional JSON destination. */
  outputPath: string | null;
}

/** Quantile and mean summary. */
interface Distribution {
  /** Sample count. */
  count: number;
  /** Minimum. */
  min: number;
  /** Median. */
  p50: number;
  /** 95th percentile. */
  p95: number;
  /** 99th percentile. */
  p99: number;
  /** Maximum. */
  max: number;
  /** Arithmetic mean. */
  mean: number;
}

/** Mutable controller telemetry collected by one socket. */
interface ControllerTelemetry {
  /** Client label. */
  name: string;
  /** Client capability sent in hello. */
  clientType: 'ui' | 'bot';
  /** Socket-open timestamp. */
  openedAtMs: number;
  /** Assignment timestamp. */
  assignedAtMs: number | null;
  /** Assigned authoritative snake id. */
  snakeId: number | null;
  /** Resume token supplied by assignment. */
  resumeToken: string | null;
  /** Most recent sensor tick. */
  latestSensorTick: number;
  /** Sensor receipt timestamps. */
  sensorTimesMs: number[];
  /** Action send timestamps. */
  actionTimesMs: number[];
  /** Timer intervals for browser-player sends. */
  actionIntervalsMs: number[];
  /** Latest action-to-next-sensor upper bounds. */
  actionToNextSensorMs: number[];
  /** Synchronous observation-to-action dispatch cost for the bot. */
  sensorToActionDispatchMs: number[];
  /** Last sent action timestamp. */
  lastActionAtMs: number | null;
  /** Last action timestamp already correlated to a sensor. */
  lastObservedActionAtMs: number | null;
  /** Socket/protocol errors. */
  errors: string[];
}

/** Mutable viewer telemetry. */
interface ViewerTelemetry {
  /** Socket-open timestamp. */
  openedAtMs: number;
  /** Binary frame receipt timestamps. */
  frameTimesMs: number[];
  /** Binary frame byte lengths. */
  frameBytes: number[];
  /** JSON stats receipt timestamps. */
  statsTimesMs: number[];
  /** Stats tick values. */
  statsTicks: number[];
  /** Stats generation-time values. */
  generationTimes: number[];
  /** Socket/protocol errors. */
  errors: string[];
}

/** Open client and readiness promise. */
interface ControllerHandle {
  /** Live socket. */
  socket: WebSocket;
  /** Mutable telemetry. */
  telemetry: ControllerTelemetry;
  /** Resolves after assignment and first sensor. */
  ready: Promise<void>;
}

/** Open spectator and readiness promise. */
interface ViewerHandle {
  /** Live socket. */
  socket: WebSocket;
  /** Mutable telemetry. */
  telemetry: ViewerTelemetry;
  /** Resolves after first binary frame and stats record. */
  ready: Promise<void>;
}

/** Minimal health response fields consumed by the runner. */
interface HealthSnapshot {
  /** Committed fixed-step id. */
  tick: number;
  /** Connected WebSocket clients. */
  clients: number;
  /** Scheduler counters. */
  scheduler: {
    /** Requested multiplier. */
    requestedMultiplier: number;
    /** Cumulative measured multiplier. */
    achievedMultiplier: number;
    /** Completed steps. */
    completedSteps: number;
    /** Wall seconds. */
    wallSeconds: number;
    /** Simulated seconds. */
    simulatedSeconds: number;
    /** Dropped simulation seconds. */
    droppedSimulationSeconds: number;
  };
  /** Current collision-grid diagnostics. */
  collisionGrid: Record<string, unknown>;
  /** Current outbound diagnostics. */
  outbound: Record<string, unknown>;
  /** Current fault state. */
  fault: { faulted: boolean; reason: string | null; tick: number | null };
  /** Active inference mode. */
  inferenceMode: Record<string, unknown>;
}

/**
 * Parse a bounded integer CLI value.
 * @param value - Text after the flag.
 * @param name - Flag name.
 * @param minimum - Inclusive minimum.
 * @param maximum - Inclusive maximum.
 * @returns Parsed integer.
 */
function parseInteger(
  value: string | undefined,
  name: string,
  minimum: number,
  maximum: number
): number {
  const parsed = Number.parseInt(value ?? '', 10);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${name} must be an integer from ${minimum} to ${maximum}`);
  }
  return parsed;
}

/**
 * Parse command-line options.
 * @param argv - Arguments after the script path.
 * @returns Validated options.
 */
function parseOptions(argv: readonly string[]): ExternalControlOptions {
  const options: ExternalControlOptions = {
    scenario: 'P0',
    playerHz: 30,
    warmupMs: 2_000,
    durationMs: 15_000,
    workers: 0,
    outputPath: null
  };
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--scenario':
        if (value !== 'P0' && value !== 'P1' && value !== 'P2') {
          throw new Error('--scenario must be P0, P1, or P2');
        }
        options.scenario = value;
        index++;
        break;
      case '--player-hz': {
        const parsed = parseInteger(value, option, 30, 60);
        if (parsed !== 30 && parsed !== 60) throw new Error('--player-hz must be 30 or 60');
        options.playerHz = parsed;
        index++;
        break;
      }
      case '--warmup-ms':
        options.warmupMs = parseInteger(value, option, 0, 60_000);
        index++;
        break;
      case '--duration-ms':
        options.durationMs = parseInteger(value, option, 1_000, 600_000);
        index++;
        break;
      case '--workers':
        options.workers = parseInteger(value, option, 0, 8);
        index++;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        options.outputPath = path.resolve(value);
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}`);
    }
  }
  return options;
}

/**
 * Summarize finite numeric samples.
 * @param values - Raw samples.
 * @returns Rounded distribution.
 */
function distribution(values: readonly number[]): Distribution {
  if (values.length === 0) {
    return { count: 0, min: 0, p50: 0, p95: 0, p99: 0, max: 0, mean: 0 };
  }
  const sorted = [...values].sort((left, right) => left - right);
  const percentile = (fraction: number): number => {
    const position = (sorted.length - 1) * fraction;
    const lower = Math.floor(position);
    const upper = Math.ceil(position);
    const lowerValue = sorted[lower]!;
    const upperValue = sorted[upper]!;
    return lowerValue + (upperValue - lowerValue) * (position - lower);
  };
  const round = (value: number): number => Number(value.toFixed(6));
  return {
    count: sorted.length,
    min: round(sorted[0]!),
    p50: round(percentile(0.5)),
    p95: round(percentile(0.95)),
    p99: round(percentile(0.99)),
    max: round(sorted[sorted.length - 1]!),
    mean: round(sorted.reduce((sum, value) => sum + value, 0) / sorted.length)
  };
}

/**
 * Convert timestamps into adjacent intervals.
 * @param timestamps - Monotonic millisecond values.
 * @returns Adjacent differences.
 */
function intervals(timestamps: readonly number[]): number[] {
  const result: number[] = [];
  for (let index = 1; index < timestamps.length; index++) {
    result.push(timestamps[index]! - timestamps[index - 1]!);
  }
  return result;
}

/**
 * Clear controller samples while retaining assignment identity and errors.
 * @param telemetry - Controller telemetry to reset at the measurement boundary.
 */
function clearControllerSamples(telemetry: ControllerTelemetry): void {
  telemetry.sensorTimesMs.length = 0;
  telemetry.actionTimesMs.length = 0;
  telemetry.actionIntervalsMs.length = 0;
  telemetry.actionToNextSensorMs.length = 0;
  telemetry.sensorToActionDispatchMs.length = 0;
  telemetry.lastActionAtMs = null;
  telemetry.lastObservedActionAtMs = null;
}

/**
 * Clear viewer samples at the measurement boundary.
 * @param telemetry - Viewer telemetry to reset.
 */
function clearViewerSamples(telemetry: ViewerTelemetry): void {
  telemetry.frameTimesMs.length = 0;
  telemetry.frameBytes.length = 0;
  telemetry.statsTimesMs.length = 0;
  telemetry.statsTicks.length = 0;
  telemetry.generationTimes.length = 0;
}

/**
 * Read current Git identity without mutating the worktree.
 * @returns Commit and dirty flag.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
  const status = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
}

/**
 * Parse a non-binary Protocol 2 payload.
 * @param data - WebSocket payload.
 * @returns Plain record or null.
 */
function parseJson(data: RawData): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(data.toString()) as unknown;
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

/**
 * Send one action and update client telemetry.
 * @param socket - Open socket.
 * @param telemetry - Controller telemetry.
 * @param turn - Requested turn.
 * @param boost - Requested boost.
 */
function sendAction(
  socket: WebSocket,
  telemetry: ControllerTelemetry,
  turn: number,
  boost: number
): void {
  const snakeId = telemetry.snakeId;
  if (snakeId === null || socket.readyState !== WebSocket.OPEN) return;
  const now = performance.now();
  if (telemetry.lastActionAtMs !== null) {
    telemetry.actionIntervalsMs.push(now - telemetry.lastActionAtMs);
  }
  socket.send(JSON.stringify({
    type: 'action',
    tick: telemetry.latestSensorTick + 1,
    snakeId,
    turn,
    boost
  }));
  telemetry.lastActionAtMs = now;
  telemetry.actionTimesMs.push(now);
}

/**
 * Open one player or bot controller.
 * @param url - Server WebSocket URL.
 * @param clientType - Protocol client type.
 * @param name - Join name.
 * @returns Socket, telemetry and readiness.
 */
function openController(
  url: string,
  clientType: 'ui' | 'bot',
  name: string
): ControllerHandle {
  const socket = new WebSocket(url);
  const telemetry: ControllerTelemetry = {
    name,
    clientType,
    openedAtMs: performance.now(),
    assignedAtMs: null,
    snakeId: null,
    resumeToken: null,
    latestSensorTick: 0,
    sensorTimesMs: [],
    actionTimesMs: [],
    actionIntervalsMs: [],
    actionToNextSensorMs: [],
    sensorToActionDispatchMs: [],
    lastActionAtMs: null,
    lastObservedActionAtMs: null,
    errors: []
  };
  let resolveReady: (() => void) | null = null;
  let rejectReady: ((error: Error) => void) | null = null;
  const ready = new Promise<void>((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
  });
  const maybeReady = (): void => {
    if (telemetry.snakeId !== null && telemetry.sensorTimesMs.length > 0) resolveReady?.();
  };
  socket.on('open', () => {
    socket.send(JSON.stringify({ type: 'hello', clientType, version: 2 }));
    socket.send(JSON.stringify({ type: 'join', mode: 'player', name }));
  });
  socket.on('error', error => {
    telemetry.errors.push(error.message);
    rejectReady?.(error);
  });
  socket.on('message', (data: RawData, isBinary: boolean) => {
    if (isBinary) return;
    const message = parseJson(data);
    if (!message) {
      telemetry.errors.push('invalid JSON message');
      return;
    }
    if (message['type'] === 'error') {
      const text = typeof message['message'] === 'string' ? message['message'] : 'server error';
      telemetry.errors.push(text);
      rejectReady?.(new Error(text));
      return;
    }
    if (message['type'] === 'assign') {
      telemetry.assignedAtMs = performance.now();
      telemetry.snakeId = typeof message['snakeId'] === 'number' ? message['snakeId'] : null;
      telemetry.resumeToken =
        typeof message['resumeToken'] === 'string' ? message['resumeToken'] : null;
      maybeReady();
      return;
    }
    if (message['type'] !== 'sensors') return;
    const now = performance.now();
    telemetry.latestSensorTick =
      typeof message['tick'] === 'number' ? Math.floor(message['tick']) : telemetry.latestSensorTick;
    telemetry.sensorTimesMs.push(now);
    if (
      telemetry.lastActionAtMs !== null &&
      telemetry.lastActionAtMs !== telemetry.lastObservedActionAtMs
    ) {
      telemetry.actionToNextSensorMs.push(now - telemetry.lastActionAtMs);
      telemetry.lastObservedActionAtMs = telemetry.lastActionAtMs;
    }
    if (clientType === 'bot') {
      const started = performance.now();
      const phase = telemetry.sensorTimesMs.length;
      sendAction(socket, telemetry, Math.sin(phase * 0.17), phase % 20 < 5 ? 1 : 0);
      telemetry.sensorToActionDispatchMs.push(performance.now() - started);
    }
    maybeReady();
  });
  return { socket, telemetry, ready };
}

/**
 * Open one full-frame spectator.
 * @param url - Server WebSocket URL.
 * @returns Socket, telemetry and readiness.
 */
function openViewer(url: string): ViewerHandle {
  const socket = new WebSocket(url);
  const telemetry: ViewerTelemetry = {
    openedAtMs: performance.now(),
    frameTimesMs: [],
    frameBytes: [],
    statsTimesMs: [],
    statsTicks: [],
    generationTimes: [],
    errors: []
  };
  let resolveReady: (() => void) | null = null;
  let rejectReady: ((error: Error) => void) | null = null;
  const ready = new Promise<void>((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
  });
  const maybeReady = (): void => {
    if (telemetry.frameTimesMs.length > 0 && telemetry.statsTimesMs.length > 0) resolveReady?.();
  };
  socket.on('open', () => {
    socket.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 2 }));
    socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
  });
  socket.on('error', error => {
    telemetry.errors.push(error.message);
    rejectReady?.(error);
  });
  socket.on('message', (data: RawData, isBinary: boolean) => {
    const now = performance.now();
    if (isBinary) {
      telemetry.frameTimesMs.push(now);
      telemetry.frameBytes.push(data.byteLength);
      maybeReady();
      return;
    }
    const message = parseJson(data);
    if (!message) {
      telemetry.errors.push('invalid JSON message');
      return;
    }
    if (message['type'] === 'error') {
      const text = typeof message['message'] === 'string' ? message['message'] : 'server error';
      telemetry.errors.push(text);
      rejectReady?.(new Error(text));
      return;
    }
    if (message['type'] === 'stats') {
      telemetry.statsTimesMs.push(now);
      if (typeof message['tick'] === 'number') telemetry.statsTicks.push(message['tick']);
      if (typeof message['generationTime'] === 'number') {
        telemetry.generationTimes.push(message['generationTime']);
      }
      maybeReady();
    }
  });
  return { socket, telemetry, ready };
}

/**
 * Add a deadline to a readiness promise.
 * @param promise - Operation to bound.
 * @param timeoutMs - Deadline.
 * @param label - Error label.
 * @returns Original result.
 */
async function withTimeout<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs} ms`)), timeoutMs);
      })
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

/**
 * Close one WebSocket without leaving a long shutdown wait.
 * @param socket - Socket to close.
 */
async function closeSocket(socket: WebSocket): Promise<void> {
  if (socket.readyState === WebSocket.CLOSED) return;
  const closed = new Promise<void>(resolve => socket.once('close', () => resolve()));
  socket.close();
  await Promise.race([closed, new Promise<void>(resolve => setTimeout(resolve, 1_000))]);
  if (socket.readyState !== WebSocket.CLOSED) socket.terminate();
}

/**
 * Create one disposable exact run-start checkpoint.
 * @param databasePath - New SQLite file.
 * @param scenarioName - Named workload.
 * @returns Installed scenario description.
 */
function createFixtureDatabase(
  databasePath: string,
  scenarioName: ExternalScenario
): ReturnType<typeof installStage2Scenario> {
  const scenario = installStage2Scenario(scenarioName);
  let checkpoint: PopulationCheckpoint | null = null;
  new World(scenario.settings, {
    seed: STAGE2_WORLD_SEED,
    runId: `stage2-p5-${scenarioName.toLowerCase()}`,
    onGenerationBoundary: (boundary, candidate) => {
      checkpoint = buildGenerationCheckpoint(candidate, boundary, 0);
    }
  });
  if (!checkpoint) throw new Error('run-start checkpoint was not captured');
  const database = initDb(databasePath);
  try {
    createPersistence(database).saveCheckpoint(checkpoint as PopulationCheckpoint);
    database.pragma('wal_checkpoint(TRUNCATE)');
  } finally {
    database.close();
  }
  return scenario;
}

/**
 * Read one health snapshot.
 * @param server - Running server.
 * @returns Parsed health result.
 */
async function readHealth(server: RunningServer): Promise<HealthSnapshot> {
  const response = await fetch(`http://127.0.0.1:${server.port}/health`);
  if (!response.ok) throw new Error(`health returned HTTP ${response.status}`);
  return await response.json() as HealthSnapshot;
}

/**
 * Run the real-server external-control baseline.
 * @param options - Benchmark options.
 * @returns Evidence object.
 */
async function runExternalControlBaseline(
  options: ExternalControlOptions
): Promise<Record<string, unknown>> {
  const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-p5-'));
  const resolvedTemporaryRoot = path.resolve(temporaryRoot);
  const resolvedSystemTemp = path.resolve(os.tmpdir());
  if (
    path.dirname(resolvedTemporaryRoot) !== resolvedSystemTemp ||
    !path.basename(resolvedTemporaryRoot).startsWith('slither-stage2-p5-')
  ) {
    throw new Error(`Unexpected temporary path: ${resolvedTemporaryRoot}`);
  }
  const databasePath = path.join(temporaryRoot, 'fixture.db');
  let server: RunningServer | null = null;
  const sockets: WebSocket[] = [];
  try {
    const scenario = createFixtureDatabase(databasePath, options.scenario);
    const config: ServerConfig = {
      ...DEFAULT_CONFIG,
      host: '127.0.0.1',
      port: 0,
      dbPath: databasePath,
      checkpointEveryGenerations: 1_000_000,
      logLevel: 'error',
      inferenceBackend: 'native',
      mtEnabled: options.workers > 0,
      mtWorkers: options.workers,
      resume: 'latest'
    };
    server = await startServer(config);
    const player = openController(server.wsUrl, 'ui', 'stage2-browser-player');
    const bot = openController(server.wsUrl, 'bot', 'stage2-protocol2-bot');
    const viewer = openViewer(server.wsUrl);
    sockets.push(player.socket, bot.socket, viewer.socket);
    await withTimeout(
      Promise.all([player.ready, bot.ready, viewer.ready]),
      30_000,
      'external clients'
    );
    if (options.warmupMs > 0) {
      await new Promise(resolve => setTimeout(resolve, options.warmupMs));
    }

    const healthBefore = await readHealth(server);
    clearControllerSamples(player.telemetry);
    clearControllerSamples(bot.telemetry);
    clearViewerSamples(viewer.telemetry);
    const eventLoop = monitorEventLoopDelay({ resolution: 10 });
    eventLoop.enable();
    const cpuBefore = process.cpuUsage();
    const memoryBefore = process.memoryUsage();
    let peakRssBytes = memoryBefore.rss;
    let peakHeapUsedBytes = memoryBefore.heapUsed;
    let peakExternalBytes = memoryBefore.external;
    const memoryTimer = setInterval(() => {
      const memory = process.memoryUsage();
      peakRssBytes = Math.max(peakRssBytes, memory.rss);
      peakHeapUsedBytes = Math.max(peakHeapUsedBytes, memory.heapUsed);
      peakExternalBytes = Math.max(peakExternalBytes, memory.external);
    }, 50);
    let playerSequence = 0;
    const playerTimer = setInterval(() => {
      playerSequence++;
      sendAction(
        player.socket,
        player.telemetry,
        Math.sin(playerSequence * 0.11),
        playerSequence % (options.playerHz * 2) < Math.floor(options.playerHz / 3) ? 1 : 0
      );
    }, 1_000 / options.playerHz);
    const measuredStartedAt = performance.now();
    await new Promise(resolve => setTimeout(resolve, options.durationMs));
    const measuredWallMs = performance.now() - measuredStartedAt;
    clearInterval(playerTimer);
    clearInterval(memoryTimer);
    const healthAfter = await readHealth(server);
    const cpu = process.cpuUsage(cpuBefore);
    const memoryAfter = process.memoryUsage();
    eventLoop.disable();

    const clientErrors = [
      ...player.telemetry.errors.map(error => `player: ${error}`),
      ...bot.telemetry.errors.map(error => `bot: ${error}`),
      ...viewer.telemetry.errors.map(error => `viewer: ${error}`)
    ];
    if (clientErrors.length > 0) throw new Error(clientErrors.join('; '));
    if (healthAfter.fault.faulted) {
      throw new Error(`simulation faulted: ${healthAfter.fault.reason ?? 'unknown'}`);
    }
    const tickDelta = healthAfter.tick - healthBefore.tick;
    const simulatedSeconds = tickDelta / DEFAULT_CONFIG.tickRateHz;
    const generationProgress =
      viewer.telemetry.generationTimes.length > 1
        ? viewer.telemetry.generationTimes.at(-1)! - viewer.telemetry.generationTimes[0]!
        : 0;
    return {
      schema: 'slither-stage2-external-control-baseline',
      version: 1,
      evidenceClass: 'new measured result',
      caveat:
        'Real current server and Protocol 2 wire-compatible bot, but loopback Node clients are not the missing owner trainer project, a browser renderer, another LAN device, or the target Debian VM.',
      source: sourceIdentity(),
      environment: {
        capturedAt: new Date().toISOString(),
        platform: process.platform,
        architecture: process.arch,
        osType: os.type(),
        osRelease: os.release(),
        osVersion: os.version(),
        hostname: os.hostname(),
        locale: Intl.DateTimeFormat().resolvedOptions().locale,
        node: process.version,
        v8: process.versions.v8,
        cpuModel: os.cpus()[0]?.model ?? 'unknown',
        logicalCpuCount: os.cpus().length,
        totalMemoryBytes: os.totalmem()
      },
      workload: {
        scenario,
        seed: STAGE2_WORLD_SEED,
        playerHz: options.playerHz,
        warmupMs: options.warmupMs,
        requestedDurationMs: options.durationMs,
        requestedWorkers: options.workers,
        displayPublicationHz: config.uiFrameRateHz,
        playerClient: 'Protocol 2 UI socket with periodic latest-value action',
        botClient: 'Protocol 2 observation-driven synthetic compatibility client',
        viewerClient: 'Protocol 2 UI spectator receiving complete frame v1'
      },
      result: {
        measuredWallMs: Number(measuredWallMs.toFixed(6)),
        tickDelta,
        simulatedSeconds: Number(simulatedSeconds.toFixed(9)),
        simulatedSecondsPerWallSecond: Number(
          (simulatedSeconds / (measuredWallMs / 1_000)).toFixed(6)
        ),
        viewerGenerationProgressSeconds: Number(generationProgress.toFixed(9)),
        player: {
          assignmentLatencyMs:
            player.telemetry.assignedAtMs === null
              ? null
              : Number((player.telemetry.assignedAtMs - player.telemetry.openedAtMs).toFixed(6)),
          actionCount: player.telemetry.actionTimesMs.length,
          sensorCount: player.telemetry.sensorTimesMs.length,
          actionIntervalMs: distribution(player.telemetry.actionIntervalsMs),
          sensorIntervalMs: distribution(intervals(player.telemetry.sensorTimesMs)),
          actionToNextSensorMs: distribution(player.telemetry.actionToNextSensorMs)
        },
        bot: {
          assignmentLatencyMs:
            bot.telemetry.assignedAtMs === null
              ? null
              : Number((bot.telemetry.assignedAtMs - bot.telemetry.openedAtMs).toFixed(6)),
          actionCount: bot.telemetry.actionTimesMs.length,
          sensorCount: bot.telemetry.sensorTimesMs.length,
          sensorIntervalMs: distribution(intervals(bot.telemetry.sensorTimesMs)),
          actionToNextSensorMs: distribution(bot.telemetry.actionToNextSensorMs),
          sensorToActionDispatchMs: distribution(bot.telemetry.sensorToActionDispatchMs)
        },
        viewer: {
          frameCount: viewer.telemetry.frameTimesMs.length,
          frameIntervalMs: distribution(intervals(viewer.telemetry.frameTimesMs)),
          frameBytes: distribution(viewer.telemetry.frameBytes),
          statsCount: viewer.telemetry.statsTimesMs.length,
          statsIntervalMs: distribution(intervals(viewer.telemetry.statsTimesMs))
        },
        eventLoopDelayMs: {
          p50: Number((eventLoop.percentile(50) / 1_000_000).toFixed(6)),
          p95: Number((eventLoop.percentile(95) / 1_000_000).toFixed(6)),
          p99: Number((eventLoop.percentile(99) / 1_000_000).toFixed(6)),
          max: Number((eventLoop.max / 1_000_000).toFixed(6))
        },
        cpu: {
          userMicros: cpu.user,
          systemMicros: cpu.system,
          oneCoreEquivalentPercent: Number(
            (((cpu.user + cpu.system) / (measuredWallMs * 1_000)) * 100).toFixed(6)
          )
        },
        memory: {
          before: memoryBefore,
          after: memoryAfter,
          peakRssBytes,
          peakHeapUsedBytes,
          peakExternalBytes
        },
        healthBefore,
        healthAfter
      }
    };
  } finally {
    if (server) await server.close();
    await Promise.allSettled(sockets.map(closeSocket));
    resetCFGToDefaults();
    fs.rmSync(resolvedTemporaryRoot, { recursive: true, force: true });
  }
}

/** Execute the CLI. */
async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  const result = await runExternalControlBaseline(options);
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (options.outputPath) {
    fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
    fs.writeFileSync(options.outputPath, json, 'utf8');
    console.info(`[stage2.external-control] wrote ${options.outputPath}`);
  } else {
    process.stdout.write(json);
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
