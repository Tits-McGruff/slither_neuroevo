import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import type { ExternalRewardEnvelopeV1 } from '../contracts/types.ts';
import { EXTERNAL_REWARD_MAX_ENVELOPE_BYTES } from '../contracts/types.ts';
import { BINARY_SENSORS_HEADER_BYTES, encodeBinarySensors } from './binarySensors.ts';

describe('binary sensor codec', () => {
  const reward = (JSON.parse(readFileSync(new URL(
    '../contracts/slither_contracts/external-reward-v1-fixture.json', import.meta.url
  ), 'utf8')) as ExternalRewardEnvelopeV1[])[0]!;
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

  it('preserves the packed body and appends a bounded reward only for explicit version 2', () => {
    const message = { type: 'sensors' as const, tick: 1, snakeId: 7, sensors: [1.25, -2.5], reward };
    expect(() => encodeBinarySensors(message)).toThrow(/negotiated/u);
    const encoded = encodeBinarySensors(message, 2);
    expect(encoded.readUInt8(4)).toBe(2);
    expect(encoded.readFloatLE(BINARY_SENSORS_HEADER_BYTES)).toBe(1.25);
    const start = BINARY_SENSORS_HEADER_BYTES + message.sensors.length * 4;
    const length = encoded.readUInt32LE(start);
    expect(length).toBeLessThanOrEqual(EXTERNAL_REWARD_MAX_ENVELOPE_BYTES);
    expect(encoded.length).toBe(start + 4 + length);
    expect(JSON.parse(encoded.subarray(start + 4).toString('utf8'))).toEqual(reward);
  });

  it('rejects missing reward, mismatched observation identities and oversized envelopes', () => {
    const message = { type: 'sensors' as const, tick: 1, snakeId: 7, sensors: [0] };
    expect(() => encodeBinarySensors(message, 2)).toThrow(/missing/u);
    expect(() => encodeBinarySensors({ ...message, reward: { ...reward, snakeId: 8 } }, 2)).toThrow(/mismatched/u);
    expect(() => encodeBinarySensors({ ...message, tick: 2, reward }, 2)).toThrow(/mismatched/u);
    expect(() => encodeBinarySensors({ ...message, reward: { ...reward,
      runId: 'x'.repeat(EXTERNAL_REWARD_MAX_ENVELOPE_BYTES) } }, 2)).toThrow(/bounded/u);
    expect(() => encodeBinarySensors({ ...message, sensors: [1e100] })).toThrow(/Float32/u);
  });
});
