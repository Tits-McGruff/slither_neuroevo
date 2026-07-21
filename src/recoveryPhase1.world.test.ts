import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';
import type { Snake } from './snake.ts';
import {
  World,
  type BatchInferenceRunner,
  type ControllerRegistryLike
} from './world.ts';

/** Phase 1 canonical World-step regression suite label. */
const SUITE = 'recovery Phase 1 — canonical World step';

/** Kind of hand-owned snake used by the ordering fixture. */
type StepSnakeKind = 'neural' | 'external' | 'baseline';

/** Fixture returned for one canonical step-path test. */
interface StepFixture {
  /** Hand-owned World using production prototype methods. */
  world: World;
  /** External-controller collaborator. */
  controllers: ControllerRegistryLike;
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.pelletCountTarget = 0;
  CFG.pelletSpawnPerSecond = 0;
  CFG.baselineBots.count = 1;
  CFG.collision.substepMaxDt = 1;
  CFG.observer.earlyEndAliveThreshold = -1;
});

afterEach(() => {
  resetCFGToDefaults();
});

/**
 * Build one minimal snake collaborator for the canonical step.
 * @param id - Stable snake id.
 * @param kind - Control branch selected for the snake.
 * @param populationSlot - Durable population slot or null.
 * @param events - Shared execution trace.
 * @returns Hand-owned snake collaborator.
 */
function buildStepSnake(
  id: number,
  kind: StepSnakeKind,
  populationSlot: number | null,
  events: string[]
): Snake {
  const sensors = new Float32Array(CFG.brain.inSize);
  return {
    id,
    alive: true,
    x: 0,
    y: 0,
    dir: 0,
    pointsScore: id,
    points: [],
    populationSlot,
    baselineBotIndex: kind === 'baseline' ? 0 : null,
    controlMode: kind === 'baseline' ? 'external-only' : 'neural',
    lastSensors: null,
    prepareForStep: (_dt: number) => events.push(`${kind}:prepare`),
    needsControlUpdate: (_dt: number) => {
      events.push(`${kind}:needs-control`);
      return true;
    },
    computeSensors: (_world: World) => {
      events.push(`${kind}:sensors`);
      return sensors;
    },
    brain: {
      forward: (_inputs: Float32Array) => {
        events.push(`${kind}:forward`);
        return Float32Array.of(0.25, 0);
      }
    },
    applyExternalControl: () => events.push(`${kind}:apply-external`),
    applyBrainOutput: () => events.push(`${kind}:apply-brain`),
    advance: (_world: World, _dt: number) => events.push(`${kind}:advance`)
  } as unknown as Snake;
}

/**
 * Build a deterministic World-shaped component using production step methods.
 * @param events - Shared execution trace.
 * @returns Canonical-step fixture.
 */
function buildStepFixture(events: string[]): StepFixture {
  const neural = buildStepSnake(11, 'neural', 0, events);
  const external = buildStepSnake(22, 'external', 1, events);
  const baseline = buildStepSnake(33, 'baseline', null, events);
  const snakes = [neural, external, baseline];
  const inputStride = CFG.brain.inSize;
  const outputStride = CFG.brain.outSize;
  const world = Object.assign(Object.create(World.prototype) as World, {
    backend: null,
    profiler: undefined,
    generation: 1,
    generationTime: 0,
    tickId: 0,
    particles: { update: (_dt: number) => events.push('step:particles') },
    bestPointsThisGen: 0,
    bestPointsSnakeId: 0,
    population: [{}, {}],
    snakes,
    baselineBots: [baseline],
    pellets: [],
    _pelletSpawnAcc: 0,
    _didWarnSensorLayout: true,
    _pendingControlSource: new Uint8Array(snakes.length),
    _pendingControlTurn: new Float32Array(snakes.length),
    _pendingControlBoost: new Float32Array(snakes.length),
    _serialControlIndices: new Uint32Array(snakes.length),
    _serialControlCount: 0,
    _controlBatch: {
      indices: new Uint32Array(snakes.length),
      snakeIndices: new Uint32Array(snakes.length),
      count: 0,
      capacity: snakes.length,
      inputStride,
      outputStride,
      inputs: new Float32Array(snakes.length * inputStride),
      outputs: new Float32Array(snakes.length * outputStride)
    },
    botManager: {
      getCount: () => 1,
      update: () => events.push('baseline:update'),
      getActionForSnake: (snakeId: number) => {
        if (snakeId !== baseline.id) return null;
        events.push('baseline:get-action');
        return { turn: 0.75, boost: 0 };
      }
    },
    _collGrid: {
      reset: (_cellSize: number) => events.push('physics:grid-reset'),
      add: () => undefined
    },
    _resolveCollisionsGrid: () => events.push('physics:collisions'),
    _updateFocus: (_dt: number) => events.push('step:focus'),
    _updateCamera: (_viewW: number, _viewH: number) => events.push('step:camera'),
    _endGeneration: () => events.push('step:end-generation')
  });
  const controllers: ControllerRegistryLike = {
    isControlled: (snakeId) => snakeId === external.id,
    getAction: (snakeId) => {
      if (snakeId !== external.id) return null;
      events.push('external:get-action');
      return { turn: -0.5, boost: 0 };
    },
    publishSensors: () => events.push('external:publish-sensors')
  };
  return { world, controllers };
}

/**
 * Normalize the intentionally different inference implementation marker.
 * @param events - Serial or pooled trace.
 * @returns Trace for semantic ordering comparison.
 */
function normalizeInference(events: string[]): string[] {
  return events.map((event) => (
    event === 'neural:forward' || event === 'pooled:infer'
      ? 'neural:inference'
      : event
  ));
}

describe(SUITE, () => {
  it('CORE-001/CORE-002 gives serial and pooled inference one equivalent control/physics ordering', async () => {
    const serialEvents: string[] = [];
    const pooledEvents: string[] = [];
    const serial = buildStepFixture(serialEvents);
    const pooled = buildStepFixture(pooledEvents);
    const observedSlots: number[] = [];
    const runner: BatchInferenceRunner = {
      runBatch: async (_inputs, outputs, indices, count, _inputStride, outputStride) => {
        pooledEvents.push('pooled:infer');
        observedSlots.push(...Array.from(indices.subarray(0, count)));
        for (let batchIndex = 0; batchIndex < count; batchIndex++) {
          outputs[batchIndex * outputStride] = 0.25;
          if (outputStride > 1) outputs[batchIndex * outputStride + 1] = 0;
        }
      }
    };

    await serial.world.step(1 / 60, 800, 600, serial.controllers, 1);
    await pooled.world.step(1 / 60, 800, 600, pooled.controllers, 1, runner);

    expect(normalizeInference(pooledEvents)).toEqual(normalizeInference(serialEvents));
    expect(observedSlots).toEqual([0]);
    expect(serialEvents.indexOf('neural:forward')).toBeGreaterThan(
      serialEvents.indexOf('baseline:get-action')
    );
    expect(serialEvents.indexOf('neural:forward')).toBeLessThan(
      serialEvents.indexOf('neural:apply-brain')
    );
    expect(serialEvents.filter(event => event === 'external:sensors')).toHaveLength(1);
    expect(serialEvents.indexOf('external:publish-sensors')).toBeLessThan(
      serialEvents.indexOf('baseline:update')
    );
    expect(serialEvents.indexOf('baseline:update')).toBeLessThan(
      serialEvents.indexOf('baseline:get-action')
    );
    expect(serialEvents.indexOf('external:get-action')).toBeLessThan(
      serialEvents.indexOf('neural:forward')
    );
    expect(serialEvents.indexOf('baseline:get-action')).toBeLessThan(
      serialEvents.indexOf('neural:forward')
    );
    expect(serial.world.tickId).toBe(1);
    expect(pooled.world.tickId).toBe(1);
  });

  it('assigns durable slots only to population-owned spawn paths', () => {
    const world = new World({ snakeCount: 3 });

    expect(world.snakes.slice(0, 3).map(snake => snake.populationSlot)).toEqual([0, 1, 2]);
    expect(world.baselineBots.map(snake => snake.populationSlot)).toEqual([null]);

    const external = world.spawnExternalSnake();
    expect(external.populationSlot).toBeNull();
    external.alive = false;
    expect(world.spawnExternalSnake().populationSlot).toBeNull();

    const genome = world.population[0];
    if (!genome) throw new Error('expected a population genome');
    const resurrectedId = world.resurrect(genome.toJSON());
    expect(world.snakes.find(snake => snake.id === resurrectedId)?.populationSlot).toBeNull();

    world._spawnAll();
    expect(world.snakes.slice(0, 3).map(snake => snake.populationSlot)).toEqual([0, 1, 2]);
    expect(world.baselineBots.map(snake => snake.populationSlot)).toEqual([null]);
  });

  it('subdivides one fixed delta for collision safety without consulting speed', async () => {
    const baseDt = 1 / 60;
    CFG.collision.substepMaxDt = baseDt / 4;
    const normalEvents: string[] = [];
    const fastEvents: string[] = [];
    const normal = buildStepFixture(normalEvents);
    const fast = buildStepFixture(fastEvents);
    normal.world.simSpeed = 1;
    fast.world.simSpeed = 12;

    await normal.world.step(baseDt, 800, 600, normal.controllers, 1);
    await fast.world.step(baseDt, 800, 600, fast.controllers, 1);

    expect(normalEvents.filter(event => event === 'physics:collisions')).toHaveLength(4);
    expect(normalEvents.filter(event => event.endsWith(':advance'))).toHaveLength(12);
    expect(fastEvents).toEqual(normalEvents);
  });

  it('MT-010 leaves movement and the published World tick uncommitted when pooled inference rejects', async () => {
    const events: string[] = [];
    const fixture = buildStepFixture(events);
    const runner: BatchInferenceRunner = {
      runBatch: async () => {
        events.push('pooled:reject');
        throw new Error('phase1 inference failure');
      }
    };

    await expect(
      fixture.world.step(1 / 60, 800, 600, fixture.controllers, 1, runner)
    ).rejects.toThrow('phase1 inference failure');
    expect(fixture.world.tickId).toBe(0);
    expect(events).toContain('neural:prepare');
    expect(events).not.toContain('neural:apply-brain');
    expect(events).not.toContain('neural:advance');
    expect(events).not.toContain('physics:collisions');
  });
});
