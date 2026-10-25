import { describe, expect, it } from 'vitest';
import type { RustBackgroundDisplay, RustBackgroundFrameCopy } from '../../src/protocol/rustBackground.ts';
import { BackgroundFramePool } from './backgroundFrames.ts';

/** Compact synthetic native metadata; byte ownership is the contract under test. */
function display(sequence: number): RustBackgroundDisplay {
  return {
    sequence: sequence.toString(16).padStart(16, '0'), worldEpoch: '0000000000000001',
    completedStep: sequence.toString(16).padStart(16, '0'), generation: '0000000000000001',
    generationTime: sequence / 60, alivePopulation: 1, baselineBotsAlive: 0,
    baselineBotsTotal: 0, totalSnakes: 1, aliveSnakes: 1, pellets: 0, frameByteLength: 16
  };
}

describe('background frame send ownership', () => {
  it('keeps both in-flight sends immutable, skips backlog, and reuses only released storage', () => {
    let sequence = 1;
    let copies = 0;
    const pool = new BackgroundFramePool({
      copyLatestFrame(destination): RustBackgroundFrameCopy {
        copies += 1;
        destination.fill(sequence, 0, 16);
        return { status: 'copied', display: display(sequence) };
      }
    }, 32);
    const first = pool.tryAcquireLatest('0000000000000000')!;
    sequence = 2;
    const second = pool.tryAcquireLatest(first.display.sequence)!;
    sequence = 8;
    expect(pool.tryAcquireLatest(second.display.sequence)).toBeNull();
    expect(copies).toBe(2);
    expect([...first.bytes]).toEqual(Array(16).fill(1));
    expect([...second.bytes]).toEqual(Array(16).fill(2));
    first.release();
    const latest = pool.tryAcquireLatest(second.display.sequence)!;
    expect(latest.display.sequence).toBe('0000000000000008');
    expect(latest.bytes.buffer).toBe(first.bytes.buffer);
    expect(latest.bytes.byteOffset).toBe(first.bytes.byteOffset);
    expect([...latest.bytes]).toEqual(Array(16).fill(8));
    first.release(); // A delayed duplicate callback cannot free a newer lease.
    expect(pool.tryAcquireLatest(latest.display.sequence)).toBeNull();
    expect([...second.bytes]).toEqual(Array(16).fill(2));
    second.release();
    latest.release();
  });

  it('honors a larger committed frame bound without changing either in-flight send', () => {
    let sequence = 1; let required = 16;
    const pool = new BackgroundFramePool({
      copyLatestFrame(destination): RustBackgroundFrameCopy {
        const current = {...display(sequence), frameByteLength: required};
        if (destination.byteLength < required) return {status: 'tooSmall', display: current};
        destination.fill(sequence, 0, required);
        return {status: 'copied', display: current};
      }
    }, 32);
    const first = pool.tryAcquireLatest('0000000000000000')!;
    sequence = 2; required = 64;
    expect(pool.tryAcquireLatest(first.display.sequence)).toBeNull();
    const second = pool.tryAcquireLatest(first.display.sequence, 64);
    expect(second).not.toBeNull();
    expect([...first.bytes]).toEqual(Array(16).fill(1));
    expect([...second!.bytes]).toEqual(Array(64).fill(2));
    expect(pool.tryAcquireLatest(second!.display.sequence, 128)).toBeNull();
    first.release(); sequence = 3;
    const third = pool.tryAcquireLatest(second!.display.sequence, 64)!;
    expect([...third.bytes]).toEqual(Array(64).fill(3));
    first.release();
    expect(pool.tryAcquireLatest(third.display.sequence, 64)).toBeNull();
    expect([...second!.bytes]).toEqual(Array(64).fill(2));
    const retained = third.bytes;
    third.release(); sequence = 4;
    const reused = pool.tryAcquireLatest(third.display.sequence, 64)!;
    expect(reused.bytes.buffer).toBe(retained.buffer);
    expect(reused.bytes.byteOffset).toBe(retained.byteOffset);
    second!.release(); reused.release();
  });

  it('does not consume a slot on native backpressure or a too-small admitted buffer', () => {
    let result: RustBackgroundFrameCopy = { status: 'busy' };
    const pool = new BackgroundFramePool({ copyLatestFrame: () => result }, 32);
    expect(pool.tryAcquireLatest('0000000000000000')).toBeNull();
    result = { status: 'tooSmall', display: { ...display(1), frameByteLength: 64 } };
    expect(pool.tryAcquireLatest('0000000000000000')).toBeNull();
    result = { status: 'unchanged' };
    expect(pool.tryAcquireLatest('0000000000000001')).toBeNull();
    result = { status: 'copied', display: display(2) };
    const first = pool.tryAcquireLatest('0000000000000001');
    const second = pool.tryAcquireLatest('0000000000000001');
    expect(first).not.toBeNull();
    expect(second).not.toBeNull();
    first?.release();
    second?.release();
  });
});
