import { describe, expect, it } from 'vitest';
import { parseRustStartupMetadata } from './startupMetadata.ts';

/** Small native-shaped welcome metadata without gameplay arrays. */
const METADATA = {
  runId: 'test-lineage', seed: 42, configRevision: '0000000000000000', configHash: 'sha256:config',
  fixedStepSeconds: 1 / 120, maximumFrameBytes: 1024, graphKey: 'native-graph', graphSpec: {
    type: 'graph', nodes: [
      { id: 'input', type: 'Input', outputSize: 2 },
      { id: 'head', type: 'Dense', inputSize: 2, outputSize: 2 }
    ], edges: [{ from: 'input', to: 'head' }], outputs: [{ nodeId: 'head' }], outputSize: 2
  }, parameterCount: 20,
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
      { settings: [{ path: 'snakeCount', value: null }] }, { graphSpec: { type: 'graph' } }
    ]) {
      expect(() => parseRustStartupMetadata(JSON.stringify({ ...METADATA, ...override }))).toThrow();
    }
  });

  it('preserves bounded legacy facts and rejects contradictory source claims', () => {
    const origin = { version: 1, sourceFormat: 'browser-json', sourceRunId: 'old-run',
      sourceGeneration: 'ffffffffffffffff', sourceSeed: 0, sourceSha256: 'a'.repeat(64),
      completeness: 'population-only', exactContinuation: false };
    expect(parseRustStartupMetadata(JSON.stringify({ ...METADATA, legacyConversion: origin })).legacyConversion).toEqual(origin);
    expect(parseRustStartupMetadata(JSON.stringify({ ...METADATA, legacyConversion: null }))).toEqual(METADATA);
    for (const override of [
      { version: 2 }, { exactContinuation: true }, { completeness: 'exact' }, { invented: true },
      { sourceSnapshotId: 1 }, { sourceFormat: 'typescript-v2' }, { sourceSeed: 0x1_0000_0000 },
      { sourceRunId: 'bad\0lineage' }, { sourceRunId: 'Ω'.repeat(129) },
      { sourceGeneration: '0000000000000000' }, { sourceGeneration: 'FFFFFFFFFFFFFFFF' },
      { sourceSha256: 'A'.repeat(64) }
    ]) {
      expect(() => parseRustStartupMetadata(JSON.stringify({ ...METADATA, legacyConversion: { ...origin, ...override } }))).toThrow();
    }
    expect(parseRustStartupMetadata(JSON.stringify({ ...METADATA, legacyConversion: {
      ...origin, sourceFormat: 'typescript-v2', sourceSnapshotId: 17
    } })).legacyConversion).toMatchObject({ sourceSnapshotId: 17 });
  });
});
