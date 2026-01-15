import { describe, it, expect, beforeAll } from 'vitest';
import { buildArch, Genome } from './mlp.ts';
import { CFG } from './config.ts';
import { loadSimdKernels } from './brains/wasmBridge.ts';

/** Test suite label for stacked brain regression coverage. */
const SUITE = 'regression: stacked brains';

describe(SUITE, () => {
  beforeAll(async () => {
    await loadSimdKernels();
  });

  it('produces deterministic outputs for fixed weights', () => {
    const settings = {
      hiddenLayers: 1,
      neurons1: 6,
      neurons2: 4,
      neurons3: 4,
      neurons4: 4,
      neurons5: 4
    };
    CFG.brain.stack.gru = 1;
    CFG.brain.stack.lstm = 1;
    CFG.brain.stack.rru = 0;
    const arch = buildArch(settings);
    const genome = Genome.random(arch);
    genome.weights.fill(0.02);
    const brain = genome.buildBrain(arch);
    const input = new Float32Array(CFG.brain.inSize).fill(0.1);
    const out1 = Array.from(brain.forward(input));
    brain.reset();
    const out2 = Array.from(brain.forward(input));
    expect(out1).toEqual(out2);
  });
});
