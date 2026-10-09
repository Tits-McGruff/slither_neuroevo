/** Evaluate actual native and WebSocket queue observations from a loaded soak. */
import type { RustQueueDiagnostics } from '../../src/protocol/rustBackground.ts';
import type { WsOutboundDiagnostics } from '../../server/wsHub.ts';

/** One complete scalar queue observation from production health. */
export interface QueueSample {
  /** Native occupancy, immutable capacities and runtime-lifetime peaks. */
  nativeQueues: RustQueueDiagnostics;
  /** WebSocket current occupancy and hub-lifetime counters. */
  outbound: WsOutboundDiagnostics;
}

/** Decode an exact native count; missing or truncated evidence must not pass. */
function count(value: string): bigint {
  if (!/^[0-9a-f]{16}$/u.test(value)) throw new Error(`invalid native queue counter: ${value}`);
  return BigInt(`0x${value}`);
}

/** Preserve a stable ordered capacity signature, including future native limits. */
function limits(queues: RustQueueDiagnostics): string {
  return JSON.stringify(Object.entries(queues).flatMap(([group, fields]) =>
    Object.entries(fields).filter(([key]) => key.startsWith('max')).map(([key, value]) => {
      if (typeof value !== 'string' || count(value) <= 0n) throw new Error('native queue capacities must be positive');
      return [`${group}.${key}`, value];
    })).sort(([left], [right]) => left!.localeCompare(right!)));
}

/** Require bounded safe WebSocket counts instead of accepting JSON's null/NaN coercions. */
function websocketCounts(outbound: WsOutboundDiagnostics): void {
  for (const key of ['connections', 'reliableQueuedMessages', 'reliableQueuedBytes', 'pendingFrames',
    'replacedFrames', 'reliableFailures', 'highWaterReliableMessagesPerConnection',
    'highWaterReliableBytesPerConnection', 'maxReliableMessagesPerConnection',
    'maxReliableBytesPerConnection', 'maxConnections'] as const) {
    const value = outbound[key];
    if (!Number.isSafeInteger(value) || Number(value) < 0 ||
        (key.startsWith('max') && Number(value) === 0)) {
      throw new Error(`missing or invalid bounded WebSocket queue counter: ${key}`);
    }
  }
}

/** Check occupancy and retained peaks; failures on closed peers stay in the summary. */
export function summarizeQueueSoak(samples: readonly QueueSample[]): {
  inspectedSamples: number; withinReportedLimits: boolean;
  inboundPeakCommands: string; inboundPeakOwnedBytes: string;
  outputPeakCount: string; outputPeakOwnedBytes: string;
  outputPeakReliable: string; outputPeakReliableOwnedBytes: string;
  outputPeakDiscrete: string; outputPeakDiscreteOwnedBytes: string;
  outputPeakFrames: string; outputPeakFrameOwnedBytes: string;
  outputPeakStats: string; outputPeakStatsOwnedBytes: string;
  websocketPeakMessagesPerConnection: number; websocketPeakBytesPerConnection: number;
  inboundRejectionsDelta: string; outputCapacityWaitsDelta: string;
  faultDiscardedCommandsDelta: string; priorityOverflowsDelta: string; reliableFailuresDelta: number;
  meetsQueueGate: boolean;
} {
  if (samples.length < 2 || samples.some(sample => !sample.nativeQueues || !sample.outbound)) {
    throw new Error('queue soak needs at least two complete native/WebSocket observations');
  }
  const initial = samples[0]!;
  const signature = limits(initial.nativeQueues);
  let withinReportedLimits = true;
  let previous = initial;
  for (const sample of samples) {
    if (limits(sample.nativeQueues) !== signature) throw new Error('native queue capacity changed during the soak');
    websocketCounts(sample.outbound);
    const { inbound, output } = sample.nativeQueues;
    for (const [current, peak, maximum] of [
      [inbound.batches, inbound.highWaterBatches, inbound.maxBatches],
      [inbound.commands, inbound.highWaterCommands, inbound.maxCommands],
      [inbound.ownedBytes, inbound.highWaterOwnedBytes, inbound.maxOwnedBytes],
      [output.ownedBytes, output.highWaterOwnedBytes, output.maxOwnedBytes],
      [output.reliable, output.highWaterReliable, output.maxReliable],
      [output.reliableOwnedBytes, output.highWaterReliableOwnedBytes, output.maxReliableOwnedBytes],
      [output.discrete, output.highWaterDiscrete, output.maxDiscrete],
      [output.discreteOwnedBytes, output.highWaterDiscreteOwnedBytes, output.maxDiscreteOwnedBytes],
      [output.frames, output.highWaterFrames, output.maxFrames]
    ]) {
      if (count(current!) > count(peak!)) throw new Error('current queue occupancy exceeds its retained peak');
      withinReportedLimits &&= count(peak!) <= count(maximum!);
    }
    for (const [current, maximum] of [
      [output.reliable, output.maxReliable], [output.reliableOwnedBytes, output.maxReliableOwnedBytes],
      [output.discrete, output.maxDiscrete], [output.discreteOwnedBytes, output.maxDiscreteOwnedBytes],
      [output.frames, output.maxFrames]
    ]) withinReportedLimits &&= count(current!) <= count(maximum!);
    const combined = count(output.reliable) + count(output.discrete) + count(output.frames) + BigInt(Number(output.hasStats));
    const combinedCapacity = count(output.maxReliable) + count(output.maxDiscrete) + count(output.maxFrames) + 1n;
    if (typeof output.hasStats !== 'boolean' || typeof output.hasReservedFault !== 'boolean' ||
        combined > count(output.highWaterCount)) throw new Error('invalid native output occupancy');
    withinReportedLimits &&= count(output.highWaterCount) <= combinedCapacity && !output.hasReservedFault;
    withinReportedLimits &&= count(output.highWaterStats) <= 1n &&
      count(output.highWaterStatsOwnedBytes) <= count(output.maxEventOwnedBytes) &&
      count(output.highWaterFrameOwnedBytes) <= count(output.maxFrames) * count(output.maxEventOwnedBytes);
    for (const peak of [output.highWaterReliable, output.highWaterDiscrete, output.highWaterFrames, output.highWaterStats]) {
      if (count(peak) > count(output.highWaterCount)) throw new Error('class count peak exceeds combined peak');
    }
    for (const peak of [output.highWaterReliableOwnedBytes, output.highWaterDiscreteOwnedBytes,
      output.highWaterFrameOwnedBytes, output.highWaterStatsOwnedBytes]) {
      if (count(peak) > count(output.highWaterOwnedBytes)) throw new Error('class byte peak exceeds combined peak');
    }
    for (const group of ['inbound', 'output'] as const) {
      for (const [key, value] of Object.entries(sample.nativeQueues[group])) {
        if (typeof value === 'string') count(value);
        if (/^(?:highWater|rejections|faultDiscarded|capacityWaits|priorityOverflows|statsReplacements|frameReplacements|staleStats|staleFrames|statsRejections|frameRejections|statsEvictions|frameEvictions)/u.test(key)) {
          const prior = previous.nativeQueues[group] as unknown as Record<string, string>;
          if (typeof value !== 'string' || count(value) < count(prior[key]!)) throw new Error('native queue lifetime counter regressed');
        }
      }
    }
    const ws = sample.outbound;
    const oldWs = previous.outbound;
    for (const key of ['maxConnections', 'maxReliableMessagesPerConnection', 'maxReliableBytesPerConnection'] as const) {
      if (ws[key] !== initial.outbound[key]) throw new Error('WebSocket queue capacity changed during the soak');
    }
    for (const key of ['replacedFrames', 'reliableFailures', 'highWaterReliableMessagesPerConnection', 'highWaterReliableBytesPerConnection'] as const) {
      if (ws[key] < oldWs[key]) throw new Error('WebSocket lifetime counter regressed after reconnect');
    }
    withinReportedLimits &&= ws.connections <= ws.maxConnections! && ws.pendingFrames <= ws.connections &&
      ws.reliableQueuedMessages <= ws.connections * ws.maxReliableMessagesPerConnection &&
      ws.reliableQueuedBytes <= ws.connections * ws.maxReliableBytesPerConnection &&
      ws.highWaterReliableMessagesPerConnection <= ws.maxReliableMessagesPerConnection &&
      ws.highWaterReliableBytesPerConnection <= ws.maxReliableBytesPerConnection;
    previous = sample;
  }
  const final = samples.at(-1)!;
  const { inbound, output } = final.nativeQueues;
  return { inspectedSamples: samples.length, withinReportedLimits,
    inboundPeakCommands: count(inbound.highWaterCommands).toString(),
    inboundPeakOwnedBytes: count(inbound.highWaterOwnedBytes).toString(),
    outputPeakCount: count(output.highWaterCount).toString(), outputPeakOwnedBytes: count(output.highWaterOwnedBytes).toString(),
    outputPeakReliable: count(output.highWaterReliable).toString(), outputPeakReliableOwnedBytes: count(output.highWaterReliableOwnedBytes).toString(),
    outputPeakDiscrete: count(output.highWaterDiscrete).toString(), outputPeakDiscreteOwnedBytes: count(output.highWaterDiscreteOwnedBytes).toString(),
    outputPeakFrames: count(output.highWaterFrames).toString(), outputPeakFrameOwnedBytes: count(output.highWaterFrameOwnedBytes).toString(),
    outputPeakStats: count(output.highWaterStats).toString(), outputPeakStatsOwnedBytes: count(output.highWaterStatsOwnedBytes).toString(),
    websocketPeakMessagesPerConnection: final.outbound.highWaterReliableMessagesPerConnection,
    websocketPeakBytesPerConnection: final.outbound.highWaterReliableBytesPerConnection,
    inboundRejectionsDelta: (count(inbound.rejections) - count(initial.nativeQueues.inbound.rejections)).toString(),
    outputCapacityWaitsDelta: (count(output.capacityWaits) - count(initial.nativeQueues.output.capacityWaits)).toString(),
    faultDiscardedCommandsDelta: (count(inbound.faultDiscardedCommands) - count(initial.nativeQueues.inbound.faultDiscardedCommands)).toString(),
    priorityOverflowsDelta: (count(output.priorityOverflows) - count(initial.nativeQueues.output.priorityOverflows)).toString(),
    reliableFailuresDelta: final.outbound.reliableFailures - initial.outbound.reliableFailures,
    meetsQueueGate: withinReportedLimits && count(inbound.faultDiscardedCommands) === 0n &&
      count(output.priorityOverflows) === 0n };
}
