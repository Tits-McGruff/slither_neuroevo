import { CFG, resetCFGToDefaults } from '../src/config.ts';
import { normalizeSeed } from '../src/rng.ts';
import type { CoreSettings } from '../src/protocol/settings.ts';
import type { WorldResumeState } from '../src/world.ts';
import { validateGraph } from '../src/brains/graph/validate.ts';
import type { LoadedResumeSnapshot, Persistence } from './persistence.ts';
import { SnapshotLoadError } from './persistence.ts';
import { applySettingsUpdates, coerceCoreSettings } from './simServer.ts';
import type { ResumeSelection } from './config.ts';

/** Fully prepared experiment state passed into SimServer construction. */
export interface StartupResumeBootstrap {
  /** Selected SQLite snapshot id. */
  snapshotId: number;
  /** Whether the checkpoint has exact generation-boundary continuation state. */
  exact: boolean;
  /** Core settings restored before World construction. */
  settings: CoreSettings;
  /** Active normalized experiment seed. */
  worldSeed: number;
  /** Restored or compatibility lineage id. */
  runId: string;
  /** Restored authoritative configuration revision. */
  configRevision: number;
  /** Strict expected config hash for current snapshots, null for legacy rows. */
  expectedConfigHash: string | null;
  /** Population-assigned state that bypasses random population initialization. */
  resume: WorldResumeState;
}

/**
 * Convert a complete core-settings value after shared range normalization.
 * @param value - Saved settings value.
 * @param snapshotId - Snapshot id used in diagnostics.
 * @returns Complete normalized settings.
 */
function normalizeSavedCoreSettings(value: unknown, snapshotId: number): CoreSettings {
  const normalized = coerceCoreSettings(value);
  const required: Array<keyof CoreSettings> = [
    'snakeCount',
    'simSpeed',
    'hiddenLayers',
    'neurons1',
    'neurons2',
    'neurons3',
    'neurons4',
    'neurons5'
  ];
  for (const key of required) {
    if (normalized[key] === undefined) {
      throw new SnapshotLoadError(snapshotId, `saved core setting ${key} is missing`);
    }
  }
  return normalized as CoreSettings;
}

/**
 * Select a startup snapshot without silently skipping a broken latest row.
 * @param persistence - Open persistence adapter.
 * @param selection - Latest or explicit snapshot id.
 * @returns Loaded snapshot or null when latest is requested from an empty DB.
 */
export function selectStartupSnapshot(
  persistence: Persistence,
  selection: Exclude<ResumeSelection, 'fresh'>
): LoadedResumeSnapshot | null {
  try {
    return persistence.loadResumeSnapshot(selection);
  } catch (error) {
    const snapshotId = error instanceof SnapshotLoadError ? error.snapshotId : undefined;
    const alternatives = persistence.listValidResumeSnapshots(5, snapshotId);
    const alternativeText = alternatives.length > 0
      ? alternatives.map((item) => `${item.id} (gen ${item.gen}, ${item.boundaryKind})`).join(', ')
      : 'none';
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(
      `resume selection ${String(selection)} failed: ${reason}; valid alternatives: ${alternativeText}`,
      { cause: error }
    );
  }
}

/**
 * Apply a selected snapshot's experiment configuration and build World resume state.
 * @param loaded - Strict current checkpoint or read-only legacy compatibility row.
 * @returns Fully prepared SimServer bootstrap inputs.
 */
export function prepareStartupResume(loaded: LoadedResumeSnapshot): StartupResumeBootstrap {
  resetCFGToDefaults();
  if (loaded.compatibility === 'current') {
    const metadata = loaded.metadata;
    applySettingsUpdates(metadata.updates);
    CFG.brain.graphSpec = metadata.graphSpec;
    const settings = normalizeSavedCoreSettings(metadata.settings, loaded.id);
    return {
      snapshotId: loaded.id,
      exact: true,
      settings,
      worldSeed: metadata.worldSeed,
      runId: metadata.runId,
      configRevision: metadata.configRevision,
      expectedConfigHash: metadata.configHash,
      resume: {
        generation: metadata.generation,
        simulationStep: metadata.simulationStep,
        population: loaded.genomes,
        rng: metadata.rng,
        allocators: metadata.allocators,
        bestFitnessEver: metadata.bestFitnessEver,
        fitnessHistory: metadata.fitnessHistory,
        lastHofEntry: metadata.lastHofEntry,
        exact: true
      }
    };
  }

  const payload = loaded.payload;
  if (payload.updates) applySettingsUpdates(payload.updates);
  if (payload.graphSpec) {
    const graphResult = validateGraph(payload.graphSpec);
    if (!graphResult.ok) {
      throw new SnapshotLoadError(loaded.id, `legacy graph is invalid: ${graphResult.reason}`);
    }
    CFG.brain.graphSpec = payload.graphSpec;
  }
  const compatibilitySettings = {
    ...coerceCoreSettings(payload.settings),
    snakeCount: loaded.genomes.length
  };
  const settings = normalizeSavedCoreSettings({
    snakeCount: loaded.genomes.length,
    simSpeed: compatibilitySettings.simSpeed ?? 1,
    hiddenLayers: compatibilitySettings.hiddenLayers ?? 2,
    neurons1: compatibilitySettings.neurons1 ?? 64,
    neurons2: compatibilitySettings.neurons2 ?? 64,
    neurons3: compatibilitySettings.neurons3 ?? 64,
    neurons4: compatibilitySettings.neurons4 ?? 48,
    neurons5: compatibilitySettings.neurons5 ?? 32
  }, loaded.id);
  const worldSeed = normalizeSeed(payload.worldSeed);
  return {
    snapshotId: loaded.id,
    exact: false,
    settings,
    worldSeed,
    runId: payload.runId?.trim() || `legacy-snapshot-${loaded.id}`,
    configRevision: Number.isSafeInteger(payload.configRevision) && payload.configRevision! >= 0
      ? payload.configRevision!
      : 0,
    expectedConfigHash: null,
    resume: {
      generation: Math.max(1, Math.floor(payload.generation)),
      simulationStep: 0,
      population: loaded.genomes,
      bestFitnessEver: loaded.genomes.reduce(
        (best, genome) => Math.max(best, genome.fitness),
        0
      ),
      fitnessHistory: [],
      lastHofEntry: null,
      exact: false
    }
  };
}
