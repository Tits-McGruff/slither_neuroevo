import type { Brain } from '../../src/brains/types.ts';
import type { ArchDefinition } from '../../src/mlp.ts';
import { Genome } from '../../src/mlp.ts';
import { StatefulRng } from '../../src/rng.ts';
import { Pellet, Snake } from '../../src/snake.ts';

/** Audited TypeScript source revision represented by this execution artifact. */
const SOURCE_REVISION = 'b2d9648';
/** Stable stream seed used by the retained death/drop fixture. */
const RNG_SEED = 0x5eed5;
/** Stable snake identity and pellet color identifier. */
const SNAKE_ID = 7;
/** Body length exercising multiple large and small corpse pellets. */
const BODY_LENGTH = 12;

/** Minimal graph shape required by `Snake`; the injected brain prevents compilation. */
const FIXTURE_ARCH: ArchDefinition = {
  key: 'stage5-death-fixture',
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

/** Inert brain used because the fixture invokes only the death path. */
const FIXTURE_BRAIN: Brain = {
  inferenceBackend: 'js',
  forward: () => new Float32Array(2),
  reset: () => undefined,
  paramLength: () => 0,
  getVizData: () => ({ kind: 'fixture', layers: [] })
};

/** Construct one explicit live snake while retaining its injected RNG continuation. */
function buildSnake(rng: StatefulRng): Snake {
  const genome = new Genome(FIXTURE_ARCH.key, new Float32Array(0));
  const snake = new Snake(SNAKE_ID, genome, FIXTURE_ARCH, {
    brain: FIXTURE_BRAIN,
    rng: rng.asSource()
  });
  snake.x = 0;
  snake.y = 0;
  snake.dir = 0;
  snake.radius = 9;
  snake.alive = true;
  snake.points = Array.from({ length: BODY_LENGTH }, (_, index) => ({
    x: -index * 7.5,
    y: index % 2 === 0 ? 0 : 1.25
  }));
  snake.targetLen = BODY_LENGTH;
  return snake;
}

/** Execute current `Snake.die` and retain its exact uniform continuation. */
function buildDocument(): Record<string, unknown> {
  const rng = new StatefulRng(RNG_SEED);
  const snake = buildSnake(rng);
  const beforeRng = rng.exportState();
  const pellets: Pellet[] = [];
  let baselineNotifications = 0;
  snake.die({
    pellets,
    particles: { spawnBurst: () => undefined, spawnBoost: () => undefined },
    addPellet: pellet => pellets.push(pellet),
    removePellet: () => undefined,
    bestPointsThisGen: 0,
    baselineBotDied: () => {
      baselineNotifications += 1;
    }
  });

  return {
    schema: 'stage5-typescript-death-fixtures-v1',
    evidenceClass: 'current-source execution',
    sourceRevision: SOURCE_REVISION,
    environment: {
      platform: process.platform,
      architecture: process.arch,
      purpose: 'formula, RNG, and ordering reference; not performance evidence'
    },
    source: {
      death: 'src/snake.ts::Snake.die',
      configuration: 'src/config.ts::CFG_DEFAULT.death',
      extractor: 'scripts/stage5/generate-death-fixtures.ts',
      runner:
        'node ./node_modules/tsx/dist/cli.mjs scripts/stage5/generate-death-fixtures.ts'
    },
    input: {
      rngSeed: RNG_SEED,
      rngBeforeDeath: beforeRng,
      snakeId: SNAKE_ID,
      body: snake.points.map(point => [point.x, point.y])
    },
    output: {
      alive: snake.alive,
      baselineNotifications,
      rngAfterDeath: rng.exportState(),
      pellets: pellets.map(pellet => ({
        position: [pellet.x, pellet.y],
        value: pellet.v,
        kind: pellet.kind,
        colorId: pellet.colorId
      }))
    },
    interpretation: {
      preserve:
        'mass fraction, pellet counts and placement, value jitter, position jitter, stable per-owner RNG draw order, and one invocation of the optional death callback; this fixture does not assign baseline identity',
      correct:
        'the Rust transaction preflights all effects and applies simultaneous deaths in stable snake-ID order instead of mutating during container traversal; Rust integration tests baseline-only event semantics',
      notPerformanceEvidence: true
    }
  };
}

process.stdout.write(`${JSON.stringify(buildDocument(), null, 2)}\n`);
