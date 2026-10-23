/** Existing native aggregate counters; these independent atomics are diagnostic, not an exact step snapshot. */
import assert from 'node:assert/strict';
import type { RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';
import type { ExperimentalRunningAuthorityNativeHandle } from '../../server/rustEngine/backgroundRuntime.ts';

/** Existing slow/terminal costs; terminalStepMaxMicros is a lifetime maximum, never an additive duration. */
export const NATIVE_STEP_COST_KEYS = [
  'stepTimingSamples', 'stepTimingTotalMicros', 'slowStepSamples', 'slowStepControlMicros',
  'slowStepControlIndexMicros', 'slowStepControlNeuralMicros', 'slowStepControlSensingMicros',
  'slowStepControlInferenceMicros', 'slowStepWorldMicros', 'slowStepOtherMicros',
  'terminalStepSamples', 'terminalStepTotalMicros', 'terminalStepMaxMicros', 'terminalStepControlMicros',
  'terminalStepWorldMicros', 'terminalStepPreparationMicros', 'terminalStepEvolutionMicros',
  'terminalStepEvolutionSummaryMicros', 'terminalStepEvolutionReproductionMicros',
  'terminalStepAdmissionMicros', 'terminalStepOtherMicros'
] as const satisfies readonly (keyof RustBackgroundHealth)[];

/** A bounded native cache read; exact histogram acceptance stays in SensorWallRead. */
export interface NativeStepCostRead {
  /** Node's monotonic bracket around the native cache read. */
  beforeMs: number;
  /** Upper end of the diagnostic observation bracket. */
  afterMs: number;
  /** Current native authority incarnation, permitted to change across explicitly recorded generations. */
  worldEpoch: string;
  /** Current native generation at this independent cache read. */
  generation: string;
  /** Current completed-step receipt; never inferred from costs. */
  completedStep: string;
  /** Exact finite unsigned hexadecimal counters; independent atomics can cover slightly different prefixes. */
  counters: Record<typeof NATIVE_STEP_COST_KEYS[number], string>;
}

/** Copy only existing scalar cost counters; never advance, copy or inspect authoritative world arrays. */
export function readNativeStepCosts(native: Pick<ExperimentalRunningAuthorityNativeHandle, 'health'>): NativeStepCostRead {
  const beforeMs = performance.now(); const health = native.health(); const afterMs = performance.now();
  const counters = {} as NativeStepCostRead['counters'];
  for (const key of NATIVE_STEP_COST_KEYS) {
    const value = health[key];
    assert(/^[0-9a-f]{16}$/u.test(value) && value !== 'f'.repeat(16), 'invalid or saturated native cost counter');
    counters[key] = value;
  }
  return {beforeMs, afterMs, worldEpoch: health.worldEpoch, generation: health.generation,
    completedStep: health.completedStep, counters};
}
