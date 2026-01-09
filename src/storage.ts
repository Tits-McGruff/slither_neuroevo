// storage.ts
// Handles persistence of the simulation state (population, generation) to
// localStorage and file export/import.

import { Genome } from './mlp.ts';
import type { CoreSettings, SettingsUpdate } from './protocol/settings.ts';
import type { GraphSpec } from './brains/graph/schema.ts';
import type { GenomeJSON, HallOfFameEntry } from './protocol/messages.ts';

/** Local storage key for population persistence. */
const STORAGE_KEY = 'slither_neuroevo_pop';
/** Local storage key for baseline bot settings persistence. */
const BASELINE_BOT_SETTINGS_KEY = 'slither_neuroevo_baseline_bot_settings';
/** Schema version for baseline bot settings persistence. */
const BASELINE_BOT_SETTINGS_VERSION = 1;

/** Payload stored in localStorage for population persistence. */
export interface PopulationStoragePayload {
  generation: number;
  genomes: GenomeJSON[];
}

/** Baseline bot settings persisted locally. */
export interface BaselineBotSettings {
  count: number;
  seed: number;
  randomizeSeedPerGen: boolean;
  respawnDelay?: number;
}

/** Baseline bot settings payload stored in localStorage. */
interface BaselineBotSettingsPayload extends BaselineBotSettings {
  version: number;
}

/** Payload stored in export files, optionally including HoF. */
export interface PopulationFilePayload extends PopulationStoragePayload {
  /** Optional architecture key for snapshot compatibility. */
  archKey?: string;
  /** Optional server configuration hash for snapshot imports. */
  cfgHash?: string;
  /** Optional world seed for snapshot imports. */
  worldSeed?: number;
  /** Optional graph spec used to rebuild the brain on import. */
  graphSpec?: GraphSpec | null;
  /** Optional core settings captured during export. */
  settings?: CoreSettings;
  /** Optional settings updates captured during export. */
  updates?: SettingsUpdate[];
  hof?: HallOfFameEntry[];
}

/** Genome type alias for storage helpers. */
type GenomeLike = Genome;

/**
 * Simple IndexedDB wrapper for large data persistence fallback.
 */
const idb = {
  db: null as IDBDatabase | null,
  async open(): Promise<IDBDatabase> {
    if (this.db) return this.db;
    if (typeof indexedDB === 'undefined') throw new Error('IndexedDB not supported');
    return new Promise((resolve, reject) => {
      const request = indexedDB.open('slither_neuroevo_db', 1);
      request.onupgradeneeded = () => {
        const db = request.result;
        if (!db.objectStoreNames.contains('kv')) {
          db.createObjectStore('kv');
        }
      };
      request.onsuccess = () => {
        this.db = request.result;
        resolve(this.db);
      };
      request.onerror = () => reject(request.error);
    });
  },
  async put(key: string, value: unknown): Promise<void> {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('kv', 'readwrite');
      const store = tx.objectStore('kv');
      const request = store.put(value, key);
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error);
    });
  },
  async get(key: string): Promise<unknown | null> {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('kv', 'readonly');
      const store = tx.objectStore('kv');
      const request = store.get(key);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  },
  async remove(key: string): Promise<void> {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('kv', 'readwrite');
      const store = tx.objectStore('kv');
      const request = store.delete(key);
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error);
    });
  },
  async clear(): Promise<void> {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('kv', 'readwrite');
      const store = tx.objectStore('kv');
      const request = store.clear();
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error);
    });
  }
};

/**
 * Saves a value to local storage with automated IndexedDB fallback on quota errors.
 * @param key - Storage key.
 * @param value - Value to store.
 * @returns Promise resolving to true on success.
 */
export async function saveWithFallback(key: string, value: unknown): Promise<boolean> {
  try {
    const json = JSON.stringify(value);
    localStorage.setItem(key, json);
    // If it succeeded in localStorage, remove it from IndexedDB to stay tidy
    void idb.remove(key).catch(() => { });
    return true;
  } catch (err) {
    if (err instanceof DOMException && (err.name === 'QuotaExceededError' || err.code === 22)) {
      console.warn(`localStorage quota exceeded for ${key}; falling back to IndexedDB.`);
      try {
        await idb.put(key, value);
        // Remove from localStorage if it happened to have a partial/old value
        localStorage.removeItem(key);
        return true;
      } catch (idbErr) {
        console.error(`IndexedDB fallback failed for ${key}:`, idbErr);
        return false;
      }
    }
    console.error(`Failed to save ${key}:`, err);
    return false;
  }
}

/**
 * Loads a value from local storage with IndexedDB fallback.
 * @param key - Storage key.
 * @returns Promise resolving to parsed value or null.
 */
export async function loadWithFallback(key: string): Promise<unknown | null> {
  try {
    const raw = localStorage.getItem(key);
    if (raw) return JSON.parse(raw);
  } catch (err) {
    console.warn(`Failed to read ${key} from localStorage, checking IndexedDB:`, err);
  }
  try {
    const data = await idb.get(key);
    // If data is a string (legacy/partial), try to parse it, but usually we put objects in IDB
    return data;
  } catch (err) {
    console.error(`Failed to read ${key} from IndexedDB:`, err);
    return null;
  }
}

/**
 * Generic Storage wrapper for localStorage with optional async fallback.
 */
export const Storage = {
  /**
   * Saves an item to localStorage.
   * @param key - Storage key to write.
   * @param value - Value to serialize and store.
   */
  save(key: string, value: unknown): boolean {
    try {
      const json = JSON.stringify(value);
      localStorage.setItem(key, json);
      return true;
    } catch (e) {
      console.error("Failed to save to localStorage:", e);
      return false;
    }
  },

  /**
   * Saves an item with async fallback to IndexedDB.
   */
  saveAsync: saveWithFallback,

  /**
   * Loads an item from localStorage.
   * @param key - Storage key to read.
   * @returns Parsed value or null when missing or invalid.
   */
  load(key: string): unknown | null {
    try {
      const json = localStorage.getItem(key);
      return json ? JSON.parse(json) : null;
    } catch (e) {
      console.error("Failed to load from localStorage:", e);
      return null;
    }
  },

  /**
   * Loads an item with async fallback from IndexedDB.
   */
  loadAsync: loadWithFallback,

  /**
   * Removes an item from storage.
   * @param key - Storage key to remove.
   */
  remove(key: string): void {
    localStorage.removeItem(key);
    void idb.remove(key).catch(() => { });
  },

  /**
   * Clears all slither-related data.
   */
  async clearAll(): Promise<void> {
    localStorage.removeItem(STORAGE_KEY);
    localStorage.removeItem(BASELINE_BOT_SETTINGS_KEY);
    localStorage.removeItem('slither_neuroevo_hof');
    localStorage.removeItem('slither_neuroevo_graph_spec');
    localStorage.removeItem('slither_server_url');
    await idb.clear();
  }
};

/**
 * Normalize a baseline bot settings payload from localStorage.
 * @param value - Raw payload to validate.
 * @returns Normalized baseline bot settings or null when invalid.
 */
function normalizeBaselineBotSettingsPayload(value: unknown): BaselineBotSettings | null {
  if (!value || typeof value !== 'object') return null;
  const payload = value as Partial<BaselineBotSettingsPayload>;
  if (payload.version !== BASELINE_BOT_SETTINGS_VERSION) return null;
  const count = payload.count;
  const seed = payload.seed;
  if (typeof count !== 'number' || !Number.isFinite(count)) return null;
  if (typeof seed !== 'number' || !Number.isFinite(seed)) return null;
  if (typeof payload.randomizeSeedPerGen !== 'boolean') return null;
  const respawnDelay = typeof payload.respawnDelay === 'number' && Number.isFinite(payload.respawnDelay)
    ? payload.respawnDelay
    : undefined;
  const result: BaselineBotSettings = {
    count: Math.max(0, Math.floor(count)),
    seed: Math.max(0, Math.floor(seed)),
    randomizeSeedPerGen: payload.randomizeSeedPerGen
  };
  if (respawnDelay !== undefined) {
    result.respawnDelay = respawnDelay;
  }
  return result;
}

/**
 * Persist baseline bot settings to localStorage.
 * @param settings - Validated baseline bot settings to store.
 * @returns True when saved, false on storage failure.
 */
export function saveBaselineBotSettings(settings: BaselineBotSettings): boolean {
  const payload: BaselineBotSettingsPayload = {
    ...settings,
    version: BASELINE_BOT_SETTINGS_VERSION
  };
  const ok = Storage.save(BASELINE_BOT_SETTINGS_KEY, payload);
  if (!ok) {
    console.warn('Failed to save baseline bot settings to localStorage.');
  }
  return ok;
}

/**
 * Load baseline bot settings from localStorage.
 * @returns Stored baseline bot settings or null when unavailable or invalid.
 */
export function loadBaselineBotSettings(): BaselineBotSettings | null {
  const raw = Storage.load(BASELINE_BOT_SETTINGS_KEY);
  if (!raw) return null;
  const normalized = normalizeBaselineBotSettingsPayload(raw);
  if (!normalized) {
    console.warn('Baseline bot settings payload invalid; clearing.');
    Storage.remove(BASELINE_BOT_SETTINGS_KEY);
    return null;
  }
  return normalized;
}

/**
 * Saves a population provided as JSON objects.
 * @param generation - Current generation number.
 * @param genomes - Genome serializations.
 * @returns Promise resolving to true on success.
 */
export async function savePopulationJSON(generation: number, genomes: GenomeJSON[]): Promise<boolean> {
  const data: PopulationStoragePayload = { generation, genomes };
  return await saveWithFallback(STORAGE_KEY, data);
}

/**
 * Saves the current population and generation to localStorage with IndexedDB fallback.
 * @param generation - Current generation number.
 * @param population - Genome instances to persist.
 */
export async function savePopulation(generation: number, population: GenomeLike[]): Promise<boolean> {
  const data: PopulationStoragePayload = {
    generation: generation,
    genomes: population.map(g => g.toJSON())
  };
  const ok = await saveWithFallback(STORAGE_KEY, data);
  if (ok) {
    console.log(`Saved generation ${generation} to storage.`);
  }
  return ok;
}

/**
 * Loads the population from localStorage with IndexedDB fallback.
 * @param arch - Neural network architecture definition (unused today).
 * @returns Population payload or null when unavailable.
 */
export async function loadPopulation(
  arch: unknown
): Promise<{ generation: number; genomes: GenomeLike[] } | null> {
  void arch;
  const data = (await loadWithFallback(STORAGE_KEY)) as PopulationStoragePayload | null;
  if (!data || !data.genomes || !Array.isArray(data.genomes)) return null;

  const genomes = data.genomes.map((gData: GenomeJSON) => Genome.fromJSON(gData));
  return {
    generation: data.generation || 1,
    genomes: genomes
  };
}

/**
 * Triggers a download of the given data object as a JSON file.
 * @param data - File payload to serialize.
 * @param filename - Downloaded file name.
 */
export function exportToFile(data: PopulationFilePayload, filename: string): void {
  exportJsonToFile(data, filename);
}

/**
 * Reads a JSON file and parses it.
 * @param file - File object selected by the user.
 * @returns Promise resolving to parsed payload.
 */
export function importFromFile(file: File): Promise<PopulationFilePayload> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = (e) => {
      try {
        const data = JSON.parse((e.target as FileReader).result as string);
        resolve(data);
      } catch (err) {
        reject(err);
      }
    };
    reader.onerror = reject;
    reader.readAsText(file);
  });
}

/**
 * Triggers a download of the given data object as a JSON file.
 * @param data - File payload to serialize.
 * @param filename - Downloaded file name.
 */
export function exportJsonToFile(data: unknown, filename: string): void {
  const json = JSON.stringify(data, null, 2);
  const blob = new Blob([json], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}
