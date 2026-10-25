import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { sensorLayoutFromContract,type SensorContractEnvelope } from '../../src/protocol/sensors.ts';

/** Cross-language fixture byte pairs are verified by native sensor-contract tests. */
const CONTRACTS = JSON.parse(readFileSync(new URL('../../native/fixtures/sensor-contract-shapes.json',import.meta.url),'utf8')) as SensorContractEnvelope[];
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
  it('uses the replacement profile when switching from v3 or between v4 profiles', () => {
    const spec = buildStackGraphSpec(CORE, CFG_DEFAULT);
    for (const [profile, width] of [[0, 72], [1, 144]] as const) {
      const cleared = clearedStackGraph(metadata(spec), {type: 'reset', graphSpec: null}, [
        {path: 'snakeCount', value: 12}, {path: 'simSpeed', value: 1},
        {path: 'sense.layoutVersion', value: 4}, {path: 'sense.profile', value: profile}, {path: 'sense.bubbleBins', value: 8}
      ]);
      expect(cleared.nodes.find(node => node.type === 'Input')?.outputSize).toBe(width);
    }
    const motion = CONTRACTS.find(contract => sensorLayoutFromContract(contract).profile === 'motion')!;
    const current = {...metadata(spec), sensorVersion: 4, sensorContract: motion};
    const cleared = clearedStackGraph(current, {type: 'reset', graphSpec: null}, [
      {path: 'snakeCount', value: 12}, {path: 'simSpeed', value: 1},
      {path: 'sense.layoutVersion', value: 4}, {path: 'sense.profile', value: 0}, {path: 'sense.bubbleBins', value: 16}
    ]);
    expect(cleared.nodes.find(node => node.type === 'Input')?.outputSize).toBe(120);
  });
  it('recovers v4 stack controls and clears graphs at the exact requested profile width',()=>{
    for (const contract of CONTRACTS) {
      const layout=sensorLayoutFromContract(contract);
      if (layout.layoutVersion==='v3') continue;
      const brain={...CFG_DEFAULT.brain,inSize:layout.inputSize};
      const spec=buildStackGraphSpec(CORE,{brain});
      const current={...metadata(spec),sensorVersion:4,sensorContract:contract};
      current.settings.push({path:'sense.layoutVersion',value:4});
      current.settings.find(setting=>setting.path==='sense.bubbleBins')!.value=layout.bins;
      expect(createRustWelcome(current,'v4-stack-session').settings.core).toMatchObject({hiddenLayers:3,neurons1:23});
      const cleared=clearedStackGraph(current,{type:'reset',graphSpec:null},[
        {path:'snakeCount',value:12},{path:'simSpeed',value:1},
        {path:'sense.layoutVersion',value:4},{path:'sense.bubbleBins',value:16}
      ]);
      expect(cleared.nodes.find(node=>node.type==='Input')?.outputSize).toBe(layout.profile==='basic'?120:264);
    }
  });
  it.each([
    { useMlp: true, stack: { gru: 0, lstm: 0, rru: 0 } },
    { useMlp: true, stack: { gru: 1, lstm: 1, rru: 1 } },
    { useMlp: false, stack: { gru: 0, lstm: 1, rru: 0 } },
    { useMlp: false, stack: { gru: 0, lstm: 0, rru: 0 } }
  ])('welcomes reconstruct the complete canonical graph: %j', flags => {
    const brain = { ...CFG_DEFAULT.brain, ...flags, gruHidden: 12, lstmHidden: 20, rruHidden: 24 };
    const spec = buildStackGraphSpec(CORE, { brain });
    const welcome = createRustWelcome(metadata(spec), 'test-session');
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
    expect(graphKey(createRustWelcome(metadata(spec), 'test-session').graphSpec!)).toBe(graphKey(spec));
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
