/** Contract tests for reproducible Stage 2 fixtures. */

import { createHash } from 'node:crypto';
import { afterEach, describe, expect, it } from 'vitest';
import { resetCFGToDefaults } from '../../src/config.ts';
import { Genome } from '../../src/mlp.ts';
import { World } from '../../src/world.ts';
import {
  buildLargeBrainGraph,
  captureStage2WorldLoad,
  describePopulation,
  evolvePopulationFixture,
  expectedStage2CollisionEntries,
  installDenseLongBodies,
  installStage2Scenario,
  packPopulationWeights,
  P4_DENSE_COLLISION_ENTRY_THRESHOLD,
  shuffleFloat32Bytes,
  STAGE2_WORLD_SEED,
  unshuffleFloat32Bytes
} from './fixtures.ts';
import { compileGraph } from '../../src/brains/graph/compiler.ts';

describe('Stage 2 fixtures', () => {
  afterEach(() => {
    resetCFGToDefaults();
  });

  it('pins the default and approved large-brain parameter counts', () => {
    const defaultScenario = installStage2Scenario('P0');
    const defaultWorld = new World(defaultScenario.settings, { seed: STAGE2_WORLD_SEED });
    expect(defaultWorld.population).toHaveLength(55);
    expect(defaultWorld.population[0]?.weights).toHaveLength(13_458);

    const large = buildLargeBrainGraph(147, 'GRU');
    const compiled = compileGraph(large);
    expect(compiled.totalParams).toBe(402_914);
    expect(compiled.order).toEqual(['input', 'features', 'memory', 'output']);
    expect(compiled.totalStateSize).toBe(96);
  });

  it('round-trips the byte-shuffled representation bit exactly', () => {
    const source = Buffer.allocUnsafe(4 * 257);
    for (let index = 0; index < source.length; index++) source[index] = (index * 73 + 19) & 0xff;
    const shuffled = shuffleFloat32Bytes(source);
    expect(shuffled).not.toEqual(source);
    expect(unshuffleFloat32Bytes(shuffled)).toEqual(source);
  });

  it('packs explicit little-endian Float32 words', () => {
    const packed = packPopulationWeights([
      new Genome('fixture', new Float32Array([1, -2.5]), 'graph')
    ]);
    expect(packed.toString('hex')).toBe('0000803f000020c0');
  });

  it('installs a P4 world above the historical collision-entry ceiling without truncation', () => {
    const scenario = installStage2Scenario('P4');
    const world = new World(scenario.settings, { seed: STAGE2_WORLD_SEED });
    const installedBodyPoints = installDenseLongBodies(world);
    const load = captureStage2WorldLoad(world);
    const expectedCollisionEntries = expectedStage2CollisionEntries(world);
    expect(installedBodyPoints).toBeGreaterThan(P4_DENSE_COLLISION_ENTRY_THRESHOLD);
    expect(load.totalBodyPoints).toBe(installedBodyPoints);
    expect(load.liveBodyPoints).toBe(installedBodyPoints);
    expect(expectedCollisionEntries).toBeGreaterThan(P4_DENSE_COLLISION_ENTRY_THRESHOLD);
    expect(load.collisionGrid.currentEntries).toBe(expectedCollisionEntries);
    expect(load.collisionGrid.outOfBoundsEntries).toBe(0);
    expect(load.collisionGrid.faultReason).toBeNull();
  });

  it('reproduces evolved-like operator fixtures from the same seed', () => {
    const scenario = installStage2Scenario('P0');
    const world = new World(scenario.settings, { seed: STAGE2_WORLD_SEED });
    const first = evolvePopulationFixture(world.population, world.arch, 3);
    const second = evolvePopulationFixture(world.population, world.arch, 3);
    const sourceDescription = describePopulation(world.population, world.arch);
    const firstDescription = describePopulation(first, world.arch);
    const secondDescription = describePopulation(second, world.arch);
    expect(firstDescription.rawSha256).toBe(secondDescription.rawSha256);
    expect(firstDescription.rawSha256).not.toBe(sourceDescription.rawSha256);
    expect(createHash('sha256').update(packPopulationWeights(first)).digest('hex'))
      .toBe(firstDescription.rawSha256);
  });
});
