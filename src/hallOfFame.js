// hallOfFame.js
// Manages the "Hall of Fame" - a registry of the best snakes from previous generations.

const HOF_STORAGE_KEY = 'slither_neuroevo_hof';
const MAX_HOF_ENTRIES = 50;

function getStorage() {
  if (typeof globalThis === 'undefined') return null;
  try {
    return globalThis.localStorage || null;
  } catch (e) {
    return null;
  }
}

export class HallOfFame {
  constructor() {
    this.entries = [];
    this.load();
  }

  /**
   * Loads entries from local storage.
   */
  load() {
    const storage = getStorage();
    if (!storage) return;
    try {
      const raw = storage.getItem(HOF_STORAGE_KEY);
      if (raw) {
        this.entries = JSON.parse(raw);
      }
    } catch (e) {
      console.error("Failed to load HoF", e);
      this.entries = [];
    }
  }

  /**
   * Saves entries to local storage.
   */
  save() {
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
  add(snakeData) {
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
  getAll() {
    return [...this.entries];
  }

  /**
   * Replaces entries with a new set and persists them.
   * @param {Array} entries
   */
  replace(entries) {
    if (!Array.isArray(entries)) return;
    this.entries = entries.filter(e => e && typeof e.fitness === 'number');
    this.entries.sort((a, b) => b.fitness - a.fitness);
    if (this.entries.length > MAX_HOF_ENTRIES) this.entries.length = MAX_HOF_ENTRIES;
    this.save();
  }

  /**
   * Clears all history.
   */
  reset() {
    this.entries = [];
    this.save();
  }
}

export const hof = new HallOfFame();
