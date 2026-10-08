import { describe, expect, it } from 'vitest';
import { CFG_DEFAULT } from '../../src/config.ts';
import { buildStackGraphSpec } from '../../src/brains/stackBuilder.ts';
import { graphKey } from '../../src/brains/graph/compiler.ts';
import { DEFAULT_CORE_SETTINGS } from '../../src/protocol/settings.ts';
import type { RustStartupMetadata } from '../../src/protocol/rustBackground.ts';
import { createRustWelcome } from './browserMetadata.ts';
import { clearedStackGraph, deriveStackPresentation } from './stackGraph.ts';

/** Non-default widths exercise recovery of every active MLP layer. */
const CORE = { ...DEFAULT_CORE_SETTINGS, hiddenLayers: 3, neurons1: 23, neurons2: 17, neurons3: 11 };

/** Minimal native metadata; architecture controls are encoded only in its graph. */
function metadata(graphSpec: RustStartupMetadata['graphSpec']): RustStartupMetadata {
  return { runId: 'stack-test', seed: 42, configRevision: '0000000000000000', configHash: 'test',
    fixedStepSeconds: 1 / 60, maximumFrameBytes: 1024, graphKey: graphKey(graphSpec), graphSpec,
    parameterCount: 1, mathBackend: 'scalar', serializerVersion: 1, sensorVersion: 3,
    settings: [{ path: 'snakeCount', value: 12 }, { path: 'simSpeed', value: 1 }, { path: 'sense.bubbleBins', value: 16 }] };
}

describe('Rust stack graph presentation', () => {
  it.each([
    { useMlp: true, stack: { gru: 0, lstm: 0, rru: 0 } },
    { useMlp: true, stack: { gru: 1, lstm: 1, rru: 1 } },
    { useMlp: false, stack: { gru: 0, lstm: 1, rru: 0 } },
    { useMlp: false, stack: { gru: 0, lstm: 0, rru: 0 } }
  ])('welcomes reconstruct the complete canonical graph: %j', flags => {
    const brain = { ...CFG_DEFAULT.brain, ...flags, gruHidden: 12, lstmHidden: 20, rruHidden: 24 };
    const spec = buildStackGraphSpec(CORE, { brain });
    const welcome = createRustWelcome(metadata(spec));
    const restored = structuredClone(CFG_DEFAULT.brain);
    for (const update of welcome.settings.updates) {
      if (update.path === 'brain.useMlp') restored.useMlp = update.value !== 0;
      else if (update.path.startsWith('brain.stack.')) restored.stack[update.path.slice(12) as 'gru' | 'lstm' | 'rru'] = update.value;
      else if (update.path.endsWith('Hidden')) restored[update.path.slice(6) as 'gruHidden' | 'lstmHidden' | 'rruHidden'] = update.value;
    }
    expect(graphKey(buildStackGraphSpec(welcome.settings.core, { brain: restored }))).toBe(graphKey(spec));
    if (flags.useMlp) expect(welcome.settings.core).toMatchObject({ hiddenLayers: 3, neurons1: 23, neurons2: 17, neurons3: 11 });
  });

  it('keeps a custom graph custom even when its dimensions match an ordinary stack', () => {
    const spec = buildStackGraphSpec(CORE, CFG_DEFAULT);
    for (const node of spec.nodes) if (node.id === 'mlp') node.id = 'features';
    for (const edge of spec.edges) {
      if (edge.from === 'mlp') edge.from = 'features';
      if (edge.to === 'mlp') edge.to = 'features';
    }
    expect(deriveStackPresentation(spec, DEFAULT_CORE_SETTINGS, 83)).toBeUndefined();
    expect(graphKey(createRustWelcome(metadata(spec)).graphSpec!)).toBe(graphKey(spec));
    const cleared = clearedStackGraph(metadata(spec), { type: 'reset', graphSpec: null }, metadata(spec).settings as Array<{ path: string; value: number }>);
    expect(graphKey(cleared)).toBe(graphKey(buildStackGraphSpec(DEFAULT_CORE_SETTINGS, CFG_DEFAULT)));
  });

  it('clears to requested stack controls and the replacement sensor layout', () => {
    const spec = buildStackGraphSpec(CORE, CFG_DEFAULT);
    const current = metadata(spec);
    const cleared = clearedStackGraph(current, { type: 'reset', graphSpec: null,
      settings: { hiddenLayers: 1, neurons1: 9 }, updates: [
        { path: 'brain.stack.gru', value: 0 }, { path: 'brain.stack.lstm', value: 1 },
        { path: 'brain.lstmHidden', value: 28 }
      ] }, [{ path: 'snakeCount', value: 12 }, { path: 'simSpeed', value: 1 }, { path: 'sense.bubbleBins', value: 8 }]);
    expect(graphKey(cleared)).toBe(graphKey(buildStackGraphSpec({ ...CORE, hiddenLayers: 1, neurons1: 9 },
      { brain: { ...CFG_DEFAULT.brain, inSize: 51, lstmHidden: 28, stack: { gru: 0, lstm: 1, rru: 0 } } })));
  });
});
