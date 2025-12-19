# Task Checklist: Slither Neuroevolution 2.0

## Phase 0: Foundation & Testing (REQUIRED FIRST)

- [x] Initialize testing framework (Vitest/Jest).
- [x] **Tests: `utils.js`**
- [x] **Tests: `mlp.js`**
- [x] **Tests: `snake.js`**
- [x] **Tests: `spatialHash.js`**
- [x] **Tests: `world.js`**
- [x] **Tests: `sensors.js`**
- [x] **Tests: `serializer.js`**
- [x] **Tests: `storage.js`**
- [x] **Tests: `particles.js`**
- [x] **Tests: `render.js`**
- [x] **Tests: `BrainViz.js` & `FitnessChart.js`**
- [x] **Tests: `hallOfFame.js`**
- [x] **Tests: `settings.js`**
- [x] **Tests: `theme.js` & `config.js`**
- [x] **Tests: `worker.js` & `main.js`**
- [x] **Tests: `chartUtils.js`**
- [x] **Verification:** `npm test` passes for the entire core simulation.

## Phase 1: Persistence Part A

- [x] Implement `Genome.toJSON()`.
- [x] Implement `Genome.fromJSON()`.
- [x] Verify serialization determinism (no data loss).

## Phase 2: Persistence Part B (Registry)

- [x] Create `src/storage.js` for localStorage handling.
- [x] Implement generational champion registry (saving logic).

## Phase 3: Hall of Fame Interface

- [x] Create "Hall of Fame" UI tab.
- [x] Implement champion data table.
- [x] Implement "Resurrect" buttons.

## Phase 4: Web Worker Infrastructure

- [x] Create `src/worker.js` skeleton.
- [x] Implement Main-to-Worker messaging handshake.
- [x] Implement hot-sync for Config/Settings.

## Phase 5: Binary Serialization Protocol

- [x] Define binary layout in `serializer.js`.
- [x] Implement `Serializer.serialize()` in Worker.
- [x] Implement binary parser in `render.js`.

## Phase 6: Performance Optimization (Grid)

- [x] Implement fixed-size `Int32Array` `FlatSpatialHash`.
- [x] Port Worker collision detection to use optimized grid.

## Phase 7: Background Visuals

- [x] Implement procedural Starfield.
- [x] Optimize background drawing (viewport culling).

## Phase 8: Entity Visuals

- [x] Restore Snake Head/Eye Rendering.
- [x] Implement Neon Glow particle effects.

## Phase 9: God Mode

- [x] Implement Screen-to-World coordinate mapping.
- [x] Implement "Kill" command for entities.
- [x] Implement "Drag/Move" command.

## Phase 10: Analytics & Graphs

- [x] Implement Fitness History logging.
- [x] Integrate Charting (Min/Avg/Max).
- [x] Implement Brain Visualizer (Activation heatmap).
