import { describe, expect, it } from 'vitest';
import type { RustGenerationAssignmentReceipt } from '../../src/protocol/rustBackground.ts';
import { BackgroundCommandAdmission } from './commandAdmission.ts';

/** Complete Rust correlation fields for one locally accepted observation. */
const RECEIPT: RustGenerationAssignmentReceipt = {
  operationEpoch: '0000000000000002', eventSequence: '0000000000000003',
  connectionId: '0000000000000004', leaseId: '0000000000000005', accepted: true
};

describe('shared background command admission', () => {
  it('pins reclaim receipts separately from ordinary step receipts', () => {
    let available = false;
    const admission = new BackgroundCommandAdmission({
      submitControllerDeliveryReceipt() { throw new Error('must not send another phase'); },
      submitControllerReclaimReceipt() { if (!available) throw new Error('QueueCountLimit: full'); }
    });
    const receipt = { requestSequence: RECEIPT.eventSequence, connectionId: RECEIPT.connectionId,
      leaseId: RECEIPT.leaseId, accepted: false };
    const sequence = admission.nextSequence();
    expect(admission.trySubmitReclaimReceipt(sequence, receipt)).toBe(false);
    expect(() => admission.trySubmitReceipt(sequence, RECEIPT)).toThrow('another receipt');
    expect(admission.trySubmitControl(() => {})).toBe(false);
    available = true;
    expect(admission.trySubmitReclaimReceipt(sequence, receipt)).toBe(true);
    expect(admission.blocked).toBe(false);
  });
  it('lets a receipt bypass an unadmitted action without regressing command order', () => {
    const sequences: string[] = [];
    const admission = new BackgroundCommandAdmission({
      submitControllerReclaimReceipt() {},
      submitControllerDeliveryReceipt(sequence) { sequences.push(sequence); }
    });
    expect(admission.trySubmitControl(() => { throw new Error('QueueCountLimit: reserved delivery slot'); })).toBe(false);
    expect(admission.trySubmitReceipt(admission.nextSequence(), RECEIPT)).toBe(true);
    expect(admission.trySubmitControl(sequence => { sequences.push(sequence); })).toBe(true);
    expect(sequences).toEqual(['0000000000000001', '0000000000000002']);
  });

  it('pins a failed receipt until exact retry, blocking later controls and mismatched receipts', () => {
    let capacity = false;
    const sequences: string[] = [];
    const admission = new BackgroundCommandAdmission({
      submitControllerReclaimReceipt() {},
      submitControllerDeliveryReceipt(sequence) {
        sequences.push(sequence);
        if (!capacity) throw new Error('QueueByteLimit: full');
      }
    });
    const sequence = admission.nextSequence();
    expect(admission.trySubmitReceipt(sequence, RECEIPT)).toBe(false);
    expect(admission.blocked).toBe(true);
    expect(admission.trySubmitControl(() => { throw new Error('must not call'); })).toBe(false);
    expect(() => admission.trySubmitReceipt(sequence, { ...RECEIPT, accepted: false })).toThrow('another receipt');
    capacity = true;
    expect(admission.trySubmitReceipt(sequence, { ...RECEIPT })).toBe(true);
    expect(sequences).toEqual([sequence, sequence]);
    expect(admission.blocked).toBe(false);
    expect(admission.nextSequence()).toBe('0000000000000002');
  });

  it('propagates native faults and bounds identity exhaustion without reentrant admission', () => {
    const native = { submitControllerDeliveryReceipt() {}, submitControllerReclaimReceipt() {} };
    expect(() => new BackgroundCommandAdmission(native, 0n)).toThrow();
    expect(() => new BackgroundCommandAdmission(native, 1 as unknown as bigint)).toThrow();
    const admission = new BackgroundCommandAdmission(native, (1n << 64n) - 1n);
    expect(() => admission.trySubmitControl(() => { throw new Error('Faulted: failed engine'); })).toThrow('Faulted');
    expect(admission.trySubmitControl(sequence => {
      expect(sequence).toBe('ffffffffffffffff');
      expect(admission.trySubmitControl(() => {})).toBe(false);
    })).toBe(true);
    expect(() => admission.nextSequence()).toThrow('exhausted');
  });
});
