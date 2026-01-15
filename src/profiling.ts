/** Profiling utilities for simulation timing breakdowns. */

/** Default interval in milliseconds between profile reports. */
const DEFAULT_REPORT_INTERVAL_MS = 1000;

/**
 * Get a high-resolution timestamp in milliseconds.
 * @returns Current timestamp in milliseconds.
 */
function nowMs(): number {
  if (typeof performance !== 'undefined' && typeof performance.now === 'function') {
    return performance.now();
  }
  return Date.now();
}

/** Configuration options for the simulation profiler. */
export interface SimProfilerOptions {
  /** Whether profiling is enabled. */
  enabled?: boolean;
  /** Interval in milliseconds between report snapshots. */
  reportIntervalMs?: number;
}

/** Snapshot of averaged timing data for a profiling window. */
export interface SimProfilerReport {
  /** Number of ticks included in the report. */
  ticks: number;
  /** Total wall-clock time covered by the window in milliseconds. */
  windowMs: number;
  /** Average total tick time in milliseconds. */
  avgTotalMs: number;
  /** Average sensor evaluation time per tick in milliseconds. */
  avgSensorsMs: number;
  /** Average brain inference time per tick in milliseconds. */
  avgBrainMs: number;
  /** Average non-brain/non-sensor time per tick in milliseconds. */
  avgPhysicsMs: number;
  /** Average sensor calls per tick. */
  avgSensorCalls: number;
  /** Average brain calls per tick. */
  avgBrainCalls: number;
  /** Average sensor time per call in microseconds. */
  avgSensorUsPerCall: number;
  /** Average brain time per call in microseconds. */
  avgBrainUsPerCall: number;
}

/** Lightweight profiler for tracking per-tick simulation timings. */
export class SimProfiler {
  /** Whether profiling is currently enabled. */
  enabled: boolean;
  /** Report interval in milliseconds. */
  reportIntervalMs: number;
  /** Timestamp when the current tick started. */
  tickStartMs: number;
  /** Accumulated sensor time for the current tick. */
  tickSensorsMs: number;
  /** Accumulated brain time for the current tick. */
  tickBrainMs: number;
  /** Sensor call count for the current tick. */
  tickSensorCalls: number;
  /** Brain call count for the current tick. */
  tickBrainCalls: number;
  /** Start time for the current reporting window. */
  windowStartMs: number;
  /** Total ticks counted in the current window. */
  windowTicks: number;
  /** Accumulated total tick time in the current window. */
  windowTotalMs: number;
  /** Accumulated sensor time in the current window. */
  windowSensorsMs: number;
  /** Accumulated brain time in the current window. */
  windowBrainMs: number;
  /** Accumulated non-brain/non-sensor time in the current window. */
  windowPhysicsMs: number;
  /** Accumulated sensor calls in the current window. */
  windowSensorCalls: number;
  /** Accumulated brain calls in the current window. */
  windowBrainCalls: number;

  /**
   * Create a new profiler with optional configuration overrides.
   * @param options - Profiler configuration options.
   */
  constructor(options: SimProfilerOptions = {}) {
    this.enabled = options.enabled ?? true;
    this.reportIntervalMs = Math.max(100, options.reportIntervalMs ?? DEFAULT_REPORT_INTERVAL_MS);
    this.tickStartMs = 0;
    this.tickSensorsMs = 0;
    this.tickBrainMs = 0;
    this.tickSensorCalls = 0;
    this.tickBrainCalls = 0;
    this.windowStartMs = nowMs();
    this.windowTicks = 0;
    this.windowTotalMs = 0;
    this.windowSensorsMs = 0;
    this.windowBrainMs = 0;
    this.windowPhysicsMs = 0;
    this.windowSensorCalls = 0;
    this.windowBrainCalls = 0;
  }

  /**
   * Read the current timestamp used for profiling.
   * @returns Current timestamp in milliseconds.
   */
  now(): number {
    return nowMs();
  }

  /**
   * Reset per-tick counters and mark the tick start time.
   * @param now - Optional timestamp override.
   */
  beginTick(now?: number): void {
    if (!this.enabled) return;
    this.tickStartMs = now ?? nowMs();
    this.tickSensorsMs = 0;
    this.tickBrainMs = 0;
    this.tickSensorCalls = 0;
    this.tickBrainCalls = 0;
  }

  /**
   * Record a sensor evaluation duration for the current tick.
   * @param durationMs - Duration in milliseconds.
   */
  recordSensors(durationMs: number): void {
    if (!this.enabled) return;
    this.tickSensorsMs += durationMs;
    this.tickSensorCalls += 1;
  }

  /**
   * Record a brain inference duration for the current tick.
   * @param durationMs - Duration in milliseconds.
   */
  recordBrain(durationMs: number): void {
    if (!this.enabled) return;
    this.tickBrainMs += durationMs;
    this.tickBrainCalls += 1;
  }

  /**
   * Finalize the current tick and accumulate window totals.
   * @param now - Optional timestamp override.
   */
  endTick(now?: number): void {
    if (!this.enabled) return;
    if (!this.tickStartMs) return;
    const end = now ?? nowMs();
    const totalMs = Math.max(0, end - this.tickStartMs);
    const physicsMs = Math.max(0, totalMs - this.tickSensorsMs - this.tickBrainMs);
    this.windowTicks += 1;
    this.windowTotalMs += totalMs;
    this.windowSensorsMs += this.tickSensorsMs;
    this.windowBrainMs += this.tickBrainMs;
    this.windowPhysicsMs += physicsMs;
    this.windowSensorCalls += this.tickSensorCalls;
    this.windowBrainCalls += this.tickBrainCalls;
  }

  /**
   * Return a report snapshot when the report interval elapses.
   * @param now - Optional timestamp override.
   * @returns Report snapshot or null when not ready.
   */
  reportIfDue(now?: number): SimProfilerReport | null {
    if (!this.enabled) return null;
    const current = now ?? nowMs();
    const elapsed = current - this.windowStartMs;
    if (elapsed < this.reportIntervalMs || this.windowTicks <= 0) return null;
    const ticks = this.windowTicks;
    const avgTotalMs = this.windowTotalMs / ticks;
    const avgSensorsMs = this.windowSensorsMs / ticks;
    const avgBrainMs = this.windowBrainMs / ticks;
    const avgPhysicsMs = this.windowPhysicsMs / ticks;
    const avgSensorCalls = this.windowSensorCalls / ticks;
    const avgBrainCalls = this.windowBrainCalls / ticks;
    const avgSensorUsPerCall = this.windowSensorCalls > 0
      ? (this.windowSensorsMs / this.windowSensorCalls) * 1000
      : 0;
    const avgBrainUsPerCall = this.windowBrainCalls > 0
      ? (this.windowBrainMs / this.windowBrainCalls) * 1000
      : 0;
    const report: SimProfilerReport = {
      ticks,
      windowMs: elapsed,
      avgTotalMs,
      avgSensorsMs,
      avgBrainMs,
      avgPhysicsMs,
      avgSensorCalls,
      avgBrainCalls,
      avgSensorUsPerCall,
      avgBrainUsPerCall
    };
    this.resetWindow(current);
    return report;
  }

  /**
   * Reset the reporting window accumulators.
   * @param now - Current timestamp in milliseconds.
   */
  resetWindow(now: number): void {
    if (!this.enabled) return;
    this.windowStartMs = now;
    this.windowTicks = 0;
    this.windowTotalMs = 0;
    this.windowSensorsMs = 0;
    this.windowBrainMs = 0;
    this.windowPhysicsMs = 0;
    this.windowSensorCalls = 0;
    this.windowBrainCalls = 0;
  }
}

/**
 * Format a profiler report for logging.
 * @param report - Report snapshot to format.
 * @returns Human-readable summary string.
 */
export function formatSimProfilerReport(report: SimProfilerReport): string {
  const total = report.avgTotalMs;
  const pct = (value: number) => (total > 0 ? (value / total) * 100 : 0);
  const brainPct = pct(report.avgBrainMs);
  const sensorPct = pct(report.avgSensorsMs);
  const physicsPct = pct(report.avgPhysicsMs);
  return [
    '[profile]',
    `ticks=${report.ticks}`,
    `avg=${report.avgTotalMs.toFixed(2)}ms`,
    `brain=${report.avgBrainMs.toFixed(2)}ms (${brainPct.toFixed(1)}%)`,
    `sensors=${report.avgSensorsMs.toFixed(2)}ms (${sensorPct.toFixed(1)}%)`,
    `physics=${report.avgPhysicsMs.toFixed(2)}ms (${physicsPct.toFixed(1)}%)`,
    `brainCalls=${report.avgBrainCalls.toFixed(1)}/tick`,
    `sensorCalls=${report.avgSensorCalls.toFixed(1)}/tick`,
    `brainCall=${report.avgBrainUsPerCall.toFixed(1)}us`,
    `sensorCall=${report.avgSensorUsPerCall.toFixed(1)}us`
  ].join(' ');
}
