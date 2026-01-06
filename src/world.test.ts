import { describe, it, expect, beforeEach } from 'vitest';
import { World } from './world.ts';
import { Pellet } from './snake.ts';
import { CFG, resetCFGToDefaults } from './config.ts';

/** Test suite label for world behaviors. */
const SUITE = 'world.ts';

describe(SUITE, () => {
    /** Baseline settings used to construct test worlds. */
    let settings: {
      snakeCount: number;
      hiddenLayers: number;
      neurons1: number;
      neurons2: number;
      neurons3: number;
      neurons4: number;
      neurons5: number;
      simSpeed: number;
    };

    beforeEach(() => {
        settings = {
            snakeCount: 2,
            hiddenLayers: 1,
            neurons1: 4,
            neurons2: 4,
            neurons3: 4,
            neurons4: 4,
            neurons5: 4,
            simSpeed: 1
        };
    });

    it('World should initialize correctly', () => {
        const world = new World(settings);
        expect(world.snakes.length).toBe(2);
        expect(world.generation).toBe(1);
        expect(world.pellets.length).toBeGreaterThan(0);
    });

    it('World defaults to overview view mode when observer settings are omitted', () => {
        resetCFGToDefaults();
        const originalTarget = CFG.pelletCountTarget;
        CFG.pelletCountTarget = 120;
        try {
            const world = new World({ snakeCount: 4 });
            expect(world.viewMode).toBe('overview');
            expect(Number.isFinite(world.bestPointsThisGen)).toBe(true);
        } finally {
            CFG.pelletCountTarget = originalTarget;
            resetCFGToDefaults();
        }
    });

    it('PelletGrid should add and remove pellets', () => {
        const world = new World(settings);
        const p = new Pellet(10, 10, 1);
        world.addPellet(p);
        
        let found = false;
        world.pelletGrid.forEachInRadius(10, 10, 5, (p2) => {
            if (p2 === p) found = true;
        });
        expect(found).toBe(true);
        
        world.removePellet(p);
        found = false;
        world.pelletGrid.forEachInRadius(10, 10, 5, (p2) => {
            if (p2 === p) found = true;
        });
        expect(found).toBe(false);
    });

    it('World update should advance physics', () => {
        const world = new World(settings);
        const snake = world.snakes[0]!;
        const initialAge = snake.age;
        
        world.update(0.1, 800, 600);
        
        expect(snake.age).toBeGreaterThan(initialAge);
    });

    it('imports a compatible population and resets generation state', () => {
        const world = new World(settings);
        const exportData = world.exportPopulation();
        exportData.generation = 5;
        const result = world.importPopulation(exportData);
        expect(result.ok).toBe(true);
        expect(world.generation).toBe(5);
        expect(world.snakes.length).toBe(settings.snakeCount);
        expect(world.fitnessHistory.length).toBe(0);
    });

    it('keeps initial sensors and points finite after the first tick', () => {
        resetCFGToDefaults();
        const originalTarget = CFG.pelletCountTarget;
        const originalGenSeconds = CFG.generationSeconds;
        CFG.pelletCountTarget = 200; // lower for test speed
        CFG.generationSeconds = 5;
        try {
            const world = new World({ ...settings, snakeCount: 6 });
            world.update(1 / 30, 800, 600);
            expect(Number.isFinite(world.bestPointsThisGen)).toBe(true);
            for (const s of world.snakes) {
                if (!s.alive) continue;
                expect(Number.isFinite(s.x)).toBe(true);
                expect(Number.isFinite(s.y)).toBe(true);
                expect(Number.isFinite(s.dir)).toBe(true);
                if (s.lastSensors) {
                    const allFinite = s.lastSensors.every(Number.isFinite);
                    expect(allFinite).toBe(true);
                }
            }
        } finally {
            CFG.pelletCountTarget = originalTarget;
            CFG.generationSeconds = originalGenSeconds;
            resetCFGToDefaults();
        }
    });

    it('records min/avg/best fitness into history at generation end', () => {
        resetCFGToDefaults();
        const originalTarget = CFG.pelletCountTarget;
        CFG.pelletCountTarget = 100;
        try {
            const world = new World({ ...settings, snakeCount: 3 });
            world._endGeneration();
            expect(world.fitnessHistory.length).toBeGreaterThan(0);
            const entry = world.fitnessHistory[0]!;
            expect(entry).toHaveProperty('gen');
            expect(entry).toHaveProperty('best');
            expect(entry).toHaveProperty('avg');
            expect(entry).toHaveProperty('min');
            expect(entry).toHaveProperty('speciesCount');
            expect(entry).toHaveProperty('topSpeciesSize');
            expect(entry).toHaveProperty('avgWeight');
            expect(entry).toHaveProperty('weightVariance');
            expect(Number.isFinite(entry.best)).toBe(true);
            expect(Number.isFinite(entry.avg)).toBe(true);
            expect(Number.isFinite(entry.min)).toBe(true);
            expect(Number.isFinite(entry.avgWeight)).toBe(true);
            expect(Number.isFinite(entry.weightVariance)).toBe(true);
        } finally {
            CFG.pelletCountTarget = originalTarget;
            resetCFGToDefaults();
        }
    });
});
