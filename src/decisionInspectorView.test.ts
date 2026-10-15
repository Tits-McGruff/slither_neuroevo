import { expect, it } from 'vitest';
import { describeInspector } from './decisionInspectorView.ts';
import { inspectorFixture, inspectorLifecycleFixture } from './test/inspectorFixture.ts';

it('labels exact native sample age, unclipped outputs and full-width counters', () => {
  const value = describeInspector(inspectorFixture());
  expect(value.summary).toContain('Sample step 1 · age 1 completed steps');
  expect(value.summary).toContain('ffffffffffffffff');
  expect(value.scalars).toContain('Kills: 18446744073709551615');
  expect(value.policy).toContain('Graph turn output: 2');
  expect(value.inputs).toContain('heading_sin: 0');
  expect(value.sectors).toContain('not-sampled');
});

it('shows unknown Motion rays and keeps absent Basic motion measurements labelled', () => {
  const motion = describeInspector(inspectorFixture('motion', 32));
  expect(motion.sectors).toContain('incomplete'); expect(motion.inputs.split('\n')).toHaveLength(504);
  const basic = describeInspector(inspectorFixture('basic', 8)); expect(basic.sectors).toContain('not-sampled');
  const clear = inspectorFixture(); clear.sample = null;
  expect(describeInspector(clear)).toMatchObject({scalars: '', sectors: '', inputs: '', policy: ''});
});


it('labels exact life accounting and negative provisional fitness without recomputing it', () => {
  const snapshot = inspectorFixture(); snapshot.sample!.lifecycle = inspectorLifecycleFixture();
  const view = describeInspector(snapshot);
  expect(view.lifecycle).toContain('At delivered sample boundary');
  expect(view.lifecycle).toContain('Provisional fitness: -2');
  expect(view.lifecycle).toContain('Selected points: 30');
  expect(view.lifecycle).toContain('Peak points: 12');
  expect(view.lifecycle).toContain('Peak points: 30');
  expect(view.lifecycle).toContain('Operations: 18446744073709551615');
  expect(view.lifecycle).toContain('Life penalty (subtracted): 17');
  expect(view.shadows).toBe('');
});

it('keeps v1 effective actions and raw shadow outputs separate from teacher policy', () => {
  const snapshot = inspectorFixture('v3', 8); const sample = snapshot.sample!;
  sample.source = {kind: 'baseline-v1'}; sample.neuralOutputs = null;
  sample.baselineV1Action = {turn: -0.5, boost: true, baselineSlot: 1, strategy: 'roam', effectiveStrategy: 'avoid',
    lifeStage: 'small', sector: 7, targetPhysicalId: 'ffffffffffffffff', uniformDraws: 2,
    strategyTimerSeconds: 2.5, wanderAngle: -1, wanderRemainingSeconds: 1};
  expect(describeInspector(snapshot).policy).toContain('Effective strategy: avoid');
  sample.source = {kind: 'baseline-v2'}; sample.baselineV1Action = null;
  sample.shadowOutputs = [{shadowSlot: 2, artifactSha256: 'b'.repeat(64), captureEpoch: 'c'.repeat(32),
    activationSequence: 'ffffffffffffffff', sampleStep: sample.sampleStep, rawOutputs: [2.5, -3]}];
  const view = describeInspector(snapshot);
  expect(view.shadows).toContain('at delivered sample step 1');
  expect(view.shadows).toContain('activation 18446744073709551615');
  expect(view.shadows).toContain('Graph turn output: 2.5000');
  expect(view.shadows).toContain('Graph boost output: -3');
});
