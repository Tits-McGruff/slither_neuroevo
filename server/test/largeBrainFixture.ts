import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

/** Deterministic production graph and finite packed weights; no binary payload in Git. */
export function largeBrainFixture(): Buffer {
  const graphManifest = JSON.parse(readFileSync(resolve('native/fixtures/production-large-manifest.json'), 'utf8')) as Record<string, unknown>;
  const count = Number(graphManifest['totalParameters']);
  const weights = Buffer.alloc(count * 4);
  for (let index = 0; index < count; index++) weights.writeFloatLE((index % 257 - 128) / 65536, index * 4);
  const manifest = Buffer.from(JSON.stringify({schemaVersion: 1, graphManifest,
    provenance: {schemaVersion: 1, sourceKind: 'distilled', details: {fixture: 'production-large-v1'}}}));
  const header = Buffer.alloc(88);
  header.write('SLTHBRN1'); header.writeUInt32LE(manifest.length, 8);
  header.writeUInt32LE(count, 16); header.writeUInt32LE(weights.length, 20);
  const digest = createHash('sha256').update(weights).digest();
  digest.copy(header, 24); digest.copy(header, 56);
  return Buffer.concat([header, manifest, weights]);
}
