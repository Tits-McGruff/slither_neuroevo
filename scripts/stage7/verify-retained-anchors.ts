/** Restore and re-export every retained checkpoint from an isolated production fixture. */

import { createWriteStream, existsSync } from 'node:fs';
import { link, mkdir, open, readdir, rm, stat } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { performance } from 'node:perf_hooks';
import { Writable } from 'node:stream';
import { pathToFileURL } from 'node:url';
import Database from 'better-sqlite3';
import { DEFAULT_CONFIG } from '../../server/config.ts';
import { startRustServer } from '../../server/rustServer.ts';

/** Scalar retained checkpoint row selected without population materialization. */
interface Anchor {
  checkpointId: string;
  runId: string;
  generation: string;
  retentionKind: string;
}

/** Ensure the caller names an existing, quiescent production fixture database. */
function fixturePath(argv: readonly string[]): string {
  if (argv.length !== 2 || argv[0] !== '--db-path' || !argv[1]) {
    throw new Error('usage: verify-retained-anchors.ts --db-path EXISTING_FIXTURE_DB');
  }
  const path = resolve(argv[1]);
  if (!existsSync(path) || !existsSync(`${path}.checkpoints`)) {
    throw new Error('fixture database and managed directory must exist');
  }
  return path;
}

/** Select every live retention row, including current, milestone, pinned, and prior-run anchors. */
function retainedAnchors(databasePath: string): Anchor[] {
  const db = new Database(databasePath, { readonly: true, fileMustExist: true });
  try {
    const anchors = db.prepare(`SELECT metadata.checkpoint_id AS checkpointId,
      metadata.run_id AS runId, metadata.generation_hex AS generation,
      retention.retention_kind AS retentionKind
      FROM rust_checkpoint_v3_metadata AS metadata
      JOIN rust_checkpoint_retention_v1 AS retention USING (checkpoint_id)
      WHERE retention.retention_kind IN ('automatic', 'pinned')
      ORDER BY CASE WHEN metadata.run_id =
        (SELECT run_id FROM rust_active_run_v1 WHERE singleton = 1) THEN 0 ELSE 1 END,
        metadata.created_at_ms, metadata.checkpoint_id`).all() as Anchor[];
    if (anchors.length === 0 || anchors.some(anchor => !/^[0-9a-f]{64}$/u.test(anchor.checkpointId))) {
      throw new Error('fixture has no valid retained checkpoint IDs');
    }
    return anchors;
  } finally { db.close(); }
}

/** Create an isolated metadata backup plus hard links to immutable managed files. */
async function copyFixture(sourcePath: string, destinationPath: string): Promise<void> {
  const source = new Database(sourcePath, { readonly: true, fileMustExist: true });
  try { await source.backup(destinationPath); }
  finally { source.close(); }
  const sourceManaged = `${sourcePath}.checkpoints`;
  const destinationManaged = `${destinationPath}.checkpoints`;
  await mkdir(destinationManaged);
  for (const entry of await readdir(sourceManaged, { withFileTypes: true })) {
    if (!entry.isFile() || !/^[0-9a-f]{64}\.(?:checkpoint-v3|hof-weights-v1)$/u.test(entry.name)) continue;
    await link(resolve(sourceManaged, entry.name), resolve(destinationManaged, entry.name));
  }
}

/** Select a prior run's retained current pointer only inside its isolated metadata copy. */
function selectPriorRunInCopy(databasePath: string, anchor: Anchor): void {
  const db = new Database(databasePath);
  try {
    const current = db.prepare('SELECT checkpoint_id AS checkpointId FROM rust_checkpoint_v3_current WHERE run_id = ?')
      .get(anchor.runId) as { checkpointId: string } | undefined;
    if (current?.checkpointId !== anchor.checkpointId) {
      throw new Error('prior-run anchor is not its retained current pointer');
    }
    db.prepare('UPDATE rust_active_run_v1 SET run_id = ? WHERE singleton = 1').run(anchor.runId);
  } finally { db.close(); }
}

/** Read only the bounded final USTAR manifest and compare its checkpoint identity. */
async function inspectArchiveManifest(path: string, anchor: Anchor): Promise<string> {
  const file = await open(path, 'r');
  try {
    const { size } = await file.stat();
    const tailSize = Math.min(size, 2 * 1024 * 1024);
    const tail = Buffer.alloc(tailSize);
    let filled = 0;
    while (filled < tail.length) {
      const result = await file.read(tail, filled, tail.length - filled, size - tailSize + filled);
      if (result.bytesRead === 0) throw new Error('archive ended while reading its manifest');
      filled += result.bytesRead;
    }
    for (let offset = tail.length - 512; offset >= 0; offset -= 512) {
      if (tail.toString('ascii', offset, offset + 13) !== 'manifest.json' || tail[offset + 13] !== 0) continue;
      const octal = tail.toString('ascii', offset + 124, offset + 136).replace(/\0.*$/u, '').trim();
      if (!/^[0-7]+$/u.test(octal)) continue;
      const manifestSize = Number.parseInt(octal, 8);
      if (manifestSize < 1 || manifestSize > 1024 * 1024 || offset + 512 + manifestSize > tail.length) continue;
      const manifest = JSON.parse(tail.toString('utf8', offset + 512,
        offset + 512 + manifestSize)) as Record<string, unknown>;
      if (manifest['magic'] !== 'slither-neuroevo-save' ||
          manifest['checkpointLogicalRootSha256'] !== anchor.checkpointId ||
          manifest['generationHex'] !== anchor.generation || manifest['runId'] !== anchor.runId ||
          typeof manifest['logicalRootSha256'] !== 'string' ||
          !/^[0-9a-f]{64}$/u.test(manifest['logicalRootSha256'])) {
        throw new Error('direct archive manifest differs from the restored checkpoint');
      }
      return manifest['logicalRootSha256'];
    }
    throw new Error('direct archive has no bounded final manifest');
  } finally { await file.close(); }
}

/** Drain one direct production archive without buffering population data in JavaScript. */
async function exportCheckpoint(port: number, anchor: Anchor, path: string): Promise<{
  bytes: number; saveLogicalRootSha256: string
}> {
  const response = await fetch(`http://127.0.0.1:${port}/api/export/latest`, {
    signal: AbortSignal.timeout(180_000)
  });
  if (!response.ok || response.headers.get('x-slither-checkpoint-id') !== anchor.checkpointId || !response.body) {
    throw new Error(`export selected the wrong checkpoint or failed: ${response.status} ${response.headers.get('x-slither-checkpoint-id')}`);
  }
  await response.body.pipeTo(Writable.toWeb(createWriteStream(path)));
  const bytes = (await stat(path)).size;
  if (bytes < 1024) throw new Error('production archive is unexpectedly short');
  return { bytes, saveLogicalRootSha256: await inspectArchiveManifest(path, anchor) };
}

/** Independently restore and re-export every exact retained boundary. */
export async function verifyRetainedAnchors(sourcePath: string): Promise<Record<string, unknown>> {
  const anchors = retainedAnchors(sourcePath);
  const original = new Database(sourcePath, { readonly: true, fileMustExist: true });
  let activeRunId: string;
  try {
    const row = original.prepare('SELECT run_id AS runId FROM rust_active_run_v1 WHERE singleton = 1')
      .get() as { runId: string } | undefined;
    if (!row) throw new Error('fixture has no active run');
    activeRunId = row.runId;
  } finally { original.close(); }
  const scratchRoot = `${sourcePath}.anchor-verification`;
  if (existsSync(scratchRoot)) throw new Error(`verification scratch already exists: ${scratchRoot}`);
  await mkdir(scratchRoot);
  const results: Array<Anchor & { selectedPriorRunInScratch: boolean;
    archiveBytes: number; saveLogicalRootSha256: string; elapsedSeconds: number }> = [];
  try {
    for (let index = 0; index < anchors.length; index++) {
      const anchor = anchors[index]!;
      const workRoot = resolve(scratchRoot, `anchor-${String(index + 1).padStart(3, '0')}`);
      const dbPath = resolve(workRoot, 'fixture.sqlite');
      await mkdir(dirname(dbPath));
      const started = performance.now();
      try {
        await copyFixture(sourcePath, dbPath);
        if (anchor.runId !== activeRunId) selectPriorRunInCopy(dbPath, anchor);
        const server = await startRustServer({ ...DEFAULT_CONFIG, port: 0,
          dbPath, resume: `sha256:${anchor.checkpointId}`, logLevel: 'error' });
        try {
          if (server.startupFault) throw new Error(`anchor startup failed: ${server.startupFault}`);
          const healthResponse = await fetch(`http://127.0.0.1:${server.port}/api/health`);
          const health = await healthResponse.json() as { ok?: boolean; startupCheckpointId?: string };
          if (!health.ok || health.startupCheckpointId !== anchor.checkpointId) {
            throw new Error('exact anchor restore selected another boundary');
          }
          const archive = await exportCheckpoint(server.port, anchor,
            resolve(workRoot, 'export.slither'));
          results.push({ ...anchor, selectedPriorRunInScratch: anchor.runId !== activeRunId,
            archiveBytes: archive.bytes, saveLogicalRootSha256: archive.saveLogicalRootSha256,
            elapsedSeconds: (performance.now() - started) / 1000 });
          process.stderr.write(`verified=${index + 1}/${anchors.length} generation=${BigInt(`0x${anchor.generation}`)} bytes=${archive.bytes}\n`);
        } finally { await server.close(); }
      } finally { await rm(workRoot, { recursive: true, force: true }); }
    }
  } finally { await rm(scratchRoot, { recursive: true, force: true }); }
  return { sourcePath, retainedAnchors: anchors.length, verified: results.length, results };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  void verifyRetainedAnchors(fixturePath(process.argv.slice(2)))
    .then(result => process.stdout.write(`${JSON.stringify(result, null, 2)}\n`))
    .catch(error => { console.error(error); process.exitCode = 1; });
}
