import type { SensorsMsg } from './protocol.ts';
import {
  EXTERNAL_REWARD_BINARY_VERSION,
  EXTERNAL_REWARD_MAX_ENVELOPE_BYTES
} from '../contracts/types.ts';

/** Four-byte discriminator for compact Protocol 2 bot observations. */
export const BINARY_SENSORS_MAGIC = 'SNS2';
/** Version of the compact sensor payload layout. */
export const BINARY_SENSORS_VERSION = 1;
/** Fixed bytes before packed float32 sensor values. */
export const BINARY_SENSORS_HEADER_BYTES = 44;

/**
 * Encode packed Float32 sensors. Revision 2 appends a Uint32 byte count and
 * bounded UTF-8 JSON reward envelope; clients must negotiate it explicitly.
 */
export function encodeBinarySensors(
  message: SensorsMsg,
  version: 1 | typeof EXTERNAL_REWARD_BINARY_VERSION = BINARY_SENSORS_VERSION
): Buffer {
  if (version !== BINARY_SENSORS_VERSION && version !== EXTERNAL_REWARD_BINARY_VERSION) {
    throw new RangeError('unsupported binary sensor version');
  }
  if (!Number.isSafeInteger(message.tick) || message.tick < 0) {
    throw new RangeError('binary sensor tick must be a non-negative safe integer');
  }
  if (!Number.isSafeInteger(message.snakeId) || message.snakeId <= 0 || message.snakeId > 0xffffffff) {
    throw new RangeError('binary sensor snakeId must fit uint32');
  }
  if (message.sensors.length > 0xffff || !message.sensors.every(Number.isFinite)) {
    throw new RangeError('binary sensor vector is invalid');
  }
  let rewardBytes: Buffer | undefined;
  if (version === EXTERNAL_REWARD_BINARY_VERSION) {
    const reward = message.reward;
    if (!reward || reward.rewardVersion !== 1 || reward.snakeId !== message.snakeId ||
        reward.toStep !== message.tick.toString(16).padStart(16, '0') ||
        !Object.values(reward.deltas).every(value => typeof value === 'number' && Number.isFinite(value))) {
      throw new TypeError('binary reward is missing or mismatched with its observation');
    }
    rewardBytes = Buffer.from(JSON.stringify(reward), 'utf8');
    if (rewardBytes.length > EXTERNAL_REWARD_MAX_ENVELOPE_BYTES) {
      throw new RangeError('binary reward exceeds the bounded envelope');
    }
  } else if (message.reward !== undefined) {
    throw new TypeError('reward-bearing observation requires negotiated binary sensor version 2');
  }
  const packed = Float32Array.from(message.sensors);
  if (!packed.every(Number.isFinite)) {
    throw new RangeError('binary sensor vector exceeds Float32 range');
  }
  const rewardStart = BINARY_SENSORS_HEADER_BYTES + packed.byteLength;
  const output = Buffer.allocUnsafe(rewardStart + (rewardBytes ? 4 + rewardBytes.length : 0));
  output.write(BINARY_SENSORS_MAGIC, 0, 4, 'ascii');
  output.writeUInt8(version, 4);
  output.writeUInt8(message.meta === undefined ? 0 : 1, 5);
  output.writeUInt16LE(message.sensors.length, 6);
  output.writeBigUInt64LE(BigInt(message.tick), 8);
  output.writeUInt32LE(message.snakeId, 16);
  output.writeDoubleLE(message.meta?.x ?? 0, 20);
  output.writeDoubleLE(message.meta?.y ?? 0, 28);
  output.writeDoubleLE(message.meta?.dir ?? 0, 36);
  Buffer.from(packed.buffer, packed.byteOffset, packed.byteLength).copy(
    output, BINARY_SENSORS_HEADER_BYTES
  );
  if (rewardBytes) {
    output.writeUInt32LE(rewardBytes.length, rewardStart);
    rewardBytes.copy(output, rewardStart + 4);
  }
  return output;
}
