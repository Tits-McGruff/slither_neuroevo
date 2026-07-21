import { afterEach, describe, expect, it, vi } from 'vitest';
import { resetCFGToDefaults } from '../src/config.ts';

/** Shared hoisted observations for the isolated startup wiring test. */
const startupState = vi.hoisted(() => ({
  latestSnapshot: {
    generation: 9,
    archKey: 'saved-arch',
    genomes: [
      {
        archKey: 'saved-arch',
        brainType: 'graph',
        weights: [0.125, -0.25, 0.5],
        fitness: 42
      }
    ],
    cfgHash: 'saved-cfg',
    worldSeed: 2468,
    settings: { snakeCount: 1 },
    updates: []
  },
  constructorSettings: [] as Array<Record<string, unknown>>,
  importCalls: [] as unknown[]
}));

vi.mock('node:http', () => ({
  createServer: () => ({
    off: () => undefined,
    once: () => undefined,
    listen: (_options: unknown, callback: () => void) => callback(),
    address: () => ({ port: 43123 }),
    close: (callback: () => void) => callback()
  })
}));

vi.mock('../src/world.ts', () => {
  /** Minimal constructor used only for startup frame-size sampling. */
  class CharacterizationWorld {
    /** Settings received from startup snapshot metadata. */
    settings: Record<string, unknown>;

    /** @param settings - Startup settings passed by `startServer`. */
    constructor(settings: Record<string, unknown> = {}) {
      this.settings = settings;
    }
  }
  return { World: CharacterizationWorld };
});

vi.mock('../src/serializer.ts', () => ({
  WorldSerializer: {
    serialize: () => new Float32Array([1, 0, 0, 0, 0, 0, 1])
  }
}));

vi.mock('./persistence.ts', () => ({
  validateSnapshotPayload: () => undefined,
  initDb: () => ({ close: () => undefined }),
  createPersistence: () => ({
    loadLatestSnapshot: () => startupState.latestSnapshot,
    saveSnapshot: () => 1,
    loadSnapshot: () => null,
    listSnapshots: () => [],
    saveHofEntry: () => undefined,
    listHofEntries: () => [],
    saveGraphPreset: () => 1,
    listGraphPresets: () => [],
    loadGraphPreset: () => null
  })
}));

vi.mock('./wsHub.ts', () => {
  /** No-network websocket hub used by the startup wiring characterization. */
  class CharacterizationWsHub {
    /** Accept current server handlers without installing sockets. */
    setHandlers(): void {}

    /** Close no-op websocket resources. */
    closeAll(): void {}

    /** @returns Current fake client count. */
    getClientCount(): number {
      return 0;
    }
  }
  return { WsHub: CharacterizationWsHub };
});

vi.mock('./simServer.ts', async () => {
  const actual = await vi.importActual<typeof import('./simServer.ts')>('./simServer.ts');
  /** Lightweight simulation-server seam that records startup orchestration. */
  class CharacterizationSimServer {
    /**
     * Record settings supplied by startup while exposing an observable import method.
     * @param _config - Server configuration (unused).
     * @param _wsHub - Websocket hub (unused).
     * @param _persistence - Persistence adapter (unused).
     * @param _cfgHash - Active configuration hash (unused).
     * @param _worldSeed - Selected startup seed (unused).
     * @param initialSettings - Settings extracted from the saved snapshot.
     */
    constructor(
      _config: unknown,
      _wsHub: unknown,
      _persistence: unknown,
      _cfgHash: string,
      _worldSeed: number,
      initialSettings: Record<string, unknown>
    ) {
      startupState.constructorSettings.push({ ...initialSettings });
    }

    /** Start no-op server work. */
    start(): void {}

    /** Stop no-op server work. */
    stop(): void {}

    /** @returns Current fake tick. */
    getTickId(): number {
      return 0;
    }

    /** @returns No fake World because HTTP is not exercised. */
    getWorld(): null {
      return null;
    }

    /** Record any attempt by startup to restore the saved population. */
    importPopulation(data: unknown): { ok: boolean } {
      startupState.importCalls.push(data);
      return { ok: true };
    }

    /** Ignore fake join routing. */
    handleJoin(): void {}

    /** Ignore fake action routing. */
    handleAction(): void {}

    /** Ignore fake view routing. */
    handleView(): void {}

    /** Ignore fake visualization routing. */
    handleViz(): void {}

    /** Ignore fake reset routing. */
    handleReset(): void {}

    /** Ignore fake disconnect routing. */
    handleDisconnect(): void {}
  }
  return { ...actual, SimServer: CharacterizationSimServer };
});

import { DEFAULT_CONFIG } from './config.ts';
import { startServer } from './index.ts';

/** Unmistakable suite label for current startup population handling. */
const SUITE = 'recovery Phase 0 characterization — current startup restore behavior';

afterEach(() => {
  startupState.constructorSettings.length = 0;
  startupState.importCalls.length = 0;
  resetCFGToDefaults();
  vi.clearAllMocks();
});

describe(SUITE, () => {
  it('PER-003 [expires/converts in Phase 7] startup applies saved settings but never restores saved genomes', async () => {
    const server = await startServer({
      ...DEFAULT_CONFIG,
      port: 0,
      dbPath: ':memory:',
      logLevel: 'error',
      mtEnabled: false
    });
    try {
      expect(startupState.constructorSettings).toEqual([{ snakeCount: 1 }]);
      expect(startupState.latestSnapshot.generation).toBe(9);
      expect(startupState.latestSnapshot.genomes[0]?.weights).toEqual([0.125, -0.25, 0.5]);
      expect(startupState.importCalls).toEqual([]);
    } finally {
      await server.close();
    }
  });
});
