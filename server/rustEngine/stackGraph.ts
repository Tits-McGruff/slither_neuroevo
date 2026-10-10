import { buildStackGraphSpec, defaultStackBrain } from '../../src/brains/stackBuilder.ts';
import { graphKey } from '../../src/brains/graph/compiler.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import { DEFAULT_CORE_SETTINGS, type CoreSettings, type SettingsUpdate } from '../../src/protocol/settings.ts';
import { getSensorLayout } from '../../src/protocol/sensors.ts';
import { SETTING_DEFINITION_BY_PATH, normalizeSettingValue } from '../../src/protocol/settingDefinitions.ts';
import type { RustStartupMetadata } from '../../src/protocol/rustBackground.ts';
import type { ResetMsg } from '../protocol.ts';

/** Brain dimensions used only to construct or describe a stack graph. */
type StackBrain = Parameters<typeof buildStackGraphSpec>[1]['brain'];

/** Core slider fields in canonical hidden-layer order. */
const NEURON_FIELDS = ['neurons1', 'neurons2', 'neurons3', 'neurons4', 'neurons5'] as const;

/** Graph-derived controls; these are not additional Rust simulation settings. */
export const STACK_SETTING_PATHS = new Set([
  'brain.useMlp', 'brain.stack.gru', 'brain.stack.lstm', 'brain.stack.rru',
  'brain.gruHidden', 'brain.lstmHidden', 'brain.rruHidden'
]);

/** Exact slider representation of a graph expressible by the stack builder. */
export interface StackPresentation {
  /** Active core dimensions, with defaults only for unused layer controls. */
  core: CoreSettings;
  /** Actual enabled modules and recurrent dimensions. */
  brain: StackBrain;
  /** Numeric wire values used to restore browser CFG controls. */
  updates: SettingsUpdate[];
}

/** Default stack configuration sized for the authoritative sensor layout. */
function defaultBrain(inputSize: number): StackBrain {
  return defaultStackBrain(inputSize);
}

/** Recover slider values only when rebuilding them reproduces the complete graph key. */
export function deriveStackPresentation(spec: GraphSpec, core: CoreSettings, inputSize: number): StackPresentation | undefined {
  const brain = defaultBrain(inputSize);
  const mlp = spec.nodes.find(node => node.id === 'mlp' && node.type === 'MLP');
  brain.useMlp = !!mlp;
  const updates: SettingsUpdate[] = [{ path: 'brain.useMlp', value: Number(brain.useMlp) }];
  let recurrent = false;
  for (const kind of ['gru', 'lstm', 'rru'] as const) {
    const node = spec.nodes.find(node => node.id === kind && node.type === kind.toUpperCase());
    const enabled = node && 'hiddenSize' in node;
    const path = `brain.${kind}Hidden` as const;
    if (enabled) {
      const definition = SETTING_DEFINITION_BY_PATH.get(path)!;
      if (normalizeSettingValue(definition, node.hiddenSize) !== node.hiddenSize) return undefined;
      brain[`${kind}Hidden`] = node.hiddenSize;
      recurrent = true;
    }
    brain.stack![kind] = Number(!!enabled);
    updates.push({ path: `brain.stack.${kind}`, value: Number(!!enabled) },
      { path, value: brain[`${kind}Hidden`]! });
  }
  const dimensions = mlp?.type === 'MLP' ? [...(mlp.hiddenSizes ?? []), ...(recurrent ? [mlp.outputSize] : [])] : [];
  if (mlp && (dimensions.length < 1 || dimensions.length > 5 || dimensions.some(size => !Number.isInteger(size) || size < 1 || size > 256))) {
    return undefined;
  }
  const actual = { ...core };
  if (mlp) {
    actual.hiddenLayers = dimensions.length;
    dimensions.forEach((size, index) => { actual[NEURON_FIELDS[index]!] = size; });
  }
  if (graphKey(spec) !== graphKey(buildStackGraphSpec(actual, { brain }))) return undefined;
  return { core: actual, brain, updates };
}

/** Validate graph-only reset hints using the shared slider bounds or exact boolean encoding. */
export function normalizeStackSetting(path: string, value: number): number {
  const definition = SETTING_DEFINITION_BY_PATH.get(path);
  if (definition) return normalizeSettingValue(definition, value);
  if (!Number.isFinite(value) || (value !== 0 && value !== 1)) throw new TypeError(`invalid stack setting ${path}`);
  return value;
}

/** Build the default stack for an explicit null override using the requested core/CFG controls. */
export function clearedStackGraph(metadata: RustStartupMetadata, message: ResetMsg,
  settings: readonly { path: string; value: number }[]): GraphSpec {
  const values = new Map(settings.map(setting => [setting.path, setting.value]));
  const inputSize = getSensorLayout(values.get('sense.bubbleBins')!).inputSize;
  const currentCore = { ...DEFAULT_CORE_SETTINGS,
    snakeCount: values.get('snakeCount')!, simSpeed: values.get('simSpeed')! };
  const currentInput = getSensorLayout(Number(metadata.settings.find(setting => setting.path === 'sense.bubbleBins')!.value)).inputSize;
  const current = deriveStackPresentation(metadata.graphSpec, currentCore, currentInput);
  const core = { ...(current?.core ?? currentCore) };
  for (const [key, value] of Object.entries(message.settings ?? {})) {
    core[key as keyof CoreSettings] = normalizeSettingValue(SETTING_DEFINITION_BY_PATH.get(key)!, value);
  }
  const brain = current?.brain ?? defaultBrain(inputSize);
  brain.inSize = inputSize;
  for (const update of message.updates ?? []) {
    if (!STACK_SETTING_PATHS.has(update.path)) continue;
    const value = normalizeStackSetting(update.path, update.value);
    if (update.path === 'brain.useMlp') brain.useMlp = value !== 0;
    else if (update.path.startsWith('brain.stack.')) brain.stack![update.path.slice(12) as 'gru' | 'lstm' | 'rru'] = value;
    else brain[update.path.slice(6) as 'gruHidden' | 'lstmHidden' | 'rruHidden'] = value;
  }
  return buildStackGraphSpec(core, { brain });
}
