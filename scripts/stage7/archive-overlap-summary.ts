/** Prove overlap using bounds on the native job's origin in the Node clock. */
export function archiveOverlap(
  clock: { beforeMs: number; afterMs: number },
  phase: { startedMicros: string; finishedMicros?: string | null },
  barrier: { startedMs: number; finishedMs: number }
): { guaranteedMs: number; possibleMs: number; originUncertaintyMs: number } {
  for (const value of [clock.beforeMs, clock.afterMs, barrier.startedMs, barrier.finishedMs]) {
    if (!Number.isFinite(value) || value < 0) throw new Error('invalid monotonic clock observation');
  }
  if (clock.afterMs < clock.beforeMs || barrier.finishedMs < barrier.startedMs) throw new Error('reversed clock interval');
  /** Keep the native microsecond conversion exact for this bounded measurement. */
  function micros(value: string | null | undefined): number {
    if (!value || !/^[0-9a-f]{16}$/u.test(value)) throw new Error('missing or invalid native phase boundary');
    const exact = BigInt(`0x${value}`);
    if (exact > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('inexact native phase boundary');
    return Number(exact) / 1000;
  }
  const start = micros(phase.startedMicros);
  const finish = micros(phase.finishedMicros);
  if (finish < start) throw new Error('reversed native phase');
  return {
    guaranteedMs: Math.max(0, Math.min(clock.beforeMs + finish, barrier.finishedMs) -
      Math.max(clock.afterMs + start, barrier.startedMs)),
    possibleMs: Math.max(0, Math.min(clock.afterMs + finish, barrier.finishedMs) -
      Math.max(clock.beforeMs + start, barrier.startedMs)),
    originUncertaintyMs: clock.afterMs - clock.beforeMs
  };
}
