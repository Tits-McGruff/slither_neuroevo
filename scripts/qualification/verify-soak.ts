/** Verify one bounded native report; the six-hour gate is explicit and cannot accept a smoke. */
import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import { summarizeNativeSoak, type NativeSoakReceipt } from './soak-summary.ts';

/** Read only a compact native scalar receipt, never checkpoints, dense weights or captured samples. */
async function main(): Promise<void> {
  const args = process.argv.slice(2);
  assert(args.length === 2 || args.length === 3, '--report PATH [--require-six-hours]');
  assert.equal(args[0], '--report'); assert(args[1]);
  if (args.length === 3) assert.equal(args[2], '--require-six-hours');
  const metadata = await stat(args[1]); assert(metadata.isFile() && metadata.size <= 1024 * 1024);
  const report = JSON.parse(await readFile(args[1], 'utf8')) as NativeSoakReceipt;
  const summary = summarizeNativeSoak(report);
  if (args.length === 3) assert(summary.sixHourPressureSoak,
    'requires six measured hours, repeated population deaths and observed slow-consumer pressure');
  console.info(JSON.stringify({gate: report.gate, sourceSha256: report.sourceSha256, ...summary}));
}

await main();
