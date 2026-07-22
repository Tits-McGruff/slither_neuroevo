import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import { compileGraph, graphKey } from '../src/brains/graph/compiler.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import { Genome } from '../src/mlp.ts';
import { World } from '../src/world.ts';
import { NodeBrainPool } from '../src/sim/NodeBrainPool.ts';
import type { BrainPoolInitOptions } from '../src/sim/BaseBrainPool.ts';
import { DEFAULT_CONFIG } from './config.ts';
import type { Persistence, PopulationSnapshotPayload } from './persistence.ts';
import { SimServer } from './simServer.ts';

/** Unmistakable suite label for server-side Phase 0 characterizations. */
const SUITE = 'recovery Phase 0 characterization — current server behavior';

/** Test-only active-pool view that exposes initial shared weights. */
class InspectableNodeBrainPool extends NodeBrainPool {
  /** Number of population sync requests observed by this fixture. */
  syncCalls = 0;

  /** Number of population-owned weight strides exposed by the fixture. */
  private observedPopulationCount = 0;

  /**
   * Allocate the same zero-filled buffers as production init without workers.
   * @param options - Explicit hand-built pool dimensions.
   */
  prime(options: BrainPoolInitOptions): void {
    this.specKey = options.specKey;
    this.paramCount = options.paramCount;
    this.inputStride = options.inputStride;
    this.outputStride = options.outputStride;
    this.observedPopulationCount = options.populationCount;
    const capacity = Math.max(Math.ceil(options.populationCount * 1.25), 256);
    this.allocateBuffers(
      capacity,
      options.paramCount,
      options.inputStride,
      options.outputStride,
      options.stateSize ?? 0
    );
    this.status = 'ready';
  }

  /**
   * Record calls before delegating to the current production copy routine.
   * @param population - Population offered to the shared pool.
   */
  override syncWeights(population: Genome[]): void {
    this.syncCalls += 1;
    super.syncWeights(population);
  }

  /** @returns Copy of the current population-owned shared-weight strides. */
  weightSnapshot(): Float32Array {
    if (!this.weightsBuffer) return new Float32Array(0);
    const usedLength = this.paramCount * this.observedPopulationCount;
    return new Float32Array(this.weightsBuffer).slice(0, usedLength);
  }
}

/** Narrow access to SimServer methods that are private in production. */
interface CharacterizationSimServerAccess {
  /** Execute current pool-consistency logic. */
  ensureBrainPool: () => Promise<NodeBrainPool | null>;
  /** Execute current generation-change persistence logic. */
  handleGenerationEnd: () => void;
}

beforeEach(() => {
  resetCFGToDefaults();
});

afterEach(() => {
  resetCFGToDefaults();
});

/** @returns Explicit graph matching the current configured sensor/output sizes. */
function buildPoolGraph(): GraphSpec {
  return {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: CFG.brain.inSize },
      {
        id: 'dense',
        type: 'Dense',
        inputSize: CFG.brain.inSize,
        outputSize: CFG.brain.outSize
      }
    ],
    edges: [{ from: 'input', to: 'dense' }],
    outputs: [{ nodeId: 'dense' }],
    outputSize: CFG.brain.outSize
  };
}

/**
 * Build a one-member World-shaped component whose real generation transition
 * logs the current new-generation construction order.
 * @param events - Shared transition event sink.
 * @returns Hand-owned World component with no random construction.
 */
function buildGenerationTransitionWorld(events: string[]): World {
  const genome = new Genome('phase0-fixture', Float32Array.of(0.5));
  const snake = {
    id: 1,
    pointsScore: 1,
    fitness: -2,
    computeFitness: () => -2,
    length: () => 3
  };
  const world = {
    population: [genome],
    snakes: [snake],
    generation: 1,
    generationTime: 3,
    bestFitnessEver: 0,
    fitnessHistory: [],
    bestPointsThisGen: 1,
    bestPointsSnakeId: 1,
    _lastHoFEntry: null,
    particles: {},
    _clearTransientGenerationState: () => events.push(`clear@${world.generation}`),
    _initPellets: () => events.push(`pellets@${world.generation}`),
    _resetBaselineBotsForGen: () => events.push(`bots@${world.generation}`),
    _emitGenerationBoundary: () => events.push(`boundary@${world.generation}`),
    _spawnAll: () => events.push(`spawn@${world.generation}`),
    _collGrid: {
      build: () => events.push(`grid@${world.generation}`)
    },
    _chooseInitialFocus: () => events.push(`focus@${world.generation}`)
  } as unknown as World;
  return world;
}

describe(SUITE, () => {
  it('MT-002 [expires/converts in Phase 4] first ready active pool keeps zero weights because generation is treated as synchronized', async () => {
    const spec = buildPoolGraph();
    const compiled = compileGraph(spec);
    const key = graphKey(spec);
    const populationWeights = new Float32Array(compiled.totalParams).fill(0.25);
    const population = [new Genome(key, populationWeights, 'graph')];
    const pool = new InspectableNodeBrainPool(1);
    pool.prime({
      specKey: key,
      graphSpec: spec,
      populationCount: 1,
      paramCount: compiled.totalParams,
      inputStride: CFG.brain.inSize,
      outputStride: CFG.brain.outSize
    });
    const core = {
      world: {
        population,
        arch: { spec, key },
        archKey: key,
        generation: 1
      },
      brainPool: pool
    };
    const server = Object.create(SimServer.prototype) as SimServer;
    Object.assign(server, {
      mtEnabled: true,
      mtWorkerCount: 1,
      core,
      brainPool: pool,
      mtGeneration: 1
    });

    const initialPoolWeights = pool.weightSnapshot();
    const returned = await (server as unknown as CharacterizationSimServerAccess).ensureBrainPool();

    expect(returned).toBe(pool);
    expect(pool.syncCalls).toBe(0);
    expect(initialPoolWeights.length).toBe(populationWeights.length);
    expect(initialPoolWeights.every((value) => value === 0)).toBe(true);
    expect(Array.from(pool.weightSnapshot())).not.toEqual(Array.from(populationWeights));
  });

  it('PER-005 [expires/converts in Phase 7] automatic checkpoints are disabled by the current default', () => {
    expect(DEFAULT_CONFIG.checkpointEveryGenerations).toBe(0);
  });

  it('PER-005 [expires/converts in Phase 7] automatic persistence still observes after the Phase 2 boundary, spawn, and focus', () => {
    const events: string[] = [];
    const world = buildGenerationTransitionWorld(events);
    World.prototype._endGeneration.call(world);

    const snapshot: PopulationSnapshotPayload = {
      generation: world.generation,
      archKey: 'phase0-fixture',
      genomes: world.population.map((genome) => genome.toJSON()),
      cfgHash: 'phase0-cfg',
      worldSeed: 77
    };
    const persistence = {
      saveSnapshot: (_payload: PopulationSnapshotPayload) => {
        events.push(`snapshot@${world.generation}`);
        return 1;
      },
      saveHofEntry: () => undefined
    } as unknown as Persistence;
    const server = Object.create(SimServer.prototype) as SimServer;
    Object.assign(server, {
      core: { world },
      persistence,
      persistenceDisabledReason: null,
      checkpointEveryGenerations: 1,
      lastGeneration: 1,
      lastHofGenSaved: 0,
      buildSnapshotPayload: () => snapshot
    });

    (server as unknown as CharacterizationSimServerAccess).handleGenerationEnd();

    expect(events).toEqual([
      'clear@2',
      'bots@2',
      'boundary@2',
      'spawn@2',
      'pellets@2',
      'grid@2',
      'focus@2',
      'snapshot@2'
    ]);
  });
});
