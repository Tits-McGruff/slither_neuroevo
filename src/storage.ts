// storage.ts
// Handles persistence of the simulation state (population, generation) to
// localStorage and file export/import.

import { Genome } from './mlp.ts';
import type { GenomeJSON, HallOfFameEntry } from './protocol/messages.ts';

/** Local storage key for population persistence. */
const STORAGE_KEY = 'slither_neuroevo_pop';

/** Payload stored in localStorage for population persistence. */
export interface PopulationStoragePayload {
  generation: number;
  genomes: GenomeJSON[];
}

/** Payload stored in export files, optionally including HoF. */
export interface PopulationFilePayload extends PopulationStoragePayload {
  hof?: HallOfFameEntry[];
}

/** Genome type alias for storage helpers. */
type GenomeLike = Genome;

/**
 * Generic Storage wrapper for localStorage.
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
     * Removes an item from localStorage.
     * @param key - Storage key to remove.
     */
    remove(key: string): void {
        localStorage.removeItem(key);
    }
};

/**
 * Saves the current population and generation to localStorage.
 * @param generation - Current generation number.
 * @param population - Genome instances to persist.
 */
export function savePopulation(generation: number, population: GenomeLike[]): void {
  try {
    const data: PopulationStoragePayload = {
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
 * @param arch - Neural network architecture definition (unused today).
 * @returns Population payload or null when unavailable.
 */
export function loadPopulation(
  arch: unknown
): { generation: number; genomes: GenomeLike[] } | null {
  try {
    void arch;
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const data = JSON.parse(raw) as PopulationStoragePayload;
    if (!data.genomes || !Array.isArray(data.genomes)) return null;

    const genomes = data.genomes.map((gData: GenomeJSON) => Genome.fromJSON(gData));
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
