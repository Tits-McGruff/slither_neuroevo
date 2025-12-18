// world.js
// Defines the World class which encapsulates the simulation state and
// manages updating snakes, spawning pellets, collision resolution and
// evolution of genomes between generations.  Also includes helper
// functions for genetic selection and pellet spawning.

import { CFG } from './config.js';
import { buildArch, archKey, Genome, crossover, mutate } from './mlp.js';
import { ParticleSystem } from './particles.js';
import { Snake, Pellet, SegmentGrid as LegacyGrid, pointSegmentDist2 } from './snake.js';
import { randInt, clamp, lerp, TAU } from './utils.js';
import { hof } from './hallOfFame.js';
import { FlatSpatialHash } from './spatialHash.js';

export class World {
  constructor(settings = {}) {
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
    this.particles = new ParticleSystem(); // Initialize particle system
    this.generation = 1;
    this.generationTime = 0;
    this.population = [];
    this.bestFitnessEver = 0;
    this.fitnessHistory = []; // Track fitness over generations
    this.bestPointsThisGen = 0;
    this.bestPointsSnakeId = 0;
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
    this._initPopulation();
    this._spawnAll();
    this._collGrid.build(this.snakes, CFG.collision.skipSegments);
    this._initPellets();
    this._chooseInitialFocus();
  }
  /**
   * Immediately adjusts the simulation speed.  Also stores the new
   * value back into the settings object.
   * @param {number} x
   */
  applyLiveSimSpeed(x) {
    this.simSpeed = clamp(x, 0.01, 500.0);
    this.settings.simSpeed = this.simSpeed;
  }
  /**
   * Toggles between overview and follow camera modes.  Ensures that a
   * valid focus snake is selected when switching to follow mode.
   */
  toggleViewMode() {
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
   * Chooses an alive snake at random.  Returns null if none.
   */
  _pickAnyAlive() {
    const alive = this.snakes.filter(s => s.alive);
    return alive.length ? alive[randInt(alive.length)] : null;
  }
  /**
   * Initialises the population with random genomes according to the
   * current architecture.
   */
  _initPopulation() {
    this.population.length = 0;
    for (let i = 0; i < this.settings.snakeCount; i++) {
      this.population.push(Genome.random(this.arch));
    }
  }
  /**
   * Spawns snakes from the current population genomes.
   */
  _spawnAll() {
    this.snakes.length = 0;
    for (let i = 0; i < this.population.length; i++) {
      this.snakes.push(new Snake(i + 1, this.population[i].clone(), this.arch));
    }
  }
  /**
   * Fills the world with pellets up to the configured target count.
   */
_initPellets() {
  this.pellets.length = 0;
  this.pelletGrid.resetForCFG();
  this._pelletSpawnAcc = 0;
  while (this.pellets.length < CFG.pelletCountTarget) this.addPellet(randomPellet());
}

/**
 * Adds a pellet to the world and to the pellet spatial hash.
 * @param {Pellet} p
 */
addPellet(p) {
  p._idx = this.pellets.length;
  this.pellets.push(p);
  this.pelletGrid.add(p);
}

/**
 * Removes a pellet from the world and from the pellet spatial hash.
 * @param {Pellet} p
 */
removePellet(p) {
  if (!p) return;
  this.pelletGrid.remove(p);
  const idx = p._idx;
  if (idx == null || idx < 0 || idx >= this.pellets.length) return;
  const last = this.pellets.pop();
  if (last !== p) {
    this.pellets[idx] = last;
    last._idx = idx;
  }
  p._idx = -1;
}

  /**
   * Advances the simulation by dt seconds (scaled by simSpeed) and
   * updates camera and focus logic.  Handles early generation termination.
   * @param {number} dt Base delta time (unscaled)
   * @param {number} viewW Canvas width in CSS pixels
   * @param {number} viewH Canvas height in CSS pixels
   */
  update(dt, viewW, viewH) {
    const rawScaled = dt * this.simSpeed;
    const scaled = clamp(rawScaled, 0, Math.max(0.004, CFG.dtClamp));
    const maxStep = clamp(CFG.collision.substepMaxDt, 0.004, 0.08);
    const steps = clamp(Math.ceil(scaled / maxStep), 1, 20);
    const stepDt = scaled / steps;
    this.generationTime += scaled;
    this.particles.update(scaled); // Update particles
    for (let s = 0; s < steps; s++) {
      this._stepPhysics(stepDt);
    }
    this._updateFocus(scaled);
    this._updateCamera(viewW, viewH);
    let bestPts = -Infinity;
    let bestId = 0;
    for (const sn of this.snakes) {
      if (!sn.alive) continue;
      if (sn.pointsScore > bestPts) {
        bestPts = sn.pointsScore;
        bestId = sn.id;
      }
    }
    const prevBest = Number.isFinite(this.bestPointsThisGen) ? this.bestPointsThisGen : 0;
    this.bestPointsThisGen = Math.max(prevBest, bestPts > -Infinity ? bestPts : 0);
    if (bestId) this.bestPointsSnakeId = bestId;
    const aliveCount = this.snakes.reduce((acc, sn) => acc + (sn.alive ? 1 : 0), 0);
    const early = aliveCount <= CFG.observer.earlyEndAliveThreshold && this.generationTime >= CFG.observer.earlyEndMinSeconds;
    if (this.generationTime >= CFG.generationSeconds || early) this._endGeneration();
  }
  /**
   * Performs a single substep of physics: spawn pellets, update snakes
   * and resolve collisions.
   * @param {number} dt
   */
  _stepPhysics(dt) {
    const deficit = Math.max(0, CFG.pelletCountTarget - this.pellets.length);
    this._pelletSpawnAcc += CFG.pelletSpawnPerSecond * dt;
    const spawnN = Math.min(deficit, Math.floor(this._pelletSpawnAcc));
    this._pelletSpawnAcc -= spawnN;
    for (let i = 0; i < spawnN; i++) this.addPellet(randomPellet());
    for (const sn of this.snakes) sn.update(this, dt);
    // Rebuild collision grid
    const skip = Math.max(0, Math.floor(CFG.collision.skipSegments));
    this._collGrid.reset(CFG.collision.cellSize);
    for (const s of this.snakes) {
      if (!s.alive) continue;
      const pts = s.points;
      // Add all segments
      for (let i = Math.max(1, skip); i < pts.length; i++) {
        const p0 = pts[i-1];
        const p1 = pts[i];
        const mx = (p0.x + p1.x) * 0.5;
        const my = (p0.y + p1.y) * 0.5;
        this._collGrid.add(mx, my, s, i);
      }
    }

    // Substep physics for collisions
    this._resolveCollisionsGrid();
  }
  /**
   * Selects an initial focus snake when a generation starts or when
   * switching view modes.
   */
  _chooseInitialFocus() {
    const alive = this.snakes.filter(s => s.alive);
    this.focusSnake = alive.length ? alive[randInt(alive.length)] : null;
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
  _leaderScore(s) {
    return s.pointsScore * 3.0 + s.length() * 1.5 + s.killScore * 35.0 + s.age * 0.15;
  }
  /**
   * Periodically reevaluates which snake should be the focus.  Uses
   * hysteresis to avoid rapid switching.
   * @param {number} dt
   */
  _updateFocus(dt) {
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
    let best = alive[0];
    let bestScore = this._leaderScore(best);
    for (let i = 1; i < alive.length; i++) {
      const s = alive[i];
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
   * @param {number} viewW
   * @param {number} viewH
   */
  _updateCamera(viewW, viewH) {
    if (!Number.isFinite(viewW) || !Number.isFinite(viewH) || viewW <= 0 || viewH <= 0) {
      viewW = CFG.worldRadius * 2;
      viewH = CFG.worldRadius * 2;
    }
    if (this.viewMode === "overview") {
      this.cameraX = 0;
      this.cameraY = 0;
      const effectiveR = CFG.worldRadius + CFG.observer.overviewExtraWorldMargin;
      const fit = Math.min(viewW, viewH) / (2 * effectiveR * CFG.observer.overviewPadding);
      const targetZoom = clamp(fit, 0.01, 2.0);
      if (CFG.observer.snapZoomOutInOverview && this.zoom > targetZoom) this.zoom = targetZoom;
      else this.zoom = lerp(this.zoom, targetZoom, CFG.observer.zoomLerpOverview);
      return;
    }
    if (this.focusSnake && this.focusSnake.alive) {
      const h = this.focusSnake.head();
      this.cameraX = h.x;
      this.cameraY = h.y;
      const focusLen = this.focusSnake.length();
      const targetZoom = clamp(1.15 - (focusLen / Math.max(1, CFG.snakeMaxLen)) * 0.55, 0.45, 1.12);
      this.zoom = lerp(this.zoom, targetZoom, CFG.observer.zoomLerpFollow);
    } else {
      this.cameraX = 0;
      this.cameraY = 0;
      this.zoom = lerp(this.zoom, 0.95, 0.05);
    }
  }
  /**
   * Resolves collisions by querying the segment grid around each head and
   * killing snakes that intersect another snake’s body.  Awards kill
   * points to the aggressor.
   */
  _resolveCollisionsGrid() {
    const cellSize = Math.max(1, CFG.collision.cellSize);
    const hitScale = CFG.collision.hitScale;
    for (const s of this.snakes) {
      if (!s.alive) continue;

      // Head point
      const hx = s.x;
      const hy = s.y;

      let collision = false;
      let killedBy = null;

      const checkNeighbor = (otherS, idx) => {
          if (collision) return; // Already found a collision for this snake

          if (otherS === s) return;
          if (!otherS || !otherS.alive) return; // Guard against empty grid entries

          const p = otherS.points;
          if (idx >= p.length || idx <= 0) return; // Ensure valid segment indices

          const p0 = p[idx - 1];
          const p1 = p[idx];
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
      const cs = cellSize; // Use cs for clarity in queries

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
          killedBy.killScore += 1;
          killedBy.pointsScore += CFG.reward.pointsPerKill;
        }
      }
    }
  }
  /**
   * Ends the current generation: computes fitness scores, selects elites
   * and breeds new genomes via tournament selection, crossover and
   * mutation.  Resets state for the new generation.
   */
  _endGeneration() {
    let maxPts = 0;
    for (const s of this.snakes) maxPts = Math.max(maxPts, s.pointsScore);
    if (maxPts <= 0) maxPts = 1;
    const logDen = Math.log(1 + maxPts);
    const topIds = new Set();
    for (const s of this.snakes) if (Math.abs(s.pointsScore - maxPts) <= 1e-6) topIds.add(s.id);
    for (let i = 0; i < this.snakes.length; i++) {
      const s = this.snakes[i];
      const pointsNorm = clamp(Math.log(1 + s.pointsScore) / logDen, 0, 1);
      const topBonus = topIds.has(s.id) ? CFG.reward.fitnessTopPointsBonus : 0;
      const fit = s.computeFitness(pointsNorm, topBonus);
      this.population[i].fitness = fit;
      s.fitness = fit; // Store on snake for HoF retrieval
      if (fit > this.bestFitnessEver) this.bestFitnessEver = fit;
    }
    this.population.sort((a, b) => b.fitness - a.fitness);
    
    // Record history
    const avgFit = this.population.reduce((sum, g) => sum + g.fitness, 0) / this.population.length;
    this.fitnessHistory.push({ gen: this.generation, best: this.population[0].fitness, avg: avgFit });
    if (this.fitnessHistory.length > 100) this.fitnessHistory.shift();

    // Hall of Fame: Record the best snake of this generation
    const bestG = this.population[0];
    // Find the actual snake object to get its length/kill stats, as genome doesn't have them
    // The population is sorted by fitness, so population[0] is the best genome.
    // However, the snakes array might not match population order unless we track IDs.
    // Easier: find the snake with the best fitness.
    let bestS = null;
    let maxFit = -1;
    for (const s of this.snakes) {
      if (s.fitness > maxFit) {
        maxFit = s.fitness;
        bestS = s;
      }
    }
    // Fallback if fitness wasn't stored on snake yet (it is computed in this function)
    if (!bestS && this.snakes.length > 0) bestS = this.snakes[0]; // Should rarely happen

    if (bestS) {
      hof.add({
        gen: this.generation,
        seed: bestS.id, // Using ID as a proxy for 'seed' or unique identifier
        fitness: bestS.fitness || 0, // Ensure fitness is set
        points: bestS.pointsScore,
        length: bestS.length(),
        genome: bestG.toJSON() // Persist the genome data
      });
    }

    const eliteN = Math.max(1, Math.floor(CFG.eliteFrac * this.population.length));
    const elites = this.population.slice(0, eliteN).map(g => g.clone());
    const newPop = [];
    for (let i = 0; i < eliteN; i++) newPop.push(elites[i].clone());
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
    this._spawnAll();
    this._collGrid.build(this.snakes, CFG.collision.skipSegments);
    this._chooseInitialFocus();
  }

  /**
   * Spawns a snake from a saved genome immediately into the world.
   * @param {Object} genomeJSON 
   */
  resurrect(genomeJSON) {
    const genome = Genome.fromJSON(genomeJSON);
    // Create a new snake with a high ID to avoid collision
    const id = 10000 + randInt(90000); 
    const snake = new Snake(id, genome, this.arch);
    
    // Give it a distinct look (e.g. golden glow) if possible, or just standard
    snake.color = '#FFD700'; // Gold color to signify HoF status
    
    this.snakes.push(snake);
    this.focusSnake = snake; // Auto-focus the resurrected snake
    this.viewMode = 'follow';
    this.zoom = 1.0;
  }
}

/**
 * Selects a genome by k‑tournament selection: chooses k random candidates
 * from the population and returns the fittest among them.  Used for
 * breeding new individuals.
 * @param {Array<Genome>} pop
 * @param {number} k
 */
function tournamentPick(pop, k) {
  let best = null;
  for (let i = 0; i < k; i++) {
    const g = pop[randInt(pop.length)];
    if (!best || g.fitness > best.fitness) best = g;
  }
  return best;
}

/**
 * Returns a random pellet located uniformly within the arena.  The
 * pellet’s value equals the configured foodValue.
 */

/**
 * Spatial hash for pellets to support fast local queries for sensing and eating.
 */
class PelletGrid {
  constructor() {
    this.cellSize = Math.max(10, CFG.pelletGrid?.cellSize ?? 120);
    this.map = new Map();
  }
  resetForCFG() {
    this.cellSize = Math.max(10, CFG.pelletGrid?.cellSize ?? 120);
    this.map.clear();
  }
  _key(cx, cy) {
    return cx + "," + cy;
  }
  _coords(x, y) {
    return { cx: Math.floor(x / this.cellSize), cy: Math.floor(y / this.cellSize) };
  }
  add(p) {
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
  remove(p) {
    const arr = p._cellArr;
    if (!arr) return;
    const idx = p._cellIndex;
    const last = arr.pop();
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
   * Iterates pellets in cells intersecting a radius around (x,y).
   * The callback receives the pellet object.
   */
  forEachInRadius(x, y, r, fn) {
    const cs = this.cellSize;
    const minCx = Math.floor((x - r) / cs);
    const maxCx = Math.floor((x + r) / cs);
    const minCy = Math.floor((y - r) / cs);
    const maxCy = Math.floor((y + r) / cs);
    for (let cy = minCy; cy <= maxCy; cy++) {
      for (let cx = minCx; cx <= maxCx; cx++) {
        const arr = this.map.get(this._key(cx, cy));
        if (!arr) continue;
        for (let i = 0; i < arr.length; i++) fn(arr[i]);
      }
    }
  }
}

function randomPellet() {
  const a = Math.random() * TAU;
  const r = Math.sqrt(Math.random()) * CFG.worldRadius;
  return new Pellet(Math.cos(a) * r, Math.sin(a) * r, CFG.foodValue, null, "ambient", 0);
}
