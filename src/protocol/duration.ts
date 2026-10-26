/** Minimum supported generation duration in simulated seconds. */
export const MIN_GENERATION_SECONDS = 8;
/** Seven-day operational ceiling for a generation. */
export const MAX_GENERATION_SECONDS = 604_800;

/**
 * Parse an unsigned integer duration without interpreting expressions or clamping.
 * @param text - Seconds, unit terms, M:SS, or H:MM:SS; at most 128 characters.
 * @returns Exact safe integer seconds, or null for malformed/overflowing input.
 */
export function parseDuration(text: string): number | null {
  if (text.length > 128) return null;
  // The complete duration may contain at most one comma separator.
  if (text.indexOf(',') !== text.lastIndexOf(',')) return null;
  const input = text.replace(/^[ \t]+|[ \t]+$/g, '');
  if (/^[0-9]+$/.test(input)) {
    const seconds = Number(input);
    return Number.isSafeInteger(seconds) ? seconds : null;
  }
  if (/^[0-9]+:[0-5][0-9](?::[0-5][0-9])?$/.test(input)) {
    const fields = input.split(':').map(Number);
    const seconds = fields.length === 2
      ? fields[0]! * 60 + fields[1]!
      : fields[0]! * 3600 + fields[1]! * 60 + fields[2]!;
    return Number.isSafeInteger(seconds) ? seconds : null;
  }
  // Spaces between a number and its unit belong to the term, not its separator.
  const pattern = /([0-9]+)[ \t]*([hms])/iy;
  const seen = new Set<string>();
  let cursor = 0;
  let seconds = 0;
  while (cursor < input.length) {
    pattern.lastIndex = cursor;
    const match = pattern.exec(input);
    if (!match) return null;
    const unit = match[2]!.toLowerCase();
    if (seen.has(unit)) return null;
    seen.add(unit);
    const amount = Number(match[1]);
    const multiplier = unit === 'h' ? 3600 : unit === 'm' ? 60 : 1;
    const contribution = amount * multiplier;
    seconds += contribution;
    if (!Number.isSafeInteger(amount) || !Number.isSafeInteger(seconds)) return null;
    cursor = pattern.lastIndex;
    if (cursor === input.length) return seconds;
    const separator = /^[ \t]*(?:,[ \t]*|[ \t]+)/.exec(input.slice(cursor));
    if (!separator) return null;
    cursor += separator[0].length;
    if (cursor === input.length) return null;
  }
  return null;
}

/**
 * Parse a generation draft and enforce the operational range without clamping.
 * @param text - User-entered duration.
 * @returns Supported seconds or null.
 */
export function parseGenerationDuration(text: string): number | null {
  const seconds = parseDuration(text);
  return seconds != null && seconds >= MIN_GENERATION_SECONDS && seconds <= MAX_GENERATION_SECONDS
    ? seconds : null;
}

/**
 * Read a duration draft while preserving an unchanged fractional legacy recording.
 * @param text - User draft or the unchanged recorded spelling.
 * @param recorded - Recorded numeric spelling installed by authoritative synchronization.
 * @returns Supported seconds, or null when the edited draft is invalid.
 */
export function parseDurationDraft(text: string, recorded?: string): number | null {
  const parsed = parseGenerationDuration(text);
  if (parsed !== null) return parsed;
  if (recorded === undefined || text !== recorded) return null;
  const seconds = Number(recorded);
  return Number.isFinite(seconds) && seconds >= MIN_GENERATION_SECONDS && seconds <= MAX_GENERATION_SECONDS
    ? seconds : null;
}

/**
 * Format recorded seconds without rounding fractional legacy numeric values.
 * @param seconds - Recorded nonnegative finite duration.
 * @returns Compact unit terms, or the original numeric spelling for fractions.
 */
export function formatDuration(seconds: number): string {
  if (!Number.isSafeInteger(seconds) || seconds < 0) return String(seconds);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainder = seconds % 60;
  return [hours ? `${hours}h` : '', minutes ? `${minutes}m` : '', remainder ? `${remainder}s` : '']
    .filter(Boolean).join(' ') || '0s';
}

/**
 * Format a simulated timer with hundredths and an hour field for long generations.
 * @param seconds - Recorded elapsed/configured simulated seconds.
 * @returns Seconds below an hour, otherwise H:MM:SS.ss.
 */
export function formatDurationTimer(seconds: number): string {
  const safe = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  const hundredths = Math.round(safe * 100);
  if (hundredths < 360000) return safe.toFixed(2);
  const hours = Math.floor(hundredths / 360000);
  const minutes = Math.floor(hundredths % 360000 / 6000);
  const remainder = (hundredths % 6000 / 100).toFixed(2).padStart(5, '0');
  return hours + ':' + String(minutes).padStart(2, '0') + ':' + remainder;
}
