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

export class SimServer {
  private world: World;
  private wsHub: WsHub;
  private tickRateHz: number;
  private uiFrameRateHz: number;
  private tickId = 0;
  private lastFrameSentAt = 0;
  private lastStatsSentAt = 0;
  private running = false;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private nextTickAt = 0;
  private lastTickAt = 0;
  private lastFps = 0;
  private viewW: number;
  private viewH: number;
  private controllers: ControllerRegistry;
  private persistence: Persistence | null;
  private cfgHash: string;
  private worldSeed: number;
  private checkpointEveryGenerations: number;
  private lastGeneration: number;
  private lastHofGenSaved: number;
  private lastHistoryLen: number;
  private vizConnections: Set<number>;

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

  start(): void {
    if (this.running) return;
    this.running = true;
    this.nextTickAt = performance.now();
    this.loop();
  }

  stop(): void {
    this.running = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  getTickId(): number {
    return this.tickId;
  }

  getWorld(): World {
    return this.world;
  }

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

  handleJoin(connId: number, mode: JoinMode, clientType: ClientType, name?: string): void {
    if (mode !== 'player') return;
    if (!name || !name.trim()) {
      this.wsHub.sendJsonTo(connId, { type: 'error', message: 'name required for player mode' });
      return;
    }
    const controller = clientType === 'bot' ? 'bot' : 'player';
    let snakeId = this.controllers.assignSnake(connId, controller);
    if (snakeId == null) {
      const spawned = this.world.spawnExternalSnake();
      snakeId = this.controllers.assignSnake(connId, controller, spawned.id);
      if (snakeId == null) {
        spawned.alive = false;
        this.wsHub.sendJsonTo(connId, { type: 'error', message: 'no available snakes' });
        return;
      }
    }
  }

  handleAction(connId: number, msg: ActionMsg): void {
    this.controllers.handleAction(connId, msg);
  }

  handleView(_connId: number, _msg: ViewMsg): void {
    // Camera/view is per-client; server ignores view messages.
  }

  handleViz(connId: number, msg: VizMsg): void {
    if (msg.enabled) this.vizConnections.add(connId);
    else this.vizConnections.delete(connId);
  }

  handleDisconnect(connId: number): void {
    this.controllers.releaseSnake(connId);
    this.vizConnections.delete(connId);
  }

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

  private buildSnapshotPayload(): PopulationSnapshotPayload {
    const exportData = this.world.exportPopulation();
    return {
      ...exportData,
      cfgHash: this.cfgHash,
      worldSeed: this.worldSeed
    };
  }

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

function buildVizData(brain: { getVizData?: () => VizData } | null | undefined): VizData | null {
  if (!brain || typeof brain.getVizData !== 'function') return null;
  return brain.getVizData();
}
