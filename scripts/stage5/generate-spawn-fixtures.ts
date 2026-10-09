/** Reproduce retained TypeScript constructor spawn evidence for Stage 5. */

import { CFG_DEFAULT } from '../../src/config.ts';
import { StatefulRng } from '../../src/rng.ts';
import { TAU } from '../../src/utils.ts';

/** Audited TypeScript source revision represented by the retained artifact. */
const SOURCE_REVISION = '6bfcb24';
/** Deterministic seed shared with the Rust correction fixture. */
const FIXTURE_SEED = 0x5afe;

/** Construct exactly the current `Snake` random geometry and `_initBody` order. */
function buildSpawnFixture(): Record<string, unknown> {
  const rng = new StatefulRng(FIXTURE_SEED);
  const angle = rng.next() * TAU;
  const radius = Math.sqrt(rng.next()) * (CFG_DEFAULT.worldRadius * 0.6);
  let x = Math.cos(angle) * radius;
  let y = Math.sin(angle) * radius;
  const direction = rng.next() * TAU;
  const body: Array<[number, number]> = [[x, y]];
  for (let index = 1; index < CFG_DEFAULT.snakeStartLen; index++) {
    x -= Math.cos(direction) * CFG_DEFAULT.snakeSpacing;
    y -= Math.sin(direction) * CFG_DEFAULT.snakeSpacing;
    body.push([x, y]);
  }
  return {
    seed: FIXTURE_SEED,
    drawOrder: ['polarAngle', 'sqrtAreaRadius', 'heading'],
    settings: {
      worldRadius: CFG_DEFAULT.worldRadius,
      spawnRadiusFraction: 0.6,
      snakeSpacing: CFG_DEFAULT.snakeSpacing,
      snakeStartLen: CFG_DEFAULT.snakeStartLen
    },
    head: body[0],
    direction,
    body,
    rngAfterThreeDraws: rng.exportState()
  };
}

/** Construct the complete retained non-performance evidence document. */
function buildDocument(): Record<string, unknown> {
  return {
    schema: 'stage5-typescript-spawn-fixtures-v1',
    evidenceClass: 'current-source execution',
    sourceRevision: SOURCE_REVISION,
    environment: {
      platform: process.platform,
      architecture: process.arch,
      purpose: 'spawn formula and RNG ordering reference, not performance evidence'
    },
    source: {
      constructor: 'src/snake.ts::Snake.constructor',
      body: 'src/snake.ts::Snake._initBody',
      rng: 'src/rng.ts::StatefulRng',
      configuration: 'src/config.ts::CFG_DEFAULT',
      extractor: 'scripts/stage5/generate-spawn-fixtures.ts',
      runner: 'node ./node_modules/tsx/dist/cli.mjs scripts/stage5/generate-spawn-fixtures.ts'
    },
    fixture: buildSpawnFixture(),
    interpretation: {
      preserve:
        'three uniform draws, area-uniform 60-percent-radius head, heading, iterative head-to-tail body construction and RNG continuation',
      correct:
        'blind admission is replaced by complete-body wall/obstacle checks, stable request order and bounded deterministic fallback',
      notPerformanceEvidence: true
    }
  };
}

process.stdout.write(`${JSON.stringify(buildDocument(), null, 2)}\n`);
