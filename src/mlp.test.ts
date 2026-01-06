import { describe, it, expect } from 'vitest';
import { MLP, GRU, LSTM, RRU, lstmParamCount, rruParamCount, Genome, mutate, buildArch } from './mlp.ts';
import { CFG } from './config.ts';

/** Test suite label for MLP and genome utilities. */
const SUITE = 'mlp.ts';

describe(SUITE, () => {
  it('MLP should initialize and perform forward pass', () => {
    const layerSizes = [2, 3, 1];
    const mlp = new MLP(layerSizes);
    expect(mlp.paramCount).toBe((3 * 2 + 3) + (1 * 3 + 1));
    
    const input = new Float32Array([0.5, -0.5]);
    const output = mlp.forward(input);
    expect(output).toBeInstanceOf(Float32Array);
    expect(output.length).toBe(1);
    expect(output[0]).toBeGreaterThanOrEqual(-1);
    expect(output[0]).toBeLessThanOrEqual(1);
  });

  it('GRU should initialize and perform step', () => {
    const inSize = 2;
    const hiddenSize = 3;
    const gru = new GRU(inSize, hiddenSize);
    
    const input = new Float32Array([0.5, -0.5]);
    const hidden = gru.step(input);
    expect(hidden).toBeInstanceOf(Float32Array);
    expect(hidden.length).toBe(hiddenSize);
  });

  it('LSTM should be deterministic for fixed weights', () => {
    const inSize = 2;
    const hiddenSize = 3;
    const weights = new Float32Array(lstmParamCount(inSize, hiddenSize)).fill(0.05);
    const lstm = new LSTM(inSize, hiddenSize, weights);
    const input = new Float32Array([0.25, -0.1]);
    const out1 = lstm.step(input).slice();
    lstm.reset();
    const out2 = lstm.step(input).slice();
    expect(Array.from(out1)).toEqual(Array.from(out2));
  });

  it('RRU should be deterministic for fixed weights', () => {
    const inSize = 2;
    const hiddenSize = 3;
    const weights = new Float32Array(rruParamCount(inSize, hiddenSize)).fill(-0.02);
    const rru = new RRU(inSize, hiddenSize, weights);
    const input = new Float32Array([0.25, -0.1]);
    const out1 = rru.step(input).slice();
    rru.reset();
    const out2 = rru.step(input).slice();
    expect(Array.from(out1)).toEqual(Array.from(out2));
  });

  it('Genome should initialize randomly', () => {
    const settings = {
      hiddenLayers: 1,
      neurons1: 4,
      neurons2: 4,
      neurons3: 4,
      neurons4: 4,
      neurons5: 4
    };
    const arch = buildArch(settings);
    const genome = Genome.random(arch);
    expect(genome.weights).not.toBeNull();
    expect(genome.weights.length).toBeGreaterThan(0);
  });

  it('mutate should modify weights', () => {
    const settings = {
      hiddenLayers: 1,
      neurons1: 4,
      neurons2: 4,
      neurons3: 4,
      neurons4: 4,
      neurons5: 4
    };
    const arch = buildArch(settings);
    const genome = Genome.random(arch);
    const originalWeights = genome.weights.slice();
    
    // Force high mutation rate for test
    const oldRate = CFG.mutationRate;
    CFG.mutationRate = 1.0;
    mutate(genome, arch);
    CFG.mutationRate = oldRate;
    
    expect(genome.weights).not.toEqual(originalWeights);
  });

  it('Genome should serialize and deserialize correctly', () => {
    const settings = {
      hiddenLayers: 1,
      neurons1: 4,
      neurons2: 4,
      neurons3: 4,
      neurons4: 4,
      neurons5: 4
    };
    const arch = buildArch(settings);
    const genome = Genome.random(arch);
    genome.fitness = 123.45;
    
    const json = genome.toJSON();
    expect(json.archKey).toBe(genome.archKey);
    expect(json.brainType).toBe(genome.brainType);
    expect(json.weights).toEqual(Array.from(genome.weights));
    expect(json.fitness).toBe(123.45);
    
    const reconstructed = Genome.fromJSON(json);
    expect(reconstructed.archKey).toBe(genome.archKey);
    expect(reconstructed.brainType).toBe(genome.brainType);
    expect(reconstructed.weights).toEqual(genome.weights);
    expect(reconstructed.fitness).toBe(genome.fitness);
    expect(reconstructed.weights).toBeInstanceOf(Float32Array);
  });

  it('Genome defaults brainType to mlp when missing', () => {
    const reconstructed = Genome.fromJSON({ archKey: 'test', weights: [], fitness: 0 });
    expect(reconstructed.brainType).toBe('mlp');
  });
});
