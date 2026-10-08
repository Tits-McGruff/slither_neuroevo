import { readFileSync, existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  automaticCapWithPhysicalReserve,
  buildCheckpointRetentionInventory,
  OWNER_CHECKPOINT_RETENTION_DEFAULTS,
  selectManagedCheckpointRetention,
  type CheckpointRetentionCandidate
} from './checkpointRetention.ts';
import { CHECKPOINT_DISK_ADMISSION_REQUEST, SQLITE_WAL_ALLOWANCE_BYTES } from './diskAdmission.ts';
import {
  admitPendingRunStartCheckpoint,
  assertReplacementCheckpointBudget,
  assertStartupCheckpointBudget
} from './experimentalStartup.ts';

/** Encode a bounded test byte count in the worker's unsigned wire form. */
function u64(value: bigint): string {
  return value.toString(16).padStart(16, '0');
}

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
  it('rejects a startup budget that cannot fit protected files and one publication', () => {
    const reserve = CHECKPOINT_DISK_ADMISSION_REQUEST.candidateSpoolBytes +
      CHECKPOINT_DISK_ADMISSION_REQUEST.finalManagedBytes + SQLITE_WAL_ALLOWANCE_BYTES;
    const protectedBytes = 800n * 1024n * 1024n;
    const databaseBytes = 2n * 1024n * 1024n;
    const storage = { databaseByteCount: u64(databaseBytes), walByteCount: u64(0n),
      shmByteCount: u64(0n) };
    const required = protectedBytes + databaseBytes + reserve;
    expect(() => assertStartupCheckpointBudget({ protectedAutomaticStoredByteCount: u64(protectedBytes),
      automaticByteCap: u64(required) }, storage)).not.toThrow();
    expect(() => assertStartupCheckpointBudget({ protectedAutomaticStoredByteCount: u64(protectedBytes),
      automaticByteCap: u64(required - 1n) }, storage)).toThrow(/cannot preserve the protected checkpoints/u);
  });

  it('removes an unpublished fresh checkpoint when budget admission rejects it', async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-startup-budget-'));
    const name = `${'a'.repeat(64)}.checkpoint-v3`;
    const path = join(root, name);
    const descriptor = { relativeFilename: name, storedByteCount: u64(800n * 1024n * 1024n) };
    const inspectStorage = async () => ({ schemaVersion: 1 as const,
      databaseByteCount: u64(2n * 1024n * 1024n), walByteCount: u64(0n), shmByteCount: u64(0n),
      pageSizeByteCount: u64(4096n), pageCount: u64(512n), freelistPageCount: u64(0n),
      usedPageByteCount: u64(2n * 1024n * 1024n) });
    try {
      writeFileSync(path, 'uncommitted');
      await expect(admitPendingRunStartCheckpoint(descriptor, inspectStorage, root,
        1280n * 1024n * 1024n)).rejects.toThrow(/cannot preserve the protected checkpoints/u);
      expect(existsSync(path)).toBe(false);
      writeFileSync(path, 'admitted');
      await expect(admitPendingRunStartCheckpoint(descriptor, inspectStorage, root,
        3072n * 1024n * 1024n)).resolves.toBeUndefined();
      expect(existsSync(path)).toBe(true);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });

  it('counts prior protected anchors before admitting a fresh append and removes only its candidate on rejection', async () => {
    const root = mkdtempSync(join(tmpdir(), 'slither-fresh-append-budget-'));
    const name = `${'a'.repeat(64)}.checkpoint-v3`;
    const prior = join(root, `${'b'.repeat(64)}.checkpoint-v3`);
    const path = join(root, name);
    const descriptor = { relativeFilename: name, storedByteCount: u64(10n * 1024n * 1024n) };
    const inspectStorage = async () => ({ schemaVersion: 1 as const,
      databaseByteCount: u64(2n * 1024n * 1024n), walByteCount: u64(0n), shmByteCount: u64(0n),
      pageSizeByteCount: u64(4096n), pageCount: u64(512n), freelistPageCount: u64(0n),
      usedPageByteCount: u64(2n * 1024n * 1024n) });
    try {
      writeFileSync(path, 'candidate');
      writeFileSync(prior, 'retained anchor');
      await expect(admitPendingRunStartCheckpoint(descriptor, inspectStorage, root,
        3072n * 1024n * 1024n, 2048n * 1024n * 1024n)).rejects.toThrow(/cannot preserve the protected checkpoints/u);
      expect(existsSync(path)).toBe(false);
      expect(readFileSync(prior, 'utf8')).toBe('retained anchor');
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
  it('counts the proposed reset boundary before replacing the current run', () => {
    const storage = { databaseByteCount: u64(2n * 1024n * 1024n), walByteCount: u64(0n),
      shmByteCount: u64(0n) };
    const previousProtected = 10n * 1024n * 1024n;
    const newCheckpoint = 400n * 1024n * 1024n;
    const retention = { protectedAutomaticStoredByteCount: u64(previousProtected),
      automaticByteCap: u64(1280n * 1024n * 1024n) };
    expect(() => assertReplacementCheckpointBudget({ storedByteCount: u64(newCheckpoint) },
      retention, storage)).toThrow(/cannot preserve the protected checkpoints/u);
    expect(() => assertReplacementCheckpointBudget({ storedByteCount: u64(newCheckpoint) },
      { ...retention, automaticByteCap: u64(2048n * 1024n * 1024n) }, storage)).not.toThrow();
  });

  it('prunes before publication against physical bytes while preserving protected anchors', () => {
    const effectiveCap = automaticCapWithPhysicalReserve(800n, 1_200n, 1_000n, 100n, 300n);
    expect(effectiveCap).toBe(400n);
    const pin = candidate(11n, 1n, 100n, { pinned: true });
    const decision = selectManagedCheckpointRetention([
      ...Array.from({ length: 10 }, (_unused, index) => candidate(BigInt(index + 1))), pin
    ], 'current', { ...OWNER_CHECKPOINT_RETENTION_DEFAULTS, automaticByteCap: effectiveCap });
    expect(decision.automaticBytes).toBe(400n);
    expect(decision.kept.filter(item => !item.pinned).map(item => item.generation))
      .toEqual([7n, 8n, 9n, 10n]);
    expect(decision.kept).toContainEqual(expect.objectContaining({ checkpointId: pin.checkpointId }));
    expect(() => automaticCapWithPhysicalReserve(800n, 1_200n, 1_000n, 100n, 800n))
      .toThrow(/no publication headroom/);
  });

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

  it('reports a pinned current checkpoint as both current and pinned', () => {
    const current = candidate(3n, 3n, 100n, { pinned: true });
    const decision = selectManagedCheckpointRetention([candidate(1n), candidate(2n), current], 'current');
    const inventory = buildCheckpointRetentionInventory(decision, 'current');
    expect(decision.kept.find(item => item.checkpointId === current.checkpointId)?.retentionClass).toBe('latest');
    expect(inventory.retained.latest.checkpointCount).toBe(1);
    expect(inventory.retained.pinned.checkpointCount).toBe(1);
    expect(inventory.pinnedStoredByteCount).toBe('0000000000000064');
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
