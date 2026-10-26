import { describe, expect, it } from 'vitest';
import { formatDuration, parseDuration, parseGenerationDuration, parseDurationDraft, formatDurationTimer } from './duration.ts';

describe('duration grammar', () => {
  it.each([
    ['240', 240], ['1h', 3600], ['2m', 120], ['30s', 30],
    ['1h 2m', 3720], ['2m 3s', 123], ['1h 3s', 3603],
    ['1h 2m 3s', 3723], ['1H, 2m 3s', 3723], ['1h 2m,3s', 3723], ['2 m', 120],
    ['2m,3s', 123], ['1:02', 62], ['1:02:03', 3723],
    ['3s\t2 M , 1 h', 3723], ['00', 0], ['168h', 604800],
    ['  8s\t', 8], ['999:59', 59999]
  ])('parses %s', (text, seconds) => expect(parseDuration(text)).toBe(seconds));

  it.each([
    '', ' ', '1.5h', '1.5', '-8', '+8', '1e3', 'NaN', 'Infinity',
    '1h+2m', '1h2m', '1h 2h', '1m 2M', '1s 2s', '2d', '8 seconds',
    '1:2', '1:60', '1:2:03', '1:02:60', '1:60:00', '1:02:03:04',
    ',2m', '2m,', '2m,,3s', '2m, ,3s', '2m ;3s', '2m\n3s',
    '1h,2m,3s', '1 H,\t2m ,3s', '3s, 2m, 1h',
    '9007199254740992', '9007199254740991h', '9'.repeat(129)
  ])('rejects %s', text => expect(parseDuration(text)).toBeNull());

  it('enforces the generation range without clamping', () => {
    expect(parseGenerationDuration('7s')).toBeNull();
    expect(parseGenerationDuration('8s')).toBe(8);
    expect(parseGenerationDuration('168h')).toBe(604800);
    expect(parseGenerationDuration('168h 1s')).toBeNull();
  });

  it('preserves unchanged recorded fractions without accepting newly typed fractions', () => {
    expect(parseDurationDraft('8.25', '8.25')).toBe(8.25);
    expect(parseDurationDraft('8.5', '8.25')).toBeNull();
    expect(parseDurationDraft('8.25')).toBeNull();
    expect(parseDurationDraft('NaN', 'NaN')).toBeNull();
    expect(parseDurationDraft('7.25', '7.25')).toBeNull();
  });

  it('formats hour-scale and seven-day timers without narrowing elapsed time', () => {
    expect(formatDurationTimer(240)).toBe('240.00');
    expect(formatDurationTimer(3600)).toBe('1:00:00.00');
    expect(formatDurationTimer(3723.45)).toBe('1:02:03.45');
    expect(formatDurationTimer(604800)).toBe('168:00:00.00');
    expect(formatDurationTimer(3599.999)).toBe('1:00:00.00');
  });

  it('roundtrips all supported integer seconds', () => {
    for (let seconds = 8; seconds <= 604800; seconds++) {
      if (parseGenerationDuration(formatDuration(seconds)) !== seconds) {
        throw new Error('duration roundtrip failed at ' + seconds);
      }
    }
    expect(formatDuration(8.25)).toBe('8.25');
  });
});
