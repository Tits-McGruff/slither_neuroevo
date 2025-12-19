import { describe, it, expect, beforeEach } from 'vitest';
import { Snake, Pellet, SegmentGrid, pointSegmentDist2 } from './snake.ts';
import { Genome, buildArch } from './mlp.ts';
import { CFG } from './config.js';

describe('snake.js', () => {
    let arch;
    let genome;

    beforeEach(() => {
        const settings = {
            hiddenLayers: 1,
            neurons1: 4,
            snakeCount: 1
        };
        arch = buildArch(settings);
        genome = Genome.random(arch);
    });

    it('Snake should initialize with correct properties', () => {
        const snake = new Snake(1, genome, arch);
        expect(snake.id).toBe(1);
        expect(snake.alive).toBe(true);
        expect(snake.points.length).toBe(CFG.snakeStartLen);
        expect(snake.radius).toBe(CFG.snakeRadius);
    });

    it('Snake radius should update correctly based on length', () => {
        const snake = new Snake(1, genome, arch);
        const initialRadius = snake.radius;
        
        // Simulate growth
        snake.targetLen = CFG.snakeStartLen + 100;
        // Manual grow
        while(snake.points.length < Math.floor(snake.targetLen)) {
            snake.points.push({x: 0, y: 0});
        }
        snake.updateRadiusFromLen();
        
        expect(snake.radius).toBeGreaterThan(initialRadius);
        expect(snake.radius).toBeLessThanOrEqual(CFG.snakeRadiusMax);
    });

    it('pointSegmentDist2 should calculate squared distance to segment', () => {
        // Horizontal segment (0,0) to (10,0)
        const d2 = pointSegmentDist2(5, 5, 0, 0, 10, 0);
        expect(d2).toBe(25); // (5,0) is closest, distance 5, 5*5=25

        // Beyond end
        const d2_beyond = pointSegmentDist2(15, 0, 0, 0, 10, 0);
        expect(d2_beyond).toBe(25); // (10,0) is closest, distance 5, 5*5=25
    });

    it('SegmentGrid should store and query segments', () => {
        const grid = new SegmentGrid();
        const snake = new Snake(1, genome, arch);
        // Ensure at least 2 points for a segment
        snake.points = [{x: 0, y: 0}, {x: 5, y: 5}];
        
        grid.addSegment(snake, 1);
        const cx = Math.floor(2.5 / grid.cellSize);
        const cy = Math.floor(2.5 / grid.cellSize);
        const results = grid.query(cx, cy);
        
        expect(results).not.toBeNull();
        expect(results.length).toBe(1);
        expect(results[0].s).toBe(snake);
        expect(results[0].i).toBe(1);
    });
});
