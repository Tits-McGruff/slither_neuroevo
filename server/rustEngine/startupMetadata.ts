import { SENSOR_HASH_DOMAIN } from "../../contracts/types.ts";
import type { RustLegacyConversionNotice, RustStartupMetadata } from '../../src/protocol/rustBackground.ts';
import { createHash } from 'node:crypto';
import { sensorLayoutFromContract, type SensorContractEnvelope } from '../../src/protocol/sensors.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import { validateGraph } from '../../src/brains/graph/validate.ts';

/** Maximum encoded response admitted by the matching native metadata method. */
const MAX_METADATA_BYTES = 1024 * 1024;

/** Validate bounded native scalar metadata before using it in a browser welcome. */
export function parseRustStartupMetadata(encoded: unknown): RustStartupMetadata {
  if (typeof encoded !== 'string' || encoded.length > MAX_METADATA_BYTES || Buffer.byteLength(encoded) > MAX_METADATA_BYTES) {
    throw new TypeError('invalid bounded Rust startup metadata');
  }
  const value: unknown = JSON.parse(encoded);
  if (value === null || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid Rust startup metadata');
  const record = value as Record<string, unknown>;
  const text = (key: string, maximum: number): string => {
    const item = record[key];
    if (typeof item !== 'string' || !item || Buffer.byteLength(item) > maximum || item.includes('\0')) {
      throw new TypeError(`invalid Rust startup ${key}`);
    }
    return item;
  };
  const integer = (key: string, minimum: number, maximum = Number.MAX_SAFE_INTEGER): number => {
    const item = record[key];
    if (typeof item !== 'number' || !Number.isSafeInteger(item) || item < minimum || item > maximum) {
      throw new TypeError(`invalid Rust startup ${key}`);
    }
    return item;
  };
  const configRevision = text('configRevision', 16);
  if (!/^[0-9a-f]{16}$/u.test(configRevision)) throw new TypeError('invalid Rust startup revision');
  const fixedStepSeconds = record['fixedStepSeconds'];
  if (typeof fixedStepSeconds !== 'number' || !Number.isFinite(fixedStepSeconds) || fixedStepSeconds <= 0 || fixedStepSeconds > 1) {
    throw new TypeError('invalid Rust startup fixed step');
  }
  const rawSettings = record['settings'];
  if (!Array.isArray(rawSettings) || !rawSettings.length || rawSettings.length > 1024) throw new TypeError('invalid Rust startup settings');
  const paths = new Set<string>();
  const settings = rawSettings.map((raw: unknown) => {
    if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) throw new TypeError('invalid Rust startup setting');
    const entry = raw as Record<string, unknown>;
    const path = entry['path'];
    const setting = entry['value'];
    if (typeof path !== 'string' || !path || Buffer.byteLength(path) > 256 || path.includes('\0') || paths.has(path) ||
      !(typeof setting === 'boolean' || (typeof setting === 'number' && Number.isFinite(setting)) ||
        (typeof setting === 'string' && Buffer.byteLength(setting) <= 65_536 && !setting.includes('\0')))) {
      throw new TypeError('invalid Rust startup setting');
    }
    paths.add(path);
    return { path, value: setting };
  });
  const rawGraph = record['graphSpec'];
  if (rawGraph === null || typeof rawGraph !== 'object' || Array.isArray(rawGraph)) {
    throw new TypeError('invalid Rust startup graphSpec');
  }
  const graphSpec = rawGraph as GraphSpec;
  if (validateGraph(graphSpec).ok !== true) throw new TypeError('invalid Rust startup graphSpec');
  const sensorVersion = integer('sensorVersion',3,4);
  const rawContract = record['sensorContract'];
  let sensorContract: SensorContractEnvelope | undefined;
  if (rawContract !== undefined) {
    if (!rawContract || typeof rawContract !== 'object' || Array.isArray(rawContract)) throw new TypeError('invalid native sensor contract');
    const envelope = rawContract as Record<string,unknown>;
    if (Object.keys(envelope).some(key=>key !== 'canonicalJson' && key !== 'sha256') ||
        typeof envelope['canonicalJson'] !== 'string' || Buffer.byteLength(envelope['canonicalJson']) > 65536 ||
        typeof envelope['sha256'] !== 'string') throw new TypeError('invalid bounded native sensor contract');
    sensorContract = {canonicalJson:envelope['canonicalJson'],sha256:envelope['sha256']};
    const digest = createHash('sha256').update(SENSOR_HASH_DOMAIN).update(sensorContract.canonicalJson).digest('hex');
    if (digest !== sensorContract.sha256) throw new TypeError('native sensor contract digest mismatch');
    const layout = sensorLayoutFromContract(sensorContract);
    const setting = (path:string): unknown => settings.find(entry=>entry.path===path)?.value;
    if (layout.layoutVersion !== 'v'+sensorVersion || setting('sense.bubbleBins') !== layout.bins ||
        setting('brain.sensorVersion') !== sensorVersion || setting('sense.layoutVersion') !== sensorVersion ||
        graphSpec.nodes.find(node=>node.type==='Input')?.outputSize !== layout.inputSize) {
      throw new TypeError('native sensor contract does not match settings or graph');
    }
    if (sensorVersion === 4 && (setting('sense.contractVersion') !== 1 ||
        setting('sense.profile') !== (layout.profile === 'basic' ? 0 : 1) ||
        setting('sense.contractSha256') !== digest)) throw new TypeError('native recorded v4 identity mismatch');
  } else if (sensorVersion === 4) throw new TypeError('v4 requires native sensor contract metadata');
  return {
    ...(sensorContract ? {sensorContract} : {}),
    ...(record['legacyConversion'] == null ? {} : { legacyConversion: parseRustLegacyOrigin(record['legacyConversion']) }),
    runId: text('runId', 256), seed: integer('seed', 0, 0xffff_ffff), configRevision,
    configHash: text('configHash', 256), fixedStepSeconds,
    maximumFrameBytes: integer('maximumFrameBytes', 1), graphKey: text('graphKey', 256 * 1024), graphSpec,
    parameterCount: integer('parameterCount', 1), mathBackend: text('mathBackend', 128),
    serializerVersion: integer('serializerVersion', 1, 1), sensorVersion, settings
  };
}

/** Validate only bounded immutable origin metadata; this never decodes a population. */
export function parseRustLegacyOrigin(value: unknown): RustLegacyConversionNotice {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid Rust legacy origin');
  const record = value as Record<string, unknown>;
  const keys = ['version', 'sourceFormat', 'sourceSnapshotId', 'sourceRunId', 'sourceGeneration', 'sourceSeed',
    'sourceSha256', 'completeness', 'exactContinuation'];
  if (Object.keys(record).some(key => !keys.includes(key)) || record['version'] !== 1 ||
      record['completeness'] !== 'population-only' || record['exactContinuation'] !== false ||
      !['typescript-v2', 'legacy-gzip', 'legacy-json', 'browser-json'].includes(String(record['sourceFormat']))) {
    throw new TypeError('invalid Rust legacy origin classification');
  }
  const browser = record['sourceFormat'] === 'browser-json';
  const snapshot = record['sourceSnapshotId'];
  if (browser ? snapshot !== undefined : typeof snapshot !== 'number' || !Number.isSafeInteger(snapshot) || snapshot <= 0) {
    throw new TypeError('invalid Rust legacy origin snapshot');
  }
  const run = record['sourceRunId'];
  if (run !== undefined && (typeof run !== 'string' || !run || Buffer.byteLength(run) > 256 || run.includes('\0'))) {
    throw new TypeError('invalid Rust legacy origin lineage');
  }
  const generation = record['sourceGeneration'];
  if (generation !== undefined && (typeof generation !== 'string' || !/^[0-9a-f]{16}$/u.test(generation) || BigInt(`0x${generation}`) === 0n)) {
    throw new TypeError('invalid Rust legacy origin generation');
  }
  const seed = record['sourceSeed'];
  if (seed !== undefined && (typeof seed !== 'number' || !Number.isInteger(seed) || seed < 0 || seed > 0xffff_ffff)) {
    throw new TypeError('invalid Rust legacy origin seed');
  }
  const digest = record['sourceSha256'];
  if (digest !== undefined && (typeof digest !== 'string' || !/^[0-9a-f]{64}$/u.test(digest))) {
    throw new TypeError('invalid Rust legacy origin digest');
  }
  return { ...record } as unknown as RustLegacyConversionNotice;
}
