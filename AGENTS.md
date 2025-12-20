# Agent instructions for slither_neuroevo

## Project overview and top-level layout

This repository is a browser-based neuroevolution simulation modeled after slither.io, built with Vite and ES modules. The user-facing entry point is `index.html`, which provides a full-screen canvas plus the control panel tabs, while `styles.css` defines the UI layout, tab visuals, and animation. The `README.md` explains how to run the dev server and why the project cannot be opened directly from the filesystem, so follow those workflow notes when suggesting run instructions.

At the top level, `package.json` and `package-lock.json` define the Node toolchain (`vite` for dev/build/preview and `vitest` for tests), and `vite.config.mjs` contains the non-default cache directory that avoids file-lock issues on network drives. Windows users have a convenience launcher in `play.bat` that installs dependencies and runs the dev server. Active planning notes live in `todo/plan.md`, while historical plans are archived under `todo/archive/`. CI is wired through `.github/workflows/node.js.yml`, which runs `npm ci`, `npm run build`, and `npm test` across multiple Node versions.

## Runtime architecture and data flow

The runtime is deliberately split between the main thread and a Web Worker. `src/main.ts` owns the DOM, canvas sizing, tab switching, settings sliders, and rendering, while `src/worker.ts` owns the physics, AI, and evolution loop by instantiating `World` from `src/world.ts`. The main thread spawns the worker with `new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' })`, stores a `proxyWorld` for UI state, and drives a requestAnimationFrame loop that renders the latest frame buffer into the canvas.

The worker runs a fixed-step loop (`FIXED_DT = 1/60`) inside `loop()` and posts a transferable buffer back to the main thread each iteration. The message protocol is simple and explicit: `init` rebuilds a world (and resets `CFG`), `updateSettings` applies a list of `path/value` updates to `CFG`, `action` is used for camera toggles, `resurrect` injects a saved genome, and `godMode` handles kill/move actions. When adding new messages or fields, update both ends (`src/main.ts` and `src/worker.ts`) and keep the tests or any parsing code in sync. Shared message and settings types live in `src/protocol/messages.ts` and `src/protocol/settings.ts`.

## Binary frame format and rendering pipeline

The fast path relies on a strict binary format for world snapshots. `src/serializer.ts` writes a `Float32Array` with a 6-float header (`generation`, `totalSnakes`, `aliveCount`, `cameraX`, `cameraY`, `zoom`), followed by a compact per-snake block and then the pellet block. Only alive snakes are serialized, and each snake starts with 8 floats (id, radius, skin flag, x, y, dir, boost flag, point count) followed by `pointCount * 2` floats for the body points. The pellet section starts with `pelletCount`, then repeats `(x, y, value, type, colorId)` where type is `0 ambient`, `1 corpse_big`, `2 corpse_small`, `3 boost`. Frame offsets and read helpers are centralized in `src/protocol/frame.ts`. The renderer uses this buffer to drive speed-based glow and boost trails, so pointer math must remain exact.

`src/render.ts` (`renderWorldStruct`) parses this buffer linearly to draw the grid, pellets, then snakes, and `src/main.ts` also parses it to support God Mode selection. Any layout change must be reflected in `src/serializer.ts`, `src/render.ts`, and the parsing logic in `src/main.ts` (for selection and camera), or you will get corrupted rendering and interactions. When you need to extend what the UI can see, prefer adding fields to the buffer rather than reintroducing heavy object cloning on the worker boundary.

## Simulation core: World, Snake, sensors, and physics

`src/world.ts` is the heart of the simulation. It builds the population based on the current settings (`buildArch` from `src/mlp.ts`), spawns snakes from genomes, manages pellets through a `PelletGrid` map, and orchestrates the per-tick update loop. Each `World.update()` scales the time step by `simSpeed`, clamps it against `CFG.dtClamp`, subdivides it according to `CFG.collision.substepMaxDt`, and then runs physics steps that spawn pellets, advance snakes, rebuild the collision grid, and resolve head-to-body collisions. Collisions are detected via `FlatSpatialHash` from `src/spatialHash.ts`, which stores segment midpoints in typed arrays to avoid per-frame allocations. When a collision is detected the victim dies, and kill points are awarded to the aggressor via `CFG.reward.pointsPerKill`.

The `Snake` class in `src/snake.ts` handles its own movement, boosting, feeding, and growth. It runs neural inference on a fixed controller timestep (`CFG.brain.controlDt`) so the GRU memory length stays stable even when physics substeps change, then converts outputs into turn and boost decisions. Boosting burns points and shrinks the snake while dropping pellets behind it (`CFG.boost`), and the death path (`Snake.die`) converts body mass into pellets based on `CFG.death` parameters. Movement uses the turn rate and speed penalties in `CFG`, clamps the snake within the world radius, updates segment positions to maintain spacing, then grows or shrinks towards `targetLen` while updating radius via a logarithmic length curve.

Sensors are built in `src/sensors.ts` and must stay in sync with `CFG.brain.inSize`. The sensor vector length is `5 + 3 * bubbleBins` where `bubbleBins` comes from `CFG.sense.bubbleBins`, and the first five values are heading sin/cos, size fraction, boost margin, and a log-scaled points percentile. The remaining values are three radial histograms (food density, hazard clearance, and wall distance) computed in a 360-degree bubble around the head. Sensor scanning uses both the pellet grid and the collision grid with work caps (`CFG.sense.maxPelletChecks` and `CFG.sense.maxSegmentChecks`), so if you expand inputs you should update both `CFG.brain.inSize` and any downstream UI that expects a fixed sensor size.

## Neural controllers and evolution

Neural architecture and genetics are centralized in `src/mlp.ts`. `buildArch(settings)` decides between a pure MLP and an MLP+GRU controller depending on `CFG.brain.useGRU`, using the user-selected hidden layer counts (`hiddenLayers`, `neurons1..5`) for the MLP sizes. `BrainController` wraps the MLP and optional GRU and head layers, and `Genome` stores weights in a `Float32Array` with `toJSON()`/`fromJSON()` for persistence compatibility. Keep `archKey()` stable if you care about loading older populations, and be mindful that `Genome.toJSON()` converts typed arrays into plain arrays for storage.

Evolution happens in `World._endGeneration()`, which computes fitness via `Snake.computeFitness()` (using `CFG.reward` weights), sorts the population, saves the best genome into the Hall of Fame, and breeds a new population using elite preservation plus tournament selection, crossover, and mutation. For GRU brains, `crossover()` and `mutate()` have a separate code path that treats the GRU block differently (block-wise or unit-wise crossover, distinct mutation rate/std from `CFG.brain.gruMutationRate` and `CFG.brain.gruMutationStd`). Any change to these operators should consider both the MLP-only and MLP+GRU cases.

## Rendering, theme, and particles

Rendering is split into a fast path and a legacy path. The worker-driven path (`renderWorldStruct` in `src/render.ts`) draws from the serialized buffer, uses `THEME` and `getPelletColor`/`getPelletGlow` from `src/theme.ts`, assigns snake colors with `hashColor` in `src/utils.ts` unless the gold skin flag is set, and adds speed/boost-based glow plus boost trails. The legacy path (`renderWorld`) draws directly from a `World` instance and includes extra overlays and particle rendering, which is useful for debugging but is not the default in worker mode.

`src/particles.ts` implements a pooled particle system with additive blending and is updated inside `World.update()` and rendered inside `renderWorld`. Those particles are not serialized into the worker buffer; the fast path instead uses lightweight render-side boost trails driven by the serialized boost flag and speed estimate. `src/theme.ts` is the single source of truth for palette and glow colors, so keep visual changes centralized there rather than scattering hard-coded colors across the renderer.

## UI, settings, and visualization panels

`index.html` defines the tabbed control panel (Settings, Visualizer, Stats, Hall of Fame) plus the God Mode log panel. `styles.css` implements the panel layout, sliders, tab buttons, and simple entry transitions. `src/main.ts` wires these DOM elements to the worker, holds a `currentFrameBuffer`, and uses a `proxyWorld` to expose minimal world-like methods (`toggleViewMode`, `resurrect`) to the UI and Hall of Fame code. The God Mode interactions (click to select, right-click to kill, drag to move) depend on parsing the buffer and converting screen coordinates to world coordinates using the camera values embedded in the frame header.

The settings system is in `src/settings.ts`, which constructs grouped sliders from `SETTING_SPECS` and uses `data-path` attributes to map slider values into `CFG` via `setByPath` from `src/utils.ts`. Sliders marked `requiresReset` only apply on world reset; live sliders call back to `src/main.ts`, which posts incremental updates to the worker. The top-level core sliders (snake count, sim speed, layer counts, neuron sizes) are wired directly in `src/main.ts` and must stay aligned with `buildArch()` in `src/mlp.ts` and defaults in `src/config.ts`.

Visualization helpers live in `src/BrainViz.ts`, `src/FitnessChart.ts`, and `src/chartUtils.ts`. The Brain Visualizer renders activation heat strips when the worker sends `stats.viz` data (enabled via the Visualizer tab; `src/main.ts` posts a `viz` message to toggle streaming). The Stats tab uses a chart selector to render fitness, species diversity, and network complexity from `fitnessHistory` entries sent by the worker (min/avg/max plus species/complexity metrics).

## Persistence and Hall of Fame

Persistence utilities are in `src/storage.ts`, which provides a small `Storage` wrapper and explicit population save/load helpers keyed by `slither_neuroevo_pop`. The Hall of Fame in `src/hallOfFame.ts` stores the top 50 snakes by fitness in `slither_neuroevo_hof` and is populated from `World._endGeneration()`. In the UI, `window.spawnHoF` is defined in `src/main.ts` and calls `proxyWorld.resurrect()` so the worker can spawn the saved genome, which also triggers follow mode and re-centers the camera on the resurrected snake.

Import/export is exposed in the Settings tab and now syncs through the worker. `src/main.ts` requests an export payload from the worker, adds HoF data, and downloads a JSON file. Imports validate the JSON, update the Hall of Fame store, persist population JSON in localStorage, and send the genomes to the worker for an in-place reset.

## Utilities and configuration

`src/config.ts` owns the full configuration surface in `CFG_DEFAULT`, and `resetCFGToDefaults()` rebuilds `CFG` by deep-cloning via JSON. This means new config entries should remain JSON-serializable and should be added to both `CFG_DEFAULT` and the UI slider specs if you want them exposed. `src/utils.ts` provides common math helpers, random utilities, and the color hashing used for snake skins. These helpers are used in hot loops, so prefer reusing them rather than introducing extra per-frame allocations.

## Documentation expectations

- `README.md` is for users and QA: explain sliders, brain types, presets, and troubleshooting. Keep dev architecture and testing details out of README.
- `AGENTS.md` is the dev reference: include system architecture, buffer contracts, and regression pitfalls.
- Slider names and meanings in README should mirror `src/settings.ts` and `src/config.ts`. If you change a slider label or path, update docs accordingly.

## Recent additions and footguns

- Fast-path visuals include speed/boost-based glow and boost trail particles in `renderWorldStruct`. The buffer contract is: header (6 floats), alive snakes (8 + 2*ptCount floats), then pellets (count + 5 floats: x, y, value, type, colorId). Update serializer, render, and tests together if this changes.
- Starfield/grid draw in the fast renderer; camera/zoom must come from the worker buffer (no main-thread overrides).
- Fitness history now ships min/avg/max from the worker when it grows. Keep histories finite and capped when syncing to the UI.
- `bestPointsThisGen` must be initialized before any sensor pass; NaNs here made first-generation snakes vanish. Preserve that initialization when refactoring world state or sensors.

## Tests and verification

The automated tests are plain Vitest suites under `src/*.test.ts`. Coverage spans module-level behaviors (MLP, sensors, world, render, serializer) plus integration guards (first-tick world safety, render-from-serialized buffers, main/worker history sync). Run them with `npm test` (which maps to `vitest run`), and keep in mind they are Node-based unit tests rather than browser integration tests.

## Build, dev, and CI workflows

The normal workflow is:

```bash
npm install
npm run dev
```

This runs Vite with ES module support and serves the app from a local server (opening `index.html` directly will not work). For production builds use `npm run build` and `npm run preview`. `play.bat` automates this on Windows and uses `npm run dev -- --open --force` so the browser opens automatically. In CI, `.github/workflows/node.js.yml` runs install, build, and test across Node 18/20/22.

## Project-specific conventions and gotchas

Performance is a constant concern in this codebase. Hot paths avoid allocations and prefer typed arrays (`Float32Array` for network weights and serialization buffers, and typed arrays plus an object list in `FlatSpatialHash`), so when adding new per-frame data keep GC pressure low. The worker buffer layout is a hard contract: modify it only if you also update `renderWorldStruct` and the God Mode parsing in `src/main.ts`. There is also a legacy render path (`renderWorld`) that references a `drawSnake` helper not defined in `src/render.ts`, which is a signal that the non-worker renderer is not the current focus; if you revive it, audit that path carefully and supply any missing drawing helpers.

Keep the sensors and brain input size aligned. If you change `CFG.sense.bubbleBins` or adjust the sensor vector layout, update `CFG.brain.inSize` and any code that assumes a fixed input length (including BrainViz or any debug panels). Similarly, changes to genetic operators or architecture keys can invalidate saved genomes in localStorage, so consider how `Genome.toJSON()` and `archKey()` are used before altering their output.

If any section here feels unclear or you want deeper coverage (for example, the collision math, the exact fitness weighting, or how God Mode parsing walks the buffer), tell me which part to expand so we can iterate.

## TypeScript policy (in-progress conversion)

- Keep runtime behavior and performance identical; types must not alter logic or hot-loop allocations.
- Use strict typechecking (`tsconfig.json`, `noEmit`) and convert files in dependency order.
- Prefer shared protocol types under `src/protocol/` for worker/main message contracts.
