import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { prepareInferenceBackend } from '../src/brains/nativeBridge.ts';
import { resetCFGToDefaults } from '../src/config.ts';
import { SimCore } from '../src/sim/SimCore.ts';
import { DEFAULT_CONFIG, normalizeConfig } from './config.ts';

/** Phase 3 backend-selection contract suite. */
const SUITE = 'recovery Phase 3 — native backend selection';

beforeEach(() => {
  resetCFGToDefaults();
});

afterEach(() => {
  resetCFGToDefaults();
});

describe(SUITE, () => {
  it('defaults normal server configuration to native and accepts explicit JS diagnostics', () => {
    const warnings: string[] = [];
    expect(DEFAULT_CONFIG.inferenceBackend).toBe('native');
    expect(normalizeConfig({ inferenceBackend: 'js' }).inferenceBackend).toBe('js');
    expect(normalizeConfig({ inferenceBackend: 'invalid' }, warning => warnings.push(warning)))
      .toMatchObject({ inferenceBackend: 'native' });
    expect(warnings).toEqual([
      'inferenceBackend "invalid" is invalid; using native.'
    ]);
  });

  it('requires native readiness before construction and binds it to every population brain', async () => {
    const options = {
      settings: {
        snakeCount: 2,
        hiddenLayers: 1,
        neurons1: 4,
        neurons2: 3,
        neurons3: 3,
        neurons4: 3,
        neurons5: 3
      },
      worldSeed: 123,
      runId: 'phase-3-native',
      inferenceBackend: 'native' as const
    };
    expect(() => new SimCore(options)).toThrow(/Native inference backend is not ready/u);

    await prepareInferenceBackend('native');
    const core = new SimCore(options);
    expect(core.inferenceBackend).toBe('native');
    expect(core.world.inferenceBackend).toBe('native');
    expect(core.world.snakes.slice(0, 2).map(snake => snake.brain.inferenceBackend))
      .toEqual(['native', 'native']);
  });
});
