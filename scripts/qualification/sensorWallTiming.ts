/** Explicit duration admission; ordinary contention cannot silently become a long soak. */
export function sensorWallTiming(seconds: number, sixHour = false) {
  if (typeof sixHour !== 'boolean' || !Number.isSafeInteger(seconds) ||
      (sixHour ? seconds !== 21_600 : seconds < 60 || seconds > 600)) {
    throw new Error('bounded 60..600-second wall window or explicit six-hour window required');
  }
  // At most 3,150 periodic reads plus 256 barrier reads and two endpoints,
  // including the bounded process-start allowance: strictly below 4,096.
  return {requestedSeconds: seconds, sixHour, maxWallSeconds: sixHour ? 21_900 : 600,
    processDeadlineMs: sixHour ? 21_930_000 : 630_000,
    // Cold verification reads the bounded browser history page (at most 120 rows).
    readIntervalMs: sixHour ? 7000 : 200, generationBoundaryLimit: sixHour ? 120 : 8};
}
