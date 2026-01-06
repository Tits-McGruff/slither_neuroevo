import { performance } from 'node:perf_hooks';
import { CFG } from '../src/config.ts';
import { World } from '../src/world.ts';
import { WorldSerializer } from '../src/serializer.ts';
import type { ServerConfig } from './config.ts';
import type { ActionMsg, ClientType, JoinMode, StatsMsg, ViewMsg, VizMsg } from './protocol.ts';
import type { PopulationImportData } from '../src/protocol/messages.ts';
import { ControllerRegistry } from './controllerRegistry.ts';
import type { Persistence, PopulationSnapshotPayload } from './persistence.ts';
import { WsHub } from './wsHub.ts';
import type { VizData } from '../src/protocol/messages.ts';

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

  /**
   * Create a simulation server instance for a websocket hub.
   * @param config - Normalized server configuration.
   * @param wsHub - WebSocket hub for broadcasting.
   * @param persistence - Optional persistence interface.
   * @param cfgHash - Hash of the config used for snapshots.
   * @param worldSeed - Seed used for world initialization.
   */
  constructor(
    config: ServerConfig,
    wsHub: WsHub,
    persistence?: Persistence,
    cfgHash = '',
    worldSeed = 0
  ) {
    this.wsHub = wsHub;
    this.tickRateHz = config.tickRateHz;
    this.uiFrameRateHz = config.uiFrameRateHz;
    this.world = new World({});
    this.controllers = new ControllerRegistry(
      {
        actionTimeoutTicks: config.actionTimeoutTicks,
        maxActionsPerTick: config.maxActionsPerTick,
        maxActionsPerSecond: config.maxActionsPerSecond
      },
      {
        getSnakes: () => this.world.snakes,
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
    if (mode !== 'player') return;
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

    const frame = WorldSerializer.serialize(this.world);
    if (now - this.lastFrameSentAt >= 1000 / this.uiFrameRateHz) {
      this.wsHub.broadcastFrame(frame);
      this.lastFrameSentAt = now;
    }
    if (now - this.lastStatsSentAt >= 1000) {
      this.wsHub.broadcastStats(this.buildStats());
      this.lastStatsSentAt = now;
    }
  }

  /**
   * Handle end-of-generation persistence checkpoints.
   */
  private handleGenerationEnd(): void {
    if (!this.persistence) return;
    const currentGen = this.world.generation;
    if (currentGen === this.lastGeneration) return;
    this.lastGeneration = currentGen;

    const hofEntry = this.world._lastHoFEntry;
    if (hofEntry && hofEntry.gen !== this.lastHofGenSaved) {
      this.lastHofGenSaved = hofEntry.gen;
      try {
        this.persistence.saveHofEntry(hofEntry);
      } catch (err) {
        console.warn('[persistence] hof save failed', err);
      }
    }

    if (this.checkpointEveryGenerations <= 0) return;
    if (currentGen % this.checkpointEveryGenerations !== 0) return;
    try {
      const snapshot = this.buildSnapshotPayload();
      this.persistence.saveSnapshot(snapshot);
    } catch (err) {
      console.warn('[persistence] snapshot save failed', err);
    }
  }

  /**
   * Build a snapshot payload for persistence.
   * @returns Snapshot payload.
   */
  private buildSnapshotPayload(): PopulationSnapshotPayload {
    const exportData = this.world.exportPopulation();
    return {
      ...exportData,
      cfgHash: this.cfgHash,
      worldSeed: this.worldSeed
    };
  }

  /**
   * Build the stats payload broadcast to clients.
   * @returns Stats message payload.
   */
  private buildStats(): StatsMsg {
    const aliveSnakes = this.world.snakes.filter(snake => snake.alive);
    let maxFit = 0;
    let minFit = Infinity;
    let sumFit = 0;
    aliveSnakes.forEach(snake => {
      const fit = snake.pointsScore || 0;
      maxFit = Math.max(maxFit, fit);
      minFit = Math.min(minFit, fit);
      sumFit += fit;
    });
    if (minFit === Infinity) minFit = 0;
    const avgFit = aliveSnakes.length ? sumFit / aliveSnakes.length : 0;
    const stats: StatsMsg = {
      type: 'stats',
      tick: this.tickId,
      gen: this.world.generation,
      alive: aliveSnakes.length,
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
      const viz = buildVizData(this.world.focusSnake?.brain);
      if (viz) stats.viz = viz;
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
