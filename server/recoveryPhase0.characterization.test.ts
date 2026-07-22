import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CFG, resetCFGToDefaults } from '../src/config.ts';
import type { SimCore } from '../src/sim/SimCore.ts';
import { World } from '../src/world.ts';
import { DEFAULT_CONFIG } from './config.ts';
import { createPersistence, initDb } from './persistence.ts';
import { SimServer } from './simServer.ts';
import type { WsHub } from './wsHub.ts';

/** Converted Phase 0 characterization label for repaired checkpoint behavior. */
const SUITE = 'recovery Phase 7 — converted PER-005 checkpoint characterization';

/** Narrow access to the SimCore owned by a test server. */
interface SimServerAccess {
  /** Unified simulation core. */
  core: SimCore;
}

/**
 * Build a no-network hub sufficient for construction and welcome refreshes.
 * @returns Minimal websocket hub seam.
 */
function buildHub(): WsHub {
  return {
    sendJsonTo: () => undefined,
    broadcastJsonToUi: () => undefined,
    updateWelcome: () => undefined,
    updateSensorSpec: () => undefined,
    broadcastError: () => undefined,
    hasFrameRecipients: () => false,
    broadcastFrame: () => undefined,
    broadcastStats: () => undefined
  } as unknown as WsHub;
}

beforeEach(() => {
  resetCFGToDefaults();
  CFG.baselineBots.count = 0;
  CFG.pelletCountTarget = 0;
  CFG.pelletSpawnPerSecond = 0;
});

afterEach(() => {
  resetCFGToDefaults();
  vi.restoreAllMocks();
});

describe(SUITE, () => {
  it('PER-005 defaults bounded automatic checkpoints to every generation', () => {
    expect(DEFAULT_CONFIG.checkpointEveryGenerations).toBe(1);
  });

  it('PER-005 commits through the typed boundary path before spawn, pellets, and focus', () => {
    const db = initDb(':memory:');
    const persistence = createPersistence(db);
    const events: string[] = [];
    const saveCheckpoint = persistence.saveCheckpoint.bind(persistence);
    persistence.saveCheckpoint = (checkpoint) => {
      events.push(`checkpoint@${checkpoint.metadata.generation}`);
      return saveCheckpoint(checkpoint);
    };
    const exportSpy = vi.spyOn(World.prototype, 'exportPopulation');
    const server = new SimServer(
      { ...DEFAULT_CONFIG, inferenceBackend: 'js', mtEnabled: false },
      buildHub(),
      persistence,
      '',
      77,
      { snakeCount: 3, simSpeed: 1 },
      'phase7-boundary'
    );
    const world = (server as unknown as SimServerAccess).core.world;
    events.length = 0;
    const originalSpawn = world._spawnAll.bind(world);
    const originalPellets = world._initPellets.bind(world);
    const originalFocus = world._chooseInitialFocus.bind(world);
    world._spawnAll = () => {
      events.push(`spawn@${world.generation}`);
      originalSpawn();
    };
    world._initPellets = () => {
      events.push(`pellets@${world.generation}`);
      originalPellets();
    };
    world._chooseInitialFocus = () => {
      events.push(`focus@${world.generation}`);
      originalFocus();
    };

    world._endGeneration(41);

    expect(events).toEqual([
      'checkpoint@2',
      'spawn@2',
      'pellets@2',
      'focus@2'
    ]);
    expect(exportSpy).not.toHaveBeenCalled();
    const latest = persistence.loadResumeSnapshot('latest');
    expect(latest?.compatibility).toBe('current');
    if (!latest || latest.compatibility !== 'current') throw new Error('checkpoint missing');
    expect(latest.metadata.simulationStep).toBe(41);
    expect(latest.metadata.generation).toBe(2);
    db.close();
  });
});
