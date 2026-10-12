import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { sensorLayoutFromContract, type SensorContractEnvelope, type SensorProfile } from '../../src/protocol/sensors.ts';
import type { RustStartupMetadata } from '../../src/protocol/rustBackground.ts';
import { createRustWelcome } from './browserMetadata.ts';

/** Rust verifies these exact canonical byte/digest pairs on every native test run. */
const CONTRACTS = JSON.parse(readFileSync(new URL('../../native/fixtures/sensor-contract-shapes.json',import.meta.url),'utf8')) as
  Array<SensorContractEnvelope & { profile: SensorProfile; bins: number }>;

/** Build bounded native-shaped metadata around one genuine Rust manifest. */
function contractMetadata(contract: (typeof CONTRACTS)[number]): RustStartupMetadata & {sensorContract:SensorContractEnvelope} {
  const layout = sensorLayoutFromContract(contract);
  const version = layout.layoutVersion === 'v3' ? 3 : 4;
  return {...METADATA,sensorVersion:version,sensorContract:{canonicalJson:contract.canonicalJson,sha256:contract.sha256},
    graphSpec:{...METADATA.graphSpec,nodes:[
      {id:'input',type:'Input',outputSize:layout.inputSize},
      {id:'head',type:'Dense',inputSize:layout.inputSize,outputSize:2}
    ]},settings:[{path:'snakeCount',value:12},{path:'simSpeed',value:1},
      {path:'brain.sensorVersion',value:version},{path:'sense.layoutVersion',value:version},{path:'sense.bubbleBins',value:layout.bins},
      ...(version===4 ? [{path:'sense.contractVersion',value:1},{path:'sense.profile',value:layout.profile==='basic' ? 0 : 1},
        {path:'sense.contractSha256',value:contract.sha256}] : [])] };
}
import { parseRustStartupMetadata } from './startupMetadata.ts';

/** Small native-shaped welcome metadata without gameplay arrays. */
const METADATA: RustStartupMetadata = {
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
  it.each(CONTRACTS)('preserves native $profile/B$bins structural bytes through startup and welcome', contract=>{
    const encoded = contractMetadata(contract);
    const metadata = parseRustStartupMetadata(JSON.stringify(encoded));
    expect(metadata.sensorContract).toEqual(encoded.sensorContract);
    const welcome = createRustWelcome(metadata,'contract-session');
    const layout = sensorLayoutFromContract(contract);
    expect(welcome.sensorSpec).toMatchObject({sensorCount:layout.inputSize,layoutVersion:layout.layoutVersion,order:layout.order,
      contract:encoded.sensorContract});
    if (layout.layoutVersion==='v4') expect(welcome.sensorSpec.profile).toBe(layout.profile);
    expect(welcome.settings.updates).toContainEqual({path:'sense.layoutVersion',value:metadata.sensorVersion});
  });

  it('rejects changed byte digests, profiles, revisions, order, recorded settings and graph width',()=>{
    const contract=CONTRACTS.find(value=>value.profile==='motion' && value.bins===32)!;
    const original=contractMetadata(contract);
    for (const override of [
      {sensorContract:undefined},{sensorVersion:3},
      {sensorContract:{...original.sensorContract,sha256:'0'.repeat(64)}},
      {sensorContract:{...original.sensorContract,canonicalJson:original.sensorContract.canonicalJson+' '}},
      {graphSpec:METADATA.graphSpec},
      {settings:original.settings.filter(setting=>setting.path!=='sense.contractSha256')},
      {settings:original.settings.map(setting=>setting.path==='sense.profile'?{...setting,value:0}:setting)}
    ]) expect(()=>parseRustStartupMetadata(JSON.stringify({...original,...override}))).toThrow();
    for (const field of ['normalizationRevision','geometryRevision','profile','order','channels']) {
      const manifest=JSON.parse(contract.canonicalJson) as Record<string,unknown>;
      if (field==='profile') manifest[field]='basic';
      else if (field==='order') (manifest[field] as string[]).reverse();
      else if (field==='channels') (manifest[field] as unknown[]).pop();
      else manifest[field]=1;
      const canonicalJson=JSON.stringify(manifest);
      const sha256=createHash('sha256').update('slither.sensor-contract.v1\0').update(canonicalJson).digest('hex');
      expect(()=>parseRustStartupMetadata(JSON.stringify({...original,sensorContract:{canonicalJson,sha256}}))).toThrow();
    }
  });
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
