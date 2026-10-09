/** Reproduce current TypeScript ambient-pellet ordering and RNG evidence. */

import { CFG } from '../../src/config.ts';
import { StatefulRng } from '../../src/rng.ts';
import { Pellet } from '../../src/snake.ts';
import { World } from '../../src/world.ts';

/** Audited TypeScript source revision represented by the retained artifact. */
const SOURCE_REVISION = 'c76cdc2';
/** Deterministic world-RNG seed shared with the Rust fixture. */
const EARLY_ACCEPT_SEED = 0xa11ce;
/** Generation time after the fixed-step increment. */
const GENERATION_TIME = 1 / 60;

/** Erased private surface used only by this current-source evidence runner. */
interface AmbientWorldFixture {
  /** Current generation time read by the density formula. */
  generationTime: number;
  /** Current fractional pellet credit. */
  _pelletSpawnAcc: number;
  /** Current pellet array read by the target-deficit formula. */
  pellets: Pellet[];
  /** Injectable world RNG surface. */
  worldRng: { next: () => number };
  /** Current source method under audit. */
  _spawnAmbientPellet: () => Pellet;
  /** Current source fixed-step accumulator method under audit. */
  _spawnAmbientForFixedStep: (dt: number) => void;
  /** Current append hook replaced with a minimal in-memory sink. */
  addPellet: (pellet: Pellet) => void;
}

/** Build a source-`World` prototype object without constructing a population. */
function fixtureWorld(
  next: () => number,
  generationTime = GENERATION_TIME
): AmbientWorldFixture {
  const world = Object.create(World.prototype) as AmbientWorldFixture;
  world.generationTime = generationTime;
  world._pelletSpawnAcc = 0;
  world.pellets = [];
  world.worldRng = { next };
  world.addPellet = pellet => world.pellets.push(pellet);
  return world;
}

/** Execute one default-config early-acceptance sampler through current source. */
function earlyAcceptanceFixture(): Record<string, unknown> {
  const rng = new StatefulRng(EARLY_ACCEPT_SEED);
  let draws = 0;
  const world = fixtureWorld(() => {
    draws += 1;
    return rng.next();
  });
  const pellet = world._spawnAmbientPellet();
  return {
    seed: EARLY_ACCEPT_SEED,
    generationTime: GENERATION_TIME,
    drawOrder: ['polarAngle', 'sqrtAreaRadius', 'acceptance'],
    draws,
    attempts: draws / 3,
    pellet: {
      position: [pellet.x, pellet.y],
      value: pellet.v,
      kind: pellet.kind,
      colorId: pellet.colorId
    },
    rngAfter: rng.exportState()
  };
}

/** Execute all eight rejected attempts and prove strict-first best tie fallback. */
function fallbackTieFixture(): Record<string, unknown> {
  const priorFoodSpawn = { ...CFG.foodSpawn };
  CFG.foodSpawn.edgeFalloffEnabled = false;
  CFG.foodSpawn.warpFreq = 0;
  CFG.foodSpawn.warpScale = 0;
  CFG.foodSpawn.freqLarge = 0;
  CFG.foodSpawn.freqMedium = 0;
  CFG.foodSpawn.freqSmall = 0;
  CFG.foodSpawn.dustStrength = 0;
  const draws: number[] = [];
  for (let attempt = 0; attempt < 8; attempt++) {
    draws.push(attempt / 8, (attempt + 1) / 9, 0.999999);
  }
  let cursor = 0;
  try {
    const world = fixtureWorld(() => {
      const value = draws[cursor];
      if (value == null) throw new Error(`ambient fallback fixture overdraw at ${cursor}`);
      cursor += 1;
      return value;
    }, 0);
    const pellet = world._spawnAmbientPellet();
    return {
      generationTime: 0,
      constantDensitySettings: {
        edgeFalloffEnabled: false,
        warpFreq: 0,
        warpScale: 0,
        freqLarge: 0,
        freqMedium: 0,
        freqSmall: 0,
        dustStrength: 0
      },
      suppliedDraws: draws,
      consumedDraws: cursor,
      attempts: cursor / 3,
      expectedTieWinner: 'first candidate',
      firstCandidate: [Math.sqrt(1 / 9) * CFG.worldRadius, 0],
      pellet: {
        position: [pellet.x, pellet.y],
        value: pellet.v,
        kind: pellet.kind,
        colorId: pellet.colorId
      }
    };
  } finally {
    Object.assign(CFG.foodSpawn, priorFoodSpawn);
  }
}

/** Execute credit accumulation while full, then a bounded refill after removal. */
function accumulatorFixture(): Record<string, unknown> {
  const priorTarget = CFG.pelletCountTarget;
  const priorRate = CFG.pelletSpawnPerSecond;
  CFG.pelletCountTarget = 2;
  CFG.pelletSpawnPerSecond = 2;
  const rng = new StatefulRng(0xacc);
  let draws = 0;
  try {
    const world = fixtureWorld(() => {
      draws += 1;
      return rng.next();
    });
    world.pellets.push(
      new Pellet(0, 0, 1, null, 'ambient', 0),
      new Pellet(1, 0, 1, null, 'ambient', 0)
    );
    world._pelletSpawnAcc = 0.25;
    world._spawnAmbientForFixedStep(0.25);
    const whileFull = {
      pelletCount: world.pellets.length,
      accumulator: world._pelletSpawnAcc,
      draws
    };
    world.pellets.pop();
    world._spawnAmbientForFixedStep(0.125);
    return {
      initial: { pelletCount: 2, accumulator: 0.25 },
      whileFull,
      afterOneRemoval: {
        pelletCount: world.pellets.length,
        accumulator: world._pelletSpawnAcc,
        draws,
        rngAfter: rng.exportState(),
        appendedPellet: world.pellets[world.pellets.length - 1]
          ? {
              position: [
                world.pellets[world.pellets.length - 1]!.x,
                world.pellets[world.pellets.length - 1]!.y
              ],
              kind: world.pellets[world.pellets.length - 1]!.kind
            }
          : null
      }
    };
  } finally {
    CFG.pelletCountTarget = priorTarget;
    CFG.pelletSpawnPerSecond = priorRate;
  }
}

/** Construct the complete retained non-performance evidence document. */
function buildDocument(): Record<string, unknown> {
  return {
    schema: 'stage5-typescript-ambient-fixtures-v1',
    evidenceClass: 'current-source execution',
    sourceRevision: SOURCE_REVISION,
    environment: {
      platform: process.platform,
      architecture: process.arch,
      purpose: 'ambient formula, accumulator and RNG ordering reference, not performance evidence'
    },
    source: {
      accumulator: 'src/world.ts::World._spawnAmbientForFixedStep',
      sampler: 'src/world.ts::World._spawnAmbientPellet',
      rng: 'src/rng.ts::StatefulRng',
      configuration: 'src/config.ts::CFG_DEFAULT',
      extractor: 'scripts/stage5/generate-ambient-fixtures.ts',
      runner: 'node ./node_modules/tsx/dist/cli.mjs scripts/stage5/generate-ambient-fixtures.ts'
    },
    settings: {
      worldRadius: CFG.worldRadius,
      pelletCountTarget: CFG.pelletCountTarget,
      pelletSpawnPerSecond: CFG.pelletSpawnPerSecond,
      foodValue: CFG.foodValue,
      foodSpawn: { ...CFG.foodSpawn }
    },
    fixtures: {
      earlyAcceptance: earlyAcceptanceFixture(),
      fallbackTie: fallbackTieFixture(),
      accumulator: accumulatorFixture()
    },
    interpretation: {
      preserve:
        'fractional credit including while full, post-generation-time density, angle/radius/acceptance draw order, eight attempts, strict-first best fallback and pellet fields',
      correct:
        'Rust stages IDs, capacity, accumulator, RNG and all generated pellets atomically instead of mutating credit/array/grid incrementally',
      notPerformanceEvidence: true
    }
  };
}

process.stdout.write(`${JSON.stringify(buildDocument(), null, 2)}\n`);
