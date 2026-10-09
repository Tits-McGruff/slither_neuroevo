/** Prove overlap using bounds on the native job's origin in the Node clock. */
export function archiveOverlap(
  clock: { beforeMs: number; afterMs: number },
  phase: { startedMicros: string; finishedMicros?: string | null },
  barrier: { startedMs: number; finishedMs: number }
): { guaranteedMs: number; possibleMs: number; originUncertaintyMs: number } {
  for (const value of [clock.beforeMs, clock.afterMs, barrier.startedMs, barrier.finishedMs]) {
    if (!Number.isFinite(value) || value < 0) throw new Error('invalid monotonic clock observation');
  }
  if (clock.afterMs < clock.beforeMs || barrier.finishedMs < barrier.startedMs) throw new Error('reversed clock interval');
  /** Keep the native microsecond conversion exact for this bounded measurement. */
  function micros(value: string | null | undefined): number {
    if (!value || !/^[0-9a-f]{16}$/u.test(value)) throw new Error('missing or invalid native phase boundary');
    const exact = BigInt(`0x${value}`);
    if (exact > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('inexact native phase boundary');
    return Number(exact) / 1000;
  }
  const start = micros(phase.startedMicros);
  const finish = micros(phase.finishedMicros);
  if (finish < start) throw new Error('reversed native phase');
  return {
    guaranteedMs: Math.max(0, Math.min(clock.beforeMs + finish, barrier.finishedMs) -
      Math.max(clock.afterMs + start, barrier.startedMs)),
    possibleMs: Math.max(0, Math.min(clock.afterMs + finish, barrier.finishedMs) -
      Math.max(clock.beforeMs + start, barrier.startedMs)),
    originUncertaintyMs: clock.afterMs - clock.beforeMs
  };
}

/** One completed-step counter read bounded by its actual health request/reply. */
export interface CounterRead {
  /** Monotonic time immediately before the request. */
  beforeMs: number;
  /** Monotonic time after the complete response has been read. */
  afterMs: number;
  /** Exact Rust completed-step counter, encoded as canonical Uint64 hex. */
  completedStep: string;
}

/** Conservative duration/progress bounds for two exact completed-step readings. */
export interface CounterWindow {
  /** Exact decimal difference, before conversion for this bounded measurement. */
  deltaSteps: string;
  /** Shortest elapsed interval allowed by both request brackets. */
  minimumWallSeconds: number;
  /** Longest elapsed interval allowed by both request brackets. */
  maximumWallSeconds: number;
  /** Progress at the longest possible elapsed interval; used for acceptance. */
  minimumSimulatedWallRatio: number;
  /** Progress at the shortest possible elapsed interval; diagnostic only. */
  maximumSimulatedWallRatio: number;
}

/** Bound elapsed time and 60-Hz progress without including later sampler cleanup. */
export function counterWindow(initial: CounterRead, final: CounterRead): CounterWindow {
  for (const read of [initial, final]) {
    if (![read.beforeMs, read.afterMs].every(value => Number.isFinite(value) && value >= 0)) {
      throw new Error('invalid counter clock observation');
    }
    if (read.afterMs < read.beforeMs) throw new Error('reversed counter clock interval');
    if (!/^[0-9a-f]{16}$/u.test(read.completedStep)) throw new Error('invalid completed-step counter');
  }
  if (final.beforeMs <= initial.afterMs) throw new Error('counter request intervals overlap or touch');
  const steps = BigInt(`0x${final.completedStep}`) - BigInt(`0x${initial.completedStep}`);
  if (steps < 0n || steps > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('regressed or inexact step delta');
  const minimumWallSeconds = (final.beforeMs - initial.afterMs) / 1000;
  const maximumWallSeconds = (final.afterMs - initial.beforeMs) / 1000;
  if (!(minimumWallSeconds > 0) || !Number.isFinite(maximumWallSeconds)) {
    throw new Error('unrepresentable counter duration');
  }
  const simulatedSeconds = Number(steps) / 60;
  const minimumSimulatedWallRatio = simulatedSeconds / maximumWallSeconds;
  const maximumSimulatedWallRatio = simulatedSeconds / minimumWallSeconds;
  if (![minimumSimulatedWallRatio, maximumSimulatedWallRatio].every(Number.isFinite)) {
    throw new Error('unrepresentable counter progress');
  }
  return {
    deltaSteps: steps.toString(), minimumWallSeconds, maximumWallSeconds,
    minimumSimulatedWallRatio, maximumSimulatedWallRatio
  };
}
