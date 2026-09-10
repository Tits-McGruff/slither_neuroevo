import { describe, expect, it } from 'vitest';
import {
  OWNER_CHECKPOINT_RETENTION_DEFAULTS,
  selectManagedCheckpointRetention,
  type CheckpointRetentionCandidate
} from './checkpointRetention.ts';

/** Build one exact production-shaped retention candidate. */
function candidate(
  ordinal: bigint,
  generation = ordinal,
  storedBytes = 100n,
  overrides: Partial<CheckpointRetentionCandidate> = {}
): CheckpointRetentionCandidate {
  return {
    checkpointId: ordinal.toString(16).padStart(64, '0'),
    runId: 'current',
    generation,
    storedBytes,
    decodedBytes: storedBytes * 2n,
    createdOrdinal: ordinal,
    pinned: false,
    priorRunAnchor: false,
    weightsEncoding: 'f32le-shuffle4-zstd-v1',
    recurrentStateEncoding: 'raw-f32le-v1',
    ...overrides
  };
}

/** Build an overnight-equivalent current lineage and two prior anchors. */
function overnight(storedBytes: bigint): CheckpointRetentionCandidate[] {
  return [
    candidate(10_001n, 81n, storedBytes, { runId: 'prior-a', priorRunAnchor: true }),
    candidate(10_002n, 44n, storedBytes, { runId: 'prior-b', priorRunAnchor: true }),
    ...Array.from({ length: 480 }, (_unused, index) => candidate(BigInt(index + 1), BigInt(index + 1), storedBytes))
  ];
}

describe('production managed checkpoint retention selection', () => {
  it('keeps eight recent boundaries, twelve milestones, and two prior-run anchors', () => {
    const result = selectManagedCheckpointRetention(overnight(100n), 'current');
    expect(result.kept.filter(item => item.retentionClass === 'latest' || item.retentionClass === 'recent')
      .map(item => item.generation)).toEqual([473n, 474n, 475n, 476n, 477n, 478n, 479n, 480n]);
    expect(result.kept.filter(item => item.retentionClass === 'milestone').map(item => item.generation))
      .toEqual([175n, 200n, 225n, 250n, 275n, 300n, 325n, 350n, 375n, 400n, 425n, 450n]);
    expect(result.kept.filter(item => item.retentionClass === 'prior-anchor')).toHaveLength(2);
    expect(result.pruned).toHaveLength(460);
  });

  it('removes old milestones and then optional recents before protected boundaries', () => {
    const result = selectManagedCheckpointRetention(overnight(400n), 'current', {
      ...OWNER_CHECKPOINT_RETENTION_DEFAULTS,
      automaticByteCap: 1_600n
    });
    expect(result.kept.filter(item => item.runId === 'current').map(item => item.generation)).toEqual([479n, 480n]);
    expect(result.kept.filter(item => item.retentionClass === 'prior-anchor')).toHaveLength(2);
    expect(result.automaticBytes).toBe(1_600n);
    expect(result.protectedAutomaticBytes).toBe(1_600n);
  });

  it('rejects a cap smaller than the latest, predecessor, and selected anchors', () => {
    expect(() => selectManagedCheckpointRetention(overnight(400n), 'current', {
      ...OWNER_CHECKPOINT_RETENTION_DEFAULTS,
      automaticByteCap: 1_599n
    })).toThrow(/protected automatic checkpoints require 1600 bytes/);
  });

  it('keeps pins outside the automatic cap and never plans them for deletion', () => {
    const pin = candidate(20_000n, 3n, 9_000n, { pinned: true });
    const result = selectManagedCheckpointRetention([...overnight(100n), pin], 'current', {
      ...OWNER_CHECKPOINT_RETENTION_DEFAULTS,
      automaticByteCap: 2_200n
    });
    expect(result.kept.find(item => item.checkpointId === pin.checkpointId)?.retentionClass).toBe('pinned');
    expect(result.automaticBytes).toBe(2_200n);
    expect(result.pinnedBytes).toBe(9_000n);
  });

  it('uses exact bigint generations instead of narrowing the persistence wire value', () => {
    const high = 9_007_199_254_740_993n;
    const result = selectManagedCheckpointRetention([
      candidate(1n, high - 1n),
      candidate(2n, high)
    ], 'current');
    expect(result.kept.map(item => item.generation)).toEqual([high - 1n, high]);
  });
});
