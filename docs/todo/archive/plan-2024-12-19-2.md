# Plan (Verbose)

## Review Summary (Codebase sweep)

I reviewed the full project tree and key runtime modules to anchor this plan:

- Entry/UI: `index.html`, `styles.css`, `src/main.js`
- Worker + simulation core: `src/worker.js`, `src/world.js`, `src/snake.js`
- Render/serialization: `src/render.js`, `src/serializer.js`
- AI + sensors: `src/mlp.js`, `src/sensors.js`
- Config + settings: `src/config.js`, `src/settings.js`
- Visualizations: `src/BrainViz.js`, `src/FitnessChart.js`, `src/chartUtils.js`
- Persistence: `src/storage.js`, `src/hallOfFame.js`
- Tests: `src/*.test.js`
- Docs: `README.md`, `.github/copilot-instructions.md`

Recent fixes and additions (from previous work) include:

- Worker-based fast rendering with serialized buffers, starfield background, boost trails.
- Fitness history min/avg/max tracking and brain viz activation heat strips.
- A NaN-first-frame regression fixed by initializing `bestPointsThisGen`.

## Goals (User request, interpreted strictly)

1) Expand tests to prevent recurrence of the specific runtime regressions.
2) Improve inline comments in the exact areas that were fragile/broken.
3) Move dev/architecture/testing guidance out of `README.md` and into `.github/copilot-instructions.md`.
4) Rewrite README for users/QA: explain every slider, give sim presets, explain MLP vs GRU, and document expected behavior.
5) Archive the previous plan and replace with a new, detailed plan (this file).

---

## Work Plan

### 1) Tests: add regression coverage

Focus on issues we actually hit: NaNs in first generation, buffer parsing errors, worker→UI history sync, default settings safety.

Planned additions:

- **`src/world.test.js`**:
  - Add “default settings are safe” test: `new World({})` should not throw; `viewMode` should be `"overview"`; `bestPointsThisGen` finite.
  - Add first-tick finite check: after one update, all alive snakes have finite `x/y/dir`, and `lastSensors` contain only finite numbers (prevents gen-1 invisibility).
  - Add fitness history structure check after `_endGeneration()` (min/avg/best should be finite).
- **`src/serializer.test.js`**:
  - Add pellet colorId test: set a pellet with `colorId != 0` and ensure serialized buffer contains it in the expected position.
- **`src/render.test.js`**:
  - Extend integration test to instantiate a real `World`, update once, serialize, and render. Assert aliveCount > 0 and drawing calls occur.
  - Optional: boosted snake buffer should not throw and should emit additional arc calls (smoke trail).
- **`src/main.test.js`**:
  - Simulate a `frame` message containing `fitnessHistory` and confirm `window.currentWorld.fitnessHistory` updates with min/avg/max (guards history mapping bugs).

Notes:

- Keep tests fast by temporarily lowering pellet target counts and generation durations.
- Restore `CFG` after tests to avoid cross-test contamination.

### 2) Inline comments where we broke things

Add comments only in the fragile spots that have historically caused regressions:

- **`src/world.js`**:
  - `bestPointsThisGen` init and update: explain why it must be finite before any sensor pass.
- **`src/render.js`**:
  - Explicit buffer layout contract (header + per-snake + pellet block).
  - Pointer math comment to explain why `ptCount` is parsed exactly and how pellet offset is determined.
- **`src/worker.js`**:
  - Explain stats payload shape and when history is shipped.
- **`src/main.js`**:
  - Explain history merge/rollover behavior and why the camera/zoom must be driven by the worker buffer.

### 3) README rewrite (user/QA focused)

Move all dev-only details to Copilot instructions; rewrite README to include:

**A. Quick start (user/QA)**  

- Install, `npm install`, `npm run dev`, open `localhost`.
- Reminder: ES modules require dev server; `index.html` won’t open directly.

## B. Controls

- `V` toggle view.
- Mouse behavior (God Mode if enabled): select, kill, drag.
- What “Apply and reset” vs “Defaults” do.

## C. Slider glossary (complete, per-group)

- Core sliders: NPC snakes, simulation speed, layer count, neurons per layer.
- Grouped settings in Settings panel:
  - World and food (radius, pellet target, spawn rate, food value, grow per food).
  - Snake physics (speed, boost speed, turn rate, radius, thickness scale, spacing, start/max/min length, size penalties).
  - Boost and mass (points cost, length loss, pellet drops).
  - Collision (substep dt, skip segments, hit scale, grid size, neighbor range).
  - Evolution (gen duration, elite fraction, mutation rate/std, crossover rate).
  - Observer and camera (focus switches, early end settings, zoom lerps, overview padding).
  - Rewards (points and fitness weights).
  - Brain and memory (use GRU, GRU size, control dt, GRU mutation and crossover).
  - Misc (dt clamp).

## D. MLP vs GRU explanation

- MLP: stateless, reacts only to current sensors; good for simple behaviors.
- GRU: includes memory; smoother turning, momentum-like tactics, better long-term planning; slower and more sensitive to mutation.

## E. Preset recipes (QA-friendly)

- “Fast evolution / quick iterations” (smaller world, shorter generations, higher mutation).
- “Stable survival” (lower mutation, longer generations, moderate rewards for survival).
- “Aggressive combat” (higher pointsPerKill and fitnessKill).
- “Exploration/foraging” (higher pointsPerFood + fitnessFood).
- “Memory-heavy” (enable GRU, higher hidden size, lower mutation std).
- Provide explicit slider ranges (not just prose).

## F. Troubleshooting

- No snakes visible → reset, check generation duration, ensure sliders not extreme.
- Slow FPS → reduce snake count and pellet target.
- Visualizer shows “Fast Mode” → focus snake or disable fast path (if toggles exist).

### 4) Copilot instructions expansion (dev-focused)

Move architecture/testing content here and expand with:

- Worker/frame pipeline: where data is authoritative, message types, and invariants.
- Binary layout contract and rule: change serializer → update render + God Mode parsing + tests.
- Settings/CFG sync rules: core sliders, settings groups, worker init order.
- Fitness history updates and buffer size constraints.
- Visualizer pipeline (when stats.viz exists, how to maintain).
- Pitfalls: NaNs in `bestPointsThisGen`, invalid slider ranges, misaligned pointer increments, and localStorage in tests.
- Testing notes: how to run, what failures typically mean, “do not ignore” errors (NaN, buffer length mismatch).

### 5) Lint/format cleanup for README

- Fix heading hierarchy, blank lines around lists, and code fences.
- Use consistent bullet formatting (no inline HTML).
- Ensure line lengths are reasonable and avoid mixed list styles.

### 6) Final sanity pass

- Run focused tests (world/render/serializer/main) or full `npm test`.
- Summarize results and any remaining risks.

---

## Deliverables checklist

- [ ] New tests: world first-tick safety + defaults + fitness history.
- [ ] New tests: serializer colorId.
- [ ] New tests: render integration (real World update).
- [ ] New tests: main history merge.
- [ ] Comments in world/render/worker/main for fragile logic.
- [ ] README rewritten for user/QA audience with full slider glossary.
- [ ] Copilot instructions expanded with dev details and invariants.
- [ ] Lint-clean Markdown.
- [ ] Test results reported.
