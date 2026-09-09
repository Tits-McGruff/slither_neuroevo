import { describe, expect, it } from 'vitest';
import { parseRustStartupMetadata } from './startupMetadata.ts';

/** Small native-shaped welcome metadata without gameplay arrays. */
const METADATA = {
  runId: 'test-lineage', seed: 42, configRevision: '0000000000000000', configHash: 'sha256:config',
  fixedStepSeconds: 1 / 120, maximumFrameBytes: 1024, graphKey: 'native-graph', parameterCount: 20,
  mathBackend: 'scalar', serializerVersion: 1, sensorVersion: 3,
  settings: [{ path: 'snakeCount', value: 64 }, { path: 'sense.debug', value: false }]
};

describe('Rust startup metadata', () => {
  it('retains exact scalar settings and native identities without deriving a world', () => {
    expect(parseRustStartupMetadata(JSON.stringify(METADATA))).toEqual(METADATA);
  });

  it('rejects oversized, ambiguous, and unsupported metadata before use', () => {
    expect(() => parseRustStartupMetadata(' '.repeat(1024 * 1024 + 1))).toThrow('bounded');
    for (const override of [
      { seed: -1 }, { configRevision: '1' }, { serializerVersion: 2 }, { fixedStepSeconds: 0 },
      { maximumFrameBytes: 0 }, { settings: [...METADATA.settings, METADATA.settings[0]] },
      { settings: [{ path: 'snakeCount', value: null }] }
    ]) {
      expect(() => parseRustStartupMetadata(JSON.stringify({ ...METADATA, ...override }))).toThrow();
    }
  });
});
