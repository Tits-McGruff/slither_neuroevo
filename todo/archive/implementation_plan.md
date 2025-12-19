# Slither Neuroevolution 2.0: Granular Implementation Roadmap

This document outlines the detailed 10-phase plan for modernizing the simulation. Each phase is designed to be an isolated, testable unit of progress.

## Phase 0: Foundation & Verification (Testing)

**Goal:** Establish 100% functional certainty for existing logic before architectural shifts.

- **Detailed Explanation:** Before we move logic to Workers or optimize memory, we must have a "source of truth." Tests will ensure that math primitives and AI logic remain identical across refactors.
- **Key Tasks:**
  - Initialize testing framework (e.g., Vitest).
  1.  **Environment:** Setup Vitest or Jest.
  2.  **`utils.js`:** Test all math (lerp, clamp, angNorm, hypot).
  3.  **`mlp.js`:** Test neural net weight initialization and forward pass.
  4.  **`genome.js`:** Test mutation (perturbation) and crossover (structured inherit).
  5.  **`snake.js`:** Test physics (movement vectors) and radius/growth curves.
  6.  **`spatialHash.js`:** Test boundary checks, cell calculation, and object retrieval.
  7.  **`world.js`:** Test spawning counts and single-step physics updates.
  8.  **`sensors.js`:** Test sensor raycasting and input normalization.
  9.  **`serializer.js`:** Test binary encoding/decoding integrity.
  10. **`storage.js`:** Test persistence, size limits, and error handling.
  11. **`particles.js`:** Test particle life cycle and recycling.
  12. **`render.js`:** Test viewport culling and coordinate transformation logic.
  13. **`BrainViz.js` & `FitnessChart.js`:** Test data mapping and chart updates.
  14. **`hallOfFame.js`:** Test registry sorting and insertion logic.
  15. **`settings.js`:** Test UI config mapping and hot-swap logic.
  16. **`theme.js` & `config.js`:** Test object immutability/loading.
  17. **`worker.js` & `main.js`:** Integration tests for the message loop and state sync.
  18. **`chartUtils.js`:** Test data smoothing and normalization helpers.
- **Verification:** `npm test` passing for 100% of core simulation functions.

---

## Phase 1: Genome Serialization (Persistence Part A)

**Goal:** Convert complex class instances into portable data objects.

- **Audit Goal:** Solve the data loss problem on browser refresh.
- **Key Tasks:**
  - Implement `Genome.toJSON()`: Converts weights (Float32Array) and architecture to a plain object.
  - Implement `Genome.fromJSON()`: Reconstructs a full class instance from stored objects.
- **Verification:** A genome can be stringified and restored with bit-perfect weight preservation.

---

## Phase 2: Registry & Storage Bridge (Persistence Part B)

**Goal:** Implement the logic for tracking champions and syncing with `localStorage`.

- **Audit Goal:** "The best of every gen should get saved."
- **Key Tasks:**
  - Create `src/storage.js`: A robust wrapper for `localStorage` with error handling.
  - Create `Registry` logic: Manages a collection of the top 1-3 snakes from every generation.
- **Verification:** Reloading the page retains a historical record of generational bests in memory.

---

## Phase 3: Hall of Fame Interface (UI)

**Goal:** Allow users to visualize and interact with the champion registry.

- **Audit Goal:** "UI: Sortable table... Action: 'Spawn Selected' button."
- **Key Tasks:**
  - Implement the "Hall of Fame" UI tab.
  - Build a sortable data table (Gen, Fitness, Length).
  - Implement "Resurrect" functionality: Inject a champion genome back into the live simulation.
- **Verification:** Clicking "Resurrect" on a list item spawns a new snake with that champion's brain.

---

## Phase 4: Web Worker Infrastructure (Performance Part A)

**Goal:** Unblock the UI thread by offloading simulation logic.

- **Audit Goal:** "Parallelization (Web Workers)".
- **Key Tasks:**
  - Create `src/worker.js`.
  - Implement the `Main <-> Worker` message bridge.
  - Implement "Dynamic Settings Sync": UI sliders send updates to the worker in real-time.
- **Verification:** The simulation loop runs in the background, visible in the DevTools "Thread" list.

---

## Phase 5: Binary Serialization Protocol (Performance Part B)

**Goal:** Optimize data transfer using zero-copy binary buffers.

- **Audit Goal:** "Technical Stack Decision: Optimized JavaScript (TypedArrays)."
- **Key Tasks:**
  - Define a strict binary layout (Header -> Entity Blocks -> Pellet Blocks).
  - Implement `Serializer` in Worker and Parser in `render.js`.
- **Verification:** Main thread receives a single `ArrayBuffer` per frame and parses it for rendering without object allocations.

---

## Phase 6: Spatial Partition optimization (Performance Part C)

**Goal:** Eliminate O(N^2) complexity with a high-performance grid.

- **Audit Goal:** "Flat-Array Linked List... Zero GC overhead."
- **Key Tasks:**
  - Implement `FlatSpatialHash` using fixed-size `Int32Array` buffers.
  - Port collision resolving logic to use the new grid inside the Worker.
- **Verification:** Simulation handles 500+ snakes with no "stutter" from Garbage Collection.

---

## Phase 7: Background Visuals (Visual Excellence)

**Goal:** Restore procedural context for the simulation.

- **Audit Goal:** "Procedural Starfield... deterministic background."
- **Key Tasks:**
  - Implement `drawStarfield` in `render.js`.
  - Optimize to only draw stars currently within the camera camera viewport.
- **Verification:** Background dots move correctly relative to the camera and look like the original project.

---

## Phase 8: Entity Rendering Polish (Visual Excellence)

**Goal:** Restore the "Neon" aesthetic and morphological accuracy.

- **Key Tasks:**
  - Refine `drawSnakeStruct` (Head circles, Eyes).
  - Implement velocity-based glow and boost particle effects.
- **Verification:** Snakes look premium and distinctive (Neon Glow) compared to the placeholder dots.

---

## Phase 9: God Mode Interactions (Advanced UX)

**Goal:** Allow direct user intervention.

- **Audit Goal:** "God Mode: Click-to-Kill, Drag-to-Move."
- **Key Tasks:**
  - Coordinate mapping (Mouse Screen Space -> Worker World Space).
  - Implement "Kill" and "Teleport" signals to the simulation engine.
- **Verification:** Clicking a snake in the simulation kills it instantly; dragging it moves it to the cursor.

---

## Phase 10: Evolution Analytics (Advanced UX)

**Goal:** Provide professional-grade simulation metrics.

- **Audit Goal:** "Advanced Stats: Diversity graphs... Tabulated data."
- **Key Tasks:**
  - Implement a rolling buffer for generational performance stats.
  - Integrate a line chart for Fitness History (Min/Avg/Max).
  - Implement the "Brain Visualizer" (Neuron activation heatmap).
- **Verification:** The "Stats" tab shows a live-updating graph of evolution progress.
