import type { HallOfFameEntry } from './protocol/messages.ts';
import { saveWithFallback, loadWithFallback } from './storage.ts';

/** Local storage key for Hall of Fame persistence. */
const HOF_STORAGE_KEY = 'slither_neuroevo_hof';
/** Maximum number of Hall of Fame entries to keep. */
const MAX_HOF_ENTRIES = 50;


/** Persistent Hall of Fame registry with localStorage backing. */
export class HallOfFame {
  /** Ordered list of Hall of Fame entries. */
  entries: HallOfFameEntry[];

  /** Create an empty Hall of Fame and load persisted entries. */
  constructor() {
    this.entries = [];
    this.load();
  }

  /**
   * Load entries from local storage with IndexedDB fallback.
   */
  async load(): Promise<void> {
    try {
      const data = await loadWithFallback(HOF_STORAGE_KEY) as HallOfFameEntry[] | null;
      if (data) {
        this.entries = data.filter((entry) => entry && typeof entry.fitness === 'number');
      }
    } catch (err) {
      console.error('Failed to load HoF', err);
      this.entries = [];
    }
  }

  /**
   * Save entries to local storage with IndexedDB fallback.
   */
  async save(): Promise<void> {
    try {
      await saveWithFallback(HOF_STORAGE_KEY, this.entries);
    } catch (err) {
      console.error('Failed to save HoF', err);
    }
  }

  async add(snakeData: HallOfFameEntry): Promise<void> {
    if (!snakeData || typeof snakeData.fitness === 'undefined') return;

    this.entries.push(snakeData);

    // Sort descending by fitness
    this.entries.sort((a, b) => (b.fitness || 0) - (a.fitness || 0));

    // Trim to max size
    if (this.entries.length > MAX_HOF_ENTRIES) {
      this.entries.length = MAX_HOF_ENTRIES;
    }

    await this.save();
  }

  /**
   * Return a copy of the current entries.
   */
  getAll(): HallOfFameEntry[] {
    return [...this.entries];
  }

  /**
   * Replace entries with a new set and persist them.
   * @param entries - New entries to store.
   */
  async replace(entries: HallOfFameEntry[]): Promise<void> {
    if (!Array.isArray(entries)) return;
    this.entries = entries.filter((entry) => entry && typeof entry.fitness !== 'undefined');
    this.entries.sort((a, b) => (b.fitness || 0) - (a.fitness || 0));
    if (this.entries.length > MAX_HOF_ENTRIES) this.entries.length = MAX_HOF_ENTRIES;
    await this.save();
  }

  /**
   * Clear all history and persist the empty list.
   */
  async reset(): Promise<void> {
    this.entries = [];
    await this.save();
  }

  /**
   * Sync Hall of Fame with the server if in server mode.
   * @param baseUrl - Server base URL.
   */
  async syncToServer(baseUrl: string): Promise<boolean> {
    try {
      const resp = await fetch(`${baseUrl}/api/hof`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ hof: this.entries })
      });
      return resp.ok;
    } catch (err) {
      console.warn('Failed to sync HoF to server', err);
      return false;
    }
  }

  /**
   * Load Hall of Fame from the server if in server mode.
   * @param baseUrl - Server base URL.
   */
  async loadFromServer(baseUrl: string): Promise<boolean> {
    try {
      const resp = await fetch(`${baseUrl}/api/hof`);
      if (!resp.ok) return false;
      const data = (await resp.json()) as { hof: HallOfFameEntry[] };
      if (Array.isArray(data.hof)) {
        await this.replace(data.hof);
        return true;
      }
      return false;
    } catch (err) {
      console.warn('Failed to load HoF from server', err);
      return false;
    }
  }
}

/** Singleton Hall of Fame registry used by the UI. */
export const hof = new HallOfFame();
