import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import type { GraphSpec } from '../src/brains/graph/schema.ts';
import type { ServerMessage } from './protocol.ts';
import { World } from '../src/world.ts';
import { buildGenerationCheckpoint } from './checkpoint.ts';
import { DEFAULT_CONFIG, parseConfig } from './config.ts';
import {
  buildLegacyNullGraphConfigHash
} from './configIdentity.ts';
import { createPersistence, initDb, type Persistence } from './persistence.ts';
import { prepareStartupResume } from './startupResume.ts';
import { SimServer } from './simServer.ts';
import {
  captureAuthoritativeWorldDigest,
  findFirstAuthoritativeWorldDivergence
} from './test/authoritativeWorldDigest.ts';
import type { WsHub } from './wsHub.ts';

/** Phase 7 exact-boundary durability and reconstruction suite. */
const SUITE = 'Phase 7 exact checkpoint durability';

/** Direct message recorded by the fake websocket hub. */
interface DirectMessage {
  /** Destination connection id. */
  connId: number;
  /** Protocol response. */
  message: ServerMessage;
}

/** Observable fake hub state. */
interface HubProbe {
  /** Requester-directed messages. */
  direct: DirectMessage[];
}

/**
 * Build a no-network websocket seam for lifecycle tests.
 * @returns Fake hub and direct-message probe.
 */
function buildHub(): { hub: WsHub; probe: HubProbe } {
  const probe: HubProbe = { direct: [] };
  const hub = {
    sendJsonTo: (connId: number, message: ServerMessage) => {
      probe.direct.push({ connId, message });
    },
    broadcastJsonToUi: () => undefined,
    updateWelcome: () => undefined,
    updateSensorSpec: () => undefined,
    broadcastError: () => undefined,
    hasFrameRecipients: () => false,
    broadcastFrame: () => undefined,
    broadcastStats: () => undefined
  } as unknown as WsHub;
  return { hub, probe };
}

/**
 * Install a compact graph that preserves recurrent zero-state coverage.
 * @returns Installed graph definition.
 */
function installRecurrentGraph(): GraphSpec {
  const inputSize = CFG.brain.inSize;
  const spec: GraphSpec = {
    type: 'graph',
    nodes: [
      { id: 'input', type: 'Input', outputSize: inputSize },
      { id: 'memory', type: 'GRU', inputSize, hiddenSize: 4 },
      { id: 'head', type: 'Dense', inputSize: 4, outputSize: 2 }
    ],
    edges: [
      { from: 'input', to: 'memory' },
      { from: 'memory', to: 'head' }
    ],
    outputs: [{ nodeId: 'head' }],
    outputSize: 2
  };
  CFG.brain.graphSpec = spec;
  return spec;
}

/**
 * Encode historical length-prefixed genome JSON records into one gzip BLOB.
 * @param genomes - Legacy genome DTOs.
 * @returns Combined historical BLOB.
 */
function encodeLegacyGenomes(genomes: unknown[]): Buffer {
  const records: Buffer[] = [];
  for (const genome of genomes) {
    const json = Buffer.from(JSON.stringify(genome), 'utf8');
    const prefix = Buffer.alloc(4);
    prefix.writeUInt32LE(json.byteLength, 0);
    records.push(prefix, json);
  }
  return zlib.gzipSync(Buffer.concat(records));
}

/**
 * Create a real persistence-backed serial server without starting its timer loop.
 * @param persistence - Persistence adapter used by exact boundary hooks.
 * @returns Server and fake-hub probe.
 */
function buildServer(persistence: Persistence): { server: SimServer; probe: HubProbe } {
  const { hub, probe } = buildHub();
  const server = new SimServer(
    {
      ...DEFAULT_CONFIG,
      inferenceBackend: 'js',
      mtEnabled: false,
      checkpointEveryGenerations: 1
    },
    hub,
    persistence,
    '',
    0x13572468,
    {
      snakeCount: 4,
      simSpeed: 1,
      hiddenLayers: 1,
      neurons1: 4,
      neurons2: 4,
      neurons3: 4,
      neurons4: 4,
      neurons5: 4
    },
    'phase7-initial-run'
  );
  return { server, probe };
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 2;
  CFG.pelletCountTarget = 100;
  CFG.pelletSpawnPerSecond = 5;
  installRecurrentGraph();
});

afterEach(() => {
  resetCFGToDefaults();
});

describe(SUITE, () => {
  it('reconstructs the exact pre-spawn RNG boundary and zero recurrent state', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const original = new World(
      {
        snakeCount: 5,
        simSpeed: 1,
        hiddenLayers: 1,
        neurons1: 4,
        neurons2: 4,
        neurons3: 4,
        neurons4: 4,
        neurons5: 4
      },
      {
        seed: 0x24681357,
        runId: 'phase7-exact-replay',
        onGenerationBoundary: (boundary, candidate) => {
          persistence.saveCheckpoint(buildGenerationCheckpoint(candidate, boundary, 9));
        }
      }
    );
    original._endGeneration(123);
    original.tickId = 123;
    const loaded = persistence.loadResumeSnapshot('latest');
    if (!loaded || loaded.compatibility !== 'current') throw new Error('current checkpoint missing');
    const bootstrap = prepareStartupResume(loaded);
    const resumed = new World(bootstrap.settings, {
      seed: bootstrap.worldSeed,
      runId: bootstrap.runId,
      resume: bootstrap.resume
    });

    const expected = captureAuthoritativeWorldDigest(original);
    const actual = captureAuthoritativeWorldDigest(resumed);
    expect(findFirstAuthoritativeWorldDivergence(expected, actual)).toBeNull();
    expect(actual.digest).toBe(expected.digest);
    expect(resumed.tickId).toBe(123);
    expect(loaded.metadata.boundaryKind).toBe('generation');
    db.close();
  });

  it('makes New Run immediately restart-resumable before any evolved generation', async () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const { server, probe } = buildServer(persistence);
    const prior = server.getRunIdentity();

    await server.handleNewRun(7, { type: 'newRun', requestId: 'durable-new-run' });

    const response = probe.direct.at(-1)?.message;
    expect(response).toMatchObject({
      type: 'newRunResult',
      requestId: 'durable-new-run',
      applied: true
    });
    const identity = server.getRunIdentity();
    expect(identity.seed).not.toBe(prior.seed);
    expect(identity.runId).not.toBe(prior.runId);
    const loaded = persistence.loadResumeSnapshot('latest');
    if (!loaded || loaded.compatibility !== 'current') throw new Error('new-run checkpoint missing');
    expect(loaded.metadata).toMatchObject({
      boundaryKind: 'run-start',
      generation: 1,
      simulationStep: 0,
      runId: identity.runId,
      worldSeed: identity.seed,
      resumable: true
    });
    const bootstrap = prepareStartupResume(loaded);
    const restarted = new World(bootstrap.settings, {
      seed: bootstrap.worldSeed,
      runId: bootstrap.runId,
      resume: bootstrap.resume
    });
    const activeDigest = captureAuthoritativeWorldDigest(server.getWorld());
    const restartDigest = captureAuthoritativeWorldDigest(restarted);
    expect(findFirstAuthoritativeWorldDivergence(activeDigest, restartDigest)).toBeNull();
    expect(server.getPersistenceStatus()).toMatchObject({
      lastDurableSnapshotId: loaded.id,
      lastDurableGeneration: 1,
      lastDurableRunId: identity.runId
    });
    db.close();
  });

  it('resumes a historical null-hash New Run checkpoint for the compiled fallback graph', async () => {
    resetCFGToDefaults();
    CFG.baselineBots.count = 0;
    CFG.pelletCountTarget = 100;
    CFG.pelletSpawnPerSecond = 5;
    CFG.brain.graphSpec = null;
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const { server } = buildServer(persistence);
    server.getWorld().applyLiveSimSpeed(0.1);

    await server.startNewRun();

    const canonical = persistence.loadResumeSnapshot('latest');
    if (!canonical || canonical.compatibility !== 'current') {
      throw new Error('canonical fallback-graph checkpoint missing');
    }
    const canonicalHash = canonical.metadata.configHash;
    const legacyHash = buildLegacyNullGraphConfigHash(server.getWorld());
    if (!legacyHash) throw new Error('legacy fallback-graph hash missing');
    const row = db.prepare(
      'SELECT payload_json FROM population_snapshots WHERE id = ?'
    ).get(canonical.id) as { payload_json: string };
    const legacyMetadata = JSON.parse(row.payload_json) as { configHash: string };
    legacyMetadata.configHash = legacyHash;
    db.prepare(
      'UPDATE population_snapshots SET payload_json = ? WHERE id = ?'
    ).run(JSON.stringify(legacyMetadata), canonical.id);

    const loaded = persistence.loadResumeSnapshot(canonical.id);
    if (!loaded || loaded.compatibility !== 'current') {
      throw new Error('fallback-graph checkpoint missing');
    }
    const bootstrap = prepareStartupResume(loaded);
    const { hub } = buildHub();
    const resumed = new SimServer(
      {
        ...DEFAULT_CONFIG,
        inferenceBackend: 'js',
        mtEnabled: false
      },
      hub,
      persistence,
      '',
      bootstrap.worldSeed,
      bootstrap.settings,
      bootstrap.runId,
      {
        resume: bootstrap.resume,
        snapshotId: bootstrap.snapshotId,
        exactResume: bootstrap.exact,
        configRevision: bootstrap.configRevision,
        expectedConfigHash: bootstrap.expectedConfigHash
      }
    );

    expect(resumed.getWorld().settings.simSpeed).toBe(0.1);
    expect(loaded.metadata.configHash).toBe(legacyHash);
    expect(resumed.getConfigState().configHash).toBe(canonicalHash);
    await server.stop();
    await resumed.stop();
    db.close();
  });

  it('does not apply null-graph compatibility to an explicit custom graph', () => {
    resetCFGToDefaults();
    installRecurrentGraph();
    const world = new World({ snakeCount: 2 }, { seed: 74, runId: 'explicit-graph' });
    expect(buildLegacyNullGraphConfigHash(world)).toBeNull();
  });

  it('keeps the prior run current when a required New Run checkpoint fails', async () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const originalSave = persistence.saveCheckpoint.bind(persistence);
    let rejectRunStart = false;
    persistence.saveCheckpoint = (checkpoint) => {
      if (rejectRunStart && checkpoint.metadata.boundaryKind === 'run-start') {
        throw new Error('synthetic required checkpoint failure');
      }
      return originalSave(checkpoint);
    };
    const { server, probe } = buildServer(persistence);
    const priorIdentity = server.getRunIdentity();
    const priorSnapshot = persistence.loadResumeSnapshot('latest');
    rejectRunStart = true;

    await server.handleNewRun(8, { type: 'newRun', requestId: 'failed-new-run' });

    expect(probe.direct.at(-1)?.message).toMatchObject({
      type: 'newRunResult',
      requestId: 'failed-new-run',
      applied: false,
      reason: expect.stringContaining('synthetic required checkpoint failure')
    });
    expect(server.getRunIdentity()).toEqual(priorIdentity);
    expect(server.getFaultStatus()).toEqual({
      faulted: true,
      reason: 'synthetic required checkpoint failure',
      tick: 0
    });
    expect(persistence.loadResumeSnapshot('latest')?.id).toBe(priorSnapshot?.id);
    expect(server.getPersistenceStatus()).toMatchObject({
      lastDurableSnapshotId: priorSnapshot?.id,
      inMemoryRunId: priorIdentity.runId
    });
    db.close();
  });

  it('throws before new-generation construction when a scheduled checkpoint fails', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const originalSave = persistence.saveCheckpoint.bind(persistence);
    let rejectGeneration = false;
    persistence.saveCheckpoint = (checkpoint) => {
      if (rejectGeneration && checkpoint.metadata.boundaryKind === 'generation') {
        throw new Error('synthetic generation checkpoint failure');
      }
      return originalSave(checkpoint);
    };
    const { server } = buildServer(persistence);
    const priorSnapshot = persistence.loadResumeSnapshot('latest');
    rejectGeneration = true;

    expect(() => server.getWorld()._endGeneration(55)).toThrow(
      'synthetic generation checkpoint failure'
    );
    expect(server.getWorld().generation).toBe(2);
    expect(server.getWorld().snakes).toHaveLength(0);
    expect(persistence.loadResumeSnapshot('latest')?.id).toBe(priorSnapshot?.id);
    expect(server.getPersistenceStatus()).toMatchObject({
      lastDurableGeneration: 1,
      inMemoryGeneration: 2
    });
    db.close();
  });

  it('commits Reset generation one before switching its same-seed lineage', async () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const { server } = buildServer(persistence);
    const prior = server.getRunIdentity();

    await server.handleReset(1, {
      type: 'reset',
      settings: {
        snakeCount: 3,
        simSpeed: 1,
        hiddenLayers: 1,
        neurons1: 4,
        neurons2: 4,
        neurons3: 4,
        neurons4: 4,
        neurons5: 4
      },
      updates: [],
      graphSpec: installRecurrentGraph()
    });

    const identity = server.getRunIdentity();
    expect(identity.seed).toBe(prior.seed);
    expect(identity.runId).not.toBe(prior.runId);
    const loaded = persistence.loadResumeSnapshot('latest');
    if (!loaded || loaded.compatibility !== 'current') throw new Error('reset checkpoint missing');
    expect(loaded.metadata).toMatchObject({
      boundaryKind: 'run-start',
      generation: 1,
      runId: identity.runId,
      worldSeed: identity.seed,
      configRevision: 1,
      populationCount: 3
    });
    db.close();
  });

  it('bootstraps a compatible legacy database without rewriting its historical row', () => {
    resetCFGToDefaults();
    CFG.baselineBots.count = 0;
    CFG.pelletCountTarget = 100;
    CFG.pelletSpawnPerSecond = 5;
    CFG.brain.graphSpec = null;
    let metadata: ReturnType<typeof buildGenerationCheckpoint>['metadata'] | null = null;
    const original = new World(
      {
        snakeCount: 3,
        simSpeed: 1,
        hiddenLayers: 1,
        neurons1: 4,
        neurons2: 4,
        neurons3: 4,
        neurons4: 4,
        neurons5: 4
      },
      {
        seed: 0xabcdef01,
        runId: 'legacy-source',
        onGenerationBoundary: (boundary, candidate) => {
          metadata = buildGenerationCheckpoint(candidate, boundary, 0).metadata;
        }
      }
    );
    if (!metadata) throw new Error('legacy metadata fixture missing');
    const capturedMetadata = metadata as ReturnType<typeof buildGenerationCheckpoint>['metadata'];
    const genomes = original.population.map((genome) => genome.toJSON());
    const payload = {
      generation: 6,
      archKey: original.archKey,
      genomes: [],
      cfgHash: 'legacy-population-only',
      worldSeed: original.seed,
      settings: capturedMetadata.settings,
      updates: capturedMetadata.updates
    };
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const inserted = db.prepare(
      `INSERT INTO population_snapshots (
         created_at, gen, payload_json, genomes_blob, format_version, boundary_kind
       ) VALUES (?, ?, ?, ?, NULL, NULL)`
    ).run(Date.now(), 6, JSON.stringify(payload), encodeLegacyGenomes(genomes));
    const id = Number(inserted.lastInsertRowid);
    const loaded = persistence.loadResumeSnapshot(id);
    if (!loaded || loaded.compatibility !== 'legacy') throw new Error('legacy snapshot missing');
    const bootstrap = prepareStartupResume(loaded);
    const resumed = new World(bootstrap.settings, {
      seed: bootstrap.worldSeed,
      runId: bootstrap.runId,
      resume: bootstrap.resume
    });
    expect(bootstrap.exact).toBe(false);
    expect(resumed.generation).toBe(6);
    expect(resumed.population.map((genome) => Array.from(genome.weights))).toEqual(
      original.population.map((genome) => Array.from(genome.weights))
    );
    const row = db.prepare(
      'SELECT format_version, genomes_blob FROM population_snapshots WHERE id = ?'
    ).get(id) as { format_version: number | null; genomes_blob: Buffer | null };
    expect(row.format_version).toBeNull();
    expect(row.genomes_blob).not.toBeNull();
    db.close();
  });

  it('parses startup and threading flags and rejects ambiguous CLI input', () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slither-phase7-config-'));
    const freshConfig = path.join(root, 'fresh.toml');
    const idConfig = path.join(root, 'id.toml');
    const latestConfig = path.join(root, 'latest.toml');
    try {
      expect(parseConfig(['--config', freshConfig, '--fresh'], {})).toMatchObject({
        resume: 'fresh',
        checkpointEveryGenerations: 1
      });
      expect(parseConfig(['--config', idConfig, '--resume', '42'], {})).toMatchObject({
        resume: 42
      });
      expect(parseConfig(['--config', latestConfig, '--resume', 'latest'], {})).toMatchObject({
        resume: 'latest'
      });
      expect(parseConfig([
        '--config',
        freshConfig,
        '--fresh',
        '--mt',
        '--mt-workers',
        '2'
      ], {})).toMatchObject({
        resume: 'fresh',
        mtEnabled: true,
        mtWorkers: 2
      });
      expect(() => parseConfig([
        '--config',
        latestConfig,
        '--fresh',
        '--resume',
        'latest'
      ], {})).toThrow('mutually exclusive');
      expect(() => parseConfig([
        '--config',
        latestConfig,
        '--resume',
        'broken'
      ], {})).toThrow('invalid --resume selection');
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
});
