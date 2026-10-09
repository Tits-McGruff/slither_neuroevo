import { describe, expect, it } from 'vitest';
import { ArchiveWorkDeadline } from './archiveWorkWatchdog.ts';
import type { RustArchiveWorkProgress } from './backgroundRuntime.ts';

/** One active native export snapshot with canonical cross-boundary counters. */
function progress(completedBytes: string, finished = false): RustArchiveWorkProgress {
  return { operationId: 'ab'.repeat(16), kind: 'export', completedBytes,
    started: true, finished };
}

describe('native archive no-progress deadline', () => {
  it('permits long jobs while real bounded work keeps advancing', () => {
    const deadline = new ArchiveWorkDeadline('ab'.repeat(16), 'export', 0, 60_000);
    expect(deadline.observe(progress('0000000000000000'), 59_999)).toBeNull();
    expect(deadline.observe(progress('0000000000000001'), 60_001)).toBeNull();
    expect(deadline.observe(progress('0000000000000002'), 120_000)).toBeNull();
    expect(deadline.observe(progress('0000000000000002', true), 500_000)).toBeNull();
  });

  it('rejects an idle, regressing, or mismatched worker', () => {
    const deadline = new ArchiveWorkDeadline('ab'.repeat(16), 'export', 0, 60_000);
    expect(deadline.observe(progress('0000000000000000'), 60_000)?.message).toContain('no progress');
    expect(deadline.observe(progress('0000000000000002'), 60_001)).toBeNull();
    expect(deadline.observe(progress('0000000000000001'), 60_002)?.message).toContain('backwards');
    expect(deadline.observe({ ...progress('0000000000000002'), operationId: 'cd'.repeat(16) },
      60_003)?.message).toContain('identity');
  });
});
