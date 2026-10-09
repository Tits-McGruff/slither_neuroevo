import { describe, expect, it } from 'vitest';
import { summarizeQueueSoak, type QueueSample } from './queue-soak-summary.ts';

/** Small bounded health fixture with all native and hub counters represented exactly. */
function sample(): QueueSample {
  return { nativeQueues: {
    inbound: { batches: '0000000000000000', commands: '0000000000000000', ownedBytes: '0000000000000000',
      highWaterBatches: '0000000000000001', highWaterCommands: '0000000000000001', highWaterOwnedBytes: '0000000000000040',
      maxBatches: '0000000000000040', maxCommands: '0000000000000040', maxOwnedBytes: '0000000000400000',
      maxBatchCommands: '0000000000000001', maxBatchOwnedBytes: '0000000000100000',
      rejections: '0000000000000000', faultDiscardedCommands: '0000000000000000' },
    output: { reliable: '0000000000000000', reliableOwnedBytes: '0000000000000000',
      discrete: '0000000000000000', discreteOwnedBytes: '0000000000000000', frames: '0000000000000000',
      hasStats: false, ownedBytes: '0000000000000000', highWaterCount: '0000000000000002', highWaterOwnedBytes: '0000000000000100',
      highWaterReliable: '0000000000000002', highWaterReliableOwnedBytes: '0000000000000100',
      highWaterDiscrete: '0000000000000000', highWaterDiscreteOwnedBytes: '0000000000000000',
      highWaterFrames: '0000000000000000', highWaterFrameOwnedBytes: '0000000000000000',
      highWaterStats: '0000000000000000', highWaterStatsOwnedBytes: '0000000000000000',
      maxReliable: '0000000000000020', maxReliableOwnedBytes: '0000000001000000',
      maxDiscrete: '0000000000000004', maxDiscreteOwnedBytes: '0000000000100000', maxFrames: '0000000000000004',
      maxOwnedBytes: '0000000002000000', maxEventOwnedBytes: '0000000000100000',
      capacityWaits: '0000000000000000', priorityOverflows: '0000000000000000', hasReservedFault: false,
      statsReplacements: '0000000000000000', frameReplacements: '0000000000000000', staleStats: '0000000000000000',
      staleFrames: '0000000000000000', statsRejections: '0000000000000000', frameRejections: '0000000000000000',
      statsEvictions: '0000000000000000', frameEvictions: '0000000000000000' }
  }, outbound: { connections: 1, reliableQueuedMessages: 0, reliableQueuedBytes: 0, pendingFrames: 1,
    replacedFrames: 2, reliableFailures: 1, highWaterReliableMessagesPerConnection: 2, highWaterReliableBytesPerConnection: 256,
    maxReliableMessagesPerConnection: 1024, maxReliableBytesPerConnection: 4 * 1024 * 1024, maxConnections: 64 } };
}

describe('loaded queue soak evidence', () => {
  it('retains peaks and visible disconnect failures after queues drain', () => {
    const final = sample();
    final.outbound.connections = 0;
    final.outbound.pendingFrames = 0;
    final.outbound.reliableFailures = 3;
    expect(summarizeQueueSoak([sample(), final])).toMatchObject({ meetsQueueGate: true,
      outputPeakCount: '2', inboundPeakOwnedBytes: '64', reliableFailuresDelta: 2 });
  });
  it('cannot turn a drained over-capacity peak into a passing result', () => {
    const final = sample();
    final.nativeQueues.inbound.highWaterCommands = '0000000000000041';
    expect(summarizeQueueSoak([sample(), final])).toMatchObject({ meetsQueueGate: false, withinReportedLimits: false });
  });
  it('rejects regressed lifetime counters and changed or unbounded capacities', () => {
    const reset = sample(); reset.outbound.reliableFailures = 0;
    expect(() => summarizeQueueSoak([sample(), reset])).toThrow(/regressed/);
    const changed = sample(); changed.nativeQueues.inbound.maxCommands = '0000000000000080';
    expect(() => summarizeQueueSoak([sample(), changed])).toThrow(/capacity changed/);
    const unbounded = sample(); unbounded.outbound.maxConnections = null;
    expect(() => summarizeQueueSoak([sample(), unbounded])).toThrow(/bounded/);
  });
  it('checks each class independently when the combined peak exceeds a smaller class capacity', () => {
    const final = sample(); final.nativeQueues.output.highWaterCount = '0000000000000005';
    final.nativeQueues.output.highWaterReliable = '0000000000000005';
    expect(summarizeQueueSoak([sample(), final])).toMatchObject({ withinReportedLimits: true,
      outputPeakReliable: '5', outputPeakDiscrete: '0', meetsQueueGate: true });
    final.nativeQueues.output.highWaterDiscrete = '0000000000000005';
    expect(summarizeQueueSoak([sample(), final])).toMatchObject({ withinReportedLimits: false, meetsQueueGate: false });
  });
  it('rejects incomplete and malformed counters and fails actual priority overflow', () => {
    const malformed = sample(); malformed.nativeQueues.inbound.commands = '1';
    expect(() => summarizeQueueSoak([sample(), malformed])).toThrow(/counter/);
    expect(() => summarizeQueueSoak([])).toThrow(/complete/);
    const missing = sample();
    delete (missing.outbound as Partial<QueueSample['outbound']>).reliableFailures;
    expect(() => summarizeQueueSoak([sample(), missing])).toThrow(/missing/);
    const final = sample(); final.nativeQueues.output.priorityOverflows = '0000000000000001';
    expect(summarizeQueueSoak([sample(), final]).meetsQueueGate).toBe(false);
  });
});
