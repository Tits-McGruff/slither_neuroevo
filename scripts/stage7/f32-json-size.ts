/** Count exact JavaScript decimal JSON bytes from streamed finite Float32 values. */
import { performance } from 'node:perf_hooks';

/** Consume one numeric role with at most one incoming chunk and three carried bytes. */
async function main(): Promise<void> {
  const declared = process.argv[2];
  if (!declared || !/^(?:0|[1-9][0-9]*)$/u.test(declared)) throw new Error('expected Float32 count required');
  const expected = Number(declared);
  if (!Number.isSafeInteger(expected) || expected > 0x4000_0000) throw new Error('Float32 count exceeds four GiB');
  let carry = Buffer.alloc(0);
  let floats = 0;
  let scalarBytes = 0;
  let formattingMs = 0;
  let maxChunkBytes = 0;
  for await (const chunk of process.stdin) {
    const incoming = chunk as Buffer;
    maxChunkBytes = Math.max(maxChunkBytes, incoming.length);
    const bytes = carry.length === 0 ? incoming : Buffer.concat([carry, incoming]);
    const end = bytes.length - bytes.length % 4;
    const started = performance.now();
    for (let offset = 0; offset < end; offset += 4) {
      const value = bytes.readFloatLE(offset);
      if (!Number.isFinite(value)) throw new Error('numeric fixture contains a nonfinite Float32');
      scalarBytes += JSON.stringify(value).length;
      if (++floats > expected) throw new Error('numeric stream exceeds its declared count');
    }
    formattingMs += performance.now() - started;
    carry = Buffer.from(bytes.subarray(end));
  }
  if (carry.length !== 0 || floats !== expected) throw new Error('numeric stream count or alignment mismatch');
  process.stdout.write(JSON.stringify({ floats, jsonArrayBytes: scalarBytes + Math.max(0, floats - 1) + 2,
    formattingMs, maxChunkBytes }) + '\n');
}

void main().catch(error => { console.error(String(error)); process.exitCode = 1; });
