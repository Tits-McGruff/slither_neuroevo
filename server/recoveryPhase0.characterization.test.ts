import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { resetCFGToDefaults } from '../src/config.ts';
import { Genome } from '../src/mlp.ts';
import { World } from '../src/world.ts';
import { DEFAULT_CONFIG } from './config.ts';
import type { Persistence, PopulationSnapshotPayload } from './persistence.ts';
import { SimServer } from './simServer.ts';

/** Unmistakable suite label for server-side Phase 0 characterizations. */
const SUITE = 'recovery Phase 0 characterization — current server behavior';

/** Narrow access to SimServer methods that are private in production. */
interface CharacterizationSimServerAccess {
  /** Execute current generation-change persistence logic. */
  handleGenerationEnd: () => void;
}

beforeEach(() => {
  resetCFGToDefaults();
});

afterEach(() => {
  resetCFGToDefaults();
});

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
