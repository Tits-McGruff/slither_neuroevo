import { SQLITE_WAL_ALLOWANCE_BYTES } from '../../server/rustEngine/diskAdmission.ts';

/** Scalar storage diagnostics retained in a loaded workload report. */
export interface StorageSample {
  /** Automatic files charged to the owner's pruning budget, encoded as decimal bytes. */
  automaticStoredBytes: string;
  /** Active automatic-file budget, encoded as decimal bytes. */
  automaticByteCap: string;
  /** Cached worker and filesystem diagnostics observed by the health request. */
  storage: {
    /** Metadata database and sidecar file lengths. */
    sqlite: { databaseBytes: string; walBytes: string; shmBytes: string };
    /** Temporary-file, filesystem-free-space and admission limits. */
    managed: { temporaryBytes: string; temporaryQuotaBytes: string; freeBytes: string; operatingReserveBytes: string };
  };
}

/** Parse exact canonical filesystem counters without rounding to JavaScript numbers. */
function bytes(value: string): bigint {
  if (typeof value !== 'string' || value.length > 20 || !/^(?:0|[1-9][0-9]*)$/u.test(value)) {
    throw new Error('invalid decimal storage counter');
  }
  const exact = BigInt(value);
  if (exact > 0xffff_ffff_ffff_ffffn) throw new Error('storage counter exceeds Uint64');
  return exact;
}

/** Check observed storage bounds; a separate final filesystem audit proves artifact cleanup. */
export function summarizeStorageSoak(samples: readonly StorageSample[], expectedFinalTemporaryBytes: string): {
  inspectedSamples: number; automaticByteCap: string; temporaryQuotaBytes: string;
  operatingReserveBytes: string; walAllowanceBytes: string;
  peakAutomaticStoredBytes: string; peakDatabaseBytes: string; peakWalBytes: string; peakShmBytes: string;
  peakTemporaryBytes: string; minimumFreeBytes: string; finalTemporaryBytes: string;
  withinAutomaticBudget: boolean; withinTemporaryQuota: boolean; withinWalAllowance: boolean;
  preservesOperatingReserve: boolean; temporaryBytesReturnedToExpectedSet: boolean; meetsObservedStorageGate: boolean;
} {
  if (samples.length < 2) throw new Error('storage soak requires initial and final observations');
  const initial = samples[0]!;
  const cap = bytes(initial.automaticByteCap);
  const quota = bytes(initial.storage.managed.temporaryQuotaBytes);
  const reserve = bytes(initial.storage.managed.operatingReserveBytes);
  const expected = bytes(expectedFinalTemporaryBytes);
  if (cap === 0n || quota === 0n || reserve === 0n || expected > quota) throw new Error('invalid storage limit');
  let peakAutomatic = 0n;
  let peakDatabase = 0n;
  let peakWal = 0n;
  let peakShm = 0n;
  let peakTemporary = 0n;
  let minimumFree = bytes(initial.storage.managed.freeBytes);
  let withinAutomaticBudget = true;
  let withinTemporaryQuota = true;
  let withinWalAllowance = true;
  let preservesOperatingReserve = true;
  for (const sample of samples) {
    const managed = sample.storage.managed;
    if (bytes(sample.automaticByteCap) !== cap || bytes(managed.temporaryQuotaBytes) !== quota ||
        bytes(managed.operatingReserveBytes) !== reserve) throw new Error('storage limit changed during the soak');
    const automatic = bytes(sample.automaticStoredBytes);
    const database = bytes(sample.storage.sqlite.databaseBytes);
    const wal = bytes(sample.storage.sqlite.walBytes);
    const shm = bytes(sample.storage.sqlite.shmBytes);
    const temporary = bytes(managed.temporaryBytes);
    const free = bytes(managed.freeBytes);
    if (automatic > peakAutomatic) peakAutomatic = automatic;
    if (database > peakDatabase) peakDatabase = database;
    if (wal > peakWal) peakWal = wal;
    if (shm > peakShm) peakShm = shm;
    if (temporary > peakTemporary) peakTemporary = temporary;
    if (free < minimumFree) minimumFree = free;
    withinAutomaticBudget &&= automatic <= cap;
    withinTemporaryQuota &&= temporary <= quota;
    withinWalAllowance &&= wal <= SQLITE_WAL_ALLOWANCE_BYTES;
    preservesOperatingReserve &&= free >= reserve;
  }
  const finalTemporary = bytes(samples.at(-1)!.storage.managed.temporaryBytes);
  const temporaryBytesReturnedToExpectedSet = finalTemporary === expected;
  return { inspectedSamples: samples.length, automaticByteCap: cap.toString(), temporaryQuotaBytes: quota.toString(),
    operatingReserveBytes: reserve.toString(), walAllowanceBytes: SQLITE_WAL_ALLOWANCE_BYTES.toString(),
    peakAutomaticStoredBytes: peakAutomatic.toString(), peakDatabaseBytes: peakDatabase.toString(),
    peakWalBytes: peakWal.toString(), peakShmBytes: peakShm.toString(), peakTemporaryBytes: peakTemporary.toString(),
    minimumFreeBytes: minimumFree.toString(), finalTemporaryBytes: finalTemporary.toString(),
    withinAutomaticBudget, withinTemporaryQuota, withinWalAllowance, preservesOperatingReserve,
    temporaryBytesReturnedToExpectedSet,
    meetsObservedStorageGate: withinAutomaticBudget && withinTemporaryQuota && withinWalAllowance &&
      preservesOperatingReserve && temporaryBytesReturnedToExpectedSet };
}
