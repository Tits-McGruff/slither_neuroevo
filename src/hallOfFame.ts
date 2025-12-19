// hallOfFame.ts
// Manages the "Hall of Fame" - a registry of the best snakes from previous generations.

import type { HallOfFameEntry } from './protocol/messages.ts';

const HOF_STORAGE_KEY = 'slither_neuroevo_hof';
const MAX_HOF_ENTRIES = 50;

function getStorage(): Storage | null {
  if (typeof globalThis === 'undefined') return null;
  try {
    return globalThis.localStorage || null;
  } catch (e) {
    return null;
  }
}

export class HallOfFame {
  entries: HallOfFameEntry[];

  constructor() {
    this.entries = [];
    this.load();
  }

  /**
   * Loads entries from local storage.
   */
  load(): void {
    const storage = getStorage();
    if (!storage) return;
    try {
      const raw = storage.getItem(HOF_STORAGE_KEY);
      if (raw) {
        this.entries = JSON.parse(raw) as HallOfFameEntry[];
      }
    } catch (e) {
      console.error("Failed to load HoF", e);
      this.entries = [];
    }
  }

  /**
   * Saves entries to local storage.
   */
  save(): void {
    const storage = getStorage();
    if (!storage) return;
    try {
      storage.setItem(HOF_STORAGE_KEY, JSON.stringify(this.entries));
    } catch (e) {
      console.error("Failed to save HoF", e);
    }
  }

  /**
   * Adds a potential candidate to the Hall of Fame.
   * Logic: We keep the top N snakes by fitness across all time.
   * Or maybe best of each generation? 
   * Let's implemented: Keep top N best ever.
   * @param {Object} snakeData { gen, seed, fitness, points, length, genomeJSON }
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
   * Returns copy of entries.
   */
  getAll(): HallOfFameEntry[] {
    return [...this.entries];
  }

  /**
   * Replaces entries with a new set and persists them.
   * @param {Array} entries
   */
  replace(entries: HallOfFameEntry[]): void {
    if (!Array.isArray(entries)) return;
    this.entries = entries.filter(e => e && typeof e.fitness === 'number');
    this.entries.sort((a, b) => b.fitness - a.fitness);
    if (this.entries.length > MAX_HOF_ENTRIES) this.entries.length = MAX_HOF_ENTRIES;
    this.save();
  }

  /**
   * Clears all history.
   */
  reset(): void {
    this.entries = [];
    this.save();
  }
}

export const hof = new HallOfFame();
