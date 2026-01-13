import { performance } from 'node:perf_hooks';
import { CFG, resetCFGToDefaults, syncBrainInputSize } from '../src/config.ts';
import { World } from '../src/world.ts';
import { WorldSerializer } from '../src/serializer.ts';
import { setByPath } from '../src/utils.ts';
import { validateGraph } from '../src/brains/graph/validate.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { coerceSettingsUpdateValue, type CoreSettings, type SettingsUpdate } from '../src/protocol/settings.ts';
import type { Snake } from '../src/snake.ts';
import type { ServerConfig } from './config.ts';
import type {
  ActionMsg,
  ClientType,
  JoinMode,
  ResetMsg,
  StatsMsg,
  ViewMsg,
  VizMsg
} from './protocol.ts';
import type { PopulationImportData } from '../src/protocol/messages.ts';
import { ControllerRegistry } from './controllerRegistry.ts';
import type { Persistence, PopulationSnapshotPayload } from './persistence.ts';
import { buildCoreSettingsSnapshot, buildSettingsUpdatesSnapshot } from './settingsSnapshot.ts';
import { WsHub } from './wsHub.ts';
import type { VizData } from '../src/protocol/messages.ts';

/** SQLite error code indicating the database or disk is full. */
const SQLITE_FULL_CODE = 'SQLITE_FULL';

/**
 * Determine whether an error is a SQLite "full" error.
 * @param err - Error thrown by persistence.
 * @returns True when the error matches SQLITE_FULL.
 */
function isSqliteFullError(err: unknown): boolean {
  if (!err || typeof err !== 'object') return false;
  return (err as { code?: string }).code === SQLITE_FULL_CODE;
}

/** Server-side simulation loop and WS broadcasting. */
export class SimServer {
  /** World instance that owns simulation state. */
  private world: World;
  /** WebSocket hub for broadcasting frames and stats. */
  private wsHub: WsHub;
  /** Simulation tick rate in hertz. */
  private tickRateHz: number;
  /** UI frame broadcast rate in hertz. */
  private uiFrameRateHz: number;
  /** Current simulation tick id. */
  private tickId = 0;
  /** Timestamp of the last sent frame in ms. */
  private lastFrameSentAt = 0;
  /** Timestamp of the last stats message in ms. */
  private lastStatsSentAt = 0;
  /** Whether the main loop is running. */
  private running = false;
  /** Active timer id for scheduled ticks. */
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** Target time for the next tick in ms. */
  private nextTickAt = 0;
  /** Timestamp for the previous tick in ms. */
  private lastTickAt = 0;
  /** Last computed simulation FPS. */
  private lastFps = 0;
  /** Current view width used for serialization. */
  private viewW: number;
  /** Current view height used for serialization. */
  private viewH: number;
  /** Controller registry for player and bot assignments. */
  private controllers: ControllerRegistry;
  /** Persistence adapter for snapshots and HoF. */
  private persistence: Persistence | null;
  /** Hash for the active configuration. */
  private cfgHash: string;
  /** Seed used for the world initialization. */
  private worldSeed: number;
  /** Interval for snapshot checkpoints in generations. */
  private checkpointEveryGenerations: number;
  /** Generation number at last checkpoint. */
  private lastGeneration: number;
  /** Last generation recorded for HoF save. */
  private lastHofGenSaved: number;
  /** Last seen fitness history length. */
  private lastHistoryLen: number;
  /** Connection ids subscribed to viz streaming. */
  private vizConnections: Set<number>;
  /** Reason persistence was disabled, if any. */
  private persistenceDisabledReason: string | null = null;

  /**
   * Create a simulation server instance for a websocket hub.
   * @param config - Normalized server configuration.
   * @param wsHub - WebSocket hub for broadcasting.
   * @param persistence - Optional persistence interface.
   * @param cfgHash - Hash of the config used for snapshots.
   * @param worldSeed - Seed used for world initialization.
   * @param initialSettings - Optional core settings snapshot.
   */
  constructor(
    config: ServerConfig,
    wsHub: WsHub,
    persistence?: Persistence,
    cfgHash = '',
    worldSeed = 0,
    initialSettings: Partial<CoreSettings> = {}
  ) {
    this.wsHub = wsHub;
    this.tickRateHz = config.tickRateHz;
    this.uiFrameRateHz = config.uiFrameRateHz;
    this.world = new World(initialSettings);
    this.controllers = new ControllerRegistry(
      {
        actionTimeoutTicks: config.actionTimeoutTicks,
        maxActionsPerTick: config.maxActionsPerTick,
        maxActionsPerSecond: config.maxActionsPerSecond
      },
      {
        getSnakes: () =>
          this.world.snakes.map((snake) => ({
            id: snake.id,
            alive: snake.alive,
            controllable: snake.baselineBotIndex == null
          })),
        send: (connId, payload) => this.wsHub.sendJsonTo(connId, payload)
      }
    );
    this.viewW = CFG.worldRadius * 2;
    this.viewH = CFG.worldRadius * 2;
    this.persistence = persistence ?? null;
    this.cfgHash = cfgHash;
    this.worldSeed = worldSeed;
    this.checkpointEveryGenerations = Math.max(0, config.checkpointEveryGenerations);
    this.lastGeneration = this.world.generation;
    this.lastHofGenSaved = 0;
    this.lastHistoryLen = this.world.fitnessHistory.length;
    this.vizConnections = new Set();
  }

  /** Start the server tick loop. */
  start(): void {
    if (this.running) return;
    this.running = true;
    this.nextTickAt = performance.now();
    this.loop();
  }

  /** Stop the server tick loop. */
  stop(): void {
    this.running = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  /**
   * Return the current server tick id.
   * @returns Tick id.
   */
  getTickId(): number {
    return this.tickId;
  }

  /**
   * Return the underlying world instance.
   * @returns World instance.
   */
  getWorld(): World {
    return this.world;
  }

  /**
   * Import a population snapshot into the world.
   * @param data - Import payload to apply.
   * @returns Import result summary.
   */
  importPopulation(data: PopulationImportData): {
    ok: boolean;
    reason?: string;
    used?: number;
    total?: number;
  } {
    const result = this.world.importPopulation(data);
    this.lastGeneration = this.world.generation;
    this.lastHofGenSaved = 0;
    this.lastHistoryLen = this.world.fitnessHistory.length;
    return result;
  }

  /**
   * Handle a join request and assign a snake if player mode.
   * @param connId - Connection id.
   * @param mode - Join mode.
   * @param clientType - Client type.
   * @param name - Optional player name.
   */
  handleJoin(connId: number, mode: JoinMode, clientType: ClientType, name?: string): void {
    if (mode !== 'player') {
      this.controllers.releaseSnake(connId);
      return;
    }
    if (!name || !name.trim()) {
      this.wsHub.sendJsonTo(connId, { type: 'error', message: 'name required for player mode' });
      return;
    }
    const controller = clientType === 'bot' ? 'bot' : 'player';
    const existingId = this.controllers.getAssignedSnakeId(connId);
    if (existingId != null) {
      const existingSnake = this.world.snakes.find(snake => snake.id === existingId);
      if (existingSnake && existingSnake.alive) {
        this.controllers.assignSnake(connId, controller, existingId);
        return;
      }
    }
    const spawned = this.world.spawnExternalSnake();
    const snakeId = this.controllers.assignSnake(connId, controller, spawned.id);
    if (snakeId == null) {
      spawned.alive = false;
      this.wsHub.sendJsonTo(connId, { type: 'error', message: 'no available snakes' });
      return;
    }
  }

  /**
   * Handle an action message from a connection.
   * @param connId - Connection id.
   * @param msg - Action message payload.
   */
  handleAction(connId: number, msg: ActionMsg): void {
    this.controllers.handleAction(connId, msg);
  }

  /**
   * Handle a view message (ignored server-side).
   * @param _connId - Connection id (unused).
   * @param _msg - View message payload (unused).
   */
  handleView(_connId: number, _msg: ViewMsg): void {
    // Camera/view is per-client; server ignores view messages.
  }

  /**
   * Handle a viz message to toggle streaming for a connection.
   * @param connId - Connection id.
   * @param msg - Viz message payload.
   */
  handleViz(connId: number, msg: VizMsg): void {
    if (msg.enabled) this.vizConnections.add(connId);
    else this.vizConnections.delete(connId);
  }

  /**
   * Handle a reset request to rebuild the world with new settings.
   * @param connId - Connection id requesting the reset.
   * @param msg - Reset message payload.
   */
  handleReset(connId: number, msg: ResetMsg): void {
    const settings = coerceCoreSettings(msg.settings);
    resetCFGToDefaults();
    applySettingsUpdates(msg.updates);
    applyGraphSpecOverride(msg.graphSpec, (reason) => {
      this.wsHub.sendJsonTo(connId, { type: 'error', message: `reset failed: ${reason}` });
    });

    this.world = new World(settings);
    this.tickId = 0;
    this.lastTickAt = 0;
    this.lastFrameSentAt = 0;
    this.lastStatsSentAt = 0;
    this.lastFps = 0;
    this.viewW = CFG.worldRadius * 2;
    this.viewH = CFG.worldRadius * 2;
    this.lastGeneration = this.world.generation;
    this.lastHofGenSaved = 0;
    this.lastHistoryLen = this.world.fitnessHistory.length;
    this.controllers.setTickId(this.tickId);
    this.controllers.reassignDeadSnakes(() => this.world.spawnExternalSnake().id);
  }

  /**
   * Handle connection teardown and cleanup.
   * @param connId - Connection id.
   */
  handleDisconnect(connId: number): void {
    this.controllers.releaseSnake(connId);
    this.vizConnections.delete(connId);
  }

  /** Main timer loop for scheduling ticks. */
  private loop(): void {
    if (!this.running) return;
    const now = performance.now();
    if (now >= this.nextTickAt) {
      this.tick(now);
      this.nextTickAt += 1000 / this.tickRateHz;
    }
    const delay = Math.max(0, this.nextTickAt - now);
    this.timer = setTimeout(() => this.loop(), delay);
  }

  /**
   * Run a single server tick and broadcast frames/stats as needed.
   * @param now - Current high-resolution timestamp.
   */
  private tick(now: number): void {
    this.tickId += 1;
    this.controllers.setTickId(this.tickId);
    if (this.lastTickAt > 0) {
      const dt = (now - this.lastTickAt) / 1000;
      if (dt > 0) this.lastFps = 1 / dt;
    }
    this.lastTickAt = now;

    const dt = 1 / this.tickRateHz;
    this.world.update(dt, this.viewW, this.viewH, this.controllers, this.tickId);
    this.controllers.reassignDeadSnakes(() => this.world.spawnExternalSnake().id);
    this.handleGenerationEnd();

    const shouldBroadcastFrame = now - this.lastFrameSentAt >= 1000 / this.uiFrameRateHz;
    if (shouldBroadcastFrame && this.wsHub.hasFrameRecipients()) {
      const frame = WorldSerializer.serialize(this.world);
      this.wsHub.broadcastFrame(frame);
      this.lastFrameSentAt = now;
    }
    if (now - this.lastStatsSentAt >= 1000) {
      this.wsHub.broadcastStats(this.buildStats());
      this.lastStatsSentAt = now;
    }
  }

  /**
   * Disable persistence after a non-recoverable storage failure.
   * @param reason - Human-readable reason for disabling.
   * @param err - Original error for logging.
   */
  private disablePersistence(reason: string, err: unknown): void {
    if (this.persistenceDisabledReason) return;
    this.persistenceDisabledReason = reason;
    this.persistence = null;
    console.warn(`[persistence] disabled (${reason})`, err);
  }

  /**
   * Handle end-of-generation persistence checkpoints.
   */
  private handleGenerationEnd(): void {
    if (!this.persistence || this.persistenceDisabledReason) return;
    const currentGen = this.world.generation;
    if (currentGen === this.lastGeneration) return;
    this.lastGeneration = currentGen;

    const hofEntry = this.world._lastHoFEntry;
    if (hofEntry && hofEntry.gen !== this.lastHofGenSaved) {
      this.lastHofGenSaved = hofEntry.gen;
      try {
        this.persistence.saveHofEntry(hofEntry);
      } catch (err) {
        if (isSqliteFullError(err)) {
          this.disablePersistence('sqlite full during hall-of-fame save', err);
          return;
        }
        console.warn('[persistence] hof save failed', err);
      }
    }

    if (this.checkpointEveryGenerations <= 0) return;
    if (currentGen % this.checkpointEveryGenerations !== 0) return;
    try {
      const snapshot = this.buildSnapshotPayload();
      this.persistence.saveSnapshot(snapshot);
    } catch (err) {
      if (isSqliteFullError(err)) {
        this.disablePersistence('sqlite full during snapshot save', err);
        return;
      }
      console.warn('[persistence] snapshot save failed', err);
    }
  }

  /**
   * Build a snapshot payload for persistence.
   * @returns Snapshot payload.
   */
  private buildSnapshotPayload(): PopulationSnapshotPayload {
    const exportData = this.world.exportPopulation();
    const settings = buildCoreSettingsSnapshot(this.world);
    const updates = buildSettingsUpdatesSnapshot();
    return {
      ...exportData,
      cfgHash: this.cfgHash,
      worldSeed: this.worldSeed,
      settings,
      updates
    };
  }

  /**
   * Select a snake for visualization, preferring AI-controlled snakes.
   * @returns Snake to visualize or null when none available.
   */
  private pickVizSnake(): Snake | null {
    const focus = this.world.focusSnake;
    if (focus && focus.alive && !this.controllers.isControlled(focus.id)) return focus;
    for (const snake of this.world.snakes) {
      if (!snake.alive) continue;
      if (this.controllers.isControlled(snake.id)) continue;
      return snake;
    }
    return focus ?? null;
  }

  /**
   * Build the stats payload broadcast to clients.
   * @returns Stats message payload.
   */
  private buildStats(): StatsMsg {
    const populationCount = this.world.population.length;
    const baselineBotsTotal = this.world.baselineBots.length;
    let alivePopulation = 0;
    let aliveTotal = 0;
    let baselineBotsAlive = 0;
    let maxFit = 0;
    let minFit = Infinity;
    let sumFit = 0;
    for (let i = 0; i < populationCount; i++) {
      const snake = this.world.snakes[i];
      if (!snake || !snake.alive) continue;
      alivePopulation += 1;
      const fit = snake.pointsScore || 0;
      maxFit = Math.max(maxFit, fit);
      minFit = Math.min(minFit, fit);
      sumFit += fit;
    }
    for (const snake of this.world.snakes) {
      if (snake.alive) aliveTotal += 1;
    }
    for (const bot of this.world.baselineBots) {
      if (bot && bot.alive) baselineBotsAlive += 1;
    }
    if (minFit === Infinity) minFit = 0;
    const avgFit = alivePopulation ? sumFit / alivePopulation : 0;
    const stats: StatsMsg = {
      type: 'stats',
      tick: this.tickId,
      gen: this.world.generation,
      alive: alivePopulation,
      aliveTotal,
      baselineBotsAlive,
      baselineBotsTotal,
      fps: this.lastFps || this.tickRateHz,
      fitnessData: {
        gen: this.world.generation,
        avgFitness: avgFit,
        maxFitness: maxFit,
        minFitness: minFit
      }
    };
    if (this.world.fitnessHistory.length !== this.lastHistoryLen) {
      stats.fitnessHistory = this.world.fitnessHistory.slice();
      this.lastHistoryLen = this.world.fitnessHistory.length;
    }
    if (this.vizConnections.size > 0) {
      const vizTarget = this.pickVizSnake();
      const viz = buildVizData(vizTarget?.brain);
      if (viz) stats.viz = viz;
    }
    if (this.world._lastHoFEntry) {
      stats.hofEntry = this.world._lastHoFEntry;
      this.world._lastHoFEntry = null;
    }
    return stats;
  }
}

/**
 * Build visualization payloads from a brain instance if supported.
 * @param brain - Brain instance or null.
 * @returns Visualization payload or null.
 */
function buildVizData(brain: { getVizData?: () => VizData } | null | undefined): VizData | null {
  if (!brain || typeof brain.getVizData !== 'function') return null;
  return brain.getVizData();
}

/**
 * Check if a value is a plain record.
 * @param value - Value to inspect.
 * @returns True when the value is a non-null object.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

/**
 * Check if a value is a finite number.
 * @param value - Value to inspect.
 * @returns True when the value is a finite number.
 */
function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/**
 * Coerce a core settings payload to a partial settings object.
 * @param value - Raw settings payload.
 * @returns Sanitized core settings values.
 */
export function coerceCoreSettings(value: unknown): Partial<CoreSettings> {
  if (!isRecord(value)) return {};
  const output: Partial<CoreSettings> = {};
  const raw = value as Record<string, unknown>;
  if (isFiniteNumber(raw['snakeCount'])) {
    output.snakeCount = Math.max(1, Math.floor(raw['snakeCount']));
  }
  if (isFiniteNumber(raw['simSpeed'])) {
    output.simSpeed = raw['simSpeed'];
  }
  if (isFiniteNumber(raw['hiddenLayers'])) {
    output.hiddenLayers = Math.max(1, Math.floor(raw['hiddenLayers']));
  }
  if (isFiniteNumber(raw['neurons1'])) output.neurons1 = Math.max(1, Math.floor(raw['neurons1']));
  if (isFiniteNumber(raw['neurons2'])) output.neurons2 = Math.max(1, Math.floor(raw['neurons2']));
  if (isFiniteNumber(raw['neurons3'])) output.neurons3 = Math.max(1, Math.floor(raw['neurons3']));
  if (isFiniteNumber(raw['neurons4'])) output.neurons4 = Math.max(1, Math.floor(raw['neurons4']));
  if (isFiniteNumber(raw['neurons5'])) output.neurons5 = Math.max(1, Math.floor(raw['neurons5']));
  return output;
}

/**
 * Apply settings updates to the global configuration.
 * @param updates - Settings updates to apply.
 */
export function applySettingsUpdates(updates: SettingsUpdate[] | undefined): void {
  if (!updates) return;
  updates.forEach((update) => {
    const coerced = coerceSettingsUpdateValue(update.path, update.value);
    setByPath(CFG, update.path, coerced);
  });
  syncBrainInputSize();
}

/**
 * Apply an optional graph spec override to the global configuration.
 * @param spec - Graph spec to apply or null to clear.
 * @param onError - Optional error callback for invalid specs.
 */
function applyGraphSpecOverride(
  spec: GraphSpec | null | undefined,
  onError?: (message: string) => void
): void {
  if (spec === undefined) return;
  if (spec === null) {
    CFG.brain.graphSpec = null;
    return;
  }
  const inputNodes = spec.nodes.filter(node => node.type === 'Input');
  if (inputNodes.length !== 1) {
    CFG.brain.graphSpec = null;
    onError?.('graph must include exactly one Input node');
    return;
  }
  const inputNode = inputNodes[0]!;
  if (inputNode.outputSize !== CFG.brain.inSize) {
    CFG.brain.graphSpec = null;
    onError?.(`input size mismatch (expected ${CFG.brain.inSize})`);
    return;
  }
  const result = validateGraph(spec);
  if (!result.ok) {
    CFG.brain.graphSpec = null;
    onError?.(result.reason);
    return;
  }
  CFG.brain.graphSpec = spec;
}
