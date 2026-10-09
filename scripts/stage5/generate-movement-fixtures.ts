/** Reproduce retained TypeScript movement/body formula evidence for Stage 5. */

import { Genome, type ArchDefinition } from '../../src/mlp.ts';
import type { Brain } from '../../src/brains/types.ts';
import { Pellet, Snake, type ControlInput } from '../../src/snake.ts';

/** Audited TypeScript source revision represented by the retained artifact. */
const SOURCE_REVISION = '6bfcb24';
/** Collision-substep duration used by the retained movement cases. */
const SUBSTEP_SECONDS = 1 / 180;
/** Exact RNG draws consumed by spawn construction before fixture fields replace it. */
const CONSTRUCTOR_RNG_DRAWS = [0, 0, 0] as const;
/** Exact draws converted to the retained boost-pellet X/Y jitter. */
const BOOST_JITTER_RNG_DRAWS = [0.23791737877763808, 0.2188026534859091] as const;

/** Minimal graph shape required by `Snake`; the injected brain prevents compilation. */
const FIXTURE_ARCH: ArchDefinition = {
  key: 'stage5-movement-fixture',
  spec: {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: 1 },
      { id: 'output', type: 'Dense', inputSize: 1, outputSize: 2 }
    ],
    edges: [{ from: 'input', to: 'output' }],
    outputs: [{ nodeId: 'output' }],
    outputSize: 2
  }
};

/** Inert brain used because these fixtures call movement directly. */
const FIXTURE_BRAIN: Brain = {
  inferenceBackend: 'js',
  forward: () => new Float32Array(2),
  reset: () => undefined,
  paramLength: () => 0,
  getVizData: () => ({ kind: 'fixture', layers: [] })
};

/** JSON shape retained for one deterministic movement execution. */
interface MovementFixtureRecord {
  /** Human-readable fixture identity. */
  name: string;
  /** Exact inputs installed after construction. */
  input: Record<string, unknown>;
  /** Values observed after `Snake.advance`. */
  output: Record<string, unknown>;
}

/** One explicit source pellet installed before `Snake.advance`. */
interface FixturePellet {
  /** World X coordinate. */
  x: number;
  /** World Y coordinate. */
  y: number;
  /** Positive food value. */
  value: number;
}

/** Build a finite deterministic RNG stream and fail if the fixture overdraws it. */
function sequenceRng(values: readonly number[]): () => number {
  let index = 0;
  return () => {
    const value = values[index];
    if (value == null) throw new Error(`movement fixture exhausted RNG at draw ${index}`);
    index += 1;
    return value;
  };
}

/** Build one source `Snake` with explicit scalar/body state and an inert brain. */
function buildSnake(
  length: number,
  control: ControlInput,
  rngValues: readonly number[]
): Snake {
  const genome = new Genome(FIXTURE_ARCH.key, new Float32Array(0));
  const snake = new Snake(1, genome, FIXTURE_ARCH, {
    brain: FIXTURE_BRAIN,
    rng: sequenceRng(rngValues)
  });
  snake.x = 0;
  snake.y = 0;
  snake.dir = 0;
  snake.radius = 9;
  snake.speed = 165;
  snake.boost = 0;
  snake.alive = true;
  snake.foodEaten = 0;
  snake.pointsScore = 10;
  snake.targetLen = length;
  snake.points = Array.from({ length }, (_, index) => ({ x: -index * 7.5, y: 0 }));
  snake.turnInput = control.turn;
  snake.boostInput = control.boost;
  return snake;
}

/** Execute one current-source `Snake.advance` case and serialize relevant fields. */
function executeFixture(
  name: string,
  length: number,
  control: ControlInput,
  rngValues: readonly number[],
  initialPellets: readonly FixturePellet[] = []
): MovementFixtureRecord {
  const snake = buildSnake(length, control, rngValues);
  const pellets = initialPellets.map(
    entry => new Pellet(entry.x, entry.y, entry.value, null, 'ambient', 0)
  );
  const world: Parameters<Snake['advance']>[0] = {
    pellets,
    particles: { spawnBurst: () => undefined, spawnBoost: () => undefined },
    addPellet: pellet => pellets.push(pellet),
    removePellet: pellet => {
      const index = pellets.indexOf(pellet);
      if (index >= 0) pellets.splice(index, 1);
    },
    bestPointsThisGen: 0
  };
  const input = {
    position: [snake.x, snake.y],
    direction: snake.dir,
    speed: snake.speed,
    turn: control.turn,
    boostInput: control.boost,
    pointsScore: snake.pointsScore,
    targetLength: snake.targetLen,
    body: snake.points.map(point => [point.x, point.y]),
    pellets: pellets.map(pellet => ({
      position: [pellet.x, pellet.y],
      value: pellet.v,
      kind: pellet.kind,
      colorId: pellet.colorId
    }))
  };
  snake.advance(world, SUBSTEP_SECONDS);
  return {
    name,
    input,
    output: {
      position: [snake.x, snake.y],
      direction: snake.dir,
      speed: snake.speed,
      boost: snake.boost,
      pointsScore: snake.pointsScore,
      targetLength: snake.targetLen,
      radius: snake.radius,
      body: snake.points.map(point => [point.x, point.y]),
      pellets: pellets.map(pellet => ({
        position: [pellet.x, pellet.y],
        value: pellet.v,
        kind: pellet.kind,
        colorId: pellet.colorId
      }))
    }
  };
}

/** Construct the complete retained current-source execution document. */
function buildDocument(): Record<string, unknown> {
  return {
    schema: 'stage5-typescript-movement-fixtures-v1',
    evidenceClass: 'current-source execution',
    sourceRevision: SOURCE_REVISION,
    environment: {
      platform: process.platform,
      architecture: process.arch,
      purpose: 'formula and ordering reference, not target-VM performance evidence'
    },
    source: {
      movement: 'src/snake.ts::Snake.advance',
      configuration: 'src/config.ts::CFG_DEFAULT',
      extractor: 'scripts/stage5/generate-movement-fixtures.ts',
      runner:
        'node ./node_modules/tsx/dist/cli.mjs scripts/stage5/generate-movement-fixtures.ts'
    },
    deterministicRng: {
      constructorDraws: CONSTRUCTOR_RNG_DRAWS,
      boostJitterDraws: BOOST_JITTER_RNG_DRAWS
    },
    substepSeconds: SUBSTEP_SECONDS,
    fixtures: [
      executeFixture(
        'ordinary-turn-length-5',
        5,
        { turn: 1, boost: 0 },
        CONSTRUCTOR_RNG_DRAWS
      ),
      executeFixture(
        'boost-turn-shrink-length-8',
        8,
        { turn: -0.5, boost: 1 },
        [...CONSTRUCTOR_RNG_DRAWS, ...BOOST_JITTER_RNG_DRAWS]
      ),
      executeFixture(
        'food-grow-length-5',
        5,
        { turn: 0, boost: 0 },
        CONSTRUCTOR_RNG_DRAWS,
        [{ x: 0, y: 0, value: 1 }]
      )
    ],
    interpretation: {
      preserve:
        'turn, boost burn, pre-movement boost shrink, speed interpolation, body following, target length and radius formulas',
      defer:
        'the deterministic jittered boost pellet is retained for later per-owner RNG realization; the Rust movement stage currently emits the deterministic pre-jitter request only',
      notPerformanceEvidence: true
    }
  };
}

process.stdout.write(`${JSON.stringify(buildDocument(), null, 2)}\n`);
