/** Simulation world state, evolution loop, and rendering helpers. */

import { CFG } from './config.ts';
import { buildArch, archKey, Genome, crossover, mutate, enrichArchInfo } from './mlp.ts';
import { ParticleSystem } from './particles.ts';
import { Snake, Pellet, pointSegmentDist2 } from './snake.ts';
import type { ControlInput } from './snake.ts';
import { randInt, clamp, lerp, TAU } from './utils.ts';
import { hof } from './hallOfFame.ts';
import { FlatSpatialHash } from './spatialHash.ts';
import { BaselineBotManager } from './bots/baselineBots.ts';
import { NullBrain } from './brains/nullBrain.ts';
import type { ArchDefinition } from './mlp.ts';
import type { GenomeJSON, HallOfFameEntry, PopulationImportData, PopulationExport } from './protocol/messages.ts';
import type { RandomSource } from './rng.ts';
import { THEME } from './theme.ts';

/** Starting id reserved for externally controlled snakes. */
const EXTERNAL_SNAKE_ID_START = 100000;
/** Starting id reserved for baseline bot snakes. */
const BASELINE_BOT_ID_START = 200000;

/** Optional settings overrides accepted by the World constructor. */
interface WorldSettingsInput {
  snakeCount?: number;
  simSpeed?: number;
  hiddenLayers?: number;
  neurons1?: number;
  neurons2?: number;
  neurons3?: number;
  neurons4?: number;
  neurons5?: number;
  worldRadius?: number;
  observer?: Partial<typeof CFG.observer>;
  collision?: Partial<typeof CFG.collision>;
}

/** Normalized settings stored by the World instance. */
interface WorldSettings {
  snakeCount: number;
  simSpeed: number;
  hiddenLayers: number;
  neurons1: number;
  neurons2: number;
  neurons3: number;
  neurons4: number;
  neurons5: number;
  worldRadius: number;
  observer: typeof CFG.observer;
  collision: typeof CFG.collision;
}

/** Fitness history record stored by the world for charts. */
interface FitnessHistoryEntry {
  gen: number;
  best: number;
  avg: number;
  min: number;
  speciesCount: number;
  topSpeciesSize: number;
  avgWeight: number;
  weightVariance: number;
}

/** Minimal controller registry interface used by the World. */
export interface ControllerRegistryLike {
  isControlled: (snakeId: number) => boolean;
  getAction: (snakeId: number, tickId: number) => ControlInput | null;
  publishSensors: (
    snakeId: number,
    tickId: number,
    sensors: Float32Array,
    meta: { x: number; y: number; dir: number }
  ) => void;
}

/** Main simulation world containing population state, pellets, and snakes. */
export class World {
  /** Normalized settings for the world instance. */
  settings: WorldSettings;
  /** Neural network architecture definition for the population. */
  arch: ArchDefinition;
  /** Stable architecture key used for persistence. */
  archKey: string;
  /** Active pellet instances in the world. */
  pellets: Pellet[];
  /** Spatial grid for pellet lookup. */
  pelletGrid: PelletGrid;
  /** Pellet spawn accumulator in seconds. */
  _pelletSpawnAcc: number;
  /** Active snake instances in the world. */
  snakes: Snake[];
  /** Baseline bot snakes appended after the population. */
  baselineBots: Snake[];
  /** Manager for baseline bot state and actions. */
  botManager: BaselineBotManager;
  /** Particle system used by the legacy renderer. */
  particles: ParticleSystem;
  /** Current generation index. */
  generation: number;
  /** Elapsed time in the current generation. */
  generationTime: number;
  /** Current population of genomes. */
  population: Genome[];
  /** Best fitness recorded across all generations. */
  bestFitnessEver: number;
  /** Rolling fitness history for charts. */
  fitnessHistory: FitnessHistoryEntry[];
  /** Best points achieved in the current generation. */
  bestPointsThisGen: number;
  /** Snake id that currently holds best points. */
  bestPointsSnakeId: number;
  /** Last Hall of Fame entry emitted by the world. */
  _lastHoFEntry: HallOfFameEntry | null;
  /** Simulation speed multiplier. */
  simSpeed: number;
  /** Camera X coordinate for rendering. */
  cameraX: number;
  /** Camera Y coordinate for rendering. */
  cameraY: number;
  /** Camera zoom factor for rendering. */
  zoom: number;
  /** Snake currently focused by the observer. */
  focusSnake: Snake | null;
  /** Cooldown timer for focus switching. */
  _focusCooldown: number;
  /** Observer view mode. */
  viewMode: string;
  /** Collision grid for snake segments. */
  _collGrid: FlatSpatialHash<Snake>;
  /** Next id to assign to externally controlled snakes. */
  _nextExternalSnakeId: number;
  /** Next id to assign to baseline bot spawns. */
  _nextBaselineBotId: number;

  /** Access the world radius from current settings. */
  get worldRadius(): number {
    return this.settings.worldRadius;
  }

  /**
   * Create a new World instance with optional settings overrides.
   * @param settings - World settings overrides from UI or worker.
   */
  constructor(settings: WorldSettingsInput = {}) {
    // Store a shallow copy of the UI settings to decouple from external
    // mutations.  The settings include snakeCount, simSpeed and hidden layer
    // sizes.
    const observerSettings = { ...CFG.observer, ...(settings.observer ?? {}) };
    const collisionSettings = { ...CFG.collision, ...(settings.collision ?? {}) };
    this.settings = {
      ...settings,
      snakeCount: settings.snakeCount ?? 55,
      hiddenLayers: settings.hiddenLayers ?? 2,
      neurons1: settings.neurons1 ?? 64,
      neurons2: settings.neurons2 ?? 64,
      neurons3: settings.neurons3 ?? 64,
      neurons4: settings.neurons4 ?? 48,
      neurons5: settings.neurons5 ?? 32,
      simSpeed: settings.simSpeed ?? 1,
      worldRadius: settings.worldRadius ?? CFG.worldRadius,
      observer: observerSettings,
      collision: collisionSettings
    };
    // Construct the neural architecture based on current settings.
    this.arch = buildArch(this.settings);
    this.archKey = this.arch.key || archKey(this.arch);
    this.pellets = [];
    this.pelletGrid = new PelletGrid();
    this._pelletSpawnAcc = 0;
    this.snakes = [];
    this.baselineBots = [];
    this.botManager = new BaselineBotManager(CFG.baselineBots);
    this.particles = new ParticleSystem(); // Initialize particle system
    this.generation = 1;
    this.generationTime = 0;
    this.population = [];
    this.bestFitnessEver = 0;
    // Must start finite or sensor percentiles will produce NaNs on the first tick.
    this.fitnessHistory = []; // Track fitness over generations
    this.bestPointsThisGen = 0;
    this.bestPointsSnakeId = 0;
    this._lastHoFEntry = null;
    // Simulation speed multiplier.  Affects how dt is scaled per frame.
    this.simSpeed = this.settings.simSpeed;
    // Camera state for panning and zooming.
    this.cameraX = 0;
    this.cameraY = 0;
    this.zoom = 1.0;
    this.focusSnake = null;
    this._focusCooldown = 0;
    this.viewMode = this.settings.observer.defaultViewMode || "overview";
    this.zoom = 1.0;

    // Init physics
    // Estimate capacity: 50 snakes * 100 len = 5000 segments. 5000 * 500 len = 2.5m.
    // Let's allocate big. 200,000 capacity safe for now?
    // worldRadius * 2 = width.
    const w = this.settings.worldRadius * 2.5;
    this._collGrid = new FlatSpatialHash(w, w, this.settings.collision.cellSize, 200000);
    this._nextExternalSnakeId = EXTERNAL_SNAKE_ID_START;
    this._nextBaselineBotId = BASELINE_BOT_ID_START;
    this._initPopulation();
    this._resetBaselineBotsForGen();
    this._spawnAll();
    this._collGrid.build(this.snakes, CFG.collision.skipSegments);
    this._initPellets();
    this._chooseInitialFocus();
  }
  /**
   * Immediately adjusts the simulation speed.  Also stores the new
   * value back into the settings object.
   * @param x - New simulation speed multiplier.
   */
  applyLiveSimSpeed(x: number): void {
    this.simSpeed = clamp(x, 0.01, 500.0);
    this.settings.simSpeed = this.simSpeed;
  }
  /**
   * Toggles between overview and follow camera modes.  Ensures that a
   * valid focus snake is selected when switching to follow mode.
   */
  toggleViewMode(): void {
    this.viewMode = this.viewMode === "overview" ? "follow" : "overview";
    if (!this.focusSnake || !this.focusSnake.alive) this.focusSnake = this._pickAnyAlive();
    if (this.viewMode === "overview") {
      this.cameraX = 0;
      this.cameraY = 0;
    } else if (this.focusSnake && this.focusSnake.alive) {
      const h = this.focusSnake.head();
      this.cameraX = h.x;
      this.cameraY = h.y;
    }
  }

  /**
   * Notify the world when a baseline bot dies.
   * @param snake - Snake that died.
   */
  baselineBotDied(snake: Snake): void {
    const idx = snake.baselineBotIndex;
    if (idx == null) return;
    this.botManager.markDead(idx);
  }
  /**
   * Chooses an alive snake at random.  Returns null if none.
   */
  _pickAnyAlive(): Snake | null {
    const alive = this.snakes.filter(s => s.alive);
    if (!alive.length) return null;
    const idx = randInt(alive.length);
    return alive[idx] ?? null;
  }
  /**
   * Initialises the population with random genomes according to the
   * current architecture.
   */
  _initPopulation(): void {
    this.population.length = 0;
    for (let i = 0; i < this.settings.snakeCount; i++) {
      this.population.push(Genome.random(this.arch));
    }
  }

  /**
   * Serializes the current population for export.
   * @returns Population export payload.
   */
  exportPopulation(): PopulationExport {
    return {
      generation: this.generation,
      archKey: this.archKey,
      genomes: this.population.map(g => g.toJSON())
    };
  }

  /**
   * Replaces the current population from imported JSON data.
   * The caller is responsible for validating the data before calling.
   * @param data - Import payload containing genomes and optional generation.
   * @returns Import result summary.
   */
  importPopulation(data: PopulationImportData): { ok: boolean; reason?: string; used?: number; total?: number } {
    if (!data || !Array.isArray(data.genomes)) {
      return { ok: false, reason: 'missing genomes' };
    }
    const info = enrichArchInfo(this.arch);
    const expectedLen = info.totalCount;
    const expectedKey = this.archKey;
    const parsed = [];
    for (const raw of data.genomes) {
      try {
        const g = Genome.fromJSON(raw);
        if (g.archKey !== expectedKey) continue;
        if (!g.weights || g.weights.length !== expectedLen) continue;
        g.fitness = 0;
        parsed.push(g);
      } catch {
        // Skip malformed entries.
      }
    }
    if (!parsed.length) {
      return { ok: false, reason: 'no compatible genomes' };
    }
    const targetCount = Math.max(1, Math.floor(this.settings.snakeCount || parsed.length));
    const nextPop = [];
    for (let i = 0; i < targetCount; i++) {
      const candidate = parsed[i];
      if (candidate) nextPop.push(candidate.clone());
      else nextPop.push(Genome.random(this.arch));
    }
    this.population = nextPop;
    this.generation = Number.isFinite(data.generation)
      ? Math.max(1, Math.floor(data.generation!))
      : 1;
    this.generationTime = 0;
    this.bestPointsThisGen = 0;
    this.bestPointsSnakeId = 0;
    this.bestFitnessEver = 0;
    this.fitnessHistory = [];
    this.particles = new ParticleSystem();
    this._initPellets();
    this._resetBaselineBotsForGen();
    this._spawnAll();
    this._collGrid.build(this.snakes, CFG.collision.skipSegments);
    this._chooseInitialFocus();
    return { ok: true, used: parsed.length, total: targetCount };
  }
  /**
   * Spawns snakes from the current population genomes.
   */
  _spawnAll(): void {
    this.snakes.length = 0;
    for (let i = 0; i < this.population.length; i++) {
      const g = this.population[i];
      if (!g) continue;
      this.snakes.push(new Snake(i + 1, g.clone(), this.arch));
    }
    this._spawnBaselineBots();
  }

  /**
   * Reset baseline bot manager state for the current generation.
   */
  _resetBaselineBotsForGen(): void {
    this.botManager.resetForGeneration(this.generation);
    this._nextBaselineBotId = BASELINE_BOT_ID_START;
  }

  /**
   * Spawn baseline bots after the population snakes.
   */
  _spawnBaselineBots(): void {
    this.baselineBots.length = 0;
    const count = this.botManager.getCount();
    if (count <= 0) return;
    for (let i = 0; i < count; i++) {
      const rng = this.botManager.prepareBotSpawn(i);
      const snake = this._createBaselineSnake(i, rng);
      if (!snake) {
        console.warn('[baselineBots] bot.respawn.failed', {
          baselineBotIndex: i,
          reason: 'invalid id range'
        });
        continue;
      }
      this.baselineBots.push(snake);
      this.snakes.push(snake);
      this.botManager.registerBot(i, snake.id);
    }
  }

  /**
   * Build a baseline bot genome with zeroed weights.
   * @returns Baseline genome instance.
   */
  _createBaselineGenome(): Genome {
    const info = enrichArchInfo(this.arch);
    const weights = new Float32Array(info.totalCount);
    return new Genome(this.archKey, weights, this.arch.spec.type);
  }

  /**
   * Create a baseline bot snake instance.
   * @param index - Baseline bot index.
   * @param rng - RNG for spawn position and heading.
   * @returns Spawned snake or null when the id allocator is exhausted.
   */
  _createBaselineSnake(index: number, rng: RandomSource): Snake | null {
    const nextId = this._nextBaselineBotId;
    if (!Number.isSafeInteger(nextId) || nextId >= Number.MAX_SAFE_INTEGER) return null;
    this._nextBaselineBotId = nextId + 1;
    const snake = new Snake(nextId, this._createBaselineGenome(), this.arch, {
      rng,
      brain: new NullBrain(),
      controlMode: 'external-only',
      baselineBotIndex: index,
      skin: 2,
    });
    snake.color = THEME.snakeRobotBody;
    return snake;
  }

  /**
   * Respawn a baseline bot and reinsert it into the snake list.
   * @param index - Baseline bot index.
   * @param rng - RNG for spawn position and heading.
   * @returns Spawned snake or null when respawn fails.
   */
  _respawnBaselineBot(index: number, rng: RandomSource): Snake | null {
    const snake = this._createBaselineSnake(index, rng);
    if (!snake) return null;
    const slot = this.population.length + index;
    if (slot < 0 || slot > this.snakes.length) return null;
    if (slot === this.snakes.length) {
      this.snakes.push(snake);
    } else {
      this.snakes[slot] = snake;
    }
    this.baselineBots[index] = snake;
    return snake;
  }
  /**
   * Fills the world with pellets up to the configured target count.
   */
  _initPellets(): void {
    this.pellets.length = 0;
    this.pelletGrid.resetForCFG();
    this._pelletSpawnAcc = 0;
    while (this.pellets.length < CFG.pelletCountTarget) this.addPellet(this._spawnAmbientPellet());
  }

  /**
   * Adds a pellet to the world and to the pellet spatial hash.
   * @param p - Pellet to add.
   */
  addPellet(p: Pellet): void {
    p._idx = this.pellets.length;
    this.pellets.push(p);
    this.pelletGrid.add(p);
  }

  /**
   * Removes a pellet from the world and from the pellet spatial hash.
   * @param p - Pellet to remove.
   */
  removePellet(p: Pellet): void {
    if (!p) return;
    this.pelletGrid.remove(p);
    const idx = p._idx;
    if (idx == null || idx < 0 || idx >= this.pellets.length) return;
    const last = this.pellets.pop()!;
    if (last !== p) {
      this.pellets[idx] = last;
      last._idx = idx;
    }
    p._idx = -1;
  }

  /**
   * Advances the simulation by dt seconds (scaled by simSpeed) and
   * updates camera and focus logic.  Handles early generation termination.
   * @param dt - Base delta time (unscaled).
   * @param viewW - Canvas width in CSS pixels.
   * @param viewH - Canvas height in CSS pixels.
   * @param controllers - Optional external controller registry.
   * @param tickId - Optional tick id for controller sync.
   */
  update(
    dt: number,
    viewW: number,
    viewH: number,
    controllers?: ControllerRegistryLike,
    tickId?: number
  ): void {
    const rawScaled = dt * this.simSpeed;
    const scaled = clamp(rawScaled, 0, Math.max(0.004, CFG.dtClamp));
    const maxStep = clamp(CFG.collision.substepMaxDt, 0.004, 0.08);
    const steps = clamp(Math.ceil(scaled / maxStep), 1, 20);
    const stepDt = scaled / steps;
    const controllerTick = Number.isFinite(tickId) ? (tickId as number) : 0;
    this.generationTime += scaled;
    this.particles.update(scaled); // Update particles
    if (controllers) this._publishControllerSensors(controllers, controllerTick);
    if (this.botManager.getCount() > 0) {
      this.botManager.update(this, scaled, (index, rng) => this._respawnBaselineBot(index, rng));
    }
    for (let s = 0; s < steps; s++) {
      this._stepPhysics(stepDt, controllers, controllerTick);
    }
    this._updateFocus(scaled);
    this._updateCamera(viewW, viewH);
    let bestPts = -Infinity;
    let bestId = 0;
    for (let i = 0; i < this.population.length; i++) {
      const sn = this.snakes[i];
      if (!sn || !sn.alive) continue;
      if (sn.pointsScore > bestPts) {
        bestPts = sn.pointsScore;
        bestId = sn.id;
      }
    }
    // Keep bestPointsThisGen finite; sensors use it for log normalization on every tick.
    const prevBest = Number.isFinite(this.bestPointsThisGen) ? this.bestPointsThisGen : 0;
    this.bestPointsThisGen = Math.max(prevBest, bestPts > -Infinity ? bestPts : 0);
    if (bestId) this.bestPointsSnakeId = bestId;
    let aliveCount = 0;
    for (let i = 0; i < this.population.length; i++) {
      const sn = this.snakes[i];
      if (sn && sn.alive) aliveCount += 1;
    }
    const early = aliveCount <= CFG.observer.earlyEndAliveThreshold && this.generationTime >= CFG.observer.earlyEndMinSeconds;
    if (this.generationTime >= CFG.generationSeconds || early) this._endGeneration();
  }
  /**
   * Performs a single substep of physics: spawn pellets, update snakes
   * and resolve collisions.
   * @param dt - Substep delta time in seconds.
   * @param controllers - Optional external controller registry.
   * @param tickId - Optional tick id for controller sync.
   */
  _stepPhysics(
    dt: number,
    controllers?: ControllerRegistryLike,
    tickId = 0
  ): void {
    const deficit = Math.max(0, CFG.pelletCountTarget - this.pellets.length);
    this._pelletSpawnAcc += CFG.pelletSpawnPerSecond * dt;
    const spawnN = Math.min(deficit, Math.floor(this._pelletSpawnAcc));
    this._pelletSpawnAcc -= spawnN;
    for (let i = 0; i < spawnN; i++) this.addPellet(this._spawnAmbientPellet());
    for (const sn of this.snakes) {
      if (!sn.alive) continue;
      const botAction = this.botManager.getActionForSnake(sn.id);
      if (botAction) {
        sn.update(this, dt, botAction);
        continue;
      }
      if (controllers && controllers.isControlled(sn.id)) {
        const control = controllers.getAction(sn.id, tickId);
        if (control) {
          sn.update(this, dt, control);
          continue;
        }
      }
      sn.update(this, dt);
    }
    // Rebuild collision grid
    const skip = Math.max(0, Math.floor(CFG.collision.skipSegments));
    this._collGrid.reset(CFG.collision.cellSize);
    for (const s of this.snakes) {
      if (!s.alive) continue;
      const pts = s.points;
      // Add all segments
      for (let i = Math.max(1, skip); i < pts.length; i++) {
        const p0 = pts[i - 1];
        const p1 = pts[i];
        if (!p0 || !p1) continue;
        const mx = (p0.x + p1.x) * 0.5;
        const my = (p0.y + p1.y) * 0.5;
        this._collGrid.add(mx, my, s, i);
      }
    }

    // Substep physics for collisions
    this._resolveCollisionsGrid();
  }
  /**
   * Publishes sensor vectors for externally controlled snakes at the start
   * of each tick so clients see a consistent snapshot.
   */
  _publishControllerSensors(controllers: ControllerRegistryLike, tickId: number): void {
    for (const sn of this.snakes) {
      if (!sn.alive) continue;
      if (!controllers.isControlled(sn.id)) continue;
      const sensors = sn.computeSensors(this);
      controllers.publishSensors(sn.id, tickId, sensors, { x: sn.x, y: sn.y, dir: sn.dir });
    }
  }
  /**
   * Selects an initial focus snake when a generation starts or when
   * switching view modes.
   */
  _chooseInitialFocus(): void {
    const alive = this.snakes.filter(s => s.alive);
    if (alive.length) {
      const idx = randInt(alive.length);
      this.focusSnake = alive[idx] ?? null;
    } else {
      this.focusSnake = null;
    }
    if (this.viewMode === "follow" && this.focusSnake) {
      const h = this.focusSnake.head();
      this.cameraX = h.x;
      this.cameraY = h.y;
    } else {
      this.cameraX = 0;
      this.cameraY = 0;
    }
    this._focusCooldown = CFG.observer.focusRecheckSeconds;
  }
  /**
   * Computes a heuristic leader score to determine which snake should be
   * followed.  Combines points, length, kills and age.
   */
  _leaderScore(s: Snake): number {
    return s.pointsScore * 3.0 + s.length() * 1.5 + s.killScore * 35.0 + s.age * 0.15;
  }
  /**
   * Periodically reevaluates which snake should be the focus.  Uses
   * hysteresis to avoid rapid switching.
   * @param dt - Delta time in seconds.
   */
  _updateFocus(dt: number): void {
    this._focusCooldown -= dt;
    if (!this.focusSnake || !this.focusSnake.alive) {
      this.focusSnake = null;
      this._focusCooldown = 0;
    }
    if (this._focusCooldown > 0) return;
    const alive = this.snakes.filter(s => s.alive);
    if (!alive.length) {
      this.focusSnake = null;
      this._focusCooldown = CFG.observer.focusRecheckSeconds;
      return;
    }
    let best = alive[0]!;
    let bestScore = this._leaderScore(best);
    for (let i = 1; i < alive.length; i++) {
      const s = alive[i];
      if (!s) continue;
      const sc = this._leaderScore(s);
      if (sc > bestScore) {
        best = s;
        bestScore = sc;
      }
    }
    if (!this.focusSnake) this.focusSnake = best;
    else {
      const cur = this.focusSnake;
      const curScore = this._leaderScore(cur);
      if (best !== cur && bestScore > curScore * CFG.observer.focusSwitchMargin) this.focusSnake = best;
    }
    this._focusCooldown = CFG.observer.focusRecheckSeconds;
  }
  /**
   * Updates camera position and zoom based on view mode and focused snake.
   * 
   * This method manages three distinct UI states:
   * 1. "Overview": Centers the world and scales zoom to fit the arena boundary (+ margin) within the viewport.
   * 2. "Follow": Focuses on the head of a specific snake and adjusts zoom dynamically based on its length (zoom out as it grows).
   * 3. "Idle/Fallback": Slowly drifts and centers when no valid focus is found.
   * 
   * @param viewW - Viewport width in pixels.
   * @param viewH - Viewport height in pixels.
   */
  _updateCamera(viewW: number, viewH: number): void {
    // Basic normalization for invalid viewport dimensions (e.g., initial load or worker state lag).
    // We default to a square that comfortably fits the entire world.
    if (!Number.isFinite(viewW) || !Number.isFinite(viewH) || viewW <= 0 || viewH <= 0) {
      const DEFAULT_NORMALIZED_DIM = CFG.worldRadius * 2;
      viewW = DEFAULT_NORMALIZED_DIM;
      viewH = DEFAULT_NORMALIZED_DIM;
    }

    if (this.viewMode === "overview") {
      this.cameraX = 0;
      this.cameraY = 0;

      // The Overview fitting calculates a 'fit' scale factor that ensures the arena (CFG.worldRadius)
      // plus an extra safety margin (overviewExtraWorldMargin) is fully visible.
      // We standardize to the smallest dimension (min(viewW, viewH)) to ensure the arena 
      // fits regardless of the window's aspect ratio.
      // We also apply internal padding (overviewPadding) to prevent objects from touching edges.
      const effectiveR = CFG.worldRadius + CFG.observer.overviewExtraWorldMargin;
      const fit = Math.min(viewW, viewH) / (2 * effectiveR * CFG.observer.overviewPadding);
      const targetZoom = clamp(fit, 0.01, 2.0);

      // Snap zoom to target if it is at default (1.0) to avoid an unnecessary "zoom glide" 
      // when the simulation first loads or when explicitly enabled via config.
      if (this.zoom === 1.0 || (CFG.observer.snapZoomOutInOverview && this.zoom > targetZoom)) {
        this.zoom = targetZoom;
      } else {
        // Smoothly interpolate towards the target zoom to provide visual continuity.
        this.zoom = lerp(this.zoom, targetZoom, CFG.observer.zoomLerpOverview);
      }
      return;
    }

    if (this.focusSnake && this.focusSnake.alive) {
      const h = this.focusSnake.head();
      this.cameraX = h.x;
      this.cameraY = h.y;

      // Follow Zoom Logic:
      // Larger snakes require a wider FOV (lower zoom) to keep their perspective manageable.
      // We map the snake's length to a zoom range [0.45, 1.12]. This "comfort corridor" 
      // ensures that even at maximum length, the snake head and its immediate surroundings 
      // remain clearly visible without the world feeling too small.
      const focusLen = this.focusSnake.length();
      const MAX_ZOOM = 1.15;
      const MIN_ZOOM = 0.45;
      const ZOOM_RANGE = 0.55;
      const targetZoom = clamp(MAX_ZOOM - (focusLen / Math.max(1, CFG.snakeMaxLen)) * ZOOM_RANGE, MIN_ZOOM, 1.12);

      this.zoom = lerp(this.zoom, targetZoom, CFG.observer.zoomLerpFollow);
    } else {
      // Fallback: Return to center with a default zoom level when focus is lost.
      this.cameraX = 0;
      this.cameraY = 0;
      const FALLBACK_ZOOM = 0.95;
      const FALLBACK_LERP = 0.05;
      this.zoom = lerp(this.zoom, FALLBACK_ZOOM, FALLBACK_LERP);
    }
  }
  /**
   * Resolves collisions by querying the segment grid around each head and
   * killing snakes that intersect another snake’s body.  Awards kill
   * points to the aggressor.
   */
  _resolveCollisionsGrid(): void {
    const cellSize = Math.max(1, CFG.collision.cellSize);
    const hitScale = CFG.collision.hitScale;
    for (const s of this.snakes) {
      if (!s.alive) continue;

      // Head point
      const hx = s.x;
      const hy = s.y;

      let collision = false;
      let killedBy: Snake | null = null;

      const checkNeighbor = (otherS: Snake, idx: number) => {
        if (collision) return; // Already found a collision for this snake

        if (otherS === s) return;
        if (!otherS || !otherS.alive) return; // Guard against empty grid entries

        const p = otherS.points;
        if (idx >= p.length || idx <= 0) return; // Ensure valid segment indices

        const p0 = p[idx - 1];
        const p1 = p[idx];
        if (!p0 || !p1) return;
        const dist2 = pointSegmentDist2(hx, hy, p0.x, p0.y, p1.x, p1.y);
        // Effective radius
        const thr = (s.radius + otherS.radius) * hitScale;
        if (dist2 <= thr * thr) {
          collision = true;
          killedBy = otherS;
        }
      };

      // Query local and neighbor cells
      const cx = Math.floor(hx / cellSize);
      const cy = Math.floor(hy / cellSize);
      // Query current cell and 8 neighbors
      for (let oy = -1; oy <= 1; oy++) {
        for (let ox = -1; ox <= 1; ox++) {
          this._collGrid.queryCell(cx + ox, cy + oy, checkNeighbor);
          if (collision) break; // Stop querying if collision found
        }
        if (collision) break;
      }

      if (collision) {
        s.die(this);
        if (killedBy && killedBy !== s) { // Award points only if killed by another snake
          const killer = killedBy as Snake;
          killer.killScore += 1;
          killer.pointsScore += CFG.reward.pointsPerKill;
        }
      }
    }
  }
  /**
   * Ends the current generation: computes fitness scores, selects elites
   * and breeds new genomes via tournament selection, crossover and
   * mutation.  Resets state for the new generation.
   */
  _endGeneration(): void {
    if (!this.population.length) return;
    const populationSnakes = this.snakes.slice(0, this.population.length);
    let maxPts = 0;
    for (const s of populationSnakes) if (s) maxPts = Math.max(maxPts, s.pointsScore);
    if (maxPts <= 0) maxPts = 1;
    const logDen = Math.log(1 + maxPts);
    const topIds = new Set();
    for (const s of populationSnakes) if (s && Math.abs(s.pointsScore - maxPts) <= 1e-6) topIds.add(s.id);
    for (let i = 0; i < this.population.length; i++) {
      const s = populationSnakes[i];
      const pop = this.population[i];
      if (!s || !pop) continue;
      const pointsNorm = clamp(Math.log(1 + s.pointsScore) / logDen, 0, 1);
      const topBonus = topIds.has(s.id) ? CFG.reward.fitnessTopPointsBonus : 0;
      const fit = s.computeFitness(pointsNorm, topBonus);
      pop.fitness = fit;
      s.fitness = fit; // Store on snake for HoF retrieval
      if (fit > this.bestFitnessEver) this.bestFitnessEver = fit;
    }
    this.population.sort((a, b) => b.fitness - a.fitness);

    // Record history
    const avgFit = this.population.reduce((sum, g) => sum + g.fitness, 0) / this.population.length;
    const minFit = this.population[this.population.length - 1]?.fitness ?? 0;
    const diversity = computeSpeciesStats(this.population);
    const complexity = computeNetworkStats(this.population);
    const bestGenome = this.population[0];
    if (!bestGenome) return;
    this.fitnessHistory.push({
      gen: this.generation,
      best: bestGenome.fitness,
      avg: avgFit,
      min: minFit,
      speciesCount: diversity.speciesCount,
      topSpeciesSize: diversity.topSpeciesSize,
      avgWeight: complexity.avgWeight,
      weightVariance: complexity.weightVariance
    });
    if (this.fitnessHistory.length > 100) this.fitnessHistory.shift();

    // Hall of Fame: Record the best snake of this generation
    const bestG = bestGenome;
    // Find the actual snake object to get its length/kill stats, as genome doesn't have them
    // The population is sorted by fitness, so population[0] is the best genome.
    // However, the snakes array might not match population order unless we track IDs.
    // Easier: find the snake with the best fitness.
    let bestS: Snake | null = null;
    let maxFit = -1;
    for (const s of populationSnakes) {
      const fit = s.fitness ?? -Infinity;
      if (fit > maxFit) {
        maxFit = fit;
        bestS = s;
      }
    }
    // Fallback if fitness wasn't stored on snake yet (it is computed in this function)
    if (!bestS && this.snakes.length > 0) bestS = this.snakes[0] ?? null; // Should rarely happen

    if (bestS) {
      const hofEntry = {
        gen: this.generation,
        seed: bestS.id, // Using ID as a proxy for 'seed' or unique identifier
        fitness: bestS.fitness ?? 0, // Ensure fitness is set
        points: bestS.pointsScore,
        length: bestS.length(),
        genome: bestG.toJSON() // Persist the genome data
      };
      void hof.add(hofEntry);
      this._lastHoFEntry = hofEntry;
    }

    const eliteN = Math.max(1, Math.floor(CFG.eliteFrac * this.population.length));
    const elites = this.population.slice(0, eliteN).map(g => g.clone());
    const newPop = [];
    for (let i = 0; i < eliteN; i++) {
      const elite = elites[i];
      if (elite) newPop.push(elite.clone());
    }
    while (newPop.length < this.population.length) {
      const parentA = tournamentPick(this.population, 5);
      const parentB = tournamentPick(this.population, 5);
      const child = crossover(parentA, parentB, this.arch);
      mutate(child, this.arch);
      child.fitness = 0;
      newPop.push(child);
    }
    this.population = newPop;
    this.generation += 1;
    this.generationTime = 0;
    this.bestPointsThisGen = 0;
    this.bestPointsSnakeId = 0;
    this.particles = new ParticleSystem(); // Reset particles
    this._initPellets();
    this._resetBaselineBotsForGen();
    this._spawnAll();
    this._collGrid.build(this.snakes, CFG.collision.skipSegments);
    this._chooseInitialFocus();
  }

  /**
   * Spawns a snake from a saved genome immediately into the world.
   * @param genomeJSON - Serialized genome to resurrect.
   * @returns Newly spawned snake id.
   */
  resurrect(genomeJSON: GenomeJSON): number {
    const genome = Genome.fromJSON(genomeJSON);
    // Create a new snake with a high ID to avoid collision
    const id = 10000 + randInt(90000);
    const snake = new Snake(id, genome, this.arch, { skin: 1 });

    // Give it a distinct look (e.g. golden glow) if possible, or just standard
    snake.color = '#FFD700'; // Gold color to signify HoF status

    this.snakes.push(snake);
    this.focusSnake = snake; // Auto-focus the resurrected snake
    this.viewMode = 'follow';
    this.zoom = 1.0;
    return id;
  }

  /**
   * Spawns a new externally controlled snake with a fresh genome.
   * Reuses dead external slots to avoid unbounded growth.
   */
  spawnExternalSnake(): Snake {
    const genome = Genome.random(this.arch);
    const reusableIndex = this.snakes.findIndex(
      (snake) => !snake.alive && snake.id >= EXTERNAL_SNAKE_ID_START && snake.baselineBotIndex == null
    );
    if (reusableIndex >= 0) {
      const existingId = this.snakes[reusableIndex]!.id;
      const snake = new Snake(existingId, genome, this.arch);
      this.snakes[reusableIndex] = snake;
      return snake;
    }
    const id = this._nextExternalSnakeId++;
    const snake = new Snake(id, genome, this.arch);
    this.snakes.push(snake);
    return snake;
  }

  /**
   * Generates a new ambient pellet using a fractal noise rejection algorithm.
   * 
   * Performance & Distribution:
   * This algorithm creates "Fractal Food" patterns where pellets form filaments and 
   * clusters rather than a uniform distribution. This encourages organic movement 
   * and strategic clustering behavior in the snakes.
   * 
   * Approach:
   * 1. We use "Interference Noise" by summing overlapping sinusoidal waves of varying 
   *    frequencies and phases. This creates complex patterns of "peaks" (filaments) 
   *    and "valleys" (voids) without the overhead of Perlin noise.
   * 2. Rejection Sampling: We pick a random spot and check the local noise density. 
   *    Higher density spots are more likely to spawn pellets, concentrating food 
   *    into strategic clusters that encourage movement and conflict.
   * 3. Fallback: If several attempts fail to meet the density criteria, we spawn 
   *    uniformly to ensure food density doesn't drop too low in "void" regions.
   * 
   * @returns A new Pellet instance.
   */
  _spawnAmbientPellet(): Pellet {
    const r = CFG.worldRadius;
    const TIME_DRIFT_SCALE = 0.04;
    const t = this.generationTime * TIME_DRIFT_SCALE;

    const foodCfg = CFG.foodSpawn ?? {};
    // Domain warp to keep filaments from aligning to a rigid grid.
    const WARP_FREQ = Math.max(0, foodCfg.warpFreq ?? 0.0013);
    const WARP_SCALE = Math.max(0, (foodCfg.warpScale ?? 0.08) * r);

    // Filament noise frequencies (feature scales).
    const FREQ_LARGE = Math.max(0, foodCfg.freqLarge ?? 0.0026);
    const FREQ_MEDIUM = Math.max(0, foodCfg.freqMedium ?? 0.0042);
    const FREQ_SMALL = Math.max(0, foodCfg.freqSmall ?? 0.0068);

    // Rejection Sampling Loop:
    // We attempt to find a location that satisfies our noise-based density requirements.
    const REJECTION_RETRIES = 8;
    const FILAMENT_POWER = Math.max(0.1, foodCfg.filamentPower ?? 4.2);
    const DUST_STRENGTH = clamp(foodCfg.dustStrength ?? 0.35, 0, 1);
    const EDGE_FADE_START = clamp(foodCfg.edgeFadeStart ?? 0.35, 0, 0.95);
    const EDGE_FADE_POWER = Math.max(0.1, foodCfg.edgeFadePower ?? 2.6);
    const edgeFalloffEnabled = foodCfg.edgeFalloffEnabled ?? true;
    let bestProb = -1;
    let bestX = 0;
    let bestY = 0;

    for (let i = 0; i < REJECTION_RETRIES; i++) {
      // Phase 1: Pick a random candidate point within the circular world.
      const a = Math.random() * TAU;
      const d = Math.sqrt(Math.random()) * r;
      const x = Math.cos(a) * d;
      const y = Math.sin(a) * d;

      // Phase 2: Domain warp + ridged interference for filament bands.
      const warpX = Math.sin(y * WARP_FREQ + t * 0.7) * WARP_SCALE
        + Math.cos(x * WARP_FREQ * 1.25 - t * 0.4) * (WARP_SCALE * 0.6);
      const warpY = Math.cos(x * WARP_FREQ - t * 0.5) * WARP_SCALE
        + Math.sin(y * WARP_FREQ * 1.1 + t * 0.8) * (WARP_SCALE * 0.6);
      const xw = x + warpX;
      const yw = y + warpY;

      const n1 = Math.sin((xw + yw) * FREQ_LARGE + t)
        + Math.cos((xw - yw) * FREQ_LARGE - t * 0.7);
      const n2 = Math.sin(xw * FREQ_MEDIUM - t * 1.1)
        + Math.cos(yw * FREQ_MEDIUM + t * 0.9);
      const n3 = Math.sin(xw * FREQ_SMALL + t * 1.6)
        * Math.cos(yw * FREQ_SMALL - t * 1.3);

      const ridgeA = clamp(1 - Math.abs(n1) / 2, 0, 1);
      const ridgeB = clamp(1 - Math.abs(n2) / 2, 0, 1);
      const ridgeC = clamp(1 - Math.abs(n3), 0, 1);

      const webA = Math.pow(ridgeA, FILAMENT_POWER);
      const webB = Math.pow(ridgeB, FILAMENT_POWER * 0.95);
      const dust = Math.pow(ridgeC, 2.2) * DUST_STRENGTH;
      let prob = clamp(Math.max(webA, webB) + dust, 0, 1);

      if (edgeFalloffEnabled) {
        const edgeT = d / r;
        const edgeRamp = clamp((edgeT - EDGE_FADE_START) / (1 - EDGE_FADE_START), 0, 1);
        const edgeSmooth = edgeRamp * edgeRamp * (3 - 2 * edgeRamp);
        const edgeFalloff = clamp(1 - Math.pow(edgeSmooth, EDGE_FADE_POWER), 0, 1);
        prob *= edgeFalloff;
      }

      if (prob > bestProb) {
        bestProb = prob;
        bestX = x;
        bestY = y;
      }

      if (Math.random() < prob) {
        return new Pellet(x, y, CFG.foodValue, null, "ambient", 0);
      }
    }

    // Phase 4: Fallback Distribution
    // If rejection sampling fails after all attempts, pick the best candidate
    // to keep the voids expansive while still guaranteeing a pellet.
    return new Pellet(bestX, bestY, CFG.foodValue, null, "ambient", 0);
  }
}

/**
 * Selects a genome by k‑tournament selection: chooses k random candidates
 * from the population and returns the fittest among them.  Used for
 * breeding new individuals.
 * @param pop - Candidate population.
 * @param k - Tournament size.
 */
function tournamentPick(pop: Genome[], k: number): Genome {
  let best: Genome | null = null;
  for (let i = 0; i < k; i++) {
    const g = pop[randInt(pop.length)] ?? pop[0]!;
    if (!best || g.fitness > best.fitness) best = g;
  }
  return best!;
}

/** Spatial hash for pellets to support fast local queries for sensing and eating. */
class PelletGrid {
  /** Grid cell size in world units. */
  cellSize: number;
  /** Map of cell keys to pellets in that cell. */
  map: Map<string, Pellet[]>;

  constructor() {
    this.cellSize = Math.max(10, CFG.pelletGrid?.cellSize ?? 120);
    this.map = new Map();
  }
  /** Reset the grid sizing based on the current CFG. */
  resetForCFG(): void {
    this.cellSize = Math.max(10, CFG.pelletGrid?.cellSize ?? 120);
    this.map.clear();
  }
  /**
   * Build the key for a cell coordinate.
   * @param cx - Cell x coordinate.
   * @param cy - Cell y coordinate.
   * @returns Map key string.
   */
  _key(cx: number, cy: number): string {
    return cx + "," + cy;
  }
  /**
   * Convert world coordinates to cell coordinates.
   * @param x - World x position.
   * @param y - World y position.
   * @returns Cell coordinate object.
   */
  _coords(x: number, y: number): { cx: number; cy: number } {
    return { cx: Math.floor(x / this.cellSize), cy: Math.floor(y / this.cellSize) };
  }
  /**
   * Add a pellet to the spatial hash.
   * @param p - Pellet to add.
   */
  add(p: Pellet): void {
    const { cx, cy } = this._coords(p.x, p.y);
    const k = this._key(cx, cy);
    let arr = this.map.get(k);
    if (!arr) {
      arr = [];
      this.map.set(k, arr);
    }
    p._pcx = cx;
    p._pcy = cy;
    p._pkey = k;
    p._cellArr = arr;
    p._cellIndex = arr.length;
    arr.push(p);
  }
  /**
   * Remove a pellet from the spatial hash.
   * @param p - Pellet to remove.
   */
  remove(p: Pellet): void {
    const arr = p._cellArr;
    if (!arr) return;
    const idx = p._cellIndex!;
    const last = arr.pop()!;
    if (last !== p) {
      arr[idx] = last;
      last._cellIndex = idx;
      last._cellArr = arr;
    }
    p._cellArr = null;
    p._cellIndex = -1;
    if (arr.length === 0 && p._pkey) {
      // Safe even if already deleted.
      this.map.delete(p._pkey);
    }
  }
  /**
   * Iterate pellets in cells intersecting a radius around (x,y).
   * @param x - World x position.
   * @param y - World y position.
   * @param r - Query radius.
   * @param fn - Callback invoked for each pellet.
   */
  forEachInRadius(x: number, y: number, r: number, fn: (p: Pellet) => void): void {
    const cs = this.cellSize;
    const minCx = Math.floor((x - r) / cs);
    const maxCx = Math.floor((x + r) / cs);
    const minCy = Math.floor((y - r) / cs);
    const maxCy = Math.floor((y + r) / cs);
    for (let cy = minCy; cy <= maxCy; cy++) {
      for (let cx = minCx; cx <= maxCx; cx++) {
        const arr = this.map.get(this._key(cx, cy));
        if (!arr) continue;
        for (let i = 0; i < arr.length; i++) {
          const pellet = arr[i];
          if (pellet) fn(pellet);
        }
      }
    }
  }
}

/** Distance threshold for species bucketing. */
const SPECIES_DISTANCE_THRESHOLD = 0.35;

/**
 * Compute RMS distance between two genomes.
 * @param a - Genome A.
 * @param b - Genome B.
 * @returns RMS distance or Infinity when incompatible.
 */
function genomeDistanceRms(a: Genome, b: Genome): number {
  const wa = a.weights;
  const wb = b.weights;
  if (!wa || !wb || wa.length !== wb.length) return Infinity;
  let sumSq = 0;
  for (let i = 0; i < wa.length; i++) {
    const d = (wa[i] ?? 0) - (wb[i] ?? 0);
    sumSq += d * d;
  }
  return Math.sqrt(sumSq / wa.length);
}

/**
 * Compute species count and top species size for a population.
 * @param population - Genomes to analyze.
 * @returns Species statistics summary.
 */
function computeSpeciesStats(population: Genome[]): { speciesCount: number; topSpeciesSize: number } {
  if (!population || population.length === 0) {
    return { speciesCount: 0, topSpeciesSize: 0 };
  }
  const species: Array<{ rep: Genome; size: number }> = [];
  for (const genome of population) {
    let assigned = false;
    for (const bucket of species) {
      if (genomeDistanceRms(genome, bucket.rep) <= SPECIES_DISTANCE_THRESHOLD) {
        bucket.size += 1;
        assigned = true;
        break;
      }
    }
    if (!assigned) {
      species.push({ rep: genome, size: 1 });
    }
  }
  let topSize = 0;
  for (const bucket of species) topSize = Math.max(topSize, bucket.size);
  return { speciesCount: species.length, topSpeciesSize: topSize };
}

/**
 * Compute weight statistics across a population.
 * @param population - Genomes to analyze.
 * @returns Network weight summary.
 */
function computeNetworkStats(population: Genome[]): { avgWeight: number; weightVariance: number } {
  if (!population || population.length === 0) {
    return { avgWeight: 0, weightVariance: 0 };
  }
  let sumAbs = 0;
  let sumAbsSq = 0;
  let count = 0;
  for (const genome of population) {
    const w = genome.weights;
    if (!w) continue;
    for (let i = 0; i < w.length; i++) {
      const aw = Math.abs(w[i] ?? 0);
      sumAbs += aw;
      sumAbsSq += aw * aw;
      count += 1;
    }
  }
  if (!count) return { avgWeight: 0, weightVariance: 0 };
  const avgWeight = sumAbs / count;
  const weightVariance = Math.max(0, sumAbsSq / count - avgWeight * avgWeight);
  return { avgWeight, weightVariance };
}
