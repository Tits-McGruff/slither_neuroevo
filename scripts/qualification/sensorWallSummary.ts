import type { RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';
import { counterWindow } from '../stage7/archive-overlap-summary.ts';
import { summarizeStepWindow, type StepHistogram } from '../stage7/step-window-summary.ts';

/** Only scalar native identity, scheduler and timing facts enter a wall window. */
export type SensorWallHealth = StepHistogram & Pick<RustBackgroundHealth,
  'worldEpoch' | 'generation' | 'completedStep' | 'calculationWorkers' |
  'schedulerCompletedSteps' | 'schedulerDroppedWallMicros' | 'schedulerOverloaded' |
  'lifecycle' | 'faultCode'>;

/** One health read bracketed by the observing Node monotonic clock. */
export interface SensorWallRead {
  /** Clock immediately before reading the native cache. */
  beforeMs: number;
  /** Clock immediately after reading the native cache. */
  afterMs: number;
  /** Native counters; no world state or reconstructed simulation. */
  health: SensorWallHealth;
}

/** Decode a canonical counter; saturation cannot establish a measurement. */
function counter(value: string): bigint {
  if (!/^[0-9a-f]{16}$/u.test(value) || value === 'ffffffffffffffff') throw new Error('invalid or saturated wall counter');
  return BigInt('0x' + value);
}

/** Apply the existing 98% progress and P0/P1 timing criteria to a real-clock window.
 * Scheduler/identity failures reject the evidence; measured performance failures
 * remain explicit results so the report never hides a failed configuration.
 */
export function summarizeSensorWall(reads: readonly SensorWallRead[], requestedSeconds: number) {
  if (!Number.isSafeInteger(requestedSeconds) || requestedSeconds < 60 || requestedSeconds > 600 ||
      reads.length < 2 || reads.length > 4096) throw new Error('bounded 60..600-second wall window required');
  const initial = reads[0]!; const final = reads.at(-1)!;
  const first = initial.health;
  let previous: SensorWallRead | undefined; let overloaded = false;
  for (const read of reads) {
    const h = read.health;
    if (![read.beforeMs, read.afterMs].every(value => Number.isFinite(value) && value >= 0) || read.afterMs < read.beforeMs ||
        (previous && read.beforeMs <= previous.afterMs)) throw new Error('invalid or overlapping wall clock brackets');
    counter(h.worldEpoch); counter(h.generation);
    if (typeof h.schedulerOverloaded !== 'boolean' || h.stepTimingHistogramConsistent !== true) throw new Error('inconsistent wall health observation');
    if (h.worldEpoch !== first.worldEpoch || h.generation !== first.generation || h.calculationWorkers !== first.calculationWorkers ||
        !Number.isSafeInteger(h.calculationWorkers) || h.calculationWorkers < 1 || h.lifecycle !== 'running' || h.faultCode !== undefined) {
      throw new Error('wall window authority, workers or lifecycle changed');
    }
    for (const key of ['completedStep', 'schedulerCompletedSteps', 'schedulerDroppedWallMicros'] as const) {
      const value = counter(h[key]);
      if (previous && value < counter(previous.health[key])) throw new Error('wall counter regressed');
    }
    overloaded ||= h.schedulerOverloaded;
    previous = read;
  }
  const progress = counterWindow({...initial, completedStep: first.completedStep},
    {...final, completedStep: final.health.completedStep});
  const stepTiming = summarizeStepWindow(first, final.health);
  const dropped = counter(final.health.schedulerDroppedWallMicros) - counter(first.schedulerDroppedWallMicros);
  const meetsRealtimeGate = progress.minimumWallSeconds >= requestedSeconds &&
    progress.minimumSimulatedWallRatio >= 0.98 && dropped === 0n && !overloaded;
  return {progress, stepTiming, droppedWallMicros: dropped.toString(), overloadedObserved: overloaded,
    schedulerCompletedSteps: (counter(final.health.schedulerCompletedSteps) - counter(first.schedulerCompletedSteps)).toString(),
    meetsRealtimeGate, meetsExistingStepTimingGate: stepTiming.meetsP0P1StepGate,
    meetsMeasuredGates: meetsRealtimeGate && stepTiming.meetsP0P1StepGate};
}
