/** Manages the Hall of Fame registry of top-performing snakes. */

import type { HallOfFameEntry } from './protocol/messages.ts';

/** Local storage key for Hall of Fame persistence. */
const HOF_STORAGE_KEY = 'slither_neuroevo_hof';
/** Maximum number of Hall of Fame entries to keep. */
const MAX_HOF_ENTRIES = 50;

/**
 * Resolve browser localStorage if available.
 * @returns Storage instance or null when unavailable.
 */
function getStorage(): Storage | null {
  if (typeof globalThis === 'undefined') return null;
  try {
    return globalThis.localStorage || null;
  } catch {
    return null;
  }
}

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
   * Load entries from local storage.
   */
  load(): void {
    const storage = getStorage();
    if (!storage) return;
    try {
      const raw = storage.getItem(HOF_STORAGE_KEY);
      if (raw) {
        this.entries = JSON.parse(raw) as HallOfFameEntry[];
      }
    } catch (err) {
      console.error('Failed to load HoF', err);
      this.entries = [];
    }
  }

  /**
   * Save entries to local storage.
   */
  save(): void {
    const storage = getStorage();
    if (!storage) return;
    try {
      storage.setItem(HOF_STORAGE_KEY, JSON.stringify(this.entries));
    } catch (err) {
      console.error('Failed to save HoF', err);
    }
  }

  /**
   * Add a candidate entry and keep the top performers by fitness.
   * @param snakeData - Hall of Fame entry to consider.
   */
  add(snakeData: HallOfFameEntry): void {
    if (!snakeData || typeof snakeData.fitness !== 'number') return;

    this.entries.push(snakeData);
    
    // Sort descending by fitness
    this.entries.sort((a, b) => b.fitness - a.fitness);

    // Trim to max size
    if (this.entries.length > MAX_HOF_ENTRIES) {
      this.entries.length = MAX_HOF_ENTRIES;
    }
    
    this.save();
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
  replace(entries: HallOfFameEntry[]): void {
    if (!Array.isArray(entries)) return;
    this.entries = entries.filter((entry) => entry && typeof entry.fitness === 'number');
    this.entries.sort((a, b) => b.fitness - a.fitness);
    if (this.entries.length > MAX_HOF_ENTRIES) this.entries.length = MAX_HOF_ENTRIES;
    this.save();
  }

  /**
   * Clear all history and persist the empty list.
   */
  reset(): void {
    this.entries = [];
    this.save();
  }
}

/** Singleton Hall of Fame registry used by the UI. */
export const hof = new HallOfFame();
