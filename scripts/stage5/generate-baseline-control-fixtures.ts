import { BaselineBotManager, type BotAction, type BotState } from '../../src/bots/baselineBots.ts';
import { CFG } from '../../src/config.ts';
import { StatefulRng } from '../../src/rng.ts';
import type { World } from '../../src/world.ts';

/** Repository revision at which the current-source fixture was extracted. */
const REPOSITORY_REVISION = '55b2062fd1294d53667d959eee4110ebe1d3dab4';
/** Last commit changing the TypeScript baseline controller itself. */
const BASELINE_SOURCE_REVISION = '24b587a0cc541dcee7068d22d86e3a4946777c34';
/** Fixed-step delta used by every retained scenario. */
const FIXED_DT = 1 / 60;
/** Current sensor-v3 bin count used by the reference fixtures. */
const BINS = CFG.sense.bubbleBins;
/** Current sensor-v3 input length. */
const INPUT_SIZE = 19 + BINS * 4;

/** Minimal structural snake surface read by the baseline controller. */
interface FixtureSnake {
  id: number;
  alive: boolean;
  radius: number;
  dir: number;
  speed: number;
  pointsScore: number;
  head(): { x: number; y: number };
  length(): number;
  computeSensors(world: World): Float32Array;
}

/** Private manager fields retained only for reproducible current-source evidence. */
interface ManagerFixtureAccess {
  botRngs: StatefulRng[];
  botStates: BotState[];
  botStateTimers: number[];
  botWanderAngles: number[];
  botWanderTimers: number[];
}

/** Compact binned observation channels used to construct one sensor-v3 input. */
interface ObservationChannels {
  food: number[];
  hazard: number[];
  wall: number[];
  head: number[];
}

/** Compact one channel without hiding any non-uniform bin value. */
function compactChannel(values: readonly number[]): Record<string, unknown> {
  const defaultValue = values[0] ?? 0;
  return {
    default: defaultValue,
    overrides: values.flatMap((value, index) =>
      Object.is(value, defaultValue) ? [] : [{ index, value }]
    )
  };
}

/** Compact four explicit source channels for the retained JSON artifact. */
function compactChannels(input: ObservationChannels): Record<string, unknown> {
  return {
    food: compactChannel(input.food),
    hazard: compactChannel(input.hazard),
    wall: compactChannel(input.wall),
    head: compactChannel(input.head)
  };
}

/** One deterministic baseline-controller fixture scenario. */
interface Scenario {
  name: string;
  rngSeed: number;
  length: number;
  position: [number, number];
  radius: number;
  direction: number;
  speed: number;
  points: number;
  channels: ObservationChannels;
  others: FixtureSnake[];
}

/** Construct one repeated channel. */
function repeated(value: number): number[] {
  return Array.from({ length: BINS }, () => value);
}

/** Construct one compact observation-channel set. */
function channels(
  food = 0,
  hazard = 0.8,
  wall = 0.7,
  head = 0.9
): ObservationChannels {
  return {
    food: repeated(food),
    hazard: repeated(hazard),
    wall: repeated(wall),
    head: repeated(head)
  };
}

/** Pack binned channels after the nineteen scalar sensor-v3 values. */
function packObservation(input: ObservationChannels): Float32Array {
  const observation = new Float32Array(INPUT_SIZE);
  observation.set(input.food, 19);
  observation.set(input.hazard, 19 + BINS);
  observation.set(input.wall, 19 + BINS * 2);
  observation.set(input.head, 19 + BINS * 3);
  return observation;
}

/** Construct a structural snake with explicit geometry and observation data. */
function snake(
  id: number,
  length: number,
  position: [number, number],
  direction: number,
  speed: number,
  radius: number,
  points: number,
  observation: Float32Array
): FixtureSnake {
  return {
    id,
    alive: true,
    radius,
    dir: direction,
    speed,
    pointsScore: points,
    head: () => ({ x: position[0], y: position[1] }),
    length: () => length,
    computeSensors: () => observation
  };
}

/** Construct an alive non-controlled target snake. */
function target(
  id: number,
  length: number,
  position: [number, number],
  direction: number,
  speed: number
): FixtureSnake {
  return snake(id, length, position, direction, speed, 8, 0, new Float32Array(INPUT_SIZE));
}

/** Execute the real public manager update around one structural world fixture. */
function executeScenario(scenario: Scenario): Record<string, unknown> {
  const observation = packObservation(scenario.channels);
  const controlled = snake(
    700,
    scenario.length,
    scenario.position,
    scenario.direction,
    scenario.speed,
    scenario.radius,
    scenario.points,
    observation
  );
  const manager = new BaselineBotManager(
    { count: 1, seed: 0, randomizeSeedPerGen: false, respawnDelay: 3 },
    0
  );
  const internals = manager as unknown as ManagerFixtureAccess;
  internals.botRngs[0] = new StatefulRng(scenario.rngSeed);
  internals.botStates[0] = 'roam';
  internals.botStateTimers[0] = 0;
  internals.botWanderAngles[0] = 0;
  internals.botWanderTimers[0] = 0;
  const beforeRng = internals.botRngs[0]!.exportState();
  const structuralWorld = {
    baselineBots: [controlled],
    snakes: [controlled, ...scenario.others]
  } as unknown as World;

  manager.update(structuralWorld, FIXED_DT, () => null);
  const action = manager.getActionByIndex(0) as BotAction;

  return {
    name: scenario.name,
    input: {
      rngSeed: scenario.rngSeed,
      rngBefore: beforeRng,
      fixedDt: FIXED_DT,
      snake: {
        id: controlled.id,
        length: scenario.length,
        position: scenario.position,
        radius: scenario.radius,
        direction: scenario.direction,
        speed: scenario.speed,
        points: scenario.points
      },
      channels: compactChannels(scenario.channels),
      otherSnakes: scenario.others.map(other => ({
        id: other.id,
        alive: other.alive,
        length: other.length(),
        position: [other.head().x, other.head().y],
        direction: other.dir,
        speed: other.speed
      }))
    },
    output: {
      durableState: internals.botStates[0],
      strategyTimerSeconds: internals.botStateTimers[0],
      wanderAngle: internals.botWanderAngles[0],
      wanderTimerSeconds: internals.botWanderTimers[0],
      action: { turn: action.turn, boost: action.boost },
      rngAfter: internals.botRngs[0]!.exportState()
    }
  };
}

/** Build all retained life-stage, transition, and RNG-draw fixtures. */
function buildDocument(): Record<string, unknown> {
  const smallAvoid = channels(0.3, -0.8, 0.8, 0.9);
  smallAvoid.hazard[10] = 0.6;
  const scenarios: Scenario[] = [
    {
      name: 'small-roam-wander',
      rngSeed: 42,
      length: 12,
      position: [0, 0],
      radius: 8,
      direction: 0,
      speed: 120,
      points: 0,
      channels: channels(),
      others: []
    },
    {
      name: 'small-avoid-escape-boost',
      rngSeed: 99,
      length: 12,
      position: [0, 0],
      radius: 8,
      direction: 0,
      speed: 120,
      points: 0,
      channels: smallAvoid,
      others: []
    },
    {
      name: 'medium-cutoff-attack-boost',
      rngSeed: 123,
      length: 40,
      position: [0, 0],
      radius: 8,
      direction: 0,
      speed: 120,
      points: 0,
      channels: channels(0.2),
      others: [target(701, 20, [100, 100], 0, 10)]
    },
    {
      name: 'medium-encircle-relative-heading',
      rngSeed: 321,
      length: 60,
      position: [0, 0],
      radius: 8,
      direction: 0.3,
      speed: 120,
      points: 0,
      channels: channels(0.2),
      others: [target(701, 20, [100, 0], 0, 10)]
    },
    {
      name: 'medium-random-boost',
      rngSeed: 1,
      length: 40,
      position: [0, 0],
      radius: 8,
      direction: 0,
      speed: 120,
      points: 10,
      channels: channels(0.2),
      others: []
    },
    {
      name: 'large-crowd-bias',
      rngSeed: 555,
      length: 100,
      position: [0, 0],
      radius: 8,
      direction: 0,
      speed: 120,
      points: 0,
      channels: channels(0.2),
      others: [target(720, 20, [100, 0], 0.3, 10), target(710, 20, [0, 100], 0.8, 12)]
    }
  ];

  return {
    schema: 'stage5-typescript-baseline-control-fixtures-v1',
    evidenceClass: 'current-source execution',
    repositoryRevisionAtExtraction: REPOSITORY_REVISION,
    baselineSourceRevision: BASELINE_SOURCE_REVISION,
    environment: {
      platform: process.platform,
      architecture: process.arch,
      purpose: 'baseline formula, state-transition, and RNG-order reference; not performance evidence'
    },
    source: {
      controller: 'src/bots/baselineBots.ts::BaselineBotManager.update',
      configuration: 'src/config.ts::CFG_DEFAULT.boost.minPointsToBoost',
      extractor: 'scripts/stage5/generate-baseline-control-fixtures.ts',
      runner:
        'node ./node_modules/tsx/dist/cli.mjs scripts/stage5/generate-baseline-control-fixtures.ts'
    },
    sensor: {
      layoutVersion: CFG.sense.layoutVersion,
      bins: BINS,
      inputSize: INPUT_SIZE,
      scalarPrefix: 'nineteen explicit zero Float32 values',
      binnedChannels: 'recorded per scenario and packed as Float32 before current-source execution'
    },
    scenarios: scenarios.map(executeScenario),
    interpretation: {
      preserve:
        'life-stage thresholds, state/timer transitions, Float32 observation interpretation, bin scoring, action formulas, and exact per-slot uniform draw order',
      correct:
        'Rust consumes the shared corrected delivered observation instead of privately resampling; medium target ties and large centroid accumulation use stable snake-ID order rather than TypeScript container order',
      notPerformanceEvidence: true
    }
  };
}

process.stdout.write(`${JSON.stringify(buildDocument(), null, 2)}\n`);
