import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync, truncateSync, unlinkSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import Database from 'better-sqlite3';
import { afterEach, describe, expect, it } from 'vitest';
import { CheckpointPersistenceClient } from './checkpointPersistenceClient.ts';
import {
  parseManagedCheckpointDescriptor,
  type ManagedCheckpointDescriptor
} from './checkpointPersistenceProtocol.ts';
import { computeNativeSourceIdentity } from './nativeSourceIdentity.ts';

/** Native source root whose exact build identity the test-hooks addon must contain. */
const NATIVE_DIRECTORY = fileURLToPath(new URL('../../native', import.meta.url));

/** Generated production addon loader, resolved independently from the test working directory. */
const PRODUCTION_NATIVE_LOADER = resolve(import.meta.dirname, '../../native/index.js');

/** Optional isolated test-hooks addon used only by the explicit Stage 3 evidence command. */
const TEST_HOOK_ADDON = process.env['SLITHER_STAGE3_CHECKPOINT_TEST_ADDON'];

/** Optional isolated production addon supplied by the self-contained evidence command. */
const TEST_PRODUCTION_ADDON = process.env['SLITHER_STAGE3_CHECKPOINT_PRODUCTION_ADDON'];

/** CommonJS loader scoped to this ESM integration-test module. */
const require = createRequire(import.meta.url);

/** Temporary fixture roots removed only after their persistence workers have stopped. */
const fixtureRoots: string[] = [];

/** Persistence clients that must stop before their SQLite files and roots are removed. */
const clients: CheckpointPersistenceClient[] = [];

/** Minimal production identity surface used by the accidental-export assertion. */
interface ProductionNativeBinding {
  /** Report whether this is a production or explicitly test-hooks build. */
  nativeAddonBuildClass(): string;
  /** Report the exact native source-tree digest embedded by build.rs. */
  nativeAddonSourceSha256(): string;
  /** Allow inspection of any accidental test-only export without declaring it present. */
  [name: string]: unknown;
}

/** Resolve an optional addon path independently from the caller's working directory. */
function resolveAddonPath(addonPath: string): string {
  return isAbsolute(addonPath) ? addonPath : resolve(addonPath);
}

/** Reject any native evidence addon that was built from a different source tree. */
function assertCurrentNativeSource(
  binding: { nativeAddonSourceSha256(): string },
  label: string
): void {
  const expectedSourceSha256 = computeNativeSourceIdentity(NATIVE_DIRECTORY).sha256;
  const embeddedSourceSha256 = binding.nativeAddonSourceSha256();
  if (embeddedSourceSha256 !== expectedSourceSha256) {
    throw new Error(
      `${label} source SHA is stale: addon=${embeddedSourceSha256}, tree=${expectedSourceSha256}`
    );
  }
}

/** Load the normal generated addon or the harness's isolated production build. */
function loadProductionBinding(): ProductionNativeBinding {
  const addonPath = TEST_PRODUCTION_ADDON
    ? resolveAddonPath(TEST_PRODUCTION_ADDON)
    : PRODUCTION_NATIVE_LOADER;
  const loaded = require(addonPath) as unknown;
  if (typeof loaded !== 'object' || loaded === null) {
    throw new TypeError('Stage 3 production addon did not export an object');
  }
  const exports = loaded as Record<string, unknown>;
  if (typeof exports['nativeAddonBuildClass'] !== 'function') {
    throw new TypeError('Stage 3 production addon is missing nativeAddonBuildClass()');
  }
  if (typeof exports['nativeAddonSourceSha256'] !== 'function') {
    throw new TypeError('Stage 3 production addon is missing nativeAddonSourceSha256()');
  }
  const binding = exports as unknown as ProductionNativeBinding;
  assertCurrentNativeSource(binding, 'Stage 3 production addon');
  return binding;
}

/** Test-only native surface emitted only by the isolated engine-test-hooks build. */
interface Stage3CheckpointHookBinding {
  /** Report the isolated addon's build class. */
  nativeAddonBuildClass(): string;
  /** Report the exact native source-tree digest embedded by build.rs. */
  nativeAddonSourceSha256(): string;
  /** Publish a real managed checkpoint and return only its scalar descriptor. */
  publishStage3CheckpointFixture(options: {
    managedDirectory: string;
    operationId: string;
    transitionEpoch: string;
  }): Promise<unknown>;
}

/** Paths belonging to one disposable cross-language handoff fixture. */
interface FixturePaths {
  /** Root removed after the worker exits. */
  root: string;
  /** Controlled directory where Rust publishes immutable checkpoint files. */
  managedRoot: string;
  /** SQLite metadata database that must not exist before Node commits the descriptor. */
  databasePath: string;
}

/** Exact current-pointer row stored after a successful descriptor commit. */
interface CurrentPointerRow {
  /** Content-addressed checkpoint identity. */
  checkpoint_id: string;
  /** Exact fixed-width transition epoch. */
  transition_epoch: string;
  /** Exact correlation token for this publication. */
  operation_id: string;
}

/** Create one empty disposable managed-checkpoint root without starting SQLite. */
function createFixturePaths(): FixturePaths {
  const root = mkdtempSync(join(tmpdir(), 'slither-stage3-checkpoint-handoff-'));
  fixtureRoots.push(root);
  const managedRoot = join(root, 'managed-checkpoints');
  mkdirSync(managedRoot);
  return {
    root,
    managedRoot,
    databasePath: join(root, 'metadata.sqlite')
  };
}

/** Load and validate the explicitly supplied isolated test-hooks addon. */
function loadTestHookBinding(): Stage3CheckpointHookBinding {
  if (!TEST_HOOK_ADDON) {
    throw new Error('SLITHER_STAGE3_CHECKPOINT_TEST_ADDON is required for this evidence test');
  }
  const addonPath = resolveAddonPath(TEST_HOOK_ADDON);
  const loaded = require(addonPath) as unknown;
  if (typeof loaded !== 'object' || loaded === null) {
    throw new TypeError('Stage 3 test-hooks addon did not export an object');
  }
  const exports = loaded as Record<string, unknown>;
  if (typeof exports['nativeAddonBuildClass'] !== 'function') {
    throw new TypeError('Stage 3 test-hooks addon is missing nativeAddonBuildClass()');
  }
  if (typeof exports['nativeAddonSourceSha256'] !== 'function') {
    throw new TypeError('Stage 3 test-hooks addon is missing nativeAddonSourceSha256()');
  }
  if (typeof exports['publishStage3CheckpointFixture'] !== 'function') {
    throw new TypeError('Stage 3 test-hooks addon is missing publishStage3CheckpointFixture()');
  }
  const binding = exports as unknown as Stage3CheckpointHookBinding;
  assertCurrentNativeSource(binding, 'Stage 3 test-hooks addon');
  return binding;
}

/** Start the isolated descriptor-only persistence worker for one fixture. */
function createClient(paths: FixturePaths): CheckpointPersistenceClient {
  const client = new CheckpointPersistenceClient({
    databasePath: paths.databasePath,
    managedRootPath: paths.managedRoot
  });
  clients.push(client);
  return client;
}

/** Publish and strictly parse one deterministic real Rust checkpoint fixture. */
async function publishFixture(
  binding: Stage3CheckpointHookBinding,
  paths: FixturePaths,
  operationId: string
): Promise<ManagedCheckpointDescriptor> {
  return parseManagedCheckpointDescriptor(await binding.publishStage3CheckpointFixture({
    managedDirectory: paths.managedRoot,
    operationId,
    transitionEpoch: '0000000000000001'
  }));
}

/** Read one current pointer after closing the worker that exclusively writes SQLite. */
function readCurrentPointer(databasePath: string, runId: string): CurrentPointerRow | undefined {
  const db = new Database(databasePath, { readonly: true });
  try {
    return db.prepare(
      'SELECT checkpoint_id, transition_epoch, operation_id FROM rust_checkpoint_v3_current WHERE run_id = ?'
    ).get(runId) as CurrentPointerRow | undefined;
  } finally {
    db.close();
  }
}

/** Count committed metadata rows without reading any managed archive bytes. */
function countMetadataRows(databasePath: string): number {
  const db = new Database(databasePath, { readonly: true });
  try {
    const row = db.prepare('SELECT COUNT(*) AS count FROM rust_checkpoint_v3_metadata').get() as {
      count: number;
    };
    return row.count;
  } finally {
    db.close();
  }
}

/** Stop a client once and remove it from the shared cleanup list. */
async function closeClient(client: CheckpointPersistenceClient): Promise<void> {
  await client.close();
  const index = clients.indexOf(client);
  if (index >= 0) clients.splice(index, 1);
}

afterEach(async () => {
  for (const client of clients.splice(0)) await client.close().catch(() => {});
  for (const root of fixtureRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('Stage 3 Rust-to-Node managed checkpoint publication handoff', () => {
  it('keeps the checkpoint fixture export out of the normal production addon', () => {
    const production = loadProductionBinding();
    expect(production.nativeAddonBuildClass()).toBe('production');
    expect(production['publishStage3CheckpointFixture']).toBeUndefined();
  });

  const hookIt = TEST_HOOK_ADDON ? it : it.skip;

  hookIt('publishes the immutable Rust file before committing its SQLite pointer', async () => {
    const binding = loadTestHookBinding();
    const paths = createFixturePaths();
    expect(binding.nativeAddonBuildClass()).toBe('test-hooks');
    expect(existsSync(paths.databasePath)).toBe(false);

    const descriptor = await publishFixture(
      binding,
      paths,
      '0123456789abcdef0123456789abcdef'
    );
    const checkpointPath = join(paths.managedRoot, descriptor.relativeFilename);
    expect(existsSync(paths.databasePath)).toBe(false);
    expect(readdirSync(paths.managedRoot)).toEqual([descriptor.relativeFilename]);
    expect(statSync(checkpointPath).isFile()).toBe(true);
    expect(BigInt(statSync(checkpointPath).size)).toBe(BigInt(`0x${descriptor.storedByteCount}`));

    const client = createClient(paths);
    await expect(client.commit(descriptor)).resolves.toEqual({
      operationId: descriptor.operationId,
      transitionEpoch: descriptor.transitionEpoch,
      runId: descriptor.runId,
      checkpointId: descriptor.logicalRootSha256
    });
    await closeClient(client);

    expect(readCurrentPointer(paths.databasePath, descriptor.runId)).toEqual({
      checkpoint_id: descriptor.logicalRootSha256,
      transition_epoch: descriptor.transitionEpoch,
      operation_id: descriptor.operationId
    });
    expect(countMetadataRows(paths.databasePath)).toBe(1);
  });

  hookIt.each(['truncate', 'delete'] as const)(
    'rejects when the final file is changed by %s without publishing SQLite metadata or a current pointer',
    async mutation => {
      const binding = loadTestHookBinding();
      const paths = createFixturePaths();
      const descriptor = await publishFixture(
        binding,
        paths,
        mutation === 'truncate'
          ? '11111111111111111111111111111111'
          : '22222222222222222222222222222222'
      );
      const checkpointPath = join(paths.managedRoot, descriptor.relativeFilename);
      if (mutation === 'truncate') truncateSync(checkpointPath, 1);
      else unlinkSync(checkpointPath);

      const client = createClient(paths);
      await expect(client.commit(descriptor)).rejects.toThrow(
        mutation === 'truncate' ? /size does not match/i : /ENOENT|no such file/i
      );
      await closeClient(client);

      expect(readCurrentPointer(paths.databasePath, descriptor.runId)).toBeUndefined();
      expect(countMetadataRows(paths.databasePath)).toBe(0);
    }
  );
});
