import { describe, expect, it } from 'vitest';
import { SQLITE_WAL_ALLOWANCE_BYTES } from '../../server/rustEngine/diskAdmission.ts';
import { summarizeStorageSoak, type StorageSample } from './storage-soak-summary.ts';

/** Small health observation with explicit admission and pruning limits. */
function sample(): StorageSample {
  return { automaticStoredBytes: '40', automaticByteCap: '100', storage: {
    sqlite: { databaseBytes: '10', walBytes: '20', shmBytes: '5' },
    managed: { temporaryBytes: '0', temporaryQuotaBytes: '1000', freeBytes: '10000', operatingReserveBytes: '2000' }
  } };
}

describe('observed storage soak bounds', () => {
  it('retains a transient active temporary set and its return to the expected final set', () => {
    const active = sample(); active.storage.managed.temporaryBytes = '500';
    active.automaticStoredBytes = '100'; active.storage.managed.freeBytes = '9000';
    expect(summarizeStorageSoak([sample(), active, sample()], '0')).toMatchObject({
      meetsObservedStorageGate: true, inspectedSamples: 3, peakTemporaryBytes: '500',
      peakAutomaticStoredBytes: '100', minimumFreeBytes: '9000', finalTemporaryBytes: '0'
    });
  });
  it('keeps prior automatic/temp/WAL/reserve breaches failing after the final sample drains', () => {
    for (const breach of [
      (value: StorageSample) => { value.automaticStoredBytes = '101'; },
      (value: StorageSample) => { value.storage.managed.temporaryBytes = '1001'; },
      (value: StorageSample) => { value.storage.sqlite.walBytes = (SQLITE_WAL_ALLOWANCE_BYTES + 1n).toString(); },
      (value: StorageSample) => { value.storage.managed.freeBytes = '1999'; }
    ]) {
      const active = sample(); breach(active);
      expect(summarizeStorageSoak([sample(), active, sample()], '0').meetsObservedStorageGate).toBe(false);
    }
  });
  it('fails an unexpected final temporary file rather than accepting its quota alone', () => {
    const final = sample(); final.storage.managed.temporaryBytes = '1';
    expect(summarizeStorageSoak([sample(), final], '0')).toMatchObject({
      withinTemporaryQuota: true, temporaryBytesReturnedToExpectedSet: false, meetsObservedStorageGate: false
    });
  });
  it('preserves single-byte budget and reserve differences above Number precision', () => {
    const initial = sample(); initial.automaticByteCap = '9007199254740992';
    initial.automaticStoredBytes = '9007199254740992';
    initial.storage.managed.operatingReserveBytes = '9007199254740993';
    initial.storage.managed.freeBytes = '9007199254740993';
    const final = structuredClone(initial);
    final.automaticStoredBytes = '9007199254740993';
    final.storage.managed.freeBytes = '9007199254740992';
    expect(summarizeStorageSoak([initial, final], '0')).toMatchObject({
      withinAutomaticBudget: false, preservesOperatingReserve: false, peakAutomaticStoredBytes: '9007199254740993',
      minimumFreeBytes: '9007199254740992', meetsObservedStorageGate: false
    });
  });
  it('rejects changed/absent limits, malformed decimal counts and incomplete observations', () => {
    for (const value of ['', '-1', '1.5', '01', '0x10', '18446744073709551616']) {
      const final = sample(); final.storage.sqlite.walBytes = value;
      expect(() => summarizeStorageSoak([sample(), final], '0')).toThrow(/storage|Uint64/);
    }
    for (const change of [
      (value: StorageSample) => { value.automaticByteCap = '101'; },
      (value: StorageSample) => { value.storage.managed.temporaryQuotaBytes = '1001'; },
      (value: StorageSample) => { value.storage.managed.operatingReserveBytes = '2001'; }
    ]) {
      const final = sample(); change(final);
      expect(() => summarizeStorageSoak([sample(), final], '0')).toThrow(/limit changed/);
    }
    const unbounded = sample(); unbounded.automaticByteCap = '0';
    expect(() => summarizeStorageSoak([unbounded, unbounded], '0')).toThrow(/limit/);
    expect(() => summarizeStorageSoak([sample()], '0')).toThrow(/initial and final/);
    expect(() => summarizeStorageSoak([sample(), sample()], '1001')).toThrow(/limit/);
  });
});
