import { describe, it, expect } from 'vitest';
import { MLP, GRU, Genome, mutate, crossover, buildArch } from './mlp.ts';
import { CFG } from './config.js';

describe('mlp.js', () => {
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

  it('Genome should initialize randomly', () => {
    const settings = {
        hiddenLayers: 1,
        neurons1: 4
    };
    const arch = buildArch(settings);
    const genome = Genome.random(arch);
    expect(genome.weights).not.toBeNull();
    expect(genome.weights.length).toBeGreaterThan(0);
  });

  it('mutate should modify weights', () => {
    const settings = {
        hiddenLayers: 1,
        neurons1: 4
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
        neurons1: 4
    };
    const arch = buildArch(settings);
    const genome = Genome.random(arch);
    genome.fitness = 123.45;
    
    const json = genome.toJSON();
    expect(json.archKey).toBe(genome.archKey);
    expect(json.weights).toEqual(Array.from(genome.weights));
    expect(json.fitness).toBe(123.45);
    
    const reconstructed = Genome.fromJSON(json);
    expect(reconstructed.archKey).toBe(genome.archKey);
    expect(reconstructed.weights).toEqual(genome.weights);
    expect(reconstructed.fitness).toBe(genome.fitness);
    expect(reconstructed.weights).toBeInstanceOf(Float32Array);
  });
});
