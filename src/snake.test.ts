import { describe, it, expect, beforeEach } from 'vitest';
import { Snake, SegmentGrid, pointSegmentDist2 } from './snake.ts';
import { Genome, buildArch } from './mlp.ts';
import { CFG } from './config.ts';

describe('snake.ts', () => {
    let arch: ReturnType<typeof buildArch>;
    let genome: Genome;

    beforeEach(() => {
        const settings = {
          hiddenLayers: 1,
          neurons1: 4,
          neurons2: 4,
          neurons3: 4,
          neurons4: 4,
          neurons5: 4,
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
        const results = grid.query(cx, cy) ?? [];
        
        expect(results.length).toBe(1);
        const first = results[0];
        expect(first).toBeDefined();
        if (!first) return;
        expect(first.s).toBe(snake);
        expect(first.i).toBe(1);
    });

    it('uses external control without running the brain', () => {
        const snake = new Snake(1, genome, arch);
        let forwardCalls = 0;
        snake.brain.forward = () => {
            forwardCalls += 1;
            return new Float32Array([0.2, 0.8]);
        };
        const world: Parameters<Snake['update']>[0] = {
            pellets: [],
            particles: { spawnBurst: () => {}, spawnBoost: () => {} },
            addPellet: () => {},
            removePellet: () => {},
            bestPointsThisGen: 1
        };

        snake.update(world, 1 / 60, { turn: 1, boost: 1 });

        expect(forwardCalls).toBe(0);
        expect(snake.turnInput).toBe(1);
        expect(snake.boostInput).toBe(1);
    });

    it('resets the brain when control mode changes', () => {
        const snake = new Snake(1, genome, arch);
        let resetCalls = 0;
        snake.brain.reset = () => {
            resetCalls += 1;
        };
        const world: Parameters<Snake['update']>[0] = {
            pellets: [],
            particles: { spawnBurst: () => {}, spawnBoost: () => {} },
            addPellet: () => {},
            removePellet: () => {},
            bestPointsThisGen: 1
        };

        snake.update(world, 1 / 60, { turn: 0, boost: 0 });
        snake.update(world, 1 / 60);

        expect(resetCalls).toBe(2);
    });

    it('computes sensors into a provided buffer', () => {
        const snake = new Snake(1, genome, arch);
        const world: Parameters<Snake['computeSensors']>[0] = {
            pellets: [],
            pelletGrid: { map: new Map(), cellSize: 120 },
            particles: { spawnBurst: () => {}, spawnBoost: () => {} },
            addPellet: () => {},
            removePellet: () => {},
            bestPointsThisGen: 1
        };
        const out = new Float32Array(CFG.brain.inSize);
        const sensors = snake.computeSensors(world, out);

        expect(sensors).toBe(out);
        expect(sensors.length).toBe(CFG.brain.inSize);
    });
});
