import { CFG, resetCFGToDefaults } from '../config.ts';
import { getSensorLayout } from '../protocol/sensors.ts';
import { buildSensors } from '../sensors.ts';

/** Serialized sensor configuration shared with the Rust fixture test. */
export interface Stage4SensorReferenceConfig {
  bins: number;
  worldRadius: number;
  foodValue: number;
  snakeBoostSpeed: number;
  snakeStartLength: number;
  snakeMaxLength: number;
  minimumBoostPoints: number;
  boostPointsCostPerSecond: number;
  boostPointsCostSizeFactor: number;
  generationSeconds: number;
  nearRadiusBase: number;
  nearRadiusScale: number;
  nearRadiusMinimum: number;
  nearRadiusMaximum: number;
  farRadiusBase: number;
  farRadiusScale: number;
  farRadiusMinimum: number;
  farRadiusMaximum: number;
  foodSaturation: number;
  collisionHitScale: number;
  maximumPelletChecks: number;
  maximumSegmentChecks: number;
}

/** Serialized body point in one shared parity case. */
export interface Stage4SensorReferencePoint {
  x: number;
  y: number;
}

/** Serialized snake fields consumed by both sensor implementations. */
export interface Stage4SensorReferenceSnake {
  id: number;
  kind: 'evolved' | 'baseline' | 'external' | 'resurrected';
  alive: boolean;
  position: Stage4SensorReferencePoint;
  direction: number;
  radius: number;
  speed: number;
  boost: boolean;
  ageSeconds: number;
  points: number;
  deliveredObservationPoints: number;
  body: Stage4SensorReferencePoint[];
}

/** Serialized pellet fields consumed by both sensor implementations. */
export interface Stage4SensorReferencePellet {
  id: number;
  position: Stage4SensorReferencePoint;
  value: number;
  kind: number;
  color: number;
  owner: number | null;
}

/** One source-shaped input and its current corrected TypeScript output. */
export interface Stage4SensorReferenceCase {
  name: string;
  config: Stage4SensorReferenceConfig;
  bestPointsThisGeneration: number;
  targetId: number;
  snakes: Stage4SensorReferenceSnake[];
  pellets: Stage4SensorReferencePellet[];
  expected: number[];
}

/** Versioned cross-language fixture document. */
export interface Stage4SensorReferenceDocument {
  fixtureVersion: 1;
  sensorLayoutVersion: 'v3';
  sourceFunctions: string[];
  cases: Stage4SensorReferenceCase[];
}

/** Runtime snake shape accepted structurally by `buildSensors`. */
interface RuntimeSnake {
  id: number;
  x: number;
  y: number;
  dir: number;
  speed: number;
  boost: number;
  pointsScore: number;
  pointsAtLastSensorSample: number;
  age: number;
  points: Stage4SensorReferencePoint[];
  radius: number;
  alive: boolean;
  length: () => number;
  sizeNorm: () => number;
}

/** Complete test-only cell map correcting the production midpoint defect. */
class CompleteReferenceBodyGrid {
  /** Cell width used by the production query adapter. */
  readonly cellSize: number;
  /** Stable insertion-ordered cell contents. */
  private readonly cells = new Map<string, Array<{ snake: RuntimeSnake; segment: number }>>();

  /**
   * Build complete AABB cell coverage for every live segment.
   * @param snakes - Stable world-order snakes.
   * @param cellSize - Positive grid cell size.
   */
  constructor(snakes: RuntimeSnake[], cellSize: number) {
    this.cellSize = cellSize;
    for (const snake of snakes) {
      if (!snake.alive) continue;
      for (let segment = 1; segment < snake.points.length; segment++) {
        const start = snake.points[segment - 1];
        const end = snake.points[segment];
        if (!start || !end) continue;
        const minimumX = Math.floor(Math.min(start.x, end.x) / cellSize);
        const maximumX = Math.floor(Math.max(start.x, end.x) / cellSize);
        const minimumY = Math.floor(Math.min(start.y, end.y) / cellSize);
        const maximumY = Math.floor(Math.max(start.y, end.y) / cellSize);
        for (let y = minimumY; y <= maximumY; y++) {
          for (let x = minimumX; x <= maximumX; x++) {
            const key = `${x},${y}`;
            const entries = this.cells.get(key) ?? [];
            entries.push({ snake, segment });
            this.cells.set(key, entries);
          }
        }
      }
    }
  }

  /**
   * Visit every segment stored in one raw cell coordinate.
   * @param cellX - Raw cell X relative to the world origin.
   * @param cellY - Raw cell Y relative to the world origin.
   * @param callback - Production-compatible visitor.
   */
  queryCell(
    cellX: number,
    cellY: number,
    callback: (snake: RuntimeSnake, segment: number) => void
  ): void {
    for (const entry of this.cells.get(`${cellX},${cellY}`) ?? []) {
      callback(entry.snake, entry.segment);
    }
  }
}

/** Construct a compact head-to-tail straight body. */
function lineBody(
  x: number,
  y: number,
  direction: number,
  length: number,
  spacing = 7.5
): Stage4SensorReferencePoint[] {
  return Array.from({ length }, (_, index) => ({
    x: x - Math.cos(direction) * spacing * index,
    y: y - Math.sin(direction) * spacing * index
  }));
}

/** Return the exact default formula configuration in a serializable shape. */
function defaultReferenceConfig(bins: number): Stage4SensorReferenceConfig {
  return {
    bins,
    worldRadius: 3500,
    foodValue: 1,
    snakeBoostSpeed: 500,
    snakeStartLength: 5,
    snakeMaxLength: 10000,
    minimumBoostPoints: 1.2,
    boostPointsCostPerSecond: 7,
    boostPointsCostSizeFactor: 1.1,
    generationSeconds: 240,
    nearRadiusBase: 520,
    nearRadiusScale: 260,
    nearRadiusMinimum: 420,
    nearRadiusMaximum: 1100,
    farRadiusBase: 1200,
    farRadiusScale: 520,
    farRadiusMinimum: 900,
    farRadiusMaximum: 2400,
    foodSaturation: 4,
    collisionHitScale: 0.82,
    maximumPelletChecks: 900,
    maximumSegmentChecks: 2200
  };
}

/** Build the named source-shaped parity inputs without expected outputs. */
function referenceInputs(): Array<Omit<Stage4SensorReferenceCase, 'expected'>> {
  const preserveConfig = defaultReferenceConfig(16);
  const preserveTargetBody = lineBody(250, -125, 0.7, 7);
  const preserveOtherBody = [
    { x: 410, y: -160 },
    { x: 410, y: -110 },
    { x: 410, y: -60 }
  ];

  const longSegmentConfig = {
    ...defaultReferenceConfig(8),
    worldRadius: 1000,
    nearRadiusBase: 200,
    nearRadiusScale: 0,
    nearRadiusMinimum: 200,
    nearRadiusMaximum: 200,
    farRadiusBase: 400,
    farRadiusScale: 0,
    farRadiusMinimum: 400,
    farRadiusMaximum: 600,
    foodSaturation: 1,
    collisionHitScale: 1
  };

  const wallConfig = defaultReferenceConfig(32);
  return [
    {
      name: 'preserve-default-16',
      config: preserveConfig,
      bestPointsThisGeneration: 20,
      targetId: 1,
      snakes: [
        {
          id: 1,
          kind: 'evolved',
          alive: true,
          position: { x: 250, y: -125 },
          direction: 0.7,
          radius: 11,
          speed: 220,
          boost: true,
          ageSeconds: 60,
          points: 12.5,
          deliveredObservationPoints: 10,
          body: preserveTargetBody
        },
        {
          id: 2,
          kind: 'evolved',
          alive: true,
          position: preserveOtherBody[0]!,
          direction: -0.4,
          radius: 13,
          speed: 180,
          boost: false,
          ageSeconds: 30,
          points: 8,
          deliveredObservationPoints: 8,
          body: preserveOtherBody
        }
      ],
      pellets: [
        { id: 100, position: { x: 320, y: -100 }, value: 1, kind: 0, color: 1, owner: null },
        { id: 101, position: { x: 100, y: -200 }, value: 2.5, kind: 1, color: 2, owner: null },
        { id: 102, position: { x: 800, y: 400 }, value: 0.5, kind: 0, color: 3, owner: 2 }
      ]
    },
    {
      name: 'corrected-long-segment-8',
      config: longSegmentConfig,
      bestPointsThisGeneration: 5,
      targetId: 10,
      snakes: [
        {
          id: 10,
          kind: 'evolved',
          alive: true,
          position: { x: 0, y: 0 },
          direction: 0,
          radius: 10,
          speed: 165,
          boost: false,
          ageSeconds: 12,
          points: 5,
          deliveredObservationPoints: 3,
          body: lineBody(0, 0, 0, 5)
        },
        {
          id: 11,
          kind: 'evolved',
          alive: true,
          position: { x: 180, y: -20 },
          direction: 0,
          radius: 10,
          speed: 165,
          boost: false,
          ageSeconds: 12,
          points: 2,
          deliveredObservationPoints: 2,
          body: [
            { x: 180, y: -20 },
            { x: 1200, y: 20 }
          ]
        }
      ],
      pellets: [
        { id: 200, position: { x: -100, y: 0 }, value: 1.5, kind: 0, color: 0, owner: null }
      ]
    },
    {
      name: 'wall-head-size-32',
      config: wallConfig,
      bestPointsThisGeneration: 30,
      targetId: 20,
      snakes: [
        {
          id: 20,
          kind: 'evolved',
          alive: true,
          position: { x: 3300, y: 0 },
          direction: Math.PI / 2,
          radius: 16,
          speed: 500,
          boost: false,
          ageSeconds: 240,
          points: 30,
          deliveredObservationPoints: 32,
          body: lineBody(3300, 0, Math.PI / 2, 20)
        },
        {
          id: 21,
          kind: 'external',
          alive: true,
          position: { x: 3250, y: 100 },
          direction: Math.PI,
          radius: 18,
          speed: 300,
          boost: true,
          ageSeconds: 20,
          points: 4,
          deliveredObservationPoints: 4,
          body: lineBody(3250, 100, Math.PI, 8)
        },
        {
          id: 22,
          kind: 'baseline',
          alive: true,
          position: { x: 3000, y: -100 },
          direction: 0.25,
          radius: 12,
          speed: 165,
          boost: false,
          ageSeconds: 80,
          points: 50,
          deliveredObservationPoints: 49,
          body: lineBody(3000, -100, 0.25, 10)
        }
      ],
      pellets: [
        { id: 300, position: { x: 3290, y: 120 }, value: 3, kind: 1, color: 5, owner: null },
        { id: 301, position: { x: 3150, y: -80 }, value: 0.25, kind: 0, color: 2, owner: null }
      ]
    }
  ];
}

/** Apply one fixture's formula settings to the temporary TypeScript oracle. */
function applyReferenceConfig(config: Stage4SensorReferenceConfig): void {
  CFG.worldRadius = config.worldRadius;
  CFG.foodValue = config.foodValue;
  CFG.snakeBoostSpeed = config.snakeBoostSpeed;
  CFG.snakeStartLen = config.snakeStartLength;
  CFG.snakeMaxLen = config.snakeMaxLength;
  CFG.boost.minPointsToBoost = config.minimumBoostPoints;
  CFG.boost.pointsCostPerSecond = config.boostPointsCostPerSecond;
  CFG.boost.pointsCostSizeFactor = config.boostPointsCostSizeFactor;
  CFG.generationSeconds = config.generationSeconds;
  CFG.sense.layoutVersion = 'v3';
  CFG.sense.bubbleBins = config.bins;
  CFG.sense.rNearBase = config.nearRadiusBase;
  CFG.sense.rNearScale = config.nearRadiusScale;
  CFG.sense.rNearMin = config.nearRadiusMinimum;
  CFG.sense.rNearMax = config.nearRadiusMaximum;
  CFG.sense.rFarBase = config.farRadiusBase;
  CFG.sense.rFarScale = config.farRadiusScale;
  CFG.sense.rFarMin = config.farRadiusMinimum;
  CFG.sense.rFarMax = config.farRadiusMaximum;
  CFG.sense.foodKBase = config.foodSaturation;
  CFG.sense.maxPelletChecks = config.maximumPelletChecks;
  CFG.sense.maxSegmentChecks = config.maximumSegmentChecks;
  CFG.collision.hitScale = config.collisionHitScale;
  CFG.brain.inSize = getSensorLayout(config.bins, 'v3').inputSize;
}

/** Convert one serialized snake into the structural TypeScript oracle shape. */
function runtimeSnake(source: Stage4SensorReferenceSnake, config: Stage4SensorReferenceConfig): RuntimeSnake {
  const runtime: RuntimeSnake = {
    id: source.id,
    x: source.position.x,
    y: source.position.y,
    dir: source.direction,
    speed: source.speed,
    boost: source.boost ? 1 : 0,
    pointsScore: source.points,
    pointsAtLastSensorSample: source.deliveredObservationPoints,
    age: source.ageSeconds,
    points: source.body.map(point => ({ ...point })),
    radius: source.radius,
    alive: source.alive,
    length: () => runtime.points.length,
    sizeNorm: () => {
      const denominator = Math.max(1, config.snakeMaxLength - config.snakeStartLength);
      return Math.max(0, Math.min(1, (runtime.points.length - config.snakeStartLength) / denominator));
    }
  };
  return runtime;
}

/** Evaluate one shared case through the corrected TypeScript formula oracle. */
function evaluateReferenceCase(
  source: Omit<Stage4SensorReferenceCase, 'expected'>
): Stage4SensorReferenceCase {
  resetCFGToDefaults();
  applyReferenceConfig(source.config);
  const snakes = source.snakes.map(snake => runtimeSnake(snake, source.config));
  const pellets = source.pellets.map(pellet => ({
    x: pellet.position.x,
    y: pellet.position.y,
    v: pellet.value
  }));
  const pelletCellSize = 120;
  const pelletMap = new Map<string, typeof pellets>();
  for (const pellet of pellets) {
    const key = `${Math.floor(pellet.x / pelletCellSize)},${Math.floor(pellet.y / pelletCellSize)}`;
    const values = pelletMap.get(key) ?? [];
    values.push(pellet);
    pelletMap.set(key, values);
  }
  const target = snakes.find(snake => snake.id === source.targetId);
  if (!target) throw new Error(`missing target ${source.targetId} in ${source.name}`);
  const world = {
    pellets,
    bestPointsThisGen: source.bestPointsThisGeneration,
    snakes,
    pelletGrid: { cellSize: pelletCellSize, map: pelletMap },
    _collGrid: new CompleteReferenceBodyGrid(snakes, 70)
  };
  return {
    ...source,
    expected: Array.from(buildSensors(world, target))
  };
}

/** Recompute the complete versioned cross-language reference document. */
export function buildStage4SensorReferenceDocument(): Stage4SensorReferenceDocument {
  try {
    return {
      fixtureVersion: 1,
      sensorLayoutVersion: 'v3',
      sourceFunctions: [
        'src/protocol/sensors.ts::getSensorLayout',
        'src/sensors.ts::buildSensors',
        'src/sensors.ts::computeSensorRadii',
        'src/sensors.ts::_fillFoodBinsV2',
        'src/sensors.ts::_fillHazardBinsV2',
        'src/sensors.ts::_fillWallBinsV2',
        'src/sensors.ts::_fillHeadBinsV2',
        'test-only complete AABB body grid correcting SPATIAL-002'
      ],
      cases: referenceInputs().map(evaluateReferenceCase)
    };
  } finally {
    resetCFGToDefaults();
  }
}
