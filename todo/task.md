# Task Checklist: Slither Neuroevolution 2.0

## Phase 0: Foundation & Testing (REQUIRED FIRST)

- [ ] Initialize testing framework (Vitest/Jest).
- [ ] **Tests: `utils.js`**
- [ ] **Tests: `mlp.js`**
- [ ] **Tests: `snake.js`**
- [ ] **Tests: `spatialHash.js`**
- [ ] **Tests: `world.js`**
- [ ] **Tests: `sensors.js`**
- [ ] **Tests: `serializer.js`**
- [ ] **Tests: `storage.js`**
- [ ] **Tests: `particles.js`**
- [ ] **Tests: `render.js`**
- [ ] **Tests: `BrainViz.js` & `FitnessChart.js`**
- [ ] **Tests: `hallOfFame.js`**
- [ ] **Tests: `settings.js`**
- [ ] **Tests: `theme.js` & `config.js`**
- [ ] **Tests: `worker.js` & `main.js`**
- [ ] **Tests: `chartUtils.js`**
- [ ] **Verification:** `npm test` passes for the entire core simulation.

## Phase 1: Persistence Part A

- [ ] Implement `Genome.toJSON()`.
- [ ] Implement `Genome.fromJSON()`.
- [ ] Verify serialization determinism (no data loss).

## Phase 2: Persistence Part B (Registry)

- [ ] Create `src/storage.js` for localStorage handling.
- [ ] Implement generational champion registry (saving logic).

## Phase 3: Hall of Fame Interface

- [ ] Create "Hall of Fame" UI tab.
- [ ] Implement champion data table.
- [ ] Implement "Resurrect" buttons.

## Phase 4: Web Worker Infrastructure

- [ ] Create `src/worker.js` skeleton.
- [ ] Implement Main-to-Worker messaging handshake.
- [ ] Implement hot-sync for Config/Settings.

## Phase 5: Binary Serialization Protocol

- [ ] Define binary layout in `serializer.js`.
- [ ] Implement `Serializer.serialize()` in Worker.
- [ ] Implement binary parser in `render.js`.

## Phase 6: Performance Optimization (Grid)

- [ ] Implement fixed-size `Int32Array` `FlatSpatialHash`.
- [ ] Port Worker collision detection to use optimized grid.

## Phase 7: Background Visuals

- [ ] Implement procedural Starfield.
- [ ] Optimize background drawing (viewport culling).

## Phase 8: Entity Visuals

- [ ] Restore Snake Head/Eye Rendering.
- [ ] Implement Neon Glow particle effects.

## Phase 9: God Mode

- [ ] Implement Screen-to-World coordinate mapping.
- [ ] Implement "Kill" command for entities.
- [ ] Implement "Drag/Move" command.

## Phase 10: Analytics & Graphs

- [ ] Implement Fitness History logging.
- [ ] Integrate Charting (Min/Avg/Max).
- [ ] Implement Brain Visualizer (Activation heatmap).
