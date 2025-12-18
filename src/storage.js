// storage.js
// Handles persistence of the simulation state (population, generation) to
// localStorage and file export/import.

import { Genome } from './mlp.js';

const STORAGE_KEY = 'slither_neuroevo_pop';

/**
 * Generic Storage wrapper for localStorage.
 */
export const Storage = {
    /**
     * Saves an item to localStorage.
     * @param {string} key
     * @param {any} value
     */
    save(key, value) {
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
     * Loads an item from localStorage.
     * @param {string} key
     * @returns {any|null}
     */
    load(key) {
        try {
            const json = localStorage.getItem(key);
            return json ? JSON.parse(json) : null;
        } catch (e) {
            console.error("Failed to load from localStorage:", e);
            return null;
        }
    },

    /**
     * Removes an item from localStorage.
     * @param {string} key
     */
    remove(key) {
        localStorage.removeItem(key);
    }
};

/**
 * Saves the current population and generation to localStorage.
 * @param {number} generation
 * @param {Array<Genome>} population
 */
export function savePopulation(generation, population) {
  try {
    const data = {
      generation: generation,
      genomes: population.map(g => g.toJSON())
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(data));
    console.log(`Saved generation ${generation} to local storage.`);
  } catch (e) {
    console.error("Failed to save to local storage", e);
  }
}

/**
 * Loads the population from localStorage.
 * @param {Object} arch Neural network architecture definition (for validation/reconstruction if needed)
 * @returns {{generation: number, genomes: Array<Genome>}|null}
 */
export function loadPopulation(arch) {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const data = JSON.parse(raw);
    if (!data.genomes || !Array.isArray(data.genomes)) return null;

    const genomes = data.genomes.map(gData => Genome.fromJSON(gData));
    return {
      generation: data.generation || 1,
      genomes: genomes
    };
  } catch (e) {
    console.error("Failed to load from local storage", e);
    return null;
  }
}

/**
 * Triggers a download of the given data object as a JSON file.
 * @param {Object} data 
 * @param {string} filename 
 */
export function exportToFile(data, filename) {
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

/**
 * Reads a JSON file and parses it.
 * @param {File} file 
 * @returns {Promise<Object>}
 */
export function importFromFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = (e) => {
      try {
        const data = JSON.parse(e.target.result);
        resolve(data);
      } catch (err) {
        reject(err);
      }
    };
    reader.onerror = reject;
    reader.readAsText(file);
  });
}
