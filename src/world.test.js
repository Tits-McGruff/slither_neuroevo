import { describe, it, expect, beforeEach } from 'vitest';
import { World } from './world.js';
import { Pellet } from './snake.js';
import { CFG } from './config.js';

describe('world.js', () => {
    let settings;

    beforeEach(() => {
        settings = {
            snakeCount: 2,
            hiddenLayers: 1,
            neurons1: 4,
            simSpeed: 1
        };
    });

    it('World should initialize correctly', () => {
        const world = new World(settings);
        expect(world.snakes.length).toBe(2);
        expect(world.generation).toBe(1);
        expect(world.pellets.length).toBeGreaterThan(0);
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
        const initialAge = world.snakes[0].age;
        
        world.update(0.1, 800, 600);
        
        expect(world.snakes[0].age).toBeGreaterThan(initialAge);
    });
});
