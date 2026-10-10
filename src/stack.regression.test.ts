import { afterEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from './config.ts';
import { buildStackGraphSpec, defaultStackBrain } from './brains/stackBuilder.ts';
import { compileGraph, graphKey } from './brains/graph/compiler.ts';
import { DEFAULT_CORE_SETTINGS } from './protocol/settings.ts';

afterEach(resetCFGToDefaults);

describe('regression: stacked graph controls', () => {
  it('keeps stack dimensions and parameter layout stable across browser reset', () => {
    const original = buildStackGraphSpec(DEFAULT_CORE_SETTINGS, { brain: defaultStackBrain(83) });
    CFG.brain.stack.gru = 0;
    CFG.brain.stack.lstm = 1;
    resetCFGToDefaults();
    const reset = buildStackGraphSpec(DEFAULT_CORE_SETTINGS, CFG);
    expect(graphKey(reset)).toBe(graphKey(original));
    expect(compileGraph(reset).totalParams).toBe(13_458);
    expect(defaultStackBrain(83).stack).toEqual({ gru: 1, lstm: 0, rru: 0 });
  });

  it('keeps differently sized recurrent modules in deterministic stack order', () => {
    const brain = defaultStackBrain(83);
    brain.stack = { gru: 1, lstm: 1, rru: 1 };
    brain.gruHidden = 8; brain.lstmHidden = 12; brain.rruHidden = 20;
    const compiled = compileGraph(buildStackGraphSpec(DEFAULT_CORE_SETTINGS, { brain }));
    expect(compiled.order.filter(id => ['gru', 'lstm', 'rru'].includes(id))).toEqual(['gru', 'lstm', 'rru']);
    expect(compiled.totalStateSize).toBe(8 + 24 + 20);
    expect(compiled.totalParams).toBeGreaterThan(0);
  });
});
