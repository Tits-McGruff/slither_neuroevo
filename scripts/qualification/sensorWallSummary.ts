import { sensorWallTiming } from './sensorWallTiming.ts';
import type { ManagedCheckpointDescriptor } from '../../server/rustEngine/checkpointPersistenceProtocol.ts';
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

/** One native manual completion joined to the independently selected SQLite boundary. */
export interface SensorGenerationBarrier {
  /** Consecutive observation indexes bracket all publication and persistence work. */
  beforeReadIndex: number;
  /** First observation after the generation router returns to idle. */
  afterReadIndex: number;
  /** Exact next-step receipt from the native finish command. */
  effectiveStep: string;
  /** Worker-selected descriptor, checked against the actual native publication by the host. */
  checkpoint: Pick<ManagedCheckpointDescriptor, 'runId' | 'boundaryKind' | 'generation' | 'completedStep' | 'logicalRootSha256'>;
  /** Bounded facts read from the SQLite experiment sidecar. */
  terminal: {generation: string; completedStep: string; reason: string; population: number; actualSeconds: number};
}

/** Explicit authority changes are accepted only with a matching native/SQLite chronology. */
export interface SensorWallDurability {
  /** Expected active run, never inferred from an arbitrary descriptor. */
  runId: string;
  /** Ordered complete barriers; every authority change must appear exactly once. */
  barriers: readonly SensorGenerationBarrier[];
}

/** Validate compact native/SQLite joins without parsing a population or deriving world state. */
function durabilityTargets(reads: readonly SensorWallRead[], durability?: SensorWallDurability): Set<number> {
  const targets = new Set<number>();
  if (!durability) return targets;
  if (!durability.runId || durability.runId.length > 256 || durability.runId.includes('\0') ||
      durability.barriers.length < 1 || durability.barriers.length > 128) throw new Error('bounded generation durability proof required');
  let previousTarget = -1;
  for (const barrier of durability.barriers) {
    const {beforeReadIndex: before, afterReadIndex: after, checkpoint: cp, terminal} = barrier;
    if (!Number.isSafeInteger(before) || !Number.isSafeInteger(after) || before < 0 || after !== before + 1 ||
        after >= reads.length || before < previousTarget) throw new Error('unordered generation barrier observation indexes');
    const source = reads[before]!.health; const target = reads[after]!.health;
    if (counter(target.generation) !== counter(source.generation) + 1n ||
        counter(target.worldEpoch) <= counter(source.worldEpoch) || cp.runId !== durability.runId ||
        cp.boundaryKind !== 'generation' || cp.generation !== target.generation ||
        !/^[0-9a-f]{64}$/u.test(cp.logicalRootSha256) || cp.completedStep !== barrier.effectiveStep ||
        counter(cp.completedStep) <= counter(source.completedStep) ||
        counter(cp.completedStep) > counter(target.completedStep) || terminal.generation !== source.generation ||
        terminal.completedStep !== cp.completedStep || terminal.reason !== 'manual' ||
        ![55, 300].includes(terminal.population) || !Number.isFinite(terminal.actualSeconds) || terminal.actualSeconds <= 0) {
      throw new Error('generation durability identity/chronology mismatch');
    }
    targets.add(after); previousTarget = after;
  }
  return targets;
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
export function summarizeSensorWall(reads: readonly SensorWallRead[], requestedSeconds: number, durability?: SensorWallDurability, sixHour = false) {
  sensorWallTiming(requestedSeconds, sixHour);
  if (reads.length < 2 || reads.length > 4096) throw new Error('bounded 60..600-second wall window required');
  const targets = durabilityTargets(reads, durability);
  const initial = reads[0]!; const final = reads.at(-1)!;
  const first = initial.health;
  let previous: SensorWallRead | undefined; let overloaded = false;
  for (const [index, read] of reads.entries()) {
    const h = read.health;
    if (![read.beforeMs, read.afterMs].every(value => Number.isFinite(value) && value >= 0) || read.afterMs < read.beforeMs ||
        (previous && read.beforeMs <= previous.afterMs)) throw new Error('invalid or overlapping wall clock brackets');
    if (sixHour && previous && read.beforeMs - previous.afterMs > 35_000) throw new Error('six-hour health coverage gap');
    counter(h.worldEpoch); counter(h.generation);
    if (typeof h.schedulerOverloaded !== 'boolean' || h.stepTimingHistogramConsistent !== true) throw new Error('inconsistent wall health observation');
    const changed = previous && (h.worldEpoch !== previous.health.worldEpoch || h.generation !== previous.health.generation);
    if (Boolean(changed) !== targets.has(index) || h.calculationWorkers !== first.calculationWorkers ||
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
  const sixHourWorkComplete = progress.minimumWallSeconds >= 21_600 && BigInt(progress.deltaSteps) >= 1_296_000n;
  return {...(sixHour ? {sixHourWall: {requiredSteps: '1296000', workComplete: sixHourWorkComplete}} : {}),
    progress, stepTiming, droppedWallMicros: dropped.toString(), overloadedObserved: overloaded,
    schedulerCompletedSteps: (counter(final.health.schedulerCompletedSteps) - counter(first.schedulerCompletedSteps)).toString(),
    meetsRealtimeGate, meetsExistingStepTimingGate: stepTiming.meetsP0P1StepGate,
    meetsMeasuredGates: meetsRealtimeGate && stepTiming.meetsP0P1StepGate && (!sixHour || sixHourWorkComplete)};
}
