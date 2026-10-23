/** Cost observations must never become simulation or exact-step acceptance evidence. */
import { expect, it, vi } from 'vitest';
import type { RustBackgroundHealth } from '../../src/protocol/rustBackground.ts';
import { NATIVE_STEP_COST_KEYS, readNativeStepCosts } from './nativeStepCosts.ts';

/** Only the native scalar contract is needed; no addon or world exists in this fixture. */
function fixture() {
  return {worldEpoch: '0000000000000001', generation: '0000000000000002', completedStep: '0000000000000003',
    ...Object.fromEntries(NATIVE_STEP_COST_KEYS.map((key, index) => [key, index.toString(16).padStart(16, '0')]))} as RustBackgroundHealth;
}

it('reads the native cache once, brackets it and retains only finite diagnostic counters', () => {
  const health = fixture(); const read = vi.fn(() => health);
  const result = readNativeStepCosts({health: read});
  expect(read).toHaveBeenCalledTimes(1); expect(read).toHaveBeenCalledWith();
  expect(result.afterMs).toBeGreaterThanOrEqual(result.beforeMs);
  expect(result).toMatchObject({worldEpoch: health.worldEpoch, generation: health.generation, completedStep: health.completedStep});
  expect(Object.keys(result.counters)).toEqual([...NATIVE_STEP_COST_KEYS]);
  expect(new Set(NATIVE_STEP_COST_KEYS).size).toBe(NATIVE_STEP_COST_KEYS.length);
  expect(result.counters.terminalStepEvolutionReproductionMicros).toBe(health.terminalStepEvolutionReproductionMicros);
});

it('rejects saturated, malformed or missing costs instead of treating unavailable attribution as zero', () => {
  for (const value of ['ffffffffffffffff', '000000000000000G', '0', undefined]) {
    const health = {...fixture(), slowStepControlInferenceMicros: value} as RustBackgroundHealth;
    expect(() => readNativeStepCosts({health: () => health})).toThrow(/cost counter/u);
  }
});
