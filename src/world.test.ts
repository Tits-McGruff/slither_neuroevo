import { describe, it, expect, beforeEach } from 'vitest';
import { World } from './world.ts';
import { Pellet } from './snake.ts';
import { CFG, resetCFGToDefaults } from './config.ts';
import { deriveBotSeed } from './bots/baselineBots.ts';

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

    /**
     * Advance the world with small steps until the first baseline bot is alive.
     * @param world - World to advance.
     * @param dt - Fixed delta time per update.
     * @param maxTime - Maximum time to wait before giving up.
     * @returns Accumulated time spent stepping.
     */
    function waitForBaselineRespawn(world: World, dt = 0.045, maxTime = 2): number {
        let elapsed = 0;
        const maxStep = Math.max(0.004, CFG.dtClamp);
        while (elapsed < maxTime) {
            const bot = world.baselineBots[0];
            if (bot && bot.alive) break;
            world.update(dt, 800, 600);
            const scaled = Math.min(Math.max(dt * world.simSpeed, 0), maxStep);
            elapsed += scaled;
        }
        return elapsed;
    }

    it('World should initialize correctly', () => {
        const world = new World(settings);
        expect(world.population.length).toBe(settings.snakeCount);
        expect(world.snakes.length).toBe(settings.snakeCount + world.baselineBots.length);
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
        // Manually set some fitness values for determinism in this test
        if (exportData.genomes[0]) exportData.genomes[0].fitness = 30;
        if (exportData.genomes[1]) exportData.genomes[1].fitness = 20;
        const result = world.importPopulation(exportData);
        expect(result.ok).toBe(true);
        expect(world.generation).toBe(5);
        expect(world.snakes.length).toBe(settings.snakeCount + world.baselineBots.length);
        expect(world.fitnessHistory.length).toBe(0);
        // Verify that the imported fitness values are reset to 0 in the new world
        for (const g of world.population) {
            expect(g.fitness).toBe(0);
        }
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

    it('keeps sensors finite after reset with v2 layout', () => {
        resetCFGToDefaults();
        const originalTarget = CFG.pelletCountTarget;
        CFG.pelletCountTarget = 150;
        try {
            expect(CFG.sense.layoutVersion).toBe('v2');
            const world = new World({ ...settings, snakeCount: 4 });
            world.update(1 / 30, 800, 600);
            for (const s of world.snakes) {
                if (!s.alive) continue;
                if (!s.lastSensors) continue;
                const allFinite = s.lastSensors.every(Number.isFinite);
                expect(allFinite).toBe(true);
            }
        } finally {
            CFG.pelletCountTarget = originalTarget;
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

    it('appends baseline bots after population', () => {
        resetCFGToDefaults();
        CFG.baselineBots.count = 2;
        CFG.baselineBots.seed = 5;
        CFG.baselineBots.randomizeSeedPerGen = false;
        try {
            const world = new World({ ...settings, snakeCount: 3 });
            expect(world.population.length).toBe(3);
            expect(world.baselineBots.length).toBe(2);
            expect(world.snakes.length).toBe(5);
            expect(world.snakes[0]?.baselineBotIndex ?? null).toBeNull();
            expect(world.snakes[1]?.baselineBotIndex ?? null).toBeNull();
            expect(world.snakes[2]?.baselineBotIndex ?? null).toBeNull();
            expect(world.snakes[3]?.baselineBotIndex).toBe(0);
            expect(world.snakes[4]?.baselineBotIndex).toBe(1);
        } finally {
            resetCFGToDefaults();
        }
    });

    it('excludes baseline bots from bestPointsThisGen', () => {
        resetCFGToDefaults();
        CFG.baselineBots.count = 1;
        const originalTarget = CFG.pelletCountTarget;
        CFG.pelletCountTarget = 0;
        try {
            const world = new World({ ...settings, snakeCount: 1 });
            const popSnake = world.snakes[0]!;
            const botSnake = world.baselineBots[0]!;
            popSnake.pointsScore = 5;
            botSnake.pointsScore = 50;
            // Move them far apart to avoid accidental collisions or kills
            popSnake.x = -100; popSnake.y = -100;
            botSnake.x = 100; botSnake.y = 100;
            world.update(0, 800, 600);
            expect(world.bestPointsThisGen).toBe(5);
            expect(world.bestPointsSnakeId).toBe(popSnake.id);
        } finally {
            CFG.pelletCountTarget = originalTarget;
            resetCFGToDefaults();
        }
    });

    it('excludes baseline bots from fitness and hof', () => {
        resetCFGToDefaults();
        CFG.baselineBots.count = 1;
        try {
            const world = new World({ ...settings, snakeCount: 2 });
            const baselineId = world.baselineBots[0]!.id;
            world.baselineBots[0]!.pointsScore = 500;
            world.snakes[0]!.pointsScore = 1;
            world.snakes[1]!.pointsScore = 2;
            world._endGeneration();
            const hofEntry = world._lastHoFEntry;
            expect(hofEntry).toBeTruthy();
            expect(hofEntry?.seed).not.toBe(baselineId);
        } finally {
            resetCFGToDefaults();
        }
    });

    it('baselineBotIndex stays stable across respawn', () => {
        resetCFGToDefaults();
        CFG.baselineBots.count = 1;
        CFG.baselineBots.respawnDelay = 0.5;
        try {
            const world = new World({ ...settings, snakeCount: 1 });
            const bot = world.baselineBots[0]!;
            const initialId = bot.id;
            const initialIndex = bot.baselineBotIndex;
            bot.die(world);
            const elapsed = waitForBaselineRespawn(world);
            const respawned = world.baselineBots[0]!;
            expect(respawned.alive).toBe(true);
            expect(respawned.baselineBotIndex).toBe(initialIndex);
            expect(respawned.id).not.toBe(initialId);
            expect(respawned.baselineBotIndex).not.toBe(respawned.id);
            expect(elapsed).toBeGreaterThan(0);
        } finally {
            resetCFGToDefaults();
        }
    });

    it('deriveBotSeed includes generation only when enabled', () => {
        const seedA = deriveBotSeed(5, 1, 2, false);
        const seedB = deriveBotSeed(5, 2, 2, false);
        const seedC = deriveBotSeed(5, 1, 2, true);
        const seedD = deriveBotSeed(5, 2, 2, true);
        expect(seedA).toBe(seedB);
        expect(seedC).not.toBe(seedD);
    });

    it('respawns baseline bots within the delay', () => {
        resetCFGToDefaults();
        CFG.baselineBots.count = 1;
        CFG.baselineBots.respawnDelay = 0.5;
        try {
            const world = new World({ ...settings, snakeCount: 1 });
            const bot = world.baselineBots[0]!;
            bot.die(world);
            const elapsed = waitForBaselineRespawn(world);
            const respawned = world.baselineBots[0]!;
            expect(respawned.alive).toBe(true);
            expect(elapsed).toBeGreaterThanOrEqual(0.45);
            expect(elapsed).toBeLessThan(1.0);
        } finally {
            resetCFGToDefaults();
        }
    });
});
