import { describe, expect, it } from 'vitest';
import { inspectorFixture, inspectorLifecycleFixture } from '../test/inspectorFixture.ts';
import { parseInspectorSnapshot } from './inspector.ts';

/** Synthetic metadata only; no graph or observation is executed in this fixture. */
function snapshot(profile = 'v3', bins = 16) {
  return {schemaVersion: 1, sequence: 'ffffffffffffffff', subscriptionSequence: '0000000000000001', worldEpoch: 'ffffffffffffffff',
    completedStep: 'ffffffffffffffff', sample: {generation: 'ffffffffffffffff', physicalId: 'ffffffffffffffff',
      snakeId: 16_777_216, populationSlot: null, sampleStep: 'fffffffffffffffe', ageSteps: '0000000000000001',
      source: {kind: 'neural'}, profile, bins, sensorSha256: 'a'.repeat(64),
      normalizedInputs: Array(profile === 'v3' ? 19 + 4 * bins : 24 + (profile === 'basic' ? 6 : 15) * bins).fill(0),
      neuralOutputs: [2.5, -4], baselineAction: null,
      raw: {kind: 'retained-neural', points: 0, previouslyDeliveredPoints: 0, firstDelivery: true,
        x: 0, y: 0, heading: 0, radius: 9, speed: 165, controlAccumulatorSeconds: 0.06},
      trace: null, diagnostics: {pelletChecks: 0, segmentChecks: 0, headChecks: 0, rayWork: 0, incompleteRays: 0}}};
}

/** Complete native-style trace with exact fixed-size channel prefixes. */
function trace(bins: number) {
  return {nearRadius: 100, farRadius: 200, bodyLength: 5, ageSeconds: 1, food: 0, fitness: -2, sizeFactor: 0,
    heldTurn: 0, previousAppliedTurn: 0, headingDelta: 0, speedDelta: 0, boostCost: 1, boostHeadroom: 0,
    generationSeconds: 240, foodSaturation: 1, boostSpeed: 500,
    assessmentPoints: 0, lifePoints: null, generationBestPoints: 0, generationElapsedSeconds: null,
    nearestFoodDistance: null, nearestFoodBearing: null, nearestBodyClearance: null, nearestHeadClearance: null,
    foodStrength: Array(bins).fill(0), farFoodStrength: Array(bins).fill(0), bodyClearance: Array(bins).fill(0),
    rayHeadings: Array(bins).fill(0), wallClearance: Array(bins).fill(100), headClearance: Array(bins).fill(100),
    headBearingOffset: Array(bins).fill(0), headLengthRatio: Array(bins).fill(0), relativeHeading: Array(bins).fill(0),
    relativeForwardSpeed: Array(bins).fill(0), relativeSideSpeed: Array(bins).fill(0), urgency: Array(bins).fill(0),
    kills: 'ffffffffffffffff', actualBoost: false, heldBoostRequest: false, boostEligible: null,
    pelletSaturated: false, bodySaturated: true, bodyRays: Array.from({length: bins}, () => ({kind: 'incomplete'})),
    headCount: Array(bins).fill(0), headIds: Array(bins).fill(null), encounterSeconds: Array(bins).fill(null)};
}

/** Modify untrusted JSON without weakening the production metadata types. */
function changed(value: unknown, path: readonly string[], replacement: unknown): unknown {
  const result: unknown = structuredClone(value);
  let parent = result as Record<string, unknown>;
  for (const key of path.slice(0, -1)) parent = parent[key] as Record<string, unknown>;
  parent[path[path.length - 1]!] = replacement;
  return result;
}

describe('native selected inspector projection', () => {
  it('preserves full-width identities, raw graph outputs and equal-width profile identities', () => {
    for (const [profile, bins] of [['v3', 16], ['basic', 20], ['motion', 8], ['motion', 32]] as const) {
      const value = snapshot(profile, bins);
      expect(parseInspectorSnapshot(value)).toBe(value);
      expect(parseInspectorSnapshot(value).sample?.profile).toBe(profile);
    }
  });

  it('accepts a small explicit clear and exact accepted external ownership', () => {
    const original = snapshot();
    const value = {...original, sample: {...original.sample,
      source: {kind: 'external', leaseId: 'ffffffffffffffff', connectionId: 'ffffffffffffffff'}, neuralOutputs: null}};
    expect(parseInspectorSnapshot(value).sample?.source.kind).toBe('external');
    expect(parseInspectorSnapshot({...value, sample: null}).sample).toBeNull();
  });

  it('retains explicit unknown rays and rejects malformed raw data without inventing clear routes', () => {
    const original = snapshot('motion', 32);
    const value = {...original, sample: {...original.sample, trace: trace(32)}};
    expect(parseInspectorSnapshot(value).sample?.trace?.bodyRays[0]).toEqual({kind: 'incomplete'});
    const hit = changed(value, ['sample', 'trace', 'bodyRays', '0'], {kind: 'hit', distance: 12.5});
    expect(parseInspectorSnapshot(hit).sample?.trace?.bodyRays[0]).toEqual({kind: 'hit', distance: 12.5});
    const corruptions: readonly [readonly string[], unknown][] = [
      [['headIds', '0'], 9007199254740992],
      [['bodyRays', '0'], {kind: 'clear', distance: 1}],
      [['bodyRays', '0'], {kind: 'hit', distance: NaN}],
      [['urgency'], Array(31).fill(0)], [['fitness'], Infinity]
    ];
    for (const [path, replacement] of corruptions) {
      expect(() => parseInspectorSnapshot(changed(value, ['sample', 'trace', ...path], replacement))).toThrow();
    }
  });

  it('accepts actual v2 tactics at B32 and rejects a sector outside that sampler', () => {
    const original = snapshot('motion', 32);
    const value = {...original, sample: {...original.sample, source: {kind: 'baseline-v2'}, neuralOutputs: null,
      baselineAction: {turn: -0.5, boost: false, sector: 31, lifeStage: 'large', strategy: 'avoid',
        normalizedScore: -0.7, unsafeRoute: true, escapeOrPursuit: false, urgentSectors: 0xffffffff,
        committedHeading: null, commitmentRemainingSeconds: 0}}};
    expect(parseInspectorSnapshot(value).sample?.baselineAction?.urgentSectors).toBe(0xffffffff);
    expect(() => parseInspectorSnapshot(changed(value, ['sample', 'baselineAction', 'sector'], 32))).toThrow();
    expect(() => parseInspectorSnapshot(changed(value, ['sample', 'neuralOutputs'], [0, 0]))).toThrow();
  });

  it('rejects stale chronology, unbounded shapes and undeclared source/output combinations', () => {
    const corruptions: readonly [readonly string[], unknown][] = [
      [['schemaVersion'], 2], [['subscriptionSequence'], '0'.repeat(16)], [['worldEpoch'], '0'.repeat(16)],
      [['sample', 'ageSteps'], '0000000000000002'], [['sample', 'physicalId'], Number.MAX_SAFE_INTEGER],
      [['sample', 'bins'], 33], [['sample', 'normalizedInputs'], Array(84).fill(0)],
      [['sample', 'normalizedInputs', '0'], 1.1],
      [['sample', 'source'], {kind: 'external', leaseId: '1'.repeat(16)}],
      [['sample', 'source'], {kind: 'baseline-v2'}], [['sample', 'neuralOutputs'], null],
      [['sample', 'diagnostics', 'incompleteRays'], 17], [['unboundedPayload'], 'x'.repeat(65536)]
    ];
    for (const [path, replacement] of corruptions) {
      expect(() => parseInspectorSnapshot(changed(snapshot(), path, replacement))).toThrow();
    }
  });

  it('preserves lifecycle ledger, assessment and provisional native fitness while rejecting invalid details', () => {
    const value = inspectorFixture(); value.sample!.lifecycle = inspectorLifecycleFixture();
    expect(parseInspectorSnapshot(value).sample!.lifecycle).toMatchObject({provisionalFitness: -2, assessment: {points: 30},
      current: {peakPoints: 12}, bestPointLife: {peakPoints: 30}, ledger: {operations: 'ffffffffffffffff'}});
    const corruptions: readonly [readonly string[], unknown][] = [
      [['sample', 'lifecycle', 'lifeOrdinal'], 2], [['sample', 'lifecycle', 'fitnessComponents'], Array(8).fill(0)],
      [['sample', 'lifecycle', 'ledger', 'lifeLocalWallet'], Infinity], [['sample', 'lifecycle', 'ledger', 'operations'], 2],
      [['sample', 'lifecycle', 'current', 'ordinal'], '0'.repeat(16)], [['sample', 'lifecycle', 'assessment', 'kills'], 9007199254740992],
      [['sample', 'lifecycle', 'pointMode'], 'invented'], [['sample', 'raw', 'kind'], 'baseline']
    ];
    for (const [path, replacement] of corruptions) expect(() => parseInspectorSnapshot(changed(value, path, replacement))).toThrow();
  });

  it('validates actual v1 actions and exact shadow step, slot and activation identities', () => {
    const original = snapshot('v3', 8);
    const v1 = {...original, sample: {...original.sample, source: {kind: 'baseline-v1'}, neuralOutputs: null,
      baselineV1Action: {turn: -0.5, boost: true, baselineSlot: 1, strategy: 'roam', effectiveStrategy: 'avoid',
        lifeStage: 'small', sector: 7, targetPhysicalId: 'ffffffffffffffff', uniformDraws: 2,
        strategyTimerSeconds: 2.5, wanderAngle: -1, wanderRemainingSeconds: 1}}};
    expect(parseInspectorSnapshot(v1).sample!.baselineV1Action?.effectiveStrategy).toBe('avoid');
    expect(() => parseInspectorSnapshot(changed(v1, ['sample', 'baselineV1Action', 'sector'], 8))).toThrow();
    expect(() => parseInspectorSnapshot(changed(v1, ['sample', 'baselineV1Action', 'targetPhysicalId'], 42))).toThrow();
    expect(() => parseInspectorSnapshot(changed(v1, ['sample', 'source'], {kind: 'neural'}))).toThrow();
    const v2 = {...v1, sample: {...v1.sample, baselineV1Action: null, source: {kind: 'baseline-v2'}, profile: 'basic', normalizedInputs: Array(72).fill(0),
      baselineAction: {turn: 0, boost: false, sector: 0, lifeStage: 'small', strategy: 'roam', normalizedScore: 0,
        unsafeRoute: false, escapeOrPursuit: false, urgentSectors: 0, committedHeading: null, commitmentRemainingSeconds: 0},
      shadowOutputs: [{shadowSlot: 0, artifactSha256: 'b'.repeat(64), captureEpoch: 'c'.repeat(32),
        activationSequence: 'ffffffffffffffff', sampleStep: original.sample.sampleStep, rawOutputs: [2.5, -3]}]}};
    expect(parseInspectorSnapshot(v2).sample!.shadowOutputs?.[0]?.rawOutputs).toEqual([2.5, -3]);
    const corruptions: readonly [readonly string[], unknown][] = [
      [['sampleStep'], '0'.repeat(16)], [['captureEpoch'], '0'.repeat(32)], [['activationSequence'], '0'.repeat(16)],
      [['rawOutputs', '0'], NaN], [['shadowSlot'], 4], [['artifactSha256'], 'b'.repeat(65)]
    ];
    for (const [path, replacement] of corruptions) expect(() => parseInspectorSnapshot(changed(v2, ['sample', 'shadowOutputs', '0', ...path], replacement))).toThrow();
    expect(() => parseInspectorSnapshot(changed(v2, ['sample', 'shadowOutputs'], [v2.sample.shadowOutputs[0], v2.sample.shadowOutputs[0]]))).toThrow();
  });

});
