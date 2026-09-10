import { monitorEventLoopDelay, performance, type IntervalHistogram } from 'node:perf_hooks';
import type { RustBackgroundDisplay, RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';

/** Inclusive millisecond ceilings for bounded interface-latency histograms. */
const LATENCY_BUCKET_UPPER_MS = [
  0.1, 0.25, 0.5, 1, 2, 4, 8, 16, 32, 64, 125, 250, 500,
  1_000, 2_000, 4_000, 8_000, 15_000, 30_000, 60_000, 120_000, Infinity
] as const;

/** Process values captured without retaining application state. */
export interface ExperimentalProcessTelemetry {
  /** Current process resident set. */
  rssBytes: number;
  /** V8 heap bytes currently in use. */
  heapUsedBytes: number;
  /** Node external allocations, including native buffers. */
  externalBytes: number;
  /** Process high-water resident set reported by Node. */
  maxRssBytes: number;
  /** Mean event-loop delay since server start. */
  eventLoopDelayMeanMs: number;
  /** Event-loop-delay 95th percentile since server start. */
  eventLoopDelayP95Ms: number;
  /** Largest event-loop delay since server start. */
  eventLoopDelayMaxMs: number;
}

/** Rust full-step computation distribution projected into milliseconds. */
export interface ExperimentalStepTelemetry {
  /** Successful fixed-step computations sampled by Rust. */
  samples: number;
  /** Arithmetic mean of sampled computation time. */
  meanMs: number;
  /** Conservative native-histogram upper bound for p95. */
  p95Ms: number;
  /** Conservative native-histogram upper bound for p99. */
  p99Ms: number;
  /** Largest exact sampled computation time. */
  maxMs: number;
  /** Documents the bounded histogram interpretation of p95 and p99. */
  percentilesAreUpperBounds: true;
}

/** One bounded Node-side latency distribution. */
export interface ExperimentalLatencyTelemetry {
  /** Completed measurements. */
  samples: number;
  /** Arithmetic mean duration. */
  meanMs: number;
  /** Conservative inclusive bucket ceiling for p95. */
  p95Ms: number;
  /** Largest exact measured duration. */
  maxMs: number;
}

/** Integrated scalar evidence available from the experimental health route. */
export interface ExperimentalRuntimeTelemetrySnapshot {
  /** Wall duration since the native owner was attached. */
  uptimeSeconds: number;
  /** Steps published since this process attached to the authority. */
  authoritativeSteps: number;
  /** Fixed simulation time represented by those published steps. */
  simulatedSeconds: number;
  /** Simulated seconds divided by elapsed wall seconds. */
  simulatedWallRatio: number;
  /** Native full-step computation timings. */
  step: ExperimentalStepTelemetry;
  /** Node/native combined-process resource and responsiveness observations. */
  process: ExperimentalProcessTelemetry;
  /** Current and largest Rust-packed display sizes observed by Node. */
  frame: { latestBytes: number; maximumObservedBytes: number };
  /** Complete transition-to-successor checkpoint barrier timings. */
  checkpointBarrier: ExperimentalLatencyTelemetry;
  /** Accepted browser-player action to Rust application timings. */
  playerAction: ExperimentalLatencyTelemetry;
  /** Accepted observation-driven trainer action to Rust application timings. */
  trainerAction: ExperimentalLatencyTelemetry;
  /** Join/reclaim request to successful assignment timings. */
  controllerLifecycle: ExperimentalLatencyTelemetry;
}

/** Allocation-once fixed histogram for low-rate interface measurements. */
class LatencyHistogram {
  /** Count in each fixed inclusive bucket. */
  private readonly buckets = LATENCY_BUCKET_UPPER_MS.map(() => 0);
  /** Completed measurement count. */
  private samples = 0;
  /** Saturating finite duration sum. */
  private totalMs = 0;
  /** Largest finite measurement. */
  private maxMs = 0;

  /** Record one finite non-negative duration. */
  public record(durationMs: number): void {
    if (!Number.isFinite(durationMs) || durationMs < 0) return;
    this.samples++;
    this.totalMs += durationMs;
    this.maxMs = Math.max(this.maxMs, durationMs);
    const index = LATENCY_BUCKET_UPPER_MS.findIndex(upper => durationMs <= upper);
    const bucket = index < 0 ? this.buckets.length - 1 : index;
    this.buckets[bucket] = this.buckets[bucket]! + 1;
  }

  /** Return a bounded distribution without exposing mutable buckets. */
  public snapshot(): ExperimentalLatencyTelemetry {
    return {
      samples: this.samples,
      meanMs: this.samples === 0 ? 0 : this.totalMs / this.samples,
      p95Ms: this.percentile(95),
      maxMs: this.maxMs
    };
  }

  /** Resolve one conservative inclusive histogram percentile. */
  private percentile(percentile: number): number {
    if (this.samples === 0) return 0;
    const rank = Math.ceil(this.samples * percentile / 100);
    let cumulative = 0;
    for (let index = 0; index < this.buckets.length; index++) {
      cumulative += this.buckets[index]!;
      if (cumulative >= rank) {
        const upper = LATENCY_BUCKET_UPPER_MS[index]!;
        return Number.isFinite(upper) ? upper : this.maxMs;
      }
    }
    return this.maxMs;
  }
}

/** Parse the native fixed-width unsigned representation without narrowing silently. */
function exactHexNumber(value: string, field: string): number {
  if (!/^[0-9a-f]{16}$/u.test(value)) throw new TypeError(`invalid native ${field}`);
  const exact = BigInt(`0x${value}`);
  if (exact > BigInt(Number.MAX_SAFE_INTEGER)) throw new RangeError(`native ${field} exceeds JavaScript telemetry range`);
  return Number(exact);
}

/** Convert Node's nanosecond event-loop observations to a finite millisecond value. */
function delayMilliseconds(value: number): number {
  return Number.isFinite(value) && value >= 0 ? value / 1_000_000 : 0;
}

/** Bounded integrated measurements for one experimental process lifetime. */
export class ExperimentalRuntimeTelemetry {
  /** Monotonic process-local start boundary. */
  private readonly startedAt = performance.now();
  /** Completed-step identity at attachment, including a restored prefix. */
  private readonly initialCompletedStep: bigint;
  /** Native fixed delta used to calculate represented simulation time. */
  private readonly fixedStepSeconds: number;
  /** Node event-loop delay histogram maintained by the runtime. */
  private readonly eventLoopDelay: IntervalHistogram;
  /** Complete generation persistence/resume barriers. */
  private readonly checkpointBarriers = new LatencyHistogram();
  /** Accepted browser-player input application latency. */
  private readonly playerActions = new LatencyHistogram();
  /** Accepted observation-driven trainer input application latency. */
  private readonly trainerActions = new LatencyHistogram();
  /** Successful assignment/reassignment lifecycle latency. */
  private readonly controllerLifecycles = new LatencyHistogram();
  /** Latest Rust-packed display byte length routed through Node. */
  private latestFrameBytes = 0;
  /** Largest Rust-packed display observed during this process. */
  private maximumFrameBytes = 0;

  /** Start telemetry from one already-created native health boundary. */
  public constructor(initialHealth: RustBackgroundHealth, fixedStepSeconds: number) {
    if (!Number.isFinite(fixedStepSeconds) || fixedStepSeconds <= 0) {
      throw new RangeError('fixedStepSeconds must be positive and finite');
    }
    this.initialCompletedStep = BigInt(`0x${initialHealth.completedStep}`);
    this.fixedStepSeconds = fixedStepSeconds;
    this.eventLoopDelay = monitorEventLoopDelay({ resolution: 10 });
    this.eventLoopDelay.enable();
  }

  /** Stop the Node event-loop sampler during server teardown. */
  public close(): void {
    this.eventLoopDelay.disable();
  }

  /** Retain only scalar display size; no frame or world bytes are copied. */
  public observeDisplay(display: RustBackgroundDisplay): void {
    this.latestFrameBytes = display.frameByteLength;
    this.maximumFrameBytes = Math.max(this.maximumFrameBytes, display.frameByteLength);
  }

  /** Record one complete generation transition checkpoint barrier. */
  public observeCheckpointBarrier(durationMs: number): void {
    this.checkpointBarriers.record(durationMs);
  }

  /** Record one admitted external action through its Rust application result. */
  public observeAction(kind: 'player' | 'reinforcementLearning', durationMs: number): void {
    (kind === 'player' ? this.playerActions : this.trainerActions).record(durationMs);
  }

  /** Record one join or reclaim through its successful assignment receipt. */
  public observeControllerLifecycle(durationMs: number): void {
    this.controllerLifecycles.record(durationMs);
  }

  /** Build one scalar-only health projection from the latest native counters. */
  public snapshot(health: RustBackgroundHealth): ExperimentalRuntimeTelemetrySnapshot {
    const now = performance.now();
    const uptimeSeconds = Math.max((now - this.startedAt) / 1_000, Number.EPSILON);
    const completed = BigInt(`0x${health.completedStep}`);
    const stepDelta = completed >= this.initialCompletedStep ? completed - this.initialCompletedStep : 0n;
    if (stepDelta > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new RangeError('process-local authoritative step count exceeds JavaScript telemetry range');
    }
    const authoritativeSteps = Number(stepDelta);
    const simulatedSeconds = authoritativeSteps * this.fixedStepSeconds;
    const samples = exactHexNumber(health.stepTimingSamples, 'step timing sample count');
    const totalMicros = exactHexNumber(health.stepTimingTotalMicros, 'step timing total');
    const memory = process.memoryUsage();
    return {
      uptimeSeconds,
      authoritativeSteps,
      simulatedSeconds,
      simulatedWallRatio: simulatedSeconds / uptimeSeconds,
      step: {
        samples,
        meanMs: samples === 0 ? 0 : totalMicros / samples / 1_000,
        p95Ms: exactHexNumber(health.stepTimingP95Micros, 'step timing p95') / 1_000,
        p99Ms: exactHexNumber(health.stepTimingP99Micros, 'step timing p99') / 1_000,
        maxMs: exactHexNumber(health.stepTimingMaxMicros, 'step timing maximum') / 1_000,
        percentilesAreUpperBounds: true
      },
      process: {
        rssBytes: memory.rss,
        heapUsedBytes: memory.heapUsed,
        externalBytes: memory.external,
        maxRssBytes: process.resourceUsage().maxRSS * 1_024,
        eventLoopDelayMeanMs: delayMilliseconds(this.eventLoopDelay.mean),
        eventLoopDelayP95Ms: delayMilliseconds(this.eventLoopDelay.percentile(95)),
        eventLoopDelayMaxMs: delayMilliseconds(this.eventLoopDelay.max)
      },
      frame: { latestBytes: this.latestFrameBytes, maximumObservedBytes: this.maximumFrameBytes },
      checkpointBarrier: this.checkpointBarriers.snapshot(),
      playerAction: this.playerActions.snapshot(),
      trainerAction: this.trainerActions.snapshot(),
      controllerLifecycle: this.controllerLifecycles.snapshot()
    };
  }
}
