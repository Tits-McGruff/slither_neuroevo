import { performance } from 'node:perf_hooks';
import { CFG, resetCFGToDefaults, syncBrainInputSize } from '../src/config.ts';
import {
  World,
  type GenerationBoundaryState,
  type WorldResumeState
} from '../src/world.ts';
import { getByPath, setByPath } from '../src/utils.ts';
import { validateGraph } from '../src/brains/graph/validate.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { enrichArchInfo } from '../src/mlp.ts';
import {
  coerceSettingsUpdateValue,
  getLiveSettingDefinition,
  normalizeLiveSettingsUpdates,
  type CoreSettings,
  type SettingsUpdate
} from '../src/protocol/settings.ts';
import { SimProfiler, formatSimProfilerReport } from '../src/profiling.ts';
import type { Snake } from '../src/snake.ts';
import type { SpatialHashDiagnostics } from '../src/spatialHash.ts';
import type { ServerConfig } from './config.ts';
import { BrainPool } from './brainPool.ts';
import {
  SimCore,
  type SchedulerDiagnostics,
  type SimulationRunIdentity
} from '../src/sim/SimCore.ts';
import {
  getNativeAddonBuildIdentifier,
  getSimdKernelStatus
} from '../src/brains/nativeBridge.ts';
import type { InferenceBackend } from '../src/brains/types.ts';
import type {
  ActionMsg,
  AuthoritativeSettingsState,
  ClientType,
  GodModeMsg,
  JoinMode,
  LiveSettingsMsg,
  NewRunMsg,
  ResetMsg,
  StatsMsg,
  ViewMsg,
  VizMsg
} from './protocol.ts';
import type { PopulationImportData } from '../src/protocol/messages.ts';
import { ControllerRegistry } from './controllerRegistry.ts';
import type { Persistence } from './persistence.ts';
import {
  buildGenerationCheckpoint,
  buildPopulationExportCheckpoint
} from './checkpoint.ts';
import { buildCoreSettingsSnapshot, buildSettingsUpdatesSnapshot } from './settingsSnapshot.ts';
import { WsHub } from './wsHub.ts';
import { buildSensorSpec } from './sensorSpec.ts';
import type { ActiveInferenceBackend, InferenceModeRecord } from './inferenceMode.ts';
import { createEntropySeed, createRunId } from './runIdentity.ts';
import {
  buildAuthoritativeConfigHash,
  buildLegacyNullGraphConfigHash
} from './configIdentity.ts';
import { normalizeSettingValue } from '../src/protocol/settingDefinitions.ts';
import { WorldSerializer } from '../src/serializer.ts';

/** Environment variable that enables server profiling output. */
const PROFILE_ENV_VAR = 'SLITHER_PROFILE';
/** Interval in milliseconds between profiling reports. */
const PROFILE_REPORT_INTERVAL_MS = 1000;
/** Minimum interval between dropped scheduler-debt warnings. */
const SCHEDULER_DROP_LOG_INTERVAL_MS = 1000;
/** Hard cap for commands waiting on an authoritative fixed-step boundary. */
const MAX_PENDING_COMMANDS = 4096;
/** Maximum overdue steps completed before yielding when no browser player is attached. */
const BACKGROUND_CATCH_UP_STEPS_PER_YIELD = 4;

/** Queued live-settings command awaiting the next fixed-step boundary. */
interface PendingSettingsCommand {
  /** Queue discriminator. */
  kind: 'settings';
  /** Requesting connection id. */
  connId: number;
  /** Strictly parsed request. */
  message: LiveSettingsMsg;
}

/** Queued God Mode command awaiting the next fixed-step boundary. */
interface PendingGodModeCommand {
  /** Queue discriminator. */
  kind: 'godMode';
  /** Requesting connection id. */
  connId: number;
  /** Strictly parsed request. */
  message: GodModeMsg;
}

/** Authoritative commands that may mutate live world state. */
type PendingAuthoritativeCommand = PendingSettingsCommand | PendingGodModeCommand;

/** Public status for a simulation object that may be unusable after a failed step. */
export interface SimulationFaultStatus {
  /** Whether authoritative stepping is prohibited. */
  faulted: boolean;
  /** Stable human-readable failure reason. */
  reason: string | null;
  /** Last successfully committed tick when the fault was recorded. */
  tick: number | null;
}

/** Optional startup state restored from one selected persistence row. */
export interface SimServerBootstrap {
  /** Population-assigned World reconstruction state. */
  resume?: WorldResumeState;
  /** Selected snapshot id used for durable-status reporting. */
  snapshotId?: number;
  /** Whether the selected snapshot supports exact reconstruction. */
  exactResume?: boolean;
  /** Restored monotonic configuration revision. */
  configRevision?: number;
  /** Strict expected current-format configuration hash. */
  expectedConfigHash?: string | null;
}

/** Public durability status reported alongside health diagnostics. */
export interface PersistenceCheckpointStatus {
  /** Whether a persistence adapter remains attached. */
  enabled: boolean;
  /** Configured automatic generation interval. */
  checkpointEveryGenerations: number;
  /** Latest successfully committed resumable snapshot id. */
  lastDurableSnapshotId: number | null;
  /** Generation held by the latest durable resumable checkpoint. */
  lastDurableGeneration: number | null;
  /** Run held by the latest durable resumable checkpoint. */
  lastDurableRunId: string | null;
  /** Current in-memory generation, which may be ahead after a failure. */
  inMemoryGeneration: number;
  /** Current in-memory lineage id. */
  inMemoryRunId: string;
  /** Whether startup used exact continuation rather than legacy compatibility. */
  exactStartupResume: boolean;
}

/**
 * Read-only authoritative world-load snapshot returned by `/health`.
 * Population membership is a cross-cut identified by `populationSlot`; the
 * remaining alive control categories are mutually exclusive and sum to
 * `aliveTotalSnakes`.
 */
export interface AuthoritativeWorldLoadDiagnostics {
  /** Fixed-step id at which this immutable snapshot was captured. */
  committedTick: number;
  /** Active evolutionary generation. */
  generation: number;
  /** Elapsed simulation seconds in the active generation. */
  generationTime: number;
  /** Number of genomes in the current evolved population. */
  populationGenomeCount: number;
  /** Number of retained snake records, alive or dead. */
  totalSnakes: number;
  /** Alive snakes holding a non-null evolved-population slot. */
  aliveEvolvedPopulationSnakes: number;
  /** Alive baseline-bot snakes, identified by a non-null baseline index. */
  aliveBaselineBots: number;
  /**
   * Alive non-baseline snakes with an external controller lease at the last
   * fixed-step boundary, including disconnect grace.
   */
  aliveExternallyOwnedSnakes: number;
  /**
   * Alive non-baseline, non-externally-owned snakes in neural control mode.
   * This is an eligibility category, not proof that a neural inference ran on
   * the most recent step.
   */
  aliveNeuralModeNonBaselineUnownedSnakes: number;
  /** Alive non-baseline snakes outside the external-owner and neural-mode categories. */
  aliveOtherNonBaselineSnakes: number;
  /** Total alive snakes across every control category. */
  aliveTotalSnakes: number;
  /** Total body points across alive snakes only. */
  aliveBodyPointCount: number;
  /** Current authoritative pellet count. */
  pelletCount: number;
}

/** Server-side simulation loop and WS broadcasting. */
export class SimServer {
  /** Unified simulation core. */
  private core: SimCore;

  /** WebSocket hub for broadcasting frames and stats. */
  private wsHub: WsHub;
  /** Simulation tick rate in hertz. */
  private tickRateHz: number;
  /** UI frame broadcast rate in hertz. */
  private uiFrameRateHz: number;

  // Removed tickId, it is now in core.
  // Removed accumulator related fields, SimCore handles it.

  /** Timestamp of the last sent frame in ms. */
  private lastFrameSentAt = 0;
  /** Timestamp of the last stats message in ms. */
  private lastStatsSentAt = 0;
  /** Whether the main loop is running. */
  private running = false;
  /** Active timer id for scheduled ticks. */
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** Currently executing loop iteration, awaited during shutdown. */
  private loopPromise: Promise<void> | null = null;
  /** Target time for the next tick in ms. */
  private nextTickAt = 0;

  /** Timestamp for the previous tick in ms. */
  private lastTickAt = 0;
  /** Timestamp of the latest dropped scheduler-debt warning. */
  private lastSchedulerDropLogAt = 0;

  /** Controller registry for player and bot assignments. */
  private controllers: ControllerRegistry;
  /** Latest fully committed/post-pump world-load snapshot served by health checks. */
  private worldLoadDiagnostics!: AuthoritativeWorldLoadDiagnostics;
  /** Persistence adapter for snapshots and HoF. */
  private persistence: Persistence | null;
  /** Hash for the active configuration. */
  private cfgHash: string;
  /** Monotonic count of accepted authoritative configuration requests. */
  private configRevision = 0;
  /** Next global sequence assigned to an applied boundary command. */
  private nextCommandSequence = 1;
  /** Parsed commands waiting for the next fixed-step boundary. */
  private pendingCommands: PendingAuthoritativeCommand[] = [];
  /** Seed used for the world initialization. */
  private worldSeed: number;
  /** Current evolutionary-lineage identifier. */
  private runId: string;
  /** Interval for snapshot checkpoints in generations. */
  private checkpointEveryGenerations: number;
  /** Generation number at last checkpoint. */
  private lastGeneration: number;
  /** Last generation recorded for HoF save. */
  private lastHofGenSaved: number;
  /** Latest successfully committed resumable snapshot id. */
  private lastDurableSnapshotId: number | null = null;
  /** Generation held by the latest committed resumable checkpoint. */
  private lastDurableGeneration: number | null = null;
  /** Run held by the latest committed resumable checkpoint. */
  private lastDurableRunId: string | null = null;
  /** Config revision staged while a candidate World emits its run-start hook. */
  private pendingCheckpointConfigRevision: number | null = null;
  /** Whether startup reconstructed an exact current-format boundary. */
  private exactStartupResume = false;

  /** Connection ids subscribed to viz streaming. */
  private vizConnections: Set<number>;
  /** Optional profiler for per-tick timing breakdowns. */
  private profiler: SimProfiler | null = null;
  /** Optional worker pool for multi-threaded inference. */
  private brainPool: BrainPool | null = null;
  /** Whether server-side MT inference is enabled. */
  private mtEnabled: boolean;
  /** Original multi-threading request retained even if the baseline silently falls back. */
  private readonly requestedMt: boolean;
  /** Requested worker count for the MT pool. */
  private mtWorkerCount: number;
  /** Last generation synchronized with the MT pool. */
  private mtGeneration: number;
  /** Barrier that prevents fixed-step entry during an explicit reset boundary. */
  private boundaryTransition: Promise<void> | null = null;
  /** Whether multi-threading was active on the last tick. */
  public mtActive = false;
  /** Failure reason prohibiting further authoritative steps. */
  private faultReason: string | null = null;
  /** Last committed tick when the failure was recorded. */
  private faultedAtTick: number | null = null;
  /** Consecutive overdue fixed steps completed since the last Node event-loop yield. */
  private overdueStepsSinceYield = 0;

  /**
   * Create a simulation server instance for a websocket hub.
   * @param config - Normalized server configuration.
   * @param wsHub - WebSocket hub for broadcasting.
   * @param persistence - Optional persistence interface.
   * @param cfgHash - Hash of the config used for snapshots.
   * @param worldSeed - Seed used for world initialization.
   * @param initialSettings - Optional core settings snapshot.
   * @param runId - Lineage id generated independently from simulation RNG.
   * @param bootstrap - Optional selected checkpoint reconstruction state.
   */
  constructor(
    config: ServerConfig,
    wsHub: WsHub,
    persistence?: Persistence,
    cfgHash = '',
    worldSeed = 0,
    initialSettings: Partial<CoreSettings> = {},
    runId = createRunId(),
    bootstrap: SimServerBootstrap = {}
  ) {
    this.wsHub = wsHub;
    this.tickRateHz = config.tickRateHz;
    this.uiFrameRateHz = config.uiFrameRateHz;
    this.persistence = persistence ?? null;
    this.configRevision = bootstrap.configRevision ?? 0;
    this.pendingCheckpointConfigRevision = this.configRevision;
    this.cfgHash = '';
    this.worldSeed = worldSeed;
    this.runId = runId;
    this.checkpointEveryGenerations = Math.max(0, config.checkpointEveryGenerations);
    this.lastHofGenSaved = 0;
    this.exactStartupResume = bootstrap.exactResume === true;
    if (this.checkpointEveryGenerations === 0) {
      console.warn(
        '[persistence] automatic generation checkpoints disabled; crash resume can lose progress'
      );
    }

    // Initialize Unified Core
    this.core = new SimCore({
      settings: initialSettings,
      tickRateHz: this.tickRateHz,
      worldSeed,
      runId,
      inferenceBackend: config.inferenceBackend,
      ...(bootstrap.resume ? { resume: bootstrap.resume } : {}),
      ...(this.persistence
        ? {
            onGenerationBoundary: (boundary: GenerationBoundaryState, world: World) =>
              this.persistGenerationBoundary(boundary, world)
          }
        : {}),
      onStepStarting: (_world, tickId) => {
        this.controllers.setTickId(tickId);
        this.controllers.refresh();
        this.drainPendingCommands(tickId);
      },
      onStepCommitted: async (world, tickId) => {
        this.refreshWorldLoadDiagnostics(world, tickId);
        await this.synchronizeBrainPoolGeneration(world);
        await this.yieldDuringCatchUp();
      }
    });

    if (process.env[PROFILE_ENV_VAR] === '1') {
      this.profiler = new SimProfiler({ reportIntervalMs: PROFILE_REPORT_INTERVAL_MS });
      this.core.world.profiler = this.profiler;
    }

    this.controllers = new ControllerRegistry(
      {
        maxActionsPerTick: config.maxActionsPerTick,
        maxActionsPerSecond: config.maxActionsPerSecond,
        inputHoldMs: config.controllerInputHoldMs,
        disconnectGraceMs: config.controllerDisconnectGraceMs
      },
      {
        getSnakes: () =>
          this.core.world.snakes.map((snake) => ({
            id: snake.id,
            alive: snake.alive,
            controllable: snake.baselineBotIndex == null
          })),
        send: (connId, payload) => this.wsHub.sendJsonTo(connId, payload),
        getLeaseScope: () => `${this.runId}:${this.worldSeed}`
      }
    );
    this.refreshWorldLoadDiagnostics();

    void cfgHash;
    const activeConfigHash = buildAuthoritativeConfigHash(this.core.world);
    const legacyNullGraphHash = bootstrap.expectedConfigHash
      ? buildLegacyNullGraphConfigHash(this.core.world)
      : null;
    if (
      bootstrap.expectedConfigHash &&
      bootstrap.expectedConfigHash !== activeConfigHash &&
      bootstrap.expectedConfigHash !== legacyNullGraphHash
    ) {
      throw new Error(
        `snapshot ${bootstrap.snapshotId ?? 'unknown'} configuration hash mismatch: expected ${bootstrap.expectedConfigHash}, reconstructed ${activeConfigHash}`
      );
    }
    this.cfgHash = activeConfigHash;
    this.worldSeed = this.core.worldSeed;
    this.runId = this.core.runId;
    this.pendingCheckpointConfigRevision = null;
    if (bootstrap.snapshotId !== undefined) {
      this.lastDurableSnapshotId = bootstrap.snapshotId;
      this.lastDurableGeneration = this.core.world.generation;
      this.lastDurableRunId = this.runId;
    }
    this.lastGeneration = this.core.world.generation;
    this.vizConnections = new Set();
    this.requestedMt = config.mtEnabled === true;
    this.mtEnabled = this.requestedMt;
    this.mtWorkerCount = config.mtWorkers ?? 0;
    this.mtGeneration = this.core.world.generation;
    this.lastTickAt = performance.now();
    const pendingHof = this.core.world._lastHoFEntry;
    if (pendingHof && this.persistence) {
      try {
        this.persistence.saveHofEntry(pendingHof);
        this.lastHofGenSaved = pendingHof.gen;
      } catch (error) {
        console.warn('[persistence] resumed hall-of-fame save failed', error);
      }
    }
    this.refreshWelcomeState();
  }

  /** 
   * Explicitly initialize the Multi-threaded pool without starting the simulation loop.
   * This is used by deterministic tests to enable MT support before manually driving ticks.
   */
  async initMT(): Promise<void> {
    if (!this.mtEnabled) return;
    await this.ensureBrainPool();
  }

  /** Start the server tick loop. */
  async start(): Promise<void> {
    if (this.running) return;

    // 1. Initialize MT if enabled
    if (this.mtEnabled) {
      await this.initMT();
    }

    this.refreshWelcomeState();

    this.running = true;
    this.lastTickAt = performance.now();
    this.nextTickAt = this.lastTickAt;
    this.overdueStepsSinceYield = 0;
    this.startLoopIteration();
  }

  /** Stop the server tick loop and await in-flight work and worker cleanup. */
  async stop(): Promise<void> {
    this.running = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    const activeLoop = this.loopPromise;
    if (activeLoop) await activeLoop;
    const activeTransition = this.boundaryTransition;
    if (activeTransition) await activeTransition;
    const pool = this.brainPool;
    this.brainPool = null;
    this.core.brainPool = null;
    if (pool) await pool.shutdown();
  }

  /**
   * Return the current server tick id.
   * @returns Tick id.
   */
  getTickId(): number {
    return this.core.tickId;
  }

  /**
   * Return current operational fixed-step scheduler measurements.
   * @returns Requested/achieved multiplier and dropped-debt diagnostics.
   */
  getSchedulerDiagnostics(): SchedulerDiagnostics {
    return this.core.getSchedulerDiagnostics();
  }

  /**
   * Return current collision-index load, capacity, and admission diagnostics.
   * @returns Operational spatial-index measurements.
   */
  getCollisionGridDiagnostics(): SpatialHashDiagnostics {
    return this.core.world.getCollisionGridDiagnostics();
  }

  /**
   * Return a copy of the latest fully committed/post-pump world-load snapshot.
   * @returns Immutable scalar health diagnostics without a live world scan.
   */
  getWorldLoadDiagnostics(): AuthoritativeWorldLoadDiagnostics {
    return { ...this.worldLoadDiagnostics };
  }

  /**
   * Capture one scalar world-load snapshot at a committed or post-pump boundary.
   * This performs one O(snakes) scan but never serializes, clones, or mutates the World.
   * @param world - Stable authoritative World at the selected boundary.
   * @param committedTick - Fixed-step identity represented by the snapshot.
   */
  private refreshWorldLoadDiagnostics(
    world = this.core.world,
    committedTick = this.core.tickId
  ): void {
    let aliveEvolvedPopulationSnakes = 0;
    let aliveBaselineBots = 0;
    let aliveExternallyOwnedSnakes = 0;
    let aliveNeuralModeNonBaselineUnownedSnakes = 0;
    let aliveOtherNonBaselineSnakes = 0;
    let aliveTotalSnakes = 0;
    let aliveBodyPointCount = 0;
    for (const snake of world.snakes) {
      if (!snake.alive) continue;
      aliveTotalSnakes++;
      aliveBodyPointCount += snake.points.length;
      if (snake.populationSlot !== null) aliveEvolvedPopulationSnakes++;
      if (snake.baselineBotIndex !== null) {
        aliveBaselineBots++;
      } else if (this.controllers.isControlled(snake.id)) {
        aliveExternallyOwnedSnakes++;
      } else if (snake.controlMode === 'neural') {
        aliveNeuralModeNonBaselineUnownedSnakes++;
      } else {
        aliveOtherNonBaselineSnakes++;
      }
    }
    this.worldLoadDiagnostics = {
      committedTick,
      generation: world.generation,
      generationTime: world.generationTime,
      populationGenomeCount: world.population.length,
      totalSnakes: world.snakes.length,
      aliveEvolvedPopulationSnakes,
      aliveBaselineBots,
      aliveExternallyOwnedSnakes,
      aliveNeuralModeNonBaselineUnownedSnakes,
      aliveOtherNonBaselineSnakes,
      aliveTotalSnakes,
      aliveBodyPointCount,
      pelletCount: world.pellets.length
    };
  }

  /**
   * Return whether a failed fixed step has made this simulation unusable.
   * @returns Current fault status.
   */
  getFaultStatus(): SimulationFaultStatus {
    return {
      faulted: this.faultReason !== null,
      reason: this.faultReason,
      tick: this.faultedAtTick
    };
  }

  /**
   * Return current checkpoint durability independently from in-memory state.
   * @returns Persistence interval and latest committed resumable identity.
   */
  getPersistenceStatus(): PersistenceCheckpointStatus {
    return {
      enabled: this.persistence !== null,
      checkpointEveryGenerations: this.checkpointEveryGenerations,
      lastDurableSnapshotId: this.lastDurableSnapshotId,
      lastDurableGeneration: this.lastDurableGeneration,
      lastDurableRunId: this.lastDurableRunId,
      inMemoryGeneration: this.core.world.generation,
      inMemoryRunId: this.runId,
      exactStartupResume: this.exactStartupResume
    };
  }

  /**
   * Return the underlying world instance.
   * @returns World instance.
   */
  getWorld(): World {
    return this.core.world;
  }

  /**
   * Return the visible seed and lineage id for the active in-memory run.
   * @returns Current run identity.
   */
  getRunIdentity(): SimulationRunIdentity {
    return { seed: this.worldSeed, runId: this.runId };
  }

  /**
   * Return the current monotonic revision and canonical content hash.
   * @returns Active authoritative configuration identity.
   */
  getConfigState(): { configRevision: number; configHash: string } {
    return { configRevision: this.configRevision, configHash: this.cfgHash };
  }

  /**
   * Return the active canonical configuration content hash.
   * @returns Current versioned hash.
   */
  getConfigHash(): string {
    return this.cfgHash;
  }

  /**
   * Persist a manual population export without treating mid-generation state as resumable.
   * @returns New SQLite snapshot id.
   */
  saveCurrentPopulationExport(): number {
    if (!this.persistence) throw new Error('persistence is unavailable');
    const checkpoint = buildPopulationExportCheckpoint(this.core.world, {
      runId: this.runId,
      configRevision: this.configRevision,
      simulationStep: this.core.tickId
    });
    return this.persistence.saveCheckpoint(checkpoint);
  }

  /**
   * Persist a scheduled exact boundary before World performs construction draws.
   * @param boundary - Exact World RNG/allocator boundary.
   * @param world - Candidate World whose population has already been assigned.
   */
  private persistGenerationBoundary(boundary: GenerationBoundaryState, world: World): void {
    if (!this.persistence) {
      if (boundary.kind === 'run-start') {
        throw new Error('required run-start checkpoint cannot commit without persistence');
      }
      return;
    }
    const required = boundary.kind === 'run-start';
    const scheduled = this.checkpointEveryGenerations > 0 &&
      boundary.generation % this.checkpointEveryGenerations === 0;
    if (!required && !scheduled) return;
    const configRevision = this.pendingCheckpointConfigRevision ?? this.configRevision;
    const checkpoint = buildGenerationCheckpoint(world, boundary, configRevision);
    const snapshotId = this.persistence.saveCheckpoint(checkpoint);
    this.lastDurableSnapshotId = snapshotId;
    this.lastDurableGeneration = boundary.generation;
    this.lastDurableRunId = boundary.runId;
  }

  /**
   * Return a complete settings snapshot suitable for a Protocol 2 welcome.
   * @returns Active core settings and CFG updates.
   */
  getAuthoritativeSettingsState(): AuthoritativeSettingsState {
    return {
      core: buildCoreSettingsSnapshot(this.core.world),
      updates: buildSettingsUpdatesSnapshot()
    };
  }

  /**
   * Return an honest snapshot of the currently attached inference path.
   * @returns Current inference-mode record for logging and status checks.
   */
  getInferenceMode(): InferenceModeRecord {
    const world = this.core.world;
    const archInfo = enrichArchInfo(world.arch);
    const readyPool = this.brainPool?.status === 'ready' ? this.brainPool : null;
    return {
      requestedBackend: this.core.inferenceBackend,
      activeBackend: readyPool?.inferenceBackend ?? this.getSerialInferenceBackend(),
      requestedMt: this.requestedMt,
      activeWorkerCount: readyPool?.getActiveWorkerCount() ?? 0,
      poolEpoch: readyPool?.poolEpoch ?? null,
      weightEpoch: readyPool?.weightEpoch ?? null,
      graphKey: world.archKey,
      parameterCount: archInfo.totalCount,
      seed: this.worldSeed,
      nativeAddonStatus: getSimdKernelStatus(),
      nativeAddonBuildIdentifier: getNativeAddonBuildIdentifier()
    };
  }

  /**
   * Summarize the backend captured by the currently constructed serial brains.
   * @returns A uniform backend, mixed when brains disagree, or unknown when unavailable.
   */
  private getSerialInferenceBackend(): ActiveInferenceBackend {
    let active: InferenceBackend | null = null;
    for (const snake of this.core.world.snakes) {
      if (snake.baselineBotIndex != null || snake.controlMode === 'external-only') continue;
      const backend = snake.brain.inferenceBackend;
      if (!backend) return 'unknown';
      if (active === null) {
        active = backend;
      } else if (active !== backend) {
        return 'mixed';
      }
    }
    return active ?? 'unknown';
  }

  /**
   * Import a population snapshot into the world.
   * @param data - Import payload to apply.
   * @returns Import result summary.
   */
  async importPopulation(data: PopulationImportData): Promise<{
    ok: boolean;
    reason?: string;
    used?: number;
    total?: number;
  }> {
    return this.runAtRecurrentResetBoundary(async () => {
      const result = this.core.world.importPopulation(data);
      if (!result.ok) return result;
      await this.disposeBrainPool();
      this.lastGeneration = this.core.world.generation;
      this.lastHofGenSaved = 0;
      if (this.mtEnabled) await this.ensureBrainPool();
      this.clearFault();
      this.refreshWorldLoadDiagnostics();
      this.refreshWelcomeState();
      return result;
    });
  }

  /**
   * Handle a join request and assign a snake if player mode.
   * @param connId - Connection id.
   * @param mode - Join mode.
   * @param clientType - Client type.
   * @param name - Optional player name.
   */
  handleJoin(
    connId: number,
    mode: JoinMode,
    clientType: ClientType,
    name?: string,
    resumeToken?: string
  ): void {
    if (mode !== 'player') {
      this.controllers.releaseSnake(connId);
      return;
    }
    if (!name || !name.trim()) {
      this.wsHub.sendJsonTo(connId, { type: 'error', message: 'name required for player mode' });
      return;
    }
    const controller = clientType === 'bot' ? 'bot' : 'player';
    const identityKey = `${controller}:${name.trim()}`;
    const existingId = this.controllers.getAssignedSnakeId(connId);
    if (existingId != null) {
      const existingSnake = this.core.world.snakes.find(snake => snake.id === existingId);
      if (existingSnake && existingSnake.alive) {
        this.controllers.assignSnake(connId, controller, existingId, identityKey);
        return;
      }
    }
    const reclaim = this.controllers.reclaimSnake(
      connId,
      controller,
      resumeToken,
      identityKey
    );
    if (reclaim.reclaimed) return;
    if (reclaim.reason === 'delivery-failed') return;
    if (resumeToken || reclaim.reason === 'ambiguous') {
      this.wsHub.sendJsonTo(connId, {
        type: 'reclaimResult',
        reclaimed: false,
        reason: reclaim.reason
      });
      return;
    }
    const spawned = this.core.world.spawnExternalSnake();
    const snakeId = this.controllers.assignSnake(connId, controller, spawned.id, identityKey);
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
   * Queue an atomic live-settings request for the next fixed-step boundary.
   * @param connId - Requesting UI connection id.
   * @param msg - Strict Protocol 2 settings request.
   */
  handleSettings(connId: number, msg: LiveSettingsMsg): void {
    if (this.pendingCommands.length >= MAX_PENDING_COMMANDS) {
      this.wsHub.sendJsonTo(connId, {
        type: 'settingsApplied',
        requestId: msg.requestId,
        applied: false,
        updates: [],
        configRevision: this.configRevision,
        configHash: this.cfgHash,
        reason: 'authoritative command queue is full'
      });
      return;
    }
    this.pendingCommands.push({ kind: 'settings', connId, message: msg });
  }

  /**
   * Queue a God Mode mutation for the next fixed-step boundary.
   * @param connId - Requesting UI connection id.
   * @param msg - Strict Protocol 2 God Mode request.
   */
  handleGodMode(connId: number, msg: GodModeMsg): void {
    if (this.pendingCommands.length >= MAX_PENDING_COMMANDS) {
      this.wsHub.sendJsonTo(connId, {
        type: 'godModeResult',
        requestId: msg.requestId,
        action: msg.action,
        snakeId: msg.snakeId,
        applied: false,
        reason: 'authoritative command queue is full'
      });
      return;
    }
    this.pendingCommands.push({ kind: 'godMode', connId, message: msg });
  }

  /**
   * Start and acknowledge New Run only after its run-start checkpoint commits.
   * @param connId - Requesting UI connection id.
   * @param msg - Strict Protocol 2 New Run request.
   */
  async handleNewRun(connId: number, msg: NewRunMsg): Promise<void> {
    if (!this.persistence) {
      this.wsHub.sendJsonTo(connId, {
        type: 'newRunResult',
        requestId: msg.requestId,
        applied: false,
        reason: 'New Run requires durable persistence'
      });
      return;
    }
    try {
      const identity = await this.startNewRun();
      this.wsHub.sendJsonTo(connId, {
        type: 'newRunResult',
        requestId: msg.requestId,
        applied: true,
        worldSeed: identity.seed,
        runId: identity.runId
      });
    } catch (error) {
      this.wsHub.sendJsonTo(connId, {
        type: 'newRunResult',
        requestId: msg.requestId,
        applied: false,
        reason: error instanceof Error ? error.message : String(error)
      });
    }
  }

  /**
   * Drain commands in arrival order immediately before one fixed step starts.
   * Commands received while that step awaits inference remain queued until the
   * following boundary because this method never runs inside inference.
   * @param stepId - Fixed step that will observe accepted mutations.
   */
  private drainPendingCommands(stepId: number): void {
    if (this.pendingCommands.length === 0) return;
    const commands = this.pendingCommands.splice(0, this.pendingCommands.length);
    for (const command of commands) {
      if (command.kind === 'settings') {
        this.applyQueuedSettings(command, stepId);
      } else {
        this.applyQueuedGodMode(command, stepId);
      }
    }
  }

  /**
   * Validate and apply one atomic settings request against prior queue results.
   * @param command - Queued settings command.
   * @param stepId - Fixed step that will first observe the result.
   */
  private applyQueuedSettings(command: PendingSettingsCommand, stepId: number): void {
    const normalized = normalizeLiveSettingsUpdates(command.message.updates);
    if (!normalized.ok) {
      this.wsHub.sendJsonTo(command.connId, {
        type: 'settingsApplied',
        requestId: command.message.requestId,
        applied: false,
        updates: [],
        configRevision: this.configRevision,
        configHash: this.cfgHash,
        reason: normalized.reason
      });
      return;
    }

    const previous = new Map<string, unknown>();
    try {
      for (const update of normalized.updates) {
        if (update.path === 'simSpeed') {
          previous.set(update.path, this.core.world.simSpeed);
          this.core.world.applyLiveSimSpeed(update.value);
          continue;
        }
        previous.set(update.path, getByPath(CFG, update.path));
        const coerced = coerceSettingsUpdateValue(update.path, update.value);
        setByPath(CFG, update.path, coerced);
        const definition = getLiveSettingDefinition(update.path);
        if (definition?.derivedState === 'baseline-respawn-delay') {
          this.core.world.applyLiveBaselineRespawnDelay(update.value);
        }
      }
      syncBrainInputSize();
      const nextHash = buildAuthoritativeConfigHash(this.core.world);
      this.configRevision += 1;
      this.cfgHash = nextHash;
    } catch (error) {
      this.rollbackLiveSettings(previous);
      const reason = error instanceof Error ? error.message : String(error);
      this.wsHub.sendJsonTo(command.connId, {
        type: 'settingsApplied',
        requestId: command.message.requestId,
        applied: false,
        updates: [],
        configRevision: this.configRevision,
        configHash: this.cfgHash,
        reason: `settings application failed: ${reason}`
      });
      return;
    }

    const sequence = this.nextCommandSequence++;
    this.refreshWelcomeState();
    this.wsHub.broadcastJsonToUi({
      type: 'settingsApplied',
      requestId: command.message.requestId,
      applied: true,
      updates: normalized.updates,
      configRevision: this.configRevision,
      configHash: this.cfgHash,
      sequence,
      step: stepId
    });
  }

  /**
   * Restore values captured before a failed atomic live-settings application.
   * @param previous - Prior values keyed by authoritative setting path.
   */
  private rollbackLiveSettings(previous: ReadonlyMap<string, unknown>): void {
    for (const [path, value] of previous) {
      if (path === 'simSpeed' && typeof value === 'number') {
        this.core.world.applyLiveSimSpeed(value);
        continue;
      }
      setByPath(CFG, path, value);
      if (path === 'baselineBots.respawnDelay' && typeof value === 'number') {
        this.core.world.applyLiveBaselineRespawnDelay(value);
      }
    }
    syncBrainInputSize();
  }

  /**
   * Apply one queued God Mode operation and report authoritative results.
   * @param command - Queued God Mode command.
   * @param stepId - Fixed step that will first observe the mutation.
   */
  private applyQueuedGodMode(command: PendingGodModeCommand, stepId: number): void {
    const message = command.message;
    const result = message.action === 'kill'
      ? this.core.world.applyGodModeKill(message.snakeId)
      : this.core.world.applyGodModeMove(message.snakeId, message.x, message.y);
    const sequence = result.applied ? this.nextCommandSequence++ : undefined;
    this.wsHub.sendJsonTo(command.connId, {
      type: 'godModeResult',
      requestId: message.requestId,
      action: message.action,
      snakeId: message.snakeId,
      applied: result.applied,
      ...(sequence === undefined ? {} : { sequence, step: stepId }),
      ...(result.reason === undefined ? {} : { reason: result.reason }),
      ...(result.x === undefined ? {} : { x: result.x }),
      ...(result.y === undefined ? {} : { y: result.y }),
      ...(result.pelletsDropped === undefined ? {} : { pelletsDropped: result.pelletsDropped })
    });
  }

  /** Refresh future handshake state from the active authoritative runtime. */
  private refreshWelcomeState(): void {
    this.wsHub.updateWelcome?.({
      worldSeed: this.worldSeed,
      runId: this.runId,
      configRevision: this.configRevision,
      configHash: this.cfgHash,
      settings: this.getAuthoritativeSettingsState(),
      inferenceMode: this.getInferenceMode(),
      sensorSpec: buildSensorSpec(),
      frameByteLength: WorldSerializer.serialize(this.core.world).byteLength
    });
  }

  /**
   * Handle a reset request to rebuild the world with new settings.
   * @param connId - Connection id requesting the reset.
   * @param msg - Reset message payload.
   */
  async handleReset(connId: number, msg: ResetMsg): Promise<void> {
    await this.runAtRecurrentResetBoundary(async () => {
      await this.disposeBrainPool();
      const priorUpdates = buildSettingsUpdatesSnapshot();
      const priorGraphSpec = CFG.brain.graphSpec
        ? structuredClone(CFG.brain.graphSpec)
        : null;
      const settings = coerceCoreSettings(msg.settings);
      const nextRevision = this.configRevision + 1;
      let identity: SimulationRunIdentity;
      try {
        resetCFGToDefaults();
        applySettingsUpdates(msg.updates);
        applyGraphSpecOverride(msg.graphSpec, (reason) => {
          throw new Error(reason);
        });
        this.pendingCheckpointConfigRevision = nextRevision;
        identity = this.core.reset(settings, { runId: createRunId() });
      } catch (error) {
        resetCFGToDefaults();
        applySettingsUpdates(priorUpdates);
        CFG.brain.graphSpec = priorGraphSpec;
        this.pendingCheckpointConfigRevision = null;
        if (this.mtEnabled) await this.ensureBrainPool();
        this.wsHub.sendJsonTo(connId, {
          type: 'error',
          message: `reset failed before durable transition: ${error instanceof Error ? error.message : String(error)}`
        });
        return;
      }
      this.pendingCheckpointConfigRevision = null;
      this.completeCoreRestart(identity);
      if (this.mtEnabled) await this.ensureBrainPool();
      this.configRevision = nextRevision;
      this.cfgHash = buildAuthoritativeConfigHash(this.core.world);
      this.wsHub.updateSensorSpec(buildSensorSpec());
      this.clearFault();
      this.refreshWelcomeState();
    });
  }

  /**
   * Start a generation-one run with a new entropy-derived seed and lineage id.
   * @returns Visible identity of the new durably checkpointed run.
   */
  async startNewRun(): Promise<SimulationRunIdentity> {
    if (!this.persistence) throw new Error('New Run requires durable persistence');
    return this.runAtRecurrentResetBoundary(async () => {
      await this.disposeBrainPool();
      const settings = buildCoreSettingsSnapshot(this.core.world);
      this.pendingCheckpointConfigRevision = this.configRevision;
      let identity: SimulationRunIdentity;
      try {
        identity = this.core.newRun(settings, {
          seed: createEntropySeed(this.worldSeed),
          runId: createRunId()
        });
      } finally {
        this.pendingCheckpointConfigRevision = null;
      }
      this.completeCoreRestart(identity);
      if (this.mtEnabled) await this.ensureBrainPool();
      this.clearFault();
      this.refreshWelcomeState();
      return identity;
    });
  }

  /**
   * Reattach operational server state after a completed core reconstruction.
   * @param identity - Identity returned by the rebuilt SimCore.
   */
  private completeCoreRestart(identity: SimulationRunIdentity): void {
    this.worldSeed = identity.seed;
    this.runId = identity.runId;
    this.exactStartupResume = false;
    this.lastTickAt = performance.now();
    // Profiler Re-attach
    if (this.profiler) this.core.world.profiler = this.profiler;

    this.lastFrameSentAt = 0;
    this.lastStatsSentAt = 0;
    this.core.viewW = CFG.worldRadius * 2;
    this.core.viewH = CFG.worldRadius * 2;
    this.lastGeneration = this.core.world.generation;
    this.lastHofGenSaved = 0;
    this.controllers.setTickId(this.core.tickId);
    this.controllers.reassignDeadSnakes(() => this.core.world.spawnExternalSnake().id);
    this.refreshWorldLoadDiagnostics();
  }

  /**
   * Handle connection teardown and cleanup.
   * @param connId - Connection id.
   */
  handleDisconnect(connId: number): void {
    this.controllers.disconnectConnection(connId);
    this.vizConnections.delete(connId);
  }

  /**
   * Yield between overdue fixed steps so socket input can reach the next boundary.
   * Browser players force a yield before every additional catch-up step; background
   * operation yields after a small bounded group.
   */
  private async yieldDuringCatchUp(): Promise<void> {
    const anotherStepIsDue =
      this.core.accumulator + this.core.fixedDt * 1e-9 >= this.core.fixedDt;
    if (!anotherStepIsDue) {
      this.overdueStepsSinceYield = 0;
      return;
    }
    this.overdueStepsSinceYield++;
    const threshold = this.controllers.hasInteractiveController()
      ? 1
      : BACKGROUND_CATCH_UP_STEPS_PER_YIELD;
    if (this.overdueStepsSinceYield < threshold) return;
    this.overdueStepsSinceYield = 0;
    await new Promise<void>((resolve) => setImmediate(resolve));
  }

  /**
   * Run one explicit zero-state transition while preventing new fixed steps.
   * @param operation - Reset/import/new-run operation executed at the boundary.
   * @returns Operation result after the previous loop iteration is quiescent.
   */
  private async runAtRecurrentResetBoundary<T>(operation: () => Promise<T>): Promise<T> {
    while (this.boundaryTransition) {
      await this.boundaryTransition;
    }
    let releaseBoundary!: () => void;
    const boundary = new Promise<void>((resolve) => {
      releaseBoundary = resolve;
    });
    this.boundaryTransition = boundary;
    try {
      const activeLoop = this.loopPromise;
      if (activeLoop) await activeLoop;
      return await operation();
    } catch (error) {
      this.enterFault(error);
      throw error;
    } finally {
      releaseBoundary();
      if (this.boundaryTransition === boundary) this.boundaryTransition = null;
    }
  }

  /**
   * Await worker termination and detach the pool from authoritative stepping.
   */
  private async disposeBrainPool(): Promise<void> {
    const pool = this.brainPool;
    this.brainPool = null;
    this.core.brainPool = null;
    this.mtActive = false;
    this.mtGeneration = -1;
    if (pool) await pool.shutdown();
  }

  /** Clear a prior fault only after an explicit boundary rebuild succeeds. */
  private clearFault(): void {
    this.faultReason = null;
    this.faultedAtTick = null;
  }

  /**
   * Ensure the canonical MT pool exactly matches the current zero-state world.
   * @returns Ready brain pool or null when MT was not requested.
   */
  private async ensureBrainPool(): Promise<BrainPool | null> {
    if (!this.mtEnabled) return null;
    const populationCount = this.core.world.population.length;
    if (populationCount <= 0) {
      throw new Error('mt pool requires a nonempty population');
    }

    const archInfo = enrichArchInfo(this.core.world.arch);
    const specKey = this.core.world.archKey;
    const actualParamCount = archInfo.compiled.totalParams;
    const inStride = CFG.brain.inSize;
    const outStride = CFG.brain.outSize;
    const existing = this.brainPool;
    if (existing) {
      if (existing.status !== 'ready') {
        throw new Error(`mt pool is ${existing.status}: ${existing.failureReason ?? 'unknown failure'}`);
      }
      const identityMatch =
        existing.specKey === specKey &&
        existing.populationCount === populationCount &&
        existing.paramCount === actualParamCount &&
        existing.inputStride === inStride &&
        existing.outputStride === outStride &&
        existing.inferenceBackend === this.core.inferenceBackend;
      if (!identityMatch) {
        throw new Error('mt pool identity changed outside a recurrent-reset boundary');
      }
      if (this.mtGeneration !== this.core.world.generation) {
        await this.synchronizeBrainPoolGeneration(this.core.world);
      }
      this.core.brainPool = existing;
      return existing;
    }

    const pool = new BrainPool(this.mtWorkerCount, this.core.inferenceBackend);
    await pool.init({
      spec: archInfo.spec,
      specKey,
      populationCount,
      paramCount: actualParamCount,
      inputStride: inStride,
      outputStride: outStride,
      maxBatch: populationCount,
      weights: packPopulationWeights(this.core.world.population, actualParamCount)
    });
    this.brainPool = pool;
    this.core.brainPool = pool;
    this.mtGeneration = this.core.world.generation;
    return pool;
  }

  /**
   * Install a new generation's weights and zero recurrent state before another step.
   * @param world - World whose just-committed step may have advanced generation.
   */
  private async synchronizeBrainPoolGeneration(world: World): Promise<void> {
    if (!this.mtEnabled || this.mtGeneration === world.generation) return;
    const pool = this.brainPool;
    if (!pool || pool.status !== 'ready') {
      throw new Error('mt generation transition has no ready canonical pool');
    }
    const archInfo = enrichArchInfo(world.arch);
    if (
      pool.specKey !== world.archKey ||
      pool.populationCount !== world.population.length ||
      pool.paramCount !== archInfo.totalCount
    ) {
      throw new Error('mt generation changed pool identity outside a zero-state boundary');
    }
    await pool.replacePopulationWeights(
      packPopulationWeights(world.population, archInfo.totalCount)
    );
    this.mtGeneration = world.generation;
  }

  /** Start one tracked loop iteration so shutdown can await it. */
  private startLoopIteration(): void {
    const iteration = this.loop();
    this.loopPromise = iteration;
    void iteration.then(
      () => {
        if (this.loopPromise === iteration) this.loopPromise = null;
      },
      (error: unknown) => {
        if (this.loopPromise === iteration) this.loopPromise = null;
        this.enterFault(error);
      }
    );
  }

  /**
   * Mark the current World object unusable after a failed authoritative step.
   * @param error - Failure that prevents safe continued stepping.
   */
  private enterFault(error: unknown): void {
    if (this.faultReason !== null) return;
    this.faultReason = error instanceof Error ? error.message : String(error);
    this.faultedAtTick = this.core.tickId;
    this.mtActive = false;
    console.error('[simulation.faulted]', {
      tick: this.faultedAtTick,
      reason: this.faultReason
    });
    this.wsHub.broadcastError?.(
      `simulation faulted at tick ${this.faultedAtTick}: ${this.faultReason}`
    );
  }

  /** Main timer loop for scheduling ticks. */
  private async loop(): Promise<void> {
    if (!this.running) return;
    const now = performance.now();
    if (this.boundaryTransition) {
      this.nextTickAt = now + 1000 / this.tickRateHz;
    } else if (now >= this.nextTickAt) {
      if (this.faultReason === null) {
        try {
          await this.tick(now);
        } catch (error) {
          this.enterFault(error);
        }
      }
      this.nextTickAt = now + 1000 / this.tickRateHz;
    }
    if (!this.running) return;
    const delay = Math.max(0, this.nextTickAt - performance.now());
    this.timer = setTimeout(() => {
      this.timer = null;
      this.startLoopIteration();
    }, delay);
  }

  /**
   * Run a single server tick and broadcast frames/stats as needed.
   * @param now - Current high-resolution timestamp.
   */
  private async tick(now: number): Promise<void> {
    if (this.faultReason !== null) return;
    // 1. Ensure Pool
    this.mtActive = false;
    if (this.mtEnabled) {
      const pool = await this.ensureBrainPool();
      if (pool) this.mtActive = true;
    }

    // 2. Core update. Controller wall time and step identity refresh before every fixed step.
    let dt = 1 / this.tickRateHz;
    if (this.lastTickAt > 0) {
      dt = (now - this.lastTickAt) / 1000;
    }
    this.lastTickAt = now;

    await this.core.update(dt, this.controllers);

    const scheduler = this.core.getSchedulerDiagnostics();
    if (
      scheduler.droppedSimulationSecondsThisPump > 0 &&
      now - this.lastSchedulerDropLogAt >= SCHEDULER_DROP_LOG_INTERVAL_MS
    ) {
      console.warn('[scheduler.debt_dropped]', {
        requestedMultiplier: scheduler.requestedMultiplier,
        achievedMultiplier: scheduler.achievedMultiplier,
        droppedSimulationSeconds: scheduler.droppedSimulationSecondsThisPump,
        maxStepsPerPump: scheduler.maxStepsPerPump
      });
      this.lastSchedulerDropLogAt = now;
    }

    const report = this.profiler?.reportIfDue(now);
    if (report) console.log(formatSimProfilerReport(report));

    this.controllers.reassignDeadSnakes(() => this.core.world.spawnExternalSnake().id);
    this.handleGenerationEnd();
    this.refreshWorldLoadDiagnostics();

    const shouldBroadcastFrame = now - this.lastFrameSentAt >= 1000 / this.uiFrameRateHz;
    if (shouldBroadcastFrame && this.wsHub.hasFrameRecipients()) {
      const frame = this.core.serialize();
      this.wsHub.broadcastFrame(frame);
      this.lastFrameSentAt = now;
    }
    if (now - this.lastStatsSentAt >= 1000) {
      this.wsHub.broadcastStats(this.buildStats());
      this.lastStatsSentAt = now;
    }
  }

  /**
   * Persist the Hall-of-Fame side table after the exact checkpoint hook has run.
   */
  private handleGenerationEnd(): void {
    if (!this.persistence) return;
    const currentGen = this.core.world.generation;
    if (currentGen === this.lastGeneration) return;
    this.lastGeneration = currentGen;

    const hofEntry = this.core.world._lastHoFEntry;
    if (hofEntry && hofEntry.gen !== this.lastHofGenSaved) {
      this.lastHofGenSaved = hofEntry.gen;
      try {
        this.persistence.saveHofEntry(hofEntry);
      } catch (err) {
        console.warn('[persistence] hof save failed', err);
      }
    }
  }

  /**
   * Build the stats payload broadcast to clients.
   * @returns Stats message payload.
   */
  private buildStats(): StatsMsg {
    const vizSnake = this.vizConnections.size > 0 ? this.pickVizSnake() : null;
    const pooledViz =
      this.mtActive &&
      this.brainPool?.status === 'ready' &&
      vizSnake?.populationSlot !== null &&
      vizSnake?.populationSlot !== undefined;
    const includeSerialViz = vizSnake !== null && !pooledViz;
    this.core.onVizSnakePick = includeSerialViz ? () => vizSnake : null;

    const coreStats = this.core.buildStats(includeSerialViz);
    if (pooledViz && vizSnake) {
      const pool = this.brainPool;
      const populationSlot = vizSnake.populationSlot;
      if (pool && populationSlot !== null) {
        const cached = pool.getCachedVisualization();
        if (
          cached?.populationSlot === populationSlot &&
          cached.poolEpoch === pool.poolEpoch &&
          cached.weightEpoch === pool.weightEpoch
        ) {
          coreStats.viz = cached;
        }
        void pool.requestVisualization(populationSlot, this.core.tickId).catch((error) => {
          this.enterFault(error);
        });
      }
    }

    const statsMsg: StatsMsg = {
      type: 'stats',
      ...coreStats
    };
    return statsMsg;
  }

  /**
   * Select a snake for visualization, preferring AI-controlled snakes.
   * @returns Snake to visualize or null when none available.
   */
  private pickVizSnake(): Snake | null {
    const focus = this.core.world.focusSnake;
    if (
      focus &&
      focus.alive &&
      !this.controllers.isControlled(focus.id) &&
      (!this.mtActive || focus.populationSlot !== null)
    ) {
      return focus;
    }
    for (const snake of this.core.world.snakes) {
      if (!snake.alive) continue;
      if (this.controllers.isControlled(snake.id)) continue;
      if (this.mtActive && snake.populationSlot === null) continue;
      return snake;
    }
    if (focus?.alive && !this.controllers.isControlled(focus.id)) return focus;
    return null;
  }
}

/**
 * Pack dense population genomes into the exact shared worker stride layout.
 * @param population - Population ordered by durable slot.
 * @param paramCount - Required parameters in one population slot.
 * @returns Packed weights whose initial contents are ready before worker init.
 */
function packPopulationWeights(
  population: World['population'],
  paramCount: number
): Float32Array {
  if (!Number.isSafeInteger(paramCount) || paramCount <= 0) {
    throw new Error('mt population parameter count must be a positive safe integer');
  }
  const totalLength = population.length * paramCount;
  if (!Number.isSafeInteger(totalLength)) {
    throw new Error('mt packed population weights exceed safe integer capacity');
  }
  const packed = new Float32Array(totalLength);
  for (let populationSlot = 0; populationSlot < population.length; populationSlot++) {
    const genome = population[populationSlot];
    if (!genome || genome.weights.length !== paramCount) {
      throw new Error(
        `mt population slot ${populationSlot} weight length mismatch: expected ${paramCount}, received ${genome?.weights.length ?? 0}`
      );
    }
    packed.set(genome.weights, populationSlot * paramCount);
  }
  return packed;
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
    output.snakeCount = normalizeCoreSetting('snakeCount', raw['snakeCount']);
  }
  if (isFiniteNumber(raw['simSpeed'])) {
    output.simSpeed = normalizeCoreSetting('simSpeed', raw['simSpeed']);
  }
  if (isFiniteNumber(raw['hiddenLayers'])) {
    output.hiddenLayers = normalizeCoreSetting('hiddenLayers', raw['hiddenLayers']);
  }
  if (isFiniteNumber(raw['neurons1'])) output.neurons1 = normalizeCoreSetting('neurons1', raw['neurons1']);
  if (isFiniteNumber(raw['neurons2'])) output.neurons2 = normalizeCoreSetting('neurons2', raw['neurons2']);
  if (isFiniteNumber(raw['neurons3'])) output.neurons3 = normalizeCoreSetting('neurons3', raw['neurons3']);
  if (isFiniteNumber(raw['neurons4'])) output.neurons4 = normalizeCoreSetting('neurons4', raw['neurons4']);
  if (isFiniteNumber(raw['neurons5'])) output.neurons5 = normalizeCoreSetting('neurons5', raw['neurons5']);
  return output;
}

/**
 * Normalize one core setting using the same metadata as live validation.
 * @param path - Core setting key.
 * @param value - Finite numeric input.
 * @returns Shared clamped/type-normalized value.
 */
function normalizeCoreSetting(path: keyof CoreSettings, value: number): number {
  const definition = getLiveSettingDefinition(path);
  if (!definition) throw new Error(`missing core setting definition: ${path}`);
  return normalizeSettingValue(definition, value);
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
