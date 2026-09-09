import type { RustStartupMetadata } from '../../src/protocol/rustBackground.ts';

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
  return {
    runId: text('runId', 256), seed: integer('seed', 0, 0xffff_ffff), configRevision,
    configHash: text('configHash', 256), fixedStepSeconds,
    maximumFrameBytes: integer('maximumFrameBytes', 1), graphKey: text('graphKey', 256 * 1024),
    parameterCount: integer('parameterCount', 1), mathBackend: text('mathBackend', 128),
    serializerVersion: integer('serializerVersion', 1, 1), sensorVersion: integer('sensorVersion', 3, 3), settings
  };
}
