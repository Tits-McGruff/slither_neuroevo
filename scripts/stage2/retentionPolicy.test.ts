/** Contract tests for the owner-selected checkpoint retention model. */

import { describe, expect, it } from 'vitest';
import {
  OWNER_RETENTION_DEFAULTS,
  selectRetainedCheckpoints,
  type RetentionCandidate
} from './retentionPolicy.ts';

/**
 * Construct one deterministic checkpoint candidate.
 * @param key - Unique key.
 * @param generation - Generation boundary.
 * @param bytes - Stored bytes.
 * @param extras - Optional role overrides.
 * @returns Retention candidate.
 */
function candidate(
  key: string,
  generation: number,
  bytes = 100,
  extras: Partial<RetentionCandidate> = {}
): RetentionCandidate {
  return {
    key,
    runId: 'current',
    generation,
    bytes,
    createdOrdinal: generation,
    pinned: false,
    priorRunAnchor: false,
    ...extras
  };
}

/**
 * Construct 480 current generations plus two prior anchors.
 * @param bytes - Bytes per checkpoint.
 * @returns Complete overnight-equivalent candidate list.
 */
function overnightCandidates(bytes: number): RetentionCandidate[] {
  const current = Array.from(
    { length: 480 },
    (_unused, index) => candidate(`g${index + 1}`, index + 1, bytes)
  );
  return [
    candidate('prior-a', 81, bytes, {
      runId: 'prior-a',
      createdOrdinal: 10_001,
      priorRunAnchor: true
    }),
    candidate('prior-b', 44, bytes, {
      runId: 'prior-b',
      createdOrdinal: 10_002,
      priorRunAnchor: true
    }),
    ...current
  ];
}

describe('Stage 2 checkpoint retention policy', () => {
  it('keeps eight recent, twelve non-overlapping milestones and two prior anchors', () => {
    const result = selectRetainedCheckpoints(overnightCandidates(100), 'current');
    const recent = result.kept
      .filter(item => item.retentionClass === 'latest' || item.retentionClass === 'recent')
      .map(item => item.generation);
    const milestones = result.kept
      .filter(item => item.retentionClass === 'milestone')
      .map(item => item.generation);

    expect(recent).toEqual([473, 474, 475, 476, 477, 478, 479, 480]);
    expect(milestones).toEqual([
      175, 200, 225, 250, 275, 300, 325, 350, 375, 400, 425, 450
    ]);
    expect(result.kept.filter(item => item.retentionClass === 'prior-anchor')).toHaveLength(2);
    expect(result.automaticBytes).toBe(2_200);
    expect(result.pruned).toHaveLength(460);
  });

  it('removes oldest milestones before reducing the recent window', () => {
    const result = selectRetainedCheckpoints(overnightCandidates(400), 'current', {
      ...OWNER_RETENTION_DEFAULTS,
      automaticByteCap: 4_000
    });

    expect(result.kept.filter(item => item.retentionClass === 'milestone')).toHaveLength(0);
    expect(result.kept.filter(item => item.retentionClass === 'recent')).toHaveLength(7);
    expect(result.kept.filter(item => item.retentionClass === 'latest')).toHaveLength(1);
    expect(result.kept.filter(item => item.retentionClass === 'prior-anchor')).toHaveLength(2);
    expect(result.automaticBytes).toBe(4_000);
  });

  it('then removes oldest optional recents while preserving latest, predecessor and anchors', () => {
    const result = selectRetainedCheckpoints(overnightCandidates(400), 'current', {
      ...OWNER_RETENTION_DEFAULTS,
      automaticByteCap: 1_600
    });
    const current = result.kept
      .filter(item => item.runId === 'current')
      .map(item => item.generation);

    expect(current).toEqual([479, 480]);
    expect(result.kept.filter(item => item.retentionClass === 'prior-anchor')).toHaveLength(2);
    expect(result.automaticBytes).toBe(1_600);
    expect(result.protectedAutomaticBytes).toBe(1_600);
  });

  it('rejects a cap that cannot hold the protected automatic minimum', () => {
    expect(() => selectRetainedCheckpoints(overnightCandidates(400), 'current', {
      ...OWNER_RETENTION_DEFAULTS,
      automaticByteCap: 1_500
    })).toThrow(/protected automatic checkpoints require 1600 bytes/);
  });

  it('never counts or selects pinned checkpoints for automatic pruning', () => {
    const pinned = candidate('pin', 3, 9_000, { pinned: true, createdOrdinal: 20_000 });
    const result = selectRetainedCheckpoints(
      [...overnightCandidates(100), pinned],
      'current',
      { ...OWNER_RETENTION_DEFAULTS, automaticByteCap: 2_200 }
    );

    expect(result.kept.find(item => item.key === 'pin')?.retentionClass).toBe('pinned');
    expect(result.automaticBytes).toBe(2_200);
    expect(result.pinnedBytes).toBe(9_000);
  });
});
