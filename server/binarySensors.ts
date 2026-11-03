import type { SensorsMsg } from './protocol.ts';

/** Four-byte discriminator for compact Protocol 2 bot observations. */
export const BINARY_SENSORS_MAGIC = 'SNS2';
/** Version of the compact sensor payload layout. */
export const BINARY_SENSORS_VERSION = 1;
/** Fixed bytes before packed float32 sensor values. */
export const BINARY_SENSORS_HEADER_BYTES = 44;

/** Encode one already-validated sensor message without decimal JSON conversion. */
export function encodeBinarySensors(message: SensorsMsg): Buffer {
  if (!Number.isSafeInteger(message.tick) || message.tick < 0) {
    throw new RangeError('binary sensor tick must be a non-negative safe integer');
  }
  if (!Number.isSafeInteger(message.snakeId) || message.snakeId <= 0 || message.snakeId > 0xffffffff) {
    throw new RangeError('binary sensor snakeId must fit uint32');
  }
  if (message.sensors.length > 0xffff || !message.sensors.every(Number.isFinite)) {
    throw new RangeError('binary sensor vector is invalid');
  }
  const packed = Float32Array.from(message.sensors);
  const output = Buffer.allocUnsafe(BINARY_SENSORS_HEADER_BYTES + packed.byteLength);
  output.write(BINARY_SENSORS_MAGIC, 0, 4, 'ascii');
  output.writeUInt8(BINARY_SENSORS_VERSION, 4);
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
  return output;
}
