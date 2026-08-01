/** Real-server Stage 2 Protocol 2 external-control baseline. */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { monitorEventLoopDelay, performance } from 'node:perf_hooks';
import { spawnSync } from 'node:child_process';
import Database from 'better-sqlite3';
import WebSocket, { type RawData } from 'ws';
import { buildGenerationCheckpoint } from '../../server/checkpoint.ts';
import { DEFAULT_CONFIG, type ServerConfig } from '../../server/config.ts';
import { startServer, type RunningServer } from '../../server/index.ts';
import type { AuthoritativeWorldLoadDiagnostics } from '../../server/simServer.ts';
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
/** Provenance declaration supplied by the benchmark operator. */
type EvidenceEnvironment = 'development' | 'owner-target-vm';

/** Explicit environment facts retained with target-sensitive evidence. */
interface EnvironmentProvenance {
  /** Operator-provided provenance class. */
  declaration: EvidenceEnvironment;
  /** Whether Node reports Linux. */
  platformIsLinux: boolean;
  /** Linux distribution ID from `/etc/os-release`. */
  distributionId: string | null;
  /** Whether the distribution ID is Debian. */
  distributionIsDebian: boolean;
  /** Reported hostname. */
  hostname: string;
  /** Whether the short hostname is oxygen. */
  hostnameIsOxygen: boolean;
  /** First reported CPU model. */
  cpuModel: string;
  /** Whether the CPU model is the Ryzen 7 2700 target. */
  cpuModelMatches: boolean;
  /** Visible logical processor count. */
  logicalCpuCount: number;
  /** Whether eight logical processors are visible. */
  logicalCpuCountMatches: boolean;
  /** Total memory visible to Node. */
  totalMemoryBytes: number;
  /** Whether at least 15 GiB of the 16-GiB allocation is visible. */
  memoryAllocationMatches: boolean;
  /** True only when explicit declaration and every target fact agree. */
  ownerTargetVmValidated: boolean;
}

/** Parsed benchmark options. */
interface ExternalControlOptions {
  /** Compatibility P5 or isolated P6 client composition. */
  profile: 'p5' | 'p6' | 'p7';
  /** Named population/brain workload. */
  scenario: ExternalScenario;
  /** Browser-player periodic action cadence retained for P5 compatibility. */
  playerHz: 30 | 60;
  /** Requested fixed-step multiplier for this P6 run. */
  simSpeed: 1 | 2 | 4 | 8 | 12;
  /** Whether exactly one UI spectator receives the current full frame. */
  viewer: boolean;
  /** Automatic generation checkpoint interval; zero is an explicit diagnostic exclusion. */
  checkpointEvery: number;
  /** P5 wall-time warm-up after all clients are ready. */
  warmupMs: number | null;
  /** P5 measured wall duration. */
  durationMs: number | null;
  /** P6 absolute tick target at or beyond which the polled warm-up sample is taken. */
  warmupTick: number | null;
  /** P6 minimum number of authoritative fixed steps required between polled samples. */
  measurementSteps: number | null;
  /** Canonical Node inference workers. */
  workers: number;
  /** Optional JSON destination. */
  outputPath: string | null;
  /** P7 bounded health/resource sample cadence. */
  sampleEveryMs: number;
  /** P7 reconnect cadence for the token-bearing UI controller. */
  reconnectEveryMs: number;
  /** P7 legacy-reference manual save cadence. */
  manualSaveEveryMs: number;
  /** Explicit test-only escape hatch for a sub-30-minute P7 integration run. */
  p7TestOnlyShort: boolean;
  /** Explicit provenance declaration; hardware similarity alone is not host identity. */
  evidenceEnvironment: EvidenceEnvironment;
}

/** Maximum retained event timestamps per client; totals remain separate. */
const TELEMETRY_SAMPLE_CAP = 4096;
/** Normal P7 wall duration. */
export const P7_MIN_DURATION_MS = 30 * 60 * 1000;
/** P7 warm-window boundary inside the same 30-minute measurement. */
export const P7_WARMUP_MS = 10 * 60 * 1000;
/** Default bounded P7 scalar sampling cadence. */
export const P7_SAMPLE_EVERY_MS = 5_000;

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
  /** First assignment timestamp; later replacement assignments do not overwrite it. */
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
  /** Adjacent action-send intervals for this external controller. */
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
  /** Total sensor messages, including samples discarded from the bounded array. */
  sensorCountTotal: number;
  /** Total action sends, including samples discarded from the bounded array. */
  actionCountTotal: number;
  /** Explicit reclaim results received by a reconnecting controller. */
  reclaimResults: Array<{ reclaimed: boolean; reason: string }>;
  /** Whether the latest assignment explicitly confirms a same-snake reclaim. */
  assignmentReclaimed: boolean;
  /** Monotonic close time, or null while this socket remains open. */
  closedAtMs: number | null;
  /** WebSocket close code, when observed. */
  closeCode: number | null;
  /** UTF-8 close reason, when observed. */
  closeReason: string | null;
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
  /** Stats generation identities for reset-safe elapsed-time comparison. */
  statsGenerations: number[];
  /** Stats generation-time values. */
  generationTimes: number[];
  /** Socket/protocol errors. */
  errors: string[];
  /** Total binary frames, including samples discarded from the bounded arrays. */
  frameCountTotal: number;
  /** Total stats messages, including samples discarded from the bounded arrays. */
  statsCountTotal: number;
  /** Monotonic close time, or null while this socket remains open. */
  closedAtMs: number | null;
  /** WebSocket close code, when observed. */
  closeCode: number | null;
  /** UTF-8 close reason, when observed. */
  closeReason: string | null;
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
  /** Resolves as soon as the spectator socket is open and its hello/join were sent. */
  connected: Promise<void>;
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
  /** Current authoritative snake/body/pellet load without full-world serialization. */
  worldLoad: AuthoritativeWorldLoadDiagnostics;
  /** Current outbound diagnostics. */
  outbound: Record<string, unknown>;
  /** Current fault state. */
  fault: { faulted: boolean; reason: string | null; tick: number | null };
  /** Active inference mode. */
  inferenceMode: Record<string, unknown>;
  /** Current in-memory and latest durable generation identities. */
  persistence: {
    /** Configured automatic checkpoint interval. */
    checkpointEveryGenerations: number;
    /** Latest durable resumable snapshot id. */
    lastDurableSnapshotId: number | null;
    /** Latest durable resumable generation. */
    lastDurableGeneration: number | null;
    /** Current in-memory generation. */
    inMemoryGeneration: number;
  };
}

/** One polled authoritative-tick wait result used to bound a P6 measurement. */
interface TickBoundaryWait {
  /** Requested inclusive tick boundary. */
  targetTick: number;
  /** First health tick observed at or beyond the boundary. */
  observedTick: number;
  /** Health-poll overshoot beyond the requested boundary. */
  overshootSteps: number;
  /** Number of health requests made while waiting. */
  pollCount: number;
  /** Runner-monotonic wall time spent waiting. */
  wallMs: number;
  /** Health response at the observed boundary. */
  health: HealthSnapshot;
}

/** Default timeout for an individual health request outside a longer bounded wait. */
const HEALTH_REQUEST_TIMEOUT_MS = 30_000;

/** Error raised when a health request exceeds its explicit wall-time bound. */
class HealthRequestTimeoutError extends Error {
  /** Wall-time request bound that expired. */
  readonly timeoutMs: number;

  /**
   * Create a health-request timeout.
   * @param timeoutMs - Expired request bound in milliseconds.
   * @param cause - Underlying fetch abort.
   */
  constructor(timeoutMs: number, cause: unknown) {
    super(`health request timed out after ${Math.ceil(timeoutMs)} ms`, { cause });
    this.name = 'HealthRequestTimeoutError';
    this.timeoutMs = timeoutMs;
  }
}

/** Delta of cumulative scheduler counters across one measured interval. */
export interface SchedulerDelta {
  /** Committed fixed steps during the interval. */
  tick: number;
  /** Scheduler completed-step counter during the interval. */
  completedSteps: number;
  /** Scheduler wall seconds during the interval. */
  wallSeconds: number;
  /** Committed simulation seconds during the interval. */
  simulatedSeconds: number;
  /** Discarded catch-up debt during the interval. */
  droppedSimulationSeconds: number;
  /** Measured simulated seconds per scheduler wall second, or null within one open pump. */
  achievedMultiplier: number | null;
  /** Achieved multiplier divided by the requested multiplier, or null with no wall delta. */
  achievedToRequestedRatio: number | null;
}

/** Exact Protocol 2 socket composition for a P6 measurement. */
export interface ExternalControlComposition {
  /** Observation-driven external trainer-compatible bot count. */
  botControllers: 1;
  /** Browser player controller count. */
  uiPlayers: 0 | 1;
  /** UI spectator count. */
  uiSpectators: 0 | 1;
  /** Total WebSocket clients created by the runner. */
  totalSockets: 1 | 2 | 3;
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
export function parseOptions(argv: readonly string[]): ExternalControlOptions {
  const options: ExternalControlOptions = {
    profile: 'p5',
    scenario: 'P0',
    playerHz: 30,
    simSpeed: 1,
    viewer: true,
    checkpointEvery: 1_000_000,
    warmupMs: 2_000,
    durationMs: 15_000,
    warmupTick: null,
    measurementSteps: null,
    workers: 0,
    outputPath: null,
    sampleEveryMs: P7_SAMPLE_EVERY_MS,
    reconnectEveryMs: 180_000,
    manualSaveEveryMs: 300_000,
    p7TestOnlyShort: false,
    evidenceEnvironment: 'development'
  };
  let checkpointExplicit = false;
  let viewerExplicit = false;
  let wallWarmupExplicit = false;
  let wallDurationExplicit = false;
  let tickWarmupExplicit = false;
  let measurementStepsExplicit = false;
  let p7OnlyOptionExplicit = false;
  for (let index = 0; index < argv.length; index++) {
    const option = argv[index];
    const value = argv[index + 1];
    switch (option) {
      case '--profile':
        if (value !== 'p5' && value !== 'p6' && value !== 'p7') {
          throw new Error('--profile must be p5, p6, or p7');
        }
        options.profile = value;
        index++;
        break;
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
      case '--sim-speed': {
        const parsed = parseInteger(value, option, 1, 12);
        if (parsed !== 1 && parsed !== 2 && parsed !== 4 && parsed !== 8 && parsed !== 12) {
          throw new Error('--sim-speed must be 1, 2, 4, 8, or 12');
        }
        options.simSpeed = parsed;
        index++;
        break;
      }
      case '--viewer':
        if (value !== 'on' && value !== 'off') {
          throw new Error('--viewer must be on or off');
        }
        options.viewer = value === 'on';
        viewerExplicit = true;
        index++;
        break;
      case '--checkpoint-every':
        options.checkpointEvery = parseInteger(value, option, 0, 1_000_000);
        checkpointExplicit = true;
        index++;
        break;
      case '--warmup-ms':
        options.warmupMs = parseInteger(value, option, 0, P7_MIN_DURATION_MS);
        wallWarmupExplicit = true;
        index++;
        break;
      case '--duration-ms':
        options.durationMs = parseInteger(value, option, 1_000, P7_MIN_DURATION_MS * 2);
        wallDurationExplicit = true;
        index++;
        break;
      case '--warmup-tick':
        options.warmupTick = parseInteger(value, option, 1, 10_000_000);
        tickWarmupExplicit = true;
        index++;
        break;
      case '--measurement-steps':
        options.measurementSteps = parseInteger(value, option, 60, 10_000_000);
        measurementStepsExplicit = true;
        index++;
        break;
      case '--workers':
        options.workers = parseInteger(value, option, 0, 8);
        index++;
        break;
      case '--sample-every-ms':
        options.sampleEveryMs = parseInteger(value, option, 1_000, 60_000);
        p7OnlyOptionExplicit = true;
        index++;
        break;
      case '--reconnect-every-ms':
        options.reconnectEveryMs = parseInteger(value, option, 1_000, P7_MIN_DURATION_MS);
        p7OnlyOptionExplicit = true;
        index++;
        break;
      case '--manual-save-every-ms':
        options.manualSaveEveryMs = parseInteger(value, option, 1_000, P7_MIN_DURATION_MS);
        p7OnlyOptionExplicit = true;
        index++;
        break;
      case '--p7-test-short':
        options.p7TestOnlyShort = true;
        p7OnlyOptionExplicit = true;
        break;
      case '--output':
        if (!value) throw new Error('--output requires a path');
        options.outputPath = path.resolve(value);
        index++;
        break;
      case '--environment':
        if (value !== 'development' && value !== 'owner-target-vm') {
          throw new Error('--environment must be development or owner-target-vm');
        }
        options.evidenceEnvironment = value;
        index++;
        break;
      default:
        throw new Error(`Unknown option ${option ?? '<missing>'}`);
    }
  }
  if (options.profile === 'p6') {
    if (p7OnlyOptionExplicit) throw new Error('P7-only options require --profile p7');
    if (!checkpointExplicit) options.checkpointEvery = 1;
    if (!viewerExplicit) options.viewer = false;
    if (wallWarmupExplicit || wallDurationExplicit) {
      throw new Error(
        'P6 uses --warmup-tick and --measurement-steps to align around a common polled ' +
        'tick target and require a minimum observed step span; wall-time warm-up/duration ' +
        'are P5-only'
      );
    }
    options.warmupMs = null;
    options.durationMs = null;
    if (!tickWarmupExplicit) options.warmupTick = 300;
    if (!measurementStepsExplicit) options.measurementSteps = 1_800;
    if (options.checkpointEvery !== 0 && options.checkpointEvery !== 1) {
      throw new Error(
        'P6 --checkpoint-every must be 1 for the primary matrix or 0 for an explicit diagnostic'
      );
    }
  } else if (options.profile === 'p5') {
    if (p7OnlyOptionExplicit) throw new Error('P7-only options require --profile p7');
    if (tickWarmupExplicit || measurementStepsExplicit) {
      throw new Error('P5 compatibility uses --warmup-ms and --duration-ms, not P6 tick windows');
    }
    if (viewerExplicit && !options.viewer) {
      throw new Error('P5 compatibility always includes its UI spectator; use --profile p6 --viewer off');
    }
    options.viewer = true;
    if (options.warmupMs !== null && options.warmupMs > 60_000) {
      throw new Error('P5 --warmup-ms must be at most 60000');
    }
    if (options.durationMs !== null && options.durationMs > 600_000) {
      throw new Error('P5 --duration-ms must be at most 600000');
    }
  } else {
    if (tickWarmupExplicit || measurementStepsExplicit) {
      throw new Error('P7 uses wall duration/samples, not P6 tick windows');
    }
    if (options.scenario !== 'P0') throw new Error('P7 current-server soak is P0-only initially');
    if (options.simSpeed !== 1) throw new Error('P7 current-server soak requires --sim-speed 1');
    if (!viewerExplicit) options.viewer = true;
    if (!options.viewer) throw new Error('P7 requires one frame-v1 spectator');
    if (!checkpointExplicit) options.checkpointEvery = 1;
    if (options.checkpointEvery !== 1) throw new Error('P7 requires automatic checkpoints every generation');
    options.warmupTick = null;
    options.measurementSteps = null;
    if (!wallWarmupExplicit) options.warmupMs = P7_WARMUP_MS;
    if (!wallDurationExplicit) options.durationMs = P7_MIN_DURATION_MS;
    if (options.durationMs === null || options.warmupMs === null) throw new Error('P7 wall timing is missing');
    if (!options.p7TestOnlyShort && options.durationMs < P7_MIN_DURATION_MS) {
      throw new Error('P7 requires at least 1800000 ms; --p7-test-short is test-only and not evidence');
    }
    if (options.warmupMs >= options.durationMs) throw new Error('P7 warmup must be inside the measurement duration');
    if (options.reconnectEveryMs >= options.durationMs) {
      throw new Error('P7 reconnect cadence must produce at least one reconnect');
    }
    if (options.manualSaveEveryMs >= options.durationMs) {
      throw new Error('P7 save cadence must produce at least one legacy reference save');
    }
  }
  return options;
}

/**
 * Describe the deliberately narrow P6 external-client composition.
 * @param viewer - Whether a single UI spectator is included.
 * @returns Exact controller and socket counts.
 */
export function p6Composition(viewer: boolean): ExternalControlComposition {
  return {
    botControllers: 1,
    uiPlayers: 0,
    uiSpectators: viewer ? 1 : 0,
    totalSockets: viewer ? 2 : 1
  };
}

/**
 * Describe the legacy P5 compatibility client arrangement.
 * @returns One browser player, one observation-driven bot, and one viewer.
 */
export function p5Composition(): ExternalControlComposition {
  return {
    botControllers: 1,
    uiPlayers: 1,
    uiSpectators: 1,
    totalSockets: 3
  };
}

/** P7 keeps one player, one bot, and one frame-v1 spectator throughout the soak. */
export function p7Composition(): ExternalControlComposition {
  return p5Composition();
}

/**
 * Resolve the exact client arrangement for one profile.
 * @param options - Parsed measurement options.
 * @returns Exact external socket counts.
 */
export function externalControlComposition(
  options: Pick<ExternalControlOptions, 'profile' | 'viewer'>
): ExternalControlComposition {
  return options.profile === 'p5' || options.profile === 'p7'
    ? p5Composition()
    : p6Composition(options.viewer);
}

/**
 * Select which spectator milestone may hold the warm-up boundary.
 *
 * P5 keeps its historical compatibility behavior. P6 waits only for the
 * spectator connection so delayed frame publication cannot give viewer-on
 * cases a later, already-collapsed world than viewer-off cases.
 *
 * @param profile - Measurement profile.
 * @returns Required spectator readiness milestone.
 */
export function viewerWarmupReadiness(
  profile: ExternalControlOptions['profile']
): 'connected' | 'first-frame-and-stats' {
  return profile === 'p6' ? 'connected' : 'first-frame-and-stats';
}

/**
 * Install one scenario and apply its requested speed before any run-start
 * checkpoint is built.
 * @param scenarioName - Named Stage 2 population/brain workload.
 * @param simSpeed - Allowed P6 requested multiplier.
 * @returns Scenario settings ready for a World constructor.
 */
export function installExternalControlScenario(
  scenarioName: ExternalScenario,
  simSpeed: ExternalControlOptions['simSpeed']
): ReturnType<typeof installStage2Scenario> {
  const scenario = installStage2Scenario(scenarioName);
  scenario.settings.simSpeed = simSpeed;
  return scenario;
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

/** Retain a bounded representative event series without growing a long soak heap. */
function pushBounded<T>(values: T[], value: T): void {
  if (values.length < TELEMETRY_SAMPLE_CAP) values.push(value);
}

/**
 * Subtract cumulative scheduler counters and reject inconsistent accounting.
 * @param before - Health captured immediately before the measured interval.
 * @param after - Health captured immediately after the measured interval.
 * @param requestedMultiplier - Requested P6 scheduler multiplier.
 * @param tickRateHz - Authoritative fixed-step rate.
 * @returns Interval-local scheduler counters and achieved speed.
 */
export function schedulerDelta(
  before: HealthSnapshot,
  after: HealthSnapshot,
  requestedMultiplier: ExternalControlOptions['simSpeed'],
  tickRateHz: number
): SchedulerDelta {
  if (before.scheduler.requestedMultiplier !== requestedMultiplier ||
    after.scheduler.requestedMultiplier !== requestedMultiplier) {
    throw new Error(
      `scheduler requested multiplier changed: before=${before.scheduler.requestedMultiplier}, ` +
      `after=${after.scheduler.requestedMultiplier}, expected=${requestedMultiplier}`
    );
  }
  const tick = after.tick - before.tick;
  const completedSteps = after.scheduler.completedSteps - before.scheduler.completedSteps;
  const wallSeconds = after.scheduler.wallSeconds - before.scheduler.wallSeconds;
  const simulatedSeconds = after.scheduler.simulatedSeconds - before.scheduler.simulatedSeconds;
  const droppedSimulationSeconds =
    after.scheduler.droppedSimulationSeconds - before.scheduler.droppedSimulationSeconds;
  if (tick < 0 || completedSteps < 0 || wallSeconds < 0 || simulatedSeconds < 0 ||
    droppedSimulationSeconds < 0) {
    throw new Error(
      'scheduler counters moved backwards: ' +
      `tick=${tick}, completedSteps=${completedSteps}, wallSeconds=${wallSeconds}, ` +
      `simulatedSeconds=${simulatedSeconds}, droppedSimulationSeconds=${droppedSimulationSeconds}`
    );
  }
  if (tick !== completedSteps) {
    throw new Error(`tick/completed-step mismatch: tick=${tick}, completedSteps=${completedSteps}`);
  }
  const expectedSimulatedSeconds = completedSteps / tickRateHz;
  const simulatedSecondsTolerance = Math.max(
    1e-9,
    Math.abs(expectedSimulatedSeconds) * 1e-11
  );
  if (Math.abs(simulatedSeconds - expectedSimulatedSeconds) > simulatedSecondsTolerance) {
    throw new Error(
      `simulated seconds disagree with completed steps: ${simulatedSeconds} versus ${expectedSimulatedSeconds}`
    );
  }
  const achievedMultiplier = wallSeconds > 0 ? simulatedSeconds / wallSeconds : null;
  return {
    tick,
    completedSteps,
    wallSeconds,
    simulatedSeconds,
    droppedSimulationSeconds,
    achievedMultiplier,
    achievedToRequestedRatio:
      achievedMultiplier === null ? null : achievedMultiplier / requestedMultiplier
  };
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
  telemetry.sensorCountTotal = 0;
  telemetry.actionCountTotal = 0;
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
  telemetry.statsGenerations.length = 0;
  telemetry.generationTimes.length = 0;
  telemetry.frameCountTotal = 0;
  telemetry.statsCountTotal = 0;
}

/**
 * Read current Git identity without mutating the worktree.
 * @returns Commit and dirty flag.
 */
function sourceIdentity(): { commit: string; dirty: boolean } {
  const gitEnvironment = { ...process.env, GIT_OPTIONAL_LOCKS: '0' };
  const commit = spawnSync('git', ['rev-parse', 'HEAD'], {
    encoding: 'utf8',
    env: gitEnvironment
  });
  const status = spawnSync('git', ['status', '--porcelain'], {
    encoding: 'utf8',
    env: gitEnvironment
  });
  return {
    commit: commit.status === 0 ? commit.stdout.trim() : 'unavailable',
    dirty: status.status !== 0 || status.stdout.trim().length > 0
  };
}

/**
 * Read the Linux distribution identifier without invoking another process.
 * @returns `/etc/os-release` ID, or null outside Linux or when unavailable.
 */
function linuxDistributionId(): string | null {
  if (process.platform !== 'linux') return null;
  try {
    const idLine = fs.readFileSync('/etc/os-release', 'utf8')
      .split(/\r?\n/u)
      .find(line => line.startsWith('ID='));
    if (!idLine) return null;
    return idLine.slice(3).trim().replace(/^['"]|['"]$/gu, '').toLowerCase();
  } catch {
    return null;
  }
}

/**
 * Capture and validate the explicit owner-target evidence declaration.
 * @param declaration - Operator-provided provenance class.
 * @returns Individual environment facts and their combined validation result.
 */
function captureEnvironmentProvenance(
  declaration: EvidenceEnvironment
): EnvironmentProvenance {
  const platformIsLinux = process.platform === 'linux';
  const distributionId = linuxDistributionId();
  const hostname = os.hostname();
  const cpuModel = os.cpus()[0]?.model ?? 'unknown';
  const logicalCpuCount = os.cpus().length;
  const totalMemoryBytes = os.totalmem();
  const facts: EnvironmentProvenance = {
    declaration,
    platformIsLinux,
    distributionId,
    distributionIsDebian: distributionId === 'debian',
    hostname,
    hostnameIsOxygen: hostname.toLowerCase().split('.')[0] === 'oxygen',
    cpuModel,
    cpuModelMatches: cpuModel.includes('AMD Ryzen 7 2700'),
    logicalCpuCount,
    logicalCpuCountMatches: logicalCpuCount === 8,
    totalMemoryBytes,
    memoryAllocationMatches: totalMemoryBytes >= 15 * 1024 * 1024 * 1024,
    ownerTargetVmValidated: false
  };
  facts.ownerTargetVmValidated =
    declaration === 'owner-target-vm' &&
    facts.platformIsLinux &&
    facts.distributionIsDebian &&
    facts.hostnameIsOxygen &&
    facts.cpuModelMatches &&
    facts.logicalCpuCountMatches &&
    facts.memoryAllocationMatches;
  if (declaration === 'owner-target-vm' && !facts.ownerTargetVmValidated) {
    throw new Error(
      `--environment owner-target-vm did not match oxygen Debian/Ryzen/8-vCPU/15-GiB facts: ${JSON.stringify(facts)}`
    );
  }
  return facts;
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
    pushBounded(telemetry.actionIntervalsMs, now - telemetry.lastActionAtMs);
  }
  socket.send(JSON.stringify({
    type: 'action',
    tick: telemetry.latestSensorTick + 1,
    snakeId,
    turn,
    boost
  }));
  telemetry.lastActionAtMs = now;
  telemetry.actionCountTotal++;
  pushBounded(telemetry.actionTimesMs, now);
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
  name: string,
  resumeToken?: string
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
    errors: [],
    sensorCountTotal: 0,
    actionCountTotal: 0,
    reclaimResults: [],
    assignmentReclaimed: false,
    closedAtMs: null,
    closeCode: null,
    closeReason: null
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
    socket.send(JSON.stringify({ type: 'join', mode: 'player', name, ...(resumeToken ? { resumeToken } : {}) }));
  });
  socket.on('error', error => {
    telemetry.errors.push(error.message);
    rejectReady?.(error);
  });
  socket.on('close', (code, reason) => {
    telemetry.closedAtMs = performance.now();
    telemetry.closeCode = code;
    telemetry.closeReason = reason.toString('utf8');
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
      telemetry.assignedAtMs ??= performance.now();
      telemetry.snakeId = typeof message['snakeId'] === 'number' ? message['snakeId'] : null;
      telemetry.resumeToken =
        typeof message['resumeToken'] === 'string' ? message['resumeToken'] : null;
      telemetry.assignmentReclaimed = message['reclaimed'] === true;
      maybeReady();
      return;
    }
    if (message['type'] === 'reclaimResult') {
      pushBounded(telemetry.reclaimResults, {
        reclaimed: message['reclaimed'] === true,
        reason: typeof message['reason'] === 'string' ? message['reason'] : 'unknown'
      });
      return;
    }
    if (message['type'] !== 'sensors') return;
    const now = performance.now();
    telemetry.latestSensorTick =
      typeof message['tick'] === 'number' ? Math.floor(message['tick']) : telemetry.latestSensorTick;
    telemetry.sensorCountTotal++;
    pushBounded(telemetry.sensorTimesMs, now);
    if (
      telemetry.lastActionAtMs !== null &&
      telemetry.lastActionAtMs !== telemetry.lastObservedActionAtMs
    ) {
      pushBounded(telemetry.actionToNextSensorMs, now - telemetry.lastActionAtMs);
      telemetry.lastObservedActionAtMs = telemetry.lastActionAtMs;
    }
    if (clientType === 'bot') {
      const started = performance.now();
      const phase = telemetry.sensorCountTotal;
      sendAction(socket, telemetry, Math.sin(phase * 0.17), phase % 20 < 5 ? 1 : 0);
      pushBounded(telemetry.sensorToActionDispatchMs, performance.now() - started);
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
    statsGenerations: [],
    generationTimes: [],
    errors: [],
    frameCountTotal: 0,
    statsCountTotal: 0,
    closedAtMs: null,
    closeCode: null,
    closeReason: null
  };
  let resolveConnected: (() => void) | null = null;
  let rejectConnected: ((error: Error) => void) | null = null;
  const connected = new Promise<void>((resolve, reject) => {
    resolveConnected = resolve;
    rejectConnected = reject;
  });
  // Each profile awaits only one milestone. Mark both promises handled so an
  // error after the earlier P6 connection milestone cannot become an
  // unhandled rejection; telemetry still fails the completed run below.
  void connected.catch(() => undefined);
  let resolveReady: (() => void) | null = null;
  let rejectReady: ((error: Error) => void) | null = null;
  const ready = new Promise<void>((resolve, reject) => {
    resolveReady = resolve;
    rejectReady = reject;
  });
  void ready.catch(() => undefined);
  const maybeReady = (): void => {
    if (telemetry.frameTimesMs.length > 0 && telemetry.statsTimesMs.length > 0) resolveReady?.();
  };
  socket.on('open', () => {
    socket.send(JSON.stringify({ type: 'hello', clientType: 'ui', version: 2 }));
    socket.send(JSON.stringify({ type: 'join', mode: 'spectator' }));
    resolveConnected?.();
  });
  socket.on('error', error => {
    telemetry.errors.push(error.message);
    rejectConnected?.(error);
    rejectReady?.(error);
  });
  socket.on('close', (code, reason) => {
    telemetry.closedAtMs = performance.now();
    telemetry.closeCode = code;
    telemetry.closeReason = reason.toString('utf8');
  });
  socket.on('message', (data: RawData, isBinary: boolean) => {
    const now = performance.now();
    if (isBinary) {
      telemetry.frameCountTotal++;
      pushBounded(telemetry.frameTimesMs, now);
      pushBounded(telemetry.frameBytes, data.byteLength);
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
      telemetry.statsCountTotal++;
      pushBounded(telemetry.statsTimesMs, now);
      if (typeof message['tick'] === 'number') pushBounded(telemetry.statsTicks, message['tick']);
      if (typeof message['gen'] === 'number') pushBounded(telemetry.statsGenerations, message['gen']);
      if (typeof message['generationTime'] === 'number') {
        pushBounded(telemetry.generationTimes, message['generationTime']);
      }
      maybeReady();
    }
  });
  return { socket, telemetry, connected, ready };
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

/** Return an absent sidecar file as zero bytes. */
function optionalFileBytes(filePath: string): number {
  try {
    return fs.statSync(filePath).size;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return 0;
    throw error;
  }
}

/** Count active resource type names when this Node runtime provides the diagnostic API. */
function activeResourceTypes(): Record<string, number> | null {
  const values = process.getActiveResourcesInfo?.();
  if (!values) return null;
  const counts: Record<string, number> = {};
  for (const value of values) counts[value] = (counts[value] ?? 0) + 1;
  return counts;
}

/** Fit a least-squares bytes-per-minute slope over bounded scalar samples. */
export function rssSlopeBytesPerMinute(
  samples: readonly { elapsedMs: number; rssBytes: number }[],
  startElapsedMs: number
): number | null {
  const window = samples.filter(sample => sample.elapsedMs >= startElapsedMs);
  if (window.length < 2) return null;
  const meanX = window.reduce((sum, sample) => sum + sample.elapsedMs / 60_000, 0) / window.length;
  const meanY = window.reduce((sum, sample) => sum + sample.rssBytes, 0) / window.length;
  let numerator = 0;
  let denominator = 0;
  for (const sample of window) {
    const x = sample.elapsedMs / 60_000 - meanX;
    numerator += x * (sample.rssBytes - meanY);
    denominator += x * x;
  }
  return denominator === 0 ? null : numerator / denominator;
}

/** Pause without measuring any tool/permission wait outside this already-running server fixture. */
function sleep(milliseconds: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, milliseconds));
}

/** Perform one current-reference legacy save with a bounded wall-time deadline. */
async function requestLegacySave(
  server: RunningServer,
  timeoutMs = HEALTH_REQUEST_TIMEOUT_MS
): Promise<{
  status: number;
  responseOk: boolean;
  body: { ok?: boolean; snapshotId?: number; message?: string };
}> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(
      `http://127.0.0.1:${server.port}/api/save`,
      { method: 'POST', signal: controller.signal }
    );
    const body = await response.json() as {
      ok?: boolean;
      snapshotId?: number;
      message?: string;
    };
    return { status: response.status, responseOk: response.ok, body };
  } catch (error) {
    if (controller.signal.aborted) {
      throw new Error(`P7 legacy save timed out after ${timeoutMs} ms`, { cause: error });
    }
    throw error;
  } finally {
    clearTimeout(timeout);
  }
}

/**
 * Create one disposable exact run-start checkpoint.
 * @param databasePath - New SQLite file.
 * @param scenarioName - Named workload.
 * @returns Installed scenario description.
 */
function createFixtureDatabase(
  databasePath: string,
  scenarioName: ExternalScenario,
  simSpeed: ExternalControlOptions['simSpeed'],
  profile: ExternalControlOptions['profile']
): ReturnType<typeof installStage2Scenario> {
  const scenario = installExternalControlScenario(scenarioName, simSpeed);
  let checkpoint: PopulationCheckpoint | null = null;
  new World(scenario.settings, {
    seed: STAGE2_WORLD_SEED,
    runId: `stage2-${profile}-${scenarioName.toLowerCase()}-${simSpeed}x`,
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
 * Read one health snapshot with an abortable wall-time bound.
 * @param server - Running server.
 * @param timeoutMs - Maximum wall time allowed for the request.
 * @returns Parsed health result.
 */
export async function readHealth(
  server: RunningServer,
  timeoutMs = HEALTH_REQUEST_TIMEOUT_MS
): Promise<HealthSnapshot> {
  const boundedTimeoutMs = Math.max(1, timeoutMs);
  const controller = new AbortController();
  let timedOut = false;
  const timeout = setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, boundedTimeoutMs);
  try {
    const response = await fetch(`http://127.0.0.1:${server.port}/health`, {
      signal: controller.signal
    });
    if (!response.ok) throw new Error(`health returned HTTP ${response.status}`);
    return await response.json() as HealthSnapshot;
  } catch (error) {
    if (timedOut) throw new HealthRequestTimeoutError(boundedTimeoutMs, error);
    throw error;
  } finally {
    clearTimeout(timeout);
  }
}

/**
 * Choose the next bounded health-poll delay for an authoritative tick wait.
 * @param remainingSteps - Steps still required to reach the boundary.
 * @param tickRateHz - Fixed-step rate.
 * @param requestedMultiplier - Requested scheduler multiplier.
 * @returns Delay in milliseconds, bounded to avoid either busy polling or a long overshoot.
 */
export function tickBoundaryPollDelayMs(
  remainingSteps: number,
  tickRateHz: number,
  requestedMultiplier: number
): number {
  const safeRemaining = Math.max(0, Number.isFinite(remainingSteps) ? remainingSteps : 0);
  const safeTickRate = Math.max(1, Number.isFinite(tickRateHz) ? tickRateHz : 1);
  const safeMultiplier = Math.max(
    0.01,
    Number.isFinite(requestedMultiplier) ? requestedMultiplier : 1
  );
  const idealRemainingMs = (safeRemaining / safeTickRate / safeMultiplier) * 1_000;
  return Math.max(25, Math.min(1_000, idealRemainingMs * 0.8));
}

/**
 * Wait until the real server reports an authoritative tick at or beyond one boundary.
 * @param server - Running real server.
 * @param targetTick - Inclusive authoritative target tick.
 * @param tickRateHz - Fixed-step rate.
 * @param requestedMultiplier - Requested scheduler multiplier used only to pace polling.
 * @param initialHealth - Optional already-read starting health.
 * @returns Boundary health plus polling and overshoot accounting.
 */
async function waitForTickBoundary(
  server: RunningServer,
  targetTick: number,
  tickRateHz: number,
  requestedMultiplier: number,
  initialHealth?: HealthSnapshot
): Promise<TickBoundaryWait> {
  const startedAt = performance.now();
  const estimatedInitialTick = initialHealth?.tick ?? 0;
  const initialRemaining = Math.max(0, targetTick - estimatedInitialTick);
  const timeoutMs = Math.min(
    600_000,
    Math.max(60_000, (initialRemaining / Math.max(1, tickRateHz)) * 20_000)
  );
  const deadlineAt = startedAt + timeoutMs;
  let health: HealthSnapshot;
  try {
    health = initialHealth ?? await readHealth(server, deadlineAt - performance.now());
  } catch (error) {
    if (error instanceof HealthRequestTimeoutError) {
      throw new Error(
        `timed out waiting for authoritative tick ${targetTick}; latest tick unavailable ` +
        'because the initial health request did not complete',
        { cause: error }
      );
    }
    throw error;
  }
  let pollCount = initialHealth ? 0 : 1;
  while (health.tick < targetTick) {
    if (health.fault.faulted) {
      throw new Error(`simulation faulted while waiting for tick ${targetTick}: ${health.fault.reason ?? 'unknown'}`);
    }
    const remainingBeforeSleepMs = deadlineAt - performance.now();
    if (remainingBeforeSleepMs <= 0) {
      throw new Error(
        `timed out waiting for authoritative tick ${targetTick}; latest tick ${health.tick}`
      );
    }
    const delayMs = tickBoundaryPollDelayMs(
      targetTick - health.tick,
      tickRateHz,
      requestedMultiplier
    );
    await new Promise(resolve => setTimeout(resolve, Math.min(delayMs, remainingBeforeSleepMs)));
    const remainingRequestMs = deadlineAt - performance.now();
    if (remainingRequestMs <= 0) {
      throw new Error(
        `timed out waiting for authoritative tick ${targetTick}; latest tick ${health.tick}`
      );
    }
    try {
      health = await readHealth(server, remainingRequestMs);
    } catch (error) {
      if (error instanceof HealthRequestTimeoutError) {
        throw new Error(
          `timed out waiting for authoritative tick ${targetTick}; latest tick ${health.tick}; ` +
          'the next health request did not complete before the overall deadline',
          { cause: error }
        );
      }
      throw error;
    }
    pollCount++;
  }
  return {
    targetTick,
    observedTick: health.tick,
    overshootSteps: health.tick - targetTick,
    pollCount,
    wallMs: Number((performance.now() - startedAt).toFixed(6)),
    health
  };
}

/**
 * Run the real-server external-control baseline.
 * @param options - Benchmark options.
 * @returns Evidence object.
 */
export async function runExternalControlBaseline(
  options: ExternalControlOptions
): Promise<Record<string, unknown>> {
  const provenance = captureEnvironmentProvenance(options.evidenceEnvironment);
  if (options.profile === 'p7') return runP7Soak(options, provenance);
  const temporaryRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-external-'));
  const resolvedTemporaryRoot = path.resolve(temporaryRoot);
  const resolvedSystemTemp = path.resolve(os.tmpdir());
  if (
    path.dirname(resolvedTemporaryRoot) !== resolvedSystemTemp ||
    !path.basename(resolvedTemporaryRoot).startsWith('slither-stage2-external-')
  ) {
    throw new Error(`Unexpected temporary path: ${resolvedTemporaryRoot}`);
  }
  const databasePath = path.join(temporaryRoot, 'fixture.db');
  let server: RunningServer | null = null;
  const sockets: WebSocket[] = [];
  let primaryError: unknown = null;
  const cleanupErrors: unknown[] = [];
  let evidence: Record<string, unknown> | null = null;
  try {
    const scenario = createFixtureDatabase(
      databasePath,
      options.scenario,
      options.simSpeed,
      options.profile
    );
    const config: ServerConfig = {
      ...DEFAULT_CONFIG,
      host: '127.0.0.1',
      port: 0,
      dbPath: databasePath,
      checkpointEveryGenerations: options.checkpointEvery,
      logLevel: 'error',
      inferenceBackend: 'native',
      mtEnabled: options.workers > 0,
      mtWorkers: options.workers,
      resume: 'latest'
    };
    server = await startServer(config);
    const player = options.profile === 'p5'
      ? openController(server.wsUrl, 'ui', 'stage2-browser-player')
      : null;
    const bot = openController(server.wsUrl, 'bot', 'stage2-protocol2-bot');
    const viewer = options.profile === 'p5' || options.viewer ? openViewer(server.wsUrl) : null;
    sockets.push(bot.socket);
    if (player) sockets.push(player.socket);
    if (viewer) sockets.push(viewer.socket);
    await withTimeout(
      Promise.all([
        bot.ready,
        ...(player ? [player.ready] : []),
        ...(viewer
          ? [viewerWarmupReadiness(options.profile) === 'connected'
              ? viewer.connected
              : viewer.ready]
          : [])
      ]),
      30_000,
      'external clients'
    );
    const composition = externalControlComposition(options);
    let warmupBoundary: TickBoundaryWait | null = null;
    let readinessHealth: HealthSnapshot;
    if (options.profile === 'p6') {
      if (options.warmupTick === null) throw new Error('P6 warm-up tick is missing');
      const connectedHealth = await readHealth(server);
      warmupBoundary = await waitForTickBoundary(
        server,
        options.warmupTick,
        config.tickRateHz,
        options.simSpeed,
        connectedHealth
      );
      readinessHealth = warmupBoundary.health;
    } else {
      if (options.warmupMs === null) throw new Error('P5 warm-up duration is missing');
      if (options.warmupMs > 0) {
        await new Promise(resolve => setTimeout(resolve, options.warmupMs ?? 0));
      }
      readinessHealth = await readHealth(server);
    }
    if (readinessHealth.clients !== composition.totalSockets) {
      throw new Error(
        `${options.profile.toUpperCase()} client composition mismatch: expected ` +
        `${composition.totalSockets}, got ${readinessHealth.clients}`
      );
    }
    if (player) clearControllerSamples(player.telemetry);
    clearControllerSamples(bot.telemetry);
    if (viewer) clearViewerSamples(viewer.telemetry);
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
    const playerTimer = player ? setInterval(() => {
      playerSequence++;
      sendAction(
        player.socket,
        player.telemetry,
        Math.sin(playerSequence * 0.11),
        playerSequence % (options.playerHz * 2) < Math.floor(options.playerHz / 3) ? 1 : 0
      );
    }, 1_000 / options.playerHz) : null;
    const measuredStartedAt = performance.now();
    let healthBefore: HealthSnapshot;
    let healthAfter: HealthSnapshot;
    let measuredWallMs: number;
    let measurementBoundary: TickBoundaryWait | null = null;
    try {
      // P6 reuses the just-observed polled warm-up sample, so no additional
      // asynchronous health request shifts the recorded start. The poll may
      // already be beyond its target, and the recorded load remains evidence.
      healthBefore = options.profile === 'p6'
        ? readinessHealth
        : await readHealth(server);
      if (healthBefore.clients !== composition.totalSockets) {
        throw new Error(
          `${options.profile.toUpperCase()} client composition changed before measurement: ` +
          `expected ${composition.totalSockets}, got ${healthBefore.clients}`
        );
      }
      if (healthBefore.persistence.checkpointEveryGenerations !== options.checkpointEvery) {
        throw new Error(
          `persistence checkpoint interval mismatch: expected ${options.checkpointEvery}, got ` +
          `${healthBefore.persistence.checkpointEveryGenerations}`
        );
      }
      if (options.profile === 'p6') {
        if (options.measurementSteps === null) {
          throw new Error('P6 measurement step count is missing');
        }
        measurementBoundary = await waitForTickBoundary(
          server,
          healthBefore.tick + options.measurementSteps,
          config.tickRateHz,
          options.simSpeed,
          healthBefore
        );
        healthAfter = measurementBoundary.health;
      } else {
        if (options.durationMs === null) throw new Error('P5 measurement duration is missing');
        await new Promise(resolve => setTimeout(resolve, options.durationMs ?? 0));
        healthAfter = await readHealth(server);
      }
      measuredWallMs = performance.now() - measuredStartedAt;
    } finally {
      if (playerTimer) clearInterval(playerTimer);
      clearInterval(memoryTimer);
      eventLoop.disable();
    }
    const cpu = process.cpuUsage(cpuBefore);
    const memoryAfter = process.memoryUsage();

    const clientErrors = [
      ...(player ? player.telemetry.errors.map(error => `player: ${error}`) : []),
      ...bot.telemetry.errors.map(error => `bot: ${error}`),
      ...(viewer ? viewer.telemetry.errors.map(error => `viewer: ${error}`) : [])
    ];
    if (clientErrors.length > 0) throw new Error(clientErrors.join('; '));
    if (healthAfter.fault.faulted) {
      throw new Error(`simulation faulted: ${healthAfter.fault.reason ?? 'unknown'}`);
    }
    const scheduler = schedulerDelta(
      healthBefore,
      healthAfter,
      options.simSpeed,
      config.tickRateHz
    );
    if (
      options.profile === 'p6' &&
      options.measurementSteps !== null &&
      scheduler.completedSteps < options.measurementSteps
    ) {
      throw new Error(
        `P6 minimum-step window ended early: requested ${options.measurementSteps}, ` +
        `observed ${scheduler.completedSteps}`
      );
    }
    if (healthAfter.clients !== composition.totalSockets) {
      throw new Error(
        `${options.profile.toUpperCase()} client composition changed during measurement: ` +
        `expected ${composition.totalSockets}, got ${healthAfter.clients}`
      );
    }
    if (healthAfter.persistence.checkpointEveryGenerations !== options.checkpointEvery) {
      throw new Error(
        `persistence checkpoint interval changed during measurement: expected ` +
        `${options.checkpointEvery}, got ` +
        `${healthAfter.persistence.checkpointEveryGenerations}`
      );
    }
    if (viewer && (viewer.telemetry.frameTimesMs.length === 0 || viewer.telemetry.statsTimesMs.length === 0)) {
      throw new Error('viewer-enabled run did not receive both a frame and stats sample');
    }
    const viewerGenerationProgressSeconds = viewer &&
      viewer.telemetry.generationTimes.length > 1 &&
      viewer.telemetry.generationTimes.length === viewer.telemetry.statsGenerations.length &&
      new Set(viewer.telemetry.statsGenerations).size === 1
      ? viewer.telemetry.generationTimes.at(-1)! - viewer.telemetry.generationTimes[0]!
      : null;
    const checkpointPersistenceExcluded = options.checkpointEvery === 0 ||
      (options.profile === 'p5' && options.checkpointEvery === 1_000_000);
    const measuredWallSeconds = measuredWallMs / 1_000;
    const monotonicAchievedMultiplier = scheduler.simulatedSeconds / measuredWallSeconds;
    evidence = {
      schema: 'slither-stage2-external-control-baseline',
      version: 4,
      evidenceClass: provenance.ownerTargetVmValidated
        ? 'new measured target-VM current-server result'
        : 'new measured development-machine current-server result',
      caveat: provenance.ownerTargetVmValidated
        ? 'Real current server measured on the oxygen Ryzen 7 2700 Debian VM with Protocol 2 wire-compatible loopback clients. Those synthetic clients are not the owner trainer, a browser renderer, or another LAN device.'
        : 'Real current server and Protocol 2 wire-compatible bot, but loopback Node clients are not the owner trainer, a browser renderer, another LAN device, or the target Debian VM.',
      source: sourceIdentity(),
      environment: {
        capturedAt: new Date().toISOString(),
        platform: process.platform,
        architecture: process.arch,
        osType: os.type(),
        osRelease: os.release(),
        osVersion: os.version(),
        hostname: os.hostname(),
        provenance,
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
        profile: options.profile,
        composition,
        playerHz: player ? options.playerHz : null,
        requestedSimSpeed: options.simSpeed,
        warmupMs: options.warmupMs,
        warmupTick: options.warmupTick,
        requestedDurationMs: options.durationMs,
        requestedMeasurementSteps: options.measurementSteps,
        measurementMode: options.profile === 'p6'
          ? 'minimum-polled-authoritative-steps'
          : 'runner-monotonic-wall-time',
        requestedWorkers: options.workers,
        displayPublicationHz: config.uiFrameRateHz,
        botClient: 'Protocol 2 observation-driven synthetic compatibility client',
        viewerClient: viewer
          ? viewerWarmupReadiness(options.profile) === 'connected'
            ? 'one Protocol 2 UI spectator; measurement readiness waits only for its open ' +
              'hello/join, and first frame/stats delivery is not a P6 warm-up condition'
            : 'one Protocol 2 UI spectator receiving complete frame v1 before P5 warm-up'
          : 'disabled; no UI player or spectator socket',
        checkpointPersistence: {
          included: !checkpointPersistenceExcluded,
          configuredGenerationInterval: config.checkpointEveryGenerations,
          reason: checkpointPersistenceExcluded
            ? options.profile === 'p5' && options.checkpointEvery === 1_000_000
              ? 'Legacy P5 compatibility baseline uses its historical large persistence-excluded interval.'
              : `Explicit ${options.profile.toUpperCase()} persistence-excluded diagnostic; it is not the primary retained P6 matrix.`
            : options.profile === 'p6'
              ? 'Primary P6 matrix keeps ordinary automatic checkpoints enabled every generation. ' +
                'persistenceProgress records whether this measured window actually crossed a ' +
                'generation boundary and published a later durable checkpoint.'
              : 'P5 compatibility run uses the explicitly requested automatic checkpoint interval.'
        },
        actionAcceptanceObservability: {
          configuredActionsPerSecondCap: config.maxActionsPerSecond,
          caveat:
            'Observed Protocol 2 observations/actions are client sends, not proof that ControllerRegistry accepted or applied every action once sends approach or exceed the cap; the current health endpoint exposes no accepted/applied action counters.'
        },
        diagnosticInstrumentation: {
          worldLoadCache:
            'The measured server performs one O(snakes) scalar world-load scan at each ' +
            'committed step and another after each completed pump. That diagnostic cost is ' +
            'included in these results and is not assumed to be free.'
        },
        crossRunComparability: {
          alignment:
            options.profile === 'p6'
              ? 'Runs use one absolute warm-up tick target and require at least the requested ' +
                'number of observed authoritative steps. Actual start/end ticks, poll overshoot, ' +
                'and world load are recorded for every run.'
              : 'P5 is a historical wall-time compatibility profile, not a tick-aligned comparison.',
          caveat:
            options.profile === 'p6'
              ? 'Independent launches can diverge before and during the sampled window because ' +
                'external join and action delivery use wall/event-loop timing, viewer publication ' +
                'changes scheduling, current external joins advance authoritative RNG, and health ' +
                'polls observe only at-or-beyond boundaries. Clearing client telemetry cannot prove ' +
                'that no pre-warm-up sensor or frame was already queued. The minimum-step window ' +
                'normalizes requested simulated progress; it does not make trajectories, loads, or ' +
                'client action timing identical.'
              : 'P5 retains its historical client-readiness and wall-time behavior.'
        }
      },
      result: {
        measuredWallMs: Number(measuredWallMs.toFixed(6)),
        tickWindow: {
          mode: options.profile === 'p6'
            ? 'minimum-polled-authoritative-steps'
            : 'runner-monotonic-wall-time',
          warmupBoundary: warmupBoundary
            ? {
                targetTick: warmupBoundary.targetTick,
                observedTick: warmupBoundary.observedTick,
                overshootSteps: warmupBoundary.overshootSteps,
                healthPollCount: warmupBoundary.pollCount,
                wallMs: warmupBoundary.wallMs,
                worldLoad: warmupBoundary.health.worldLoad,
                collisionGridEntries:
                  warmupBoundary.health.collisionGrid['currentEntries'] ?? null
              }
            : null,
          measurementStartTick: healthBefore.tick,
          measurementTargetTick: measurementBoundary?.targetTick ?? null,
          measurementEndTick: healthAfter.tick,
          requestedMeasurementSteps: options.measurementSteps,
          observedMeasurementSteps: scheduler.completedSteps,
          measurementOvershootSteps: measurementBoundary?.overshootSteps ?? null,
          measurementHealthPollCount: measurementBoundary?.pollCount ?? null,
          minimumStepTargetReached:
            options.profile === 'p6'
              ? options.measurementSteps !== null &&
                scheduler.completedSteps >= options.measurementSteps
              : null
        },
        monotonicWindow: {
          wallSeconds: Number(measuredWallSeconds.toFixed(9)),
          simulatedSecondsPerWallSecond: Number(monotonicAchievedMultiplier.toFixed(6)),
          achievedToRequestedRatio: Number(
            (monotonicAchievedMultiplier / options.simSpeed).toFixed(6)
          )
        },
        schedulerDelta: {
          tick: scheduler.tick,
          completedSteps: scheduler.completedSteps,
          wallSeconds: Number(scheduler.wallSeconds.toFixed(9)),
          simulatedSeconds: Number(scheduler.simulatedSeconds.toFixed(9)),
          droppedSimulationSeconds: Number(scheduler.droppedSimulationSeconds.toFixed(9)),
          achievedMultiplier:
            scheduler.achievedMultiplier === null
              ? null
              : Number(scheduler.achievedMultiplier.toFixed(6)),
          achievedToRequestedRatio:
            scheduler.achievedToRequestedRatio === null
              ? null
              : Number(scheduler.achievedToRequestedRatio.toFixed(6)),
          wallCounterCaveat:
            scheduler.wallSeconds === 0
              ? 'Both health samples occurred inside one awaited scheduler pump. Tick and ' +
                'simulated-time deltas remain exact, but the pump charged wall time before ' +
                'the first sample; use monotonicWindow for interval speed.'
              : null
        },
        persistenceProgress: {
          inMemoryGenerationBefore: healthBefore.persistence.inMemoryGeneration,
          inMemoryGenerationAfter: healthAfter.persistence.inMemoryGeneration,
          generationsAdvanced:
            healthAfter.persistence.inMemoryGeneration -
            healthBefore.persistence.inMemoryGeneration,
          durableGenerationBefore: healthBefore.persistence.lastDurableGeneration,
          durableGenerationAfter: healthAfter.persistence.lastDurableGeneration,
          durableSnapshotIdBefore: healthBefore.persistence.lastDurableSnapshotId,
          durableSnapshotIdAfter: healthAfter.persistence.lastDurableSnapshotId,
          durableCheckpointAdvanced:
            healthBefore.persistence.lastDurableSnapshotId !== null &&
            healthAfter.persistence.lastDurableSnapshotId !== null &&
            healthAfter.persistence.lastDurableSnapshotId >
              healthBefore.persistence.lastDurableSnapshotId,
          durableGenerationCaughtUp:
            healthAfter.persistence.lastDurableGeneration ===
            healthAfter.persistence.inMemoryGeneration
        },
        ...(player ? {
          viewerGenerationProgressSeconds,
          player: {
            assignmentLatencyMs:
              player.telemetry.assignedAtMs === null
                ? null
                : Number((player.telemetry.assignedAtMs - player.telemetry.openedAtMs).toFixed(6)),
            actionCount: player.telemetry.actionCountTotal,
            sensorCount: player.telemetry.sensorCountTotal,
            actionIntervalMs: distribution(player.telemetry.actionIntervalsMs),
            sensorIntervalMs: distribution(intervals(player.telemetry.sensorTimesMs)),
            actionToNextSensorMs: distribution(player.telemetry.actionToNextSensorMs)
          }
        } : {}),
        bot: {
          assignmentLatencyMs:
            bot.telemetry.assignedAtMs === null
              ? null
              : Number((bot.telemetry.assignedAtMs - bot.telemetry.openedAtMs).toFixed(6)),
          actionCount: bot.telemetry.actionCountTotal,
          sensorCount: bot.telemetry.sensorCountTotal,
          observationsPerWallSecond: Number(
            (bot.telemetry.sensorCountTotal / measuredWallSeconds).toFixed(6)
          ),
          actionsSentPerWallSecond: Number(
            (bot.telemetry.actionCountTotal / measuredWallSeconds).toFixed(6)
          ),
          sensorIntervalMs: distribution(intervals(bot.telemetry.sensorTimesMs)),
          actionToNextSensorMs: distribution(bot.telemetry.actionToNextSensorMs),
          sensorToActionDispatchMs: distribution(bot.telemetry.sensorToActionDispatchMs)
        },
        viewer: viewer ? {
          enabled: true,
          uiSocketConnected: true,
          frameCount: viewer.telemetry.frameCountTotal,
          frameIntervalMs: distribution(intervals(viewer.telemetry.frameTimesMs)),
          frameBytes: distribution(viewer.telemetry.frameBytes),
          statsCount: viewer.telemetry.statsCountTotal,
          statsIntervalMs: distribution(intervals(viewer.telemetry.statsTimesMs))
        } : {
          enabled: false,
          uiSocketConnected: false,
          frameCount: 0,
          statsCount: 0
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
  } catch (error) {
    primaryError = error;
  } finally {
    if (server) {
      try {
        await server.close();
      } catch (error) {
        cleanupErrors.push(error);
      }
    }
    const socketResults = await Promise.allSettled(sockets.map(closeSocket));
    for (const result of socketResults) {
      if (result.status === 'rejected') cleanupErrors.push(result.reason);
    }
    try {
      resetCFGToDefaults();
    } catch (error) {
      cleanupErrors.push(error);
    }
    try {
      fs.rmSync(resolvedTemporaryRoot, { recursive: true, force: true });
    } catch (error) {
      cleanupErrors.push(error);
    }
  }
  if (primaryError !== null) {
    if (cleanupErrors.length > 0) {
      console.error('[stage2.external-control.cleanup]', cleanupErrors);
    }
    throw primaryError;
  }
  if (cleanupErrors.length > 0) throw cleanupErrors[0];
  if (evidence === null) throw new Error('external-control measurement produced no evidence');
  return evidence;
}

/** Run the bounded current-reference P7 soak without claiming target-VM or production-Rust coverage. */
async function runP7Soak(
  options: ExternalControlOptions,
  provenance: EnvironmentProvenance
): Promise<Record<string, unknown>> {
  const temporaryRoot = path.resolve(fs.mkdtempSync(path.join(os.tmpdir(), 'slither-stage2-p7-')));
  const systemTemp = path.resolve(os.tmpdir());
  if (path.dirname(temporaryRoot) !== systemTemp || !path.basename(temporaryRoot).startsWith('slither-stage2-p7-')) {
    throw new Error(`Unexpected P7 temporary root: ${temporaryRoot}`);
  }
  const databasePath = path.join(temporaryRoot, 'fixture.db');
  const samples: Array<Record<string, unknown>> = [];
  const saves: Array<Record<string, unknown>> = [];
  const reconnects: Array<Record<string, unknown>> = [];
  let server: RunningServer | null = null;
  let player: ControllerHandle | null = null;
  let bot: ControllerHandle | null = null;
  let viewer: ViewerHandle | null = null;
  let outcome: 'completed' | 'faulted' | 'failed' | 'timeout' | 'cleanup-failed' = 'completed';
  let terminalReason: string | null = null;
  let startedAt = 0;
  let measuredWallMs = 0;
  let memoryBefore: NodeJS.MemoryUsage | null = null;
  let eventLoop: ReturnType<typeof monitorEventLoopDelay> | null = null;
  let initialHealth: HealthSnapshot | null = null;
  let finalStorage: Record<string, unknown> | null = null;
  let resourcesAfterCleanup: Record<string, number> | null = null;
  let scenario: ReturnType<typeof installStage2Scenario> | null = null;
  let memoryTimer: NodeJS.Timeout | null = null;
  let peakRssBytes = 0;
  let peakHeapUsedBytes = 0;
  let peakExternalBytes = 0;
  let postWarmupCounts: {
    botSensors: number;
    viewerFrames: number;
    viewerStats: number;
  } | null = null;
  const priorPlayer = { actions: 0, sensors: 0, errors: [] as string[] };
  try {
    scenario = createFixtureDatabase(databasePath, 'P0', 1, 'p7');
    server = await startServer({
      ...DEFAULT_CONFIG,
      host: '127.0.0.1', port: 0, dbPath: databasePath,
      checkpointEveryGenerations: 1, logLevel: 'error', inferenceBackend: 'native',
      mtEnabled: options.workers > 0, mtWorkers: options.workers, resume: 'latest'
    });
    player = openController(server.wsUrl, 'ui', 'stage2-p7-player');
    bot = openController(server.wsUrl, 'bot', 'stage2-p7-bot');
    viewer = openViewer(server.wsUrl);
    await withTimeout(Promise.all([player.ready, bot.ready, viewer.ready]), 30_000, 'P7 clients');
    initialHealth = await readHealth(server);
    if (initialHealth.clients !== p7Composition().totalSockets) {
      throw new Error(
        `P7 client composition mismatch: expected ${p7Composition().totalSockets}, ` +
        `got ${initialHealth.clients}`
      );
    }
    clearControllerSamples(player.telemetry);
    clearControllerSamples(bot.telemetry);
    clearViewerSamples(viewer.telemetry);
    // Timing begins only after local server/client readiness. Tool permission waits cannot enter this window.
    startedAt = performance.now();
    memoryBefore = process.memoryUsage();
    peakRssBytes = memoryBefore.rss;
    peakHeapUsedBytes = memoryBefore.heapUsed;
    peakExternalBytes = memoryBefore.external;
    memoryTimer = setInterval(() => {
      const memory = process.memoryUsage();
      peakRssBytes = Math.max(peakRssBytes, memory.rss);
      peakHeapUsedBytes = Math.max(peakHeapUsedBytes, memory.heapUsed);
      peakExternalBytes = Math.max(peakExternalBytes, memory.external);
    }, 250);
    eventLoop = monitorEventLoopDelay({ resolution: 10 });
    eventLoop.enable();
    let previousCpu = process.cpuUsage();
    let nextSample = 0;
    let nextReconnect = options.reconnectEveryMs;
    let nextSave = options.manualSaveEveryMs;
    let playerSequence = 0;
    let nextAction = 0;
    while (performance.now() - startedAt < (options.durationMs ?? 0)) {
      const elapsedMs = performance.now() - startedAt;
      if (
        postWarmupCounts === null &&
        elapsedMs >= (options.warmupMs ?? 0) &&
        bot &&
        viewer
      ) {
        postWarmupCounts = {
          botSensors: bot.telemetry.sensorCountTotal,
          viewerFrames: viewer.telemetry.frameCountTotal,
          viewerStats: viewer.telemetry.statsCountTotal
        };
      }
      if (elapsedMs >= nextAction && player) {
        playerSequence++;
        sendAction(player.socket, player.telemetry, Math.sin(playerSequence * 0.11), playerSequence % 20 < 6 ? 1 : 0);
        nextAction += 1000 / options.playerHz;
      }
      if (elapsedMs >= nextSample && server) {
        const health = await readHealth(server);
        const memory = process.memoryUsage();
        const currentCpu = process.cpuUsage();
        samples.push({
          elapsedMs: Number(elapsedMs.toFixed(3)), tick: health.tick, health,
          memory, databaseBytes: optionalFileBytes(databasePath), walBytes: optionalFileBytes(`${databasePath}-wal`),
          shmBytes: optionalFileBytes(`${databasePath}-shm`), activeResourceTypes: activeResourceTypes(),
          cpuMicros: {
            user: currentCpu.user - previousCpu.user,
            system: currentCpu.system - previousCpu.system
          },
          eventLoopMs: {
            p50: Number((eventLoop.percentile(50) / 1e6).toFixed(6)),
            p95: Number((eventLoop.percentile(95) / 1e6).toFixed(6)),
            p99: Number((eventLoop.percentile(99) / 1e6).toFixed(6)),
            max: Number((eventLoop.max / 1e6).toFixed(6))
          }
        });
        previousCpu = currentCpu;
        eventLoop.reset();
        nextSample = elapsedMs + options.sampleEveryMs;
        if (health.clients !== p7Composition().totalSockets) {
          outcome = 'failed';
          terminalReason =
            `P7 client count changed: expected ${p7Composition().totalSockets}, ` +
            `got ${health.clients}`;
          break;
        }
        if (
          (player && player.telemetry.closedAtMs !== null) ||
          (bot && bot.telemetry.closedAtMs !== null) ||
          (viewer && viewer.telemetry.closedAtMs !== null)
        ) {
          outcome = 'failed';
          terminalReason = 'P7 observed an unexpected active-client disconnect';
          break;
        }
        if (health.fault.faulted) {
          outcome = 'faulted'; terminalReason = health.fault.reason ?? 'unknown simulation fault'; break;
        }
      }
      if (elapsedMs >= nextReconnect && player && server) {
        const token = player.telemetry.resumeToken;
        const priorSnakeId = player.telemetry.snakeId;
        const oldPlayer = player;
        priorPlayer.actions += oldPlayer.telemetry.actionCountTotal;
        priorPlayer.sensors += oldPlayer.telemetry.sensorCountTotal;
        priorPlayer.errors.push(...oldPlayer.telemetry.errors);
        await closeSocket(oldPlayer.socket);
        player = openController(server.wsUrl, 'ui', 'stage2-p7-player', token ?? undefined);
        await withTimeout(player.ready, 30_000, 'P7 player reclaim');
        const resultSeen = player.telemetry.reclaimResults.some(result => result.reclaimed);
        const assignmentReclaimed = player.telemetry.assignmentReclaimed;
        const sameSnake = priorSnakeId !== null && player.telemetry.snakeId === priorSnakeId;
        const rotatedToken = token !== null && player.telemetry.resumeToken !== token;
        const reclaimed = resultSeen && assignmentReclaimed && sameSnake && rotatedToken;
        reconnects.push({
          elapsedMs: Number(elapsedMs.toFixed(3)),
          priorTokenPresent: token !== null,
          priorSnakeId,
          assignedSnakeId: player.telemetry.snakeId,
          resultSeen,
          assignmentReclaimed,
          sameSnake,
          rotatedToken,
          reclaimed,
          reclaimResults: player.telemetry.reclaimResults
        });
        if (!reclaimed) {
          outcome = 'failed';
          terminalReason =
            'P7 reconnect did not receive both reclaim confirmations for the same snake and a rotated token';
          break;
        }
        nextReconnect += options.reconnectEveryMs;
      }
      if (elapsedMs >= nextSave && server) {
        const saveStarted = performance.now();
        const save = await requestLegacySave(server);
        saves.push({ elapsedMs: Number(elapsedMs.toFixed(3)), status: save.status, ok: save.body.ok === true,
          snapshotId: save.body.snapshotId ?? null, message: save.body.message ?? null,
          durationMs: Number((performance.now() - saveStarted).toFixed(6)),
          behavior: 'legacy current-reference non-resumable population save; not future pin/export behavior' });
        if (!save.responseOk || save.body.ok !== true) {
          outcome = 'failed';
          terminalReason =
            `P7 legacy save failed: ${save.body.message ?? save.status}`;
          break;
        }
        if (
          !Number.isSafeInteger(save.body.snapshotId) ||
          Number(save.body.snapshotId) < 1
        ) {
          outcome = 'failed';
          terminalReason = 'P7 legacy save succeeded without a valid snapshot id';
          break;
        }
        nextSave += options.manualSaveEveryMs;
      }
      await sleep(Math.min(20, Math.max(5, Math.floor(500 / options.playerHz))));
    }
    measuredWallMs = performance.now() - startedAt;
    if (outcome === 'completed' && server) {
      const finalHealth = await readHealth(server);
      if (
        finalHealth.clients !== p7Composition().totalSockets ||
        finalHealth.fault.faulted ||
        (player && player.telemetry.closedAtMs !== null) ||
        (bot && bot.telemetry.closedAtMs !== null) ||
        (viewer && viewer.telemetry.closedAtMs !== null)
      ) {
        outcome = finalHealth.fault.faulted ? 'faulted' : 'failed';
        terminalReason = finalHealth.fault.faulted
          ? finalHealth.fault.reason ?? 'unknown simulation fault'
          : 'P7 final liveness check found a disconnected external client';
      }
    }
    const clientErrors = [
      ...priorPlayer.errors.map(error => `player: ${error}`),
      ...(player?.telemetry.errors.map(error => `player: ${error}`) ?? []),
      ...(bot?.telemetry.errors.map(error => `bot: ${error}`) ?? []),
      ...(viewer?.telemetry.errors.map(error => `viewer: ${error}`) ?? [])
    ];
    if (outcome === 'completed' && clientErrors.length > 0) {
      outcome = 'failed';
      terminalReason = clientErrors.join('; ');
    }
    if (
      outcome === 'completed' &&
      (!viewer || viewer.telemetry.frameCountTotal === 0 || viewer.telemetry.statsCountTotal === 0)
    ) {
      outcome = 'failed';
      terminalReason = 'P7 spectator did not retain both frame and stats liveness';
    }
    if (outcome === 'completed' && (saves.length === 0 || reconnects.length === 0)) {
      outcome = 'failed';
      terminalReason = 'P7 completed without the required save and reconnect events';
    }
    if (outcome === 'completed' && samples.length < 2) {
      outcome = 'failed';
      terminalReason = 'P7 completed without enough bounded scalar samples';
    }
    if (
      outcome === 'completed' &&
      (
        postWarmupCounts === null ||
        !bot ||
        !viewer ||
        bot.telemetry.sensorCountTotal <= postWarmupCounts.botSensors ||
        viewer.telemetry.frameCountTotal <= postWarmupCounts.viewerFrames ||
        viewer.telemetry.statsCountTotal <= postWarmupCounts.viewerStats
      )
    ) {
      outcome = 'failed';
      terminalReason =
        'P7 bot observations and viewer frame/stats streams did not all progress after warm-up';
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    outcome = /timed out|timeout/iu.test(message) ? 'timeout' : 'failed';
    terminalReason = message;
  } finally {
    if (startedAt > 0 && measuredWallMs === 0) measuredWallMs = performance.now() - startedAt;
    if (memoryTimer) clearInterval(memoryTimer);
    if (eventLoop) eventLoop.disable();
    const closeResults: PromiseSettledResult<unknown>[] = [];
    if (server) closeResults.push(await server.close().then(() => ({ status: 'fulfilled', value: undefined } as const), reason => ({ status: 'rejected', reason } as const)));
    closeResults.push(...await Promise.allSettled([
      ...(player ? [closeSocket(player.socket)] : []), ...(bot ? [closeSocket(bot.socket)] : []),
      ...(viewer ? [closeSocket(viewer.socket)] : [])
    ]));
    const cleanupFailure = closeResults.find(result => result.status === 'rejected');
    if (cleanupFailure) { outcome = 'cleanup-failed'; terminalReason ??= String((cleanupFailure as PromiseRejectedResult).reason); }
    try {
      const databaseBytes = optionalFileBytes(databasePath);
      const walBytes = optionalFileBytes(`${databasePath}-wal`);
      const shmBytes = optionalFileBytes(`${databasePath}-shm`);
      if (databaseBytes > 0) {
        const database = new Database(databasePath, { readonly: true, fileMustExist: true });
        try {
          finalStorage = {
            databaseBytes,
            walBytes,
            shmBytes,
            pageCount: database.pragma('page_count', { simple: true }) as number,
            freelistCount: database.pragma('freelist_count', { simple: true }) as number,
            snapshotCount: (
              database.prepare('SELECT COUNT(*) AS count FROM population_snapshots').get() as
                { count: number }
            ).count,
            genomeRowCount: (
              database.prepare('SELECT COUNT(*) AS count FROM snapshot_genomes').get() as
                { count: number }
            ).count
          };
          if (outcome === 'completed') {
            const snapshotCount = Number(finalStorage['snapshotCount']);
            const genomeRowCount = Number(finalStorage['genomeRowCount']);
            const expectedSnapshots = 1 + saves.length;
            const expectedGenomeRows =
              expectedSnapshots * (scenario?.settings.snakeCount ?? 0);
            if (
              snapshotCount < expectedSnapshots ||
              genomeRowCount < expectedGenomeRows
            ) {
              outcome = 'failed';
              terminalReason =
                'P7 final SQLite counts do not contain the run-start plus every ' +
                'successful manual save';
            }
          }
        } finally {
          database.close();
        }
      }
    } catch (error) {
      outcome = 'cleanup-failed';
      terminalReason ??=
        `P7 final database inspection failed: ${error instanceof Error ? error.message : String(error)}`;
    }
    try {
      resetCFGToDefaults();
    } catch (error) {
      outcome = 'cleanup-failed';
      terminalReason ??=
        `P7 config reset failed: ${error instanceof Error ? error.message : String(error)}`;
    }
    try {
      fs.rmSync(temporaryRoot, { recursive: true, force: true });
    } catch (error) {
      outcome = 'cleanup-failed';
      terminalReason ??=
        `P7 temporary cleanup failed: ${error instanceof Error ? error.message : String(error)}`;
    }
    resourcesAfterCleanup = activeResourceTypes();
  }
  const memoryAfter = process.memoryUsage();
  const rssSamples = samples.map(sample => {
    const memory = sample['memory'] as NodeJS.MemoryUsage;
    return { elapsedMs: sample['elapsedMs'] as number, rssBytes: memory.rss };
  });
  const durationMs = options.durationMs ?? 0;
  const warmupMs = options.warmupMs ?? 0;
  const warmRss = rssSamples
    .filter(sample => sample.elapsedMs >= warmupMs)
    .map(sample => sample.rssBytes)
    .sort((left, right) => left - right);
  const warmRssMedian = warmRss.length === 0
    ? null
    : warmRss.length % 2 === 1
      ? warmRss[(warmRss.length - 1) / 2]!
      : (warmRss[warmRss.length / 2 - 1]! + warmRss[warmRss.length / 2]!) / 2;
  const finalRssMinusWarmMedian = warmRssMedian === null
    ? null
    : memoryAfter.rss - warmRssMedian;
  const sampledOutboundMaxima = {
    reliableQueuedMessages: Math.max(
      0,
      ...samples.map(sample => Number(
        ((sample['health'] as HealthSnapshot).outbound['reliableQueuedMessages']) ?? 0
      ))
    ),
    reliableQueuedBytes: Math.max(
      0,
      ...samples.map(sample => Number(
        ((sample['health'] as HealthSnapshot).outbound['reliableQueuedBytes']) ?? 0
      ))
    ),
    pendingFrames: Math.max(
      0,
      ...samples.map(sample => Number(
        ((sample['health'] as HealthSnapshot).outbound['pendingFrames']) ?? 0
      ))
    )
  };
  const playerActions =
    priorPlayer.actions + (player?.telemetry.actionCountTotal ?? 0);
  const playerSensors =
    priorPlayer.sensors + (player?.telemetry.sensorCountTotal ?? 0);
  const firstPersistence = initialHealth?.persistence ?? null;
  const lastPersistence =
    samples.length > 0
      ? (samples.at(-1)!['health'] as HealthSnapshot).persistence
      : firstPersistence;
  return {
    schema: 'slither-stage2-p7-current-server-soak', version: 2,
    evidenceClass: options.p7TestOnlyShort
      ? 'test-only short diagnostic'
      : provenance.ownerTargetVmValidated
        ? 'new measured target-VM current-server soak result'
        : 'new measured development-machine current-server soak result',
    caveat: provenance.ownerTargetVmValidated
      ? 'Current TypeScript server plus loopback synthetic clients measured on the oxygen Ryzen 7 2700 Debian VM. Combined runner/server memory, sampled queue maxima, and current SQLite saves are not Rust, LAN browser, owner trainer, managed retention, or production persistence evidence.'
      : 'Current TypeScript server plus loopback synthetic clients in one process. Combined runner/server memory, sampled queue maxima, and current SQLite saves are not Rust, LAN browser, owner trainer, target-VM, retention, or production persistence evidence.',
    source: sourceIdentity(),
    environment: {
      capturedAt: new Date().toISOString(),
      platform: process.platform,
      architecture: process.arch,
      osRelease: os.release(),
      provenance,
      node: process.version,
      cpuModel: os.cpus()[0]?.model ?? 'unknown',
      logicalCpuCount: os.cpus().length,
      totalMemoryBytes: os.totalmem()
    },
    outcome,
    terminalReason,
    workload: { profile: 'p7', scenario, simSpeed: 1, durationMs, warmupMs, sampleEveryMs: options.sampleEveryMs,
      reconnectEveryMs: options.reconnectEveryMs,
      manualSaveEveryMs: options.manualSaveEveryMs,
      playerHz: options.playerHz,
      testOnlyShort: options.p7TestOnlyShort,
      legacyManualSave: true },
    result: { samples, saves, reconnects, memoryBefore, memoryAfter,
      measuredWallMs: Number(measuredWallMs.toFixed(6)),
      sampledMemoryPeaks: {
        cadenceMs: 250,
        rssBytes: peakRssBytes,
        heapUsedBytes: peakHeapUsedBytes,
        externalBytes: peakExternalBytes,
        caveat:
          'Timer samples can still miss shorter spikes; the values include runner and ' +
          'in-process current server memory.'
      },
      sampleCountBound: Math.ceil(durationMs / options.sampleEveryMs) + 2,
      sampledQueueMaximumCaveat:
        'Queue depth maxima are only periodic health samples. Replacement/failure counters are ' +
        'connection-lifetime values for connections still active at each sample, so reconnecting ' +
        'a client can remove its earlier counters from later aggregate health values.',
      sampledOutboundMaxima,
      rssSlopeBytesPerMinuteAfterWarmup: rssSlopeBytesPerMinute(rssSamples, warmupMs),
      finalRssMinusWarmWindowMedianBytes: finalRssMinusWarmMedian,
      controllers: {
        playerActionsSent: playerActions,
        playerSensorsReceived: playerSensors,
        botActionsSent: bot?.telemetry.actionCountTotal ?? 0,
        botSensorsReceived: bot?.telemetry.sensorCountTotal ?? 0,
        viewerFramesReceived: viewer?.telemetry.frameCountTotal ?? 0,
        viewerStatsReceived: viewer?.telemetry.statsCountTotal ?? 0,
        caveat:
          'Action counts are client sends, not proof of ControllerRegistry acceptance or fixed-step application.'
      },
      persistenceProgress: {
        before: firstPersistence,
        after: lastPersistence,
        durableSnapshotAdvanced:
          firstPersistence?.lastDurableSnapshotId !== null &&
          firstPersistence?.lastDurableSnapshotId !== undefined &&
          lastPersistence?.lastDurableSnapshotId !== null &&
          lastPersistence?.lastDurableSnapshotId !== undefined &&
          lastPersistence.lastDurableSnapshotId > firstPersistence.lastDurableSnapshotId
      },
      finalStorage,
      resourcesAfterCleanup,
      fullP7SoakEligible:
        !options.p7TestOnlyShort &&
        durationMs >= P7_MIN_DURATION_MS &&
        measuredWallMs >= P7_MIN_DURATION_MS &&
        warmupMs === P7_WARMUP_MS &&
        samples.length >= 2 }
  };
}

/** Execute the CLI. */
async function main(): Promise<void> {
  const options = parseOptions(process.argv.slice(2));
  const result = await runExternalControlBaseline(options);
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (options.outputPath) {
    fs.mkdirSync(path.dirname(options.outputPath), { recursive: true });
    try {
      fs.writeFileSync(options.outputPath, json, { encoding: 'utf8', flag: 'wx' });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
        throw new Error(`Refusing to overwrite existing evidence file: ${options.outputPath}`);
      }
      throw error;
    }
    console.info(`[stage2.external-control] wrote ${options.outputPath}`);
  } else {
    process.stdout.write(json);
  }
  if (
    options.profile === 'p7' &&
    (result as { outcome?: unknown }).outcome !== 'completed'
  ) {
    process.exitCode = 1;
  }
}

const invokedPath = process.argv[1];
if (invokedPath && import.meta.url === pathToFileURL(path.resolve(invokedPath)).href) {
  main().catch(error => {
    console.error(error);
    process.exitCode = 1;
  });
}
