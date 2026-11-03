import { describe, expect, it } from 'vitest';
import { BINARY_SENSORS_HEADER_BYTES, encodeBinarySensors } from './binarySensors.ts';

describe('binary sensor codec', () => {
  it('packs the fixed header and float32 body in little-endian order', () => {
    const encoded = encodeBinarySensors({
      type: 'sensors', tick: 0x12345678, snakeId: 9, sensors: [1.25, -2.5, 0.125],
      meta: { x: 3.5, y: -4.25, dir: 0.75 }
    });
    expect(encoded.subarray(0, 4).toString('ascii')).toBe('SNS2');
    expect(encoded.readUInt8(4)).toBe(1);
    expect(encoded.readUInt8(5)).toBe(1);
    expect(encoded.readUInt16LE(6)).toBe(3);
    expect(encoded.readBigUInt64LE(8)).toBe(0x12345678n);
    expect(encoded.readUInt32LE(16)).toBe(9);
    expect(encoded.readDoubleLE(20)).toBe(3.5);
    expect(encoded.readDoubleLE(28)).toBe(-4.25);
    expect(encoded.readDoubleLE(36)).toBe(0.75);
    expect(encoded.readFloatLE(BINARY_SENSORS_HEADER_BYTES)).toBeCloseTo(1.25);
    expect(encoded.readFloatLE(BINARY_SENSORS_HEADER_BYTES + 4)).toBeCloseTo(-2.5);
    expect(encoded.readFloatLE(BINARY_SENSORS_HEADER_BYTES + 8)).toBeCloseTo(0.125);
  });
});
