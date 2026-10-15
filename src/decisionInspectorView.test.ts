import { expect, it } from 'vitest';
import { describeInspector } from './decisionInspectorView.ts';
import { inspectorFixture } from './test/inspectorFixture.ts';

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
