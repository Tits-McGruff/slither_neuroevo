/** Contract tests for the disposable Stage 2 managed-checkpoint format probe. */

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  assembleUstarFixture,
  computeLogicalRoot,
  createUstarHeader,
  decodeShuffledZstdBlocks,
  encodeShuffledZstdBlocks,
  logicalSha256,
  ManagedCheckpointFormatError,
  scanUstarBuffer,
  selectNumericEncoding,
  USTAR_BLOCK_BYTES,
  USTAR_TRAILER_BYTES,
  verifyLogicalRole,
  verifyLogicalRoot,
  type LogicalRoleDigest
} from './managedCheckpointFormat.ts';

/** USTAR checksum field byte offset. */
const CHECKSUM_OFFSET = 148;
/** USTAR checksum field width. */
const CHECKSUM_BYTES = 8;

/**
 * Recalculate a mutated test header's USTAR checksum.
 * @param header - Mutable exact header.
 */
function repairHeaderChecksum(header: Buffer): void {
  header.fill(0x20, CHECKSUM_OFFSET, CHECKSUM_OFFSET + CHECKSUM_BYTES);
  let sum = 0;
  for (const value of header) sum += value;
  header.write(sum.toString(8).padStart(6, '0'), CHECKSUM_OFFSET, 6, 'ascii');
  header[154] = 0;
  header[155] = 0x20;
}

/**
 * Assert that one operation fails with the expected format code.
 * @param operation - Operation expected to fail.
 * @param code - Stable diagnosis.
 */
function expectFormatCode(operation: () => unknown, code: string): void {
  try {
    operation();
  } catch (error) {
    expect(error).toBeInstanceOf(ManagedCheckpointFormatError);
    expect((error as ManagedCheckpointFormatError).code).toBe(code);
    return;
  }
  throw new Error(`expected format error ${code}`);
}

/**
 * Build a deterministic high-entropy multiple-of-four byte sequence.
 * @param bytes - Required byte count.
 * @returns Hash-derived bytes.
 */
function deterministicEntropy(bytes: number): Buffer {
  const chunks: Buffer[] = [];
  for (let counter = 0; Buffer.concat(chunks).length < bytes; counter++) {
    chunks.push(createHash('sha256').update(`stage2-entropy-${counter}`).digest());
  }
  return Buffer.concat(chunks).subarray(0, bytes);
}

/**
 * Build one minimal valid format fixture.
 * @returns Complete archive and its ordered logical roles.
 */
function validFixture(): { archive: Buffer; roles: LogicalRoleDigest[]; root: string } {
  const checkpoint = Buffer.from('{"magic":"stage2-checkpoint"}\n', 'utf8');
  const weights = Buffer.allocUnsafe(64);
  for (let index = 0; index < weights.length; index++) weights[index] = (index * 37 + 11) & 0xff;
  const roles: LogicalRoleDigest[] = [
    {
      role: 'checkpoint',
      logicalLength: checkpoint.length,
      logicalSha256: logicalSha256(checkpoint)
    },
    {
      role: 'population-weights',
      logicalLength: weights.length,
      logicalSha256: logicalSha256(weights)
    }
  ];
  const root = computeLogicalRoot(roles);
  const manifest = Buffer.from(`${JSON.stringify({ version: 1, roles, logicalRoot: root })}\n`);
  return {
    archive: assembleUstarFixture([
      { name: 'checkpoint.json', data: checkpoint },
      { name: 'population/weights.f32le', data: weights },
      { name: 'manifest.json', data: manifest }
    ]),
    roles,
    root
  };
}

describe('Stage 2 managed-checkpoint format primitives', () => {
  it('assembles and strictly scans a manifest-last USTAR fixture', () => {
    const fixture = validFixture();
    const scanned = scanUstarBuffer(fixture.archive, {
      allowedNames: [
        'checkpoint.json',
        'population/weights.f32le',
        'manifest.json'
      ],
      requiredNames: [
        'checkpoint.json',
        'population/weights.f32le',
        'manifest.json'
      ]
    });
    expect(scanned.entries.map(entry => entry.name)).toEqual([
      'checkpoint.json',
      'population/weights.f32le',
      'manifest.json'
    ]);
    expect(scanned.byteLength % USTAR_BLOCK_BYTES).toBe(0);
    expect(scanned.byteLength).toBeGreaterThanOrEqual(USTAR_TRAILER_BYTES);
    expect(scanned.manifest).toMatchObject({ version: 1, logicalRoot: fixture.root });
    verifyLogicalRoot(fixture.roles, fixture.root);
  });

  it('round-trips multiple bounded shuffled-Zstandard blocks bit exactly', () => {
    const raw = Buffer.allocUnsafe(4 * 257);
    for (let index = 0; index < raw.length; index++) raw[index] = (index * 73 + 19) & 0xff;
    for (const checksum of [false, true]) {
      const encoded = encodeShuffledZstdBlocks(raw, { blockBytes: 128, checksum });
      const decoded = decodeShuffledZstdBlocks(encoded, {
        maxBlockBytes: 128,
        maxTotalDecodedBytes: raw.length
      });
      expect(decoded).toEqual(raw);
    }
  });

  it('falls back to raw packed bytes when compression expands high-entropy data', () => {
    const raw = deterministicEntropy(4 * 4096);
    const selected = selectNumericEncoding(raw, { blockBytes: 1024, checksum: false });
    expect(selected.shuffledCandidateBytes).toBeGreaterThan(raw.length);
    expect(selected.encoding).toBe('raw-f32le-v1');
    expect(selected.stored).toBe(raw);
  });

  it('rejects truncated headers, trailers, entries and extra trailing bytes', () => {
    const { archive } = validFixture();
    expectFormatCode(
      () => scanUstarBuffer(archive.subarray(0, archive.length - USTAR_BLOCK_BYTES)),
      'USTAR_TRUNCATED_TRAILER'
    );
    expectFormatCode(
      () => scanUstarBuffer(archive.subarray(0, archive.length - USTAR_TRAILER_BYTES + 31)),
      'USTAR_TRUNCATED_HEADER'
    );
    expectFormatCode(
      () => scanUstarBuffer(Buffer.concat([archive, Buffer.alloc(USTAR_BLOCK_BYTES)])),
      'USTAR_TRAILING_BYTES'
    );
    const falseSize = Buffer.from(archive);
    falseSize.write('77777777777\0', 124, 12, 'ascii');
    repairHeaderChecksum(falseSize.subarray(0, USTAR_BLOCK_BYTES));
    expectFormatCode(() => scanUstarBuffer(falseSize), 'USTAR_TRUNCATED_ENTRY');
  });

  it('rejects damaged headers, duplicate roles, missing roles and manifest ordering', () => {
    const { archive } = validFixture();
    const corrupt = Buffer.from(archive);
    corrupt[12] = corrupt[12]! ^ 0x01;
    expectFormatCode(() => scanUstarBuffer(corrupt), 'USTAR_HEADER_CHECKSUM');
    expectFormatCode(
      () => assembleUstarFixture([
        { name: 'checkpoint.json', data: Buffer.alloc(0) },
        { name: 'checkpoint.json', data: Buffer.alloc(0) }
      ]),
      'USTAR_DUPLICATE_ENTRY'
    );
    expectFormatCode(
      () => scanUstarBuffer(archive, { requiredNames: ['missing.bin'] }),
      'USTAR_MISSING_ENTRY'
    );
    const wrongOrder = assembleUstarFixture([
      { name: 'manifest.json', data: Buffer.from('{}') },
      { name: 'checkpoint.json', data: Buffer.from('{}') }
    ]);
    expectFormatCode(() => scanUstarBuffer(wrongOrder), 'USTAR_MANIFEST_ORDER');
  });

  it('rejects unsafe paths and every non-regular USTAR type', () => {
    expectFormatCode(() => createUstarHeader('../weights.bin', 0), 'USTAR_UNSAFE_PATH');
    expectFormatCode(() => createUstarHeader('C:/weights.bin', 0), 'USTAR_UNSAFE_PATH');
    expectFormatCode(() => createUstarHeader('population\\weights.bin', 0), 'USTAR_UNSAFE_PATH');

    const { archive } = validFixture();
    const unsafe = Buffer.from(archive);
    unsafe.fill(0, 0, 100);
    unsafe.write('../bad', 0, 'ascii');
    repairHeaderChecksum(unsafe.subarray(0, USTAR_BLOCK_BYTES));
    expectFormatCode(() => scanUstarBuffer(unsafe), 'USTAR_UNSAFE_PATH');

    for (const type of ['1', '2', '3', '4', '5', '6', '7', 'g', 'x', 'L', 'K', 'S']) {
      const typed = Buffer.from(archive);
      typed.write(type, 156, 1, 'ascii');
      repairHeaderChecksum(typed.subarray(0, USTAR_BLOCK_BYTES));
      expectFormatCode(() => scanUstarBuffer(typed), 'USTAR_ENTRY_TYPE');
    }
  });

  it('rejects corrupt frames, false frame lengths and declared decode excess', () => {
    const raw = Buffer.alloc(1024, 0x3f);
    const encoded = encodeShuffledZstdBlocks(raw, { blockBytes: 512, checksum: true });
    const corrupt = Buffer.from(encoded);
    corrupt[20] = corrupt[20]! ^ 0x80;
    expectFormatCode(
      () => decodeShuffledZstdBlocks(corrupt, {
        maxBlockBytes: 512,
        maxTotalDecodedBytes: raw.length
      }),
      'SHUFFLED_BLOCK_DECODE'
    );
    const falseFrame = Buffer.from(encoded);
    falseFrame.writeUInt32LE(encoded.length, 8);
    expectFormatCode(
      () => decodeShuffledZstdBlocks(falseFrame, {
        maxBlockBytes: 512,
        maxTotalDecodedBytes: raw.length
      }),
      'SHUFFLED_BLOCK_FRAME'
    );
    const falseDecoded = Buffer.from(encoded);
    falseDecoded.writeUInt32LE(1024, 4);
    expectFormatCode(
      () => decodeShuffledZstdBlocks(falseDecoded, {
        maxBlockBytes: 512,
        maxTotalDecodedBytes: raw.length
      }),
      'SHUFFLED_BLOCK_LIMIT'
    );
  });

  it('rejects false logical role hashes, lengths and roots without extra digest layers', () => {
    const logical = Buffer.from('logical-role-bytes');
    const role: LogicalRoleDigest = {
      role: 'checkpoint',
      logicalLength: logical.length,
      logicalSha256: logicalSha256(logical)
    };
    verifyLogicalRole(role, logical);
    expectFormatCode(
      () => verifyLogicalRole({ ...role, logicalLength: role.logicalLength + 1 }, logical),
      'LOGICAL_ROLE_MISMATCH'
    );
    expectFormatCode(
      () => verifyLogicalRole({ ...role, logicalSha256: '0'.repeat(64) }, logical),
      'LOGICAL_ROLE_MISMATCH'
    );
    const root = computeLogicalRoot([role]);
    verifyLogicalRoot([role], root);
    expectFormatCode(() => verifyLogicalRoot([role], 'f'.repeat(64)), 'LOGICAL_ROOT_MISMATCH');
  });
});
