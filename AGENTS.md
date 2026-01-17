# Agent instructions for slither_neuroevo

## Project overview and top-level layout

This repository is a browser-based neuroevolution simulation modeled after slither.io, built with Vite and ES modules. The user-facing entry point is `index.html`, which provides a full-screen canvas plus the control panel tabs, while `styles.css` defines the UI layout, tab visuals, and animation. The `README.md` explains how to run the dev server and why the project cannot be opened directly from the filesystem, so follow those workflow notes when suggesting run instructions.

At the top level, `package.json` and `package-lock.json` define the Node toolchain (`vite` for dev/build/preview and `vitest` for tests), and `vite.config.ts` contains the non-default cache directory that avoids file-lock issues on network drives. TypeScript configuration lives in `tsconfig.json` with server overrides in `server/tsconfig.json`, and lint rules live in `eslint.config.cjs` (including TSDoc enforcement for `src/`, `server/`, and `scripts/`). The Node server lives under `server/` and persists data in `data/slither.db` (SQLite). Server defaults are in `server/config.toml`, loaded by `server/config.ts`, and `vite.config.ts` reads the TOML (or `SERVER_CONFIG`) to inject UI defaults. Test grouping utilities live in `scripts/run-tests.ts`. Windows users have a convenience launcher in `play.bat`, and POSIX users can use `play.sh`, both installing dependencies and running the dev server. Active planning notes live in `docs/todo/*.md`, while historical plans are archived under `docs/todo/archive/`. External feedback notes live in `docs/feedback-from-outside-llms/`. CI is wired through `.github/workflows/node.js.yml`, which runs `npm ci`, `npm run build`, `npm run typecheck`, and `npm test` across Node 20/22/24 on Ubuntu and Windows.

## Runtime architecture and data flow

The runtime has two modes: a Node server that owns the `World` and streams frames over WebSocket, and a local Web Worker fallback when the server is unavailable. `src/main.ts` owns the DOM, canvas sizing, tab switching, settings sliders, and rendering. It connects to the server with `src/net/wsClient.ts` (see `server/protocol.ts` for message shapes) and falls back to a worker via `new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' })` if the WS connection fails.

Server mode is implemented in `server/index.ts` + `server/simServer.ts` + `server/wsHub.ts`, with player/session routing in `server/controllerRegistry.ts` and the HTTP API in `server/httpApi.ts`. The server runs the fixed-step loop, serializes frames with `WorldSerializer`, and sends a binary buffer plus stats messages. Clients send `hello`, `join`, `ping`, `action`, `view` (viewport + follow/overview toggle), `viz` (visualizer streaming), and `reset` messages; the server replies with `welcome`, `stats`, `assign`, `sensors`, `error`, and the raw frame buffer. Player control relies on per-snake sensor messages and tick-aligned actions; when a controlled snake dies, the controller registry spawns a fresh external snake and emits a new `assign`.

On a successful server connection, the client auto-joins as a spectator, enables Visualizer streaming when the tab is active, and shows the join overlay until the user submits a nickname. Overview view is requested when the user explicitly spectates or toggles view mode.

If the server handshake fails, a 2-second fallback starts the worker, hides the join overlay, and keeps reconnection attempts running in the background.

Server URLs resolve from `?server=ws://...`, then the `slither_server_url` localStorage key, then the build-time defaults injected by `vite.config.ts` (from `server/config.toml` or `SERVER_CONFIG`), and finally the runtime hostname + injected server port fallback (default `ws://localhost:5174`).

Worker mode runs the same loop inside `src/worker.ts` and posts a transferable buffer back to the main thread each iteration. The worker message protocol is explicit: `init` rebuilds a world (and can reset `CFG`), `updateSettings` applies `path/value` updates to `CFG`, `action` handles view/sim speed toggles, `resize` updates the viewport, `viz` toggles streaming, `resurrect` injects a saved genome, `import`/`export` drive population transfer, and `godMode` handles kill/move actions. `init` can include `graphSpec`, `population`, `generation`, and `stackOrder` to override the brain layout and state. Worker responses include `frame` (buffer + stats), `exportResult`, and `importResult`. When adding new messages or fields, update both ends (`src/main.ts` and `src/worker.ts`) and keep tests or parsing code in sync. Shared worker message and settings types live in `src/protocol/messages.ts` and `src/protocol/settings.ts`.

### Multi-threaded inference pool

To overcome the single-threaded bottleneck of JavaScript, the simulation now employs a parallel inference engine (`src/workerPool.ts`) that distributes neural network forward passes across multiple dedicated worker threads (`src/worker/inferWorker.ts`). This system is designed for zero-copy synchronization using `SharedArrayBuffer` and `Atomics`.

1. **Initialization**: On startup, `src/workerPool.ts` spawns `navigator.hardwareConcurrency - 1` workers. It allocates three primary shared buffers:
    * **Inputs**: A flat f32 buffer storing sensor data for all agents.
    * **Outputs**: A flat f32 buffer where workers write turn/boost decisions.
    * **Weights**: A large buffer storing the optimized network weights for the entire population.
2. **Dispatch**: During the game loop, the main simulation worker writes sensor data to the Shared Input Buffer. It then dispatches batches of agents to the worker pool by writing atomic flags. Workers wake up, read their assigned slice of inputs, execute the WASM inference kernels, and write directly to the Shared Output Buffer.
3. **Synchronization**: The main thread waits (via `Atomics.wait` or a spin-lock fallback) for all workers to signal completion before proceeding to apply the control outputs. This ensures deterministic lock-step execution.
4. **Fallback**: If the environment lacks `SharedArrayBuffer` support (e.g., due to missing `Cross-Origin-Opener-Policy: same-origin` headers), the pool detects this capability failure and gracefully disables itself. The simulation then reverts to the legacy single-threaded JS/SIMD loop (`BatchInferenceRunner`), ensuring the application remains functional on restrictive hosts.

## Binary frame format and rendering pipeline

The fast path relies on a strict binary format for world snapshots. `src/serializer.ts` writes a `Float32Array` with a 7-float header (`generation`, `totalSnakes`, `aliveCount`, `worldRadius`, `cameraX`, `cameraY`, `zoom`), followed by a compact per-snake block and then the pellet block. Only alive snakes are serialized, and each snake starts with 8 floats (id, radius, skin flag, x, y, dir, boost flag, point count) followed by `pointCount * 2` floats for the body points. The pellet section starts with `pelletCount`, then repeats `(x, y, value, type, colorId)` where type is `0 ambient`, `1 corpse_big`, `2 corpse_small`, `3 boost`. Frame offsets and read helpers are centralized in `src/protocol/frame.ts`. The renderer uses this buffer to drive speed-based glow and boost trails, so pointer math must remain exact. Server and worker modes share this exact buffer layout.

`src/render.ts` (`renderWorldStruct`) parses this buffer linearly to draw the grid, pellets, then snakes, and `src/main.ts` also parses it to support God Mode selection. Any layout change must be reflected in `src/serializer.ts`, `src/render.ts`, and the parsing logic in `src/main.ts` (for selection and camera), or you will get corrupted rendering and interactions. When you need to extend what the UI can see, prefer adding fields to the buffer rather than reintroducing heavy object cloning on the worker boundary.

## Simulation core: World, Snake, sensors, and physics

`src/world.ts` is the heart of the simulation. It builds the population based on the current settings (`buildArch` from `src/mlp.ts`), spawns snakes from genomes, manages pellets through a `PelletGrid` map, and orchestrates the per-tick update loop. Each `World.update()` scales the time step by `simSpeed`, clamps it against `CFG.dtClamp`, subdivides it according to `CFG.collision.substepMaxDt`, and then runs physics steps that spawn pellets, advance snakes, rebuild the collision grid, and resolve head-to-body collisions. Collisions are detected via `FlatSpatialHash` from `src/spatialHash.ts`, which stores segment midpoints in typed arrays to avoid per-frame allocations. When a collision is detected the victim dies, and kill points are awarded to the aggressor via `CFG.reward.pointsPerKill`. Ambient food spawning uses a "Fractal Food" algorithm (`_spawnAmbientPellet`), employing interference noise and rejection sampling to create filaments and voids, encouraging movement and strategy.

The `Snake` class in `src/snake.ts` handles its own movement, boosting, feeding, and growth. It runs neural inference on a fixed controller timestep (`CFG.brain.controlDt`) so recurrent memory length stays stable even when physics substeps change, then converts outputs into turn and boost decisions. Boosting burns points and shrinks the snake while dropping pellets behind it (`CFG.boost`), and the death path (`Snake.die`) converts body mass into pellets based on `CFG.death` parameters. Movement uses the turn rate and speed penalties in `CFG`, clamps the snake within the world radius, updates segment positions to maintain spacing, then grows or shrinks towards `targetLen` while updating radius via a logarithmic length curve.

Sensors are built in `src/sensors.ts` and must stay in sync with `CFG.brain.inSize`. The sensor vector length is `5 + 3 * bubbleBins`, where `bubbleBins` is `Math.max(8, floor(CFG.sense.bubbleBins))` (default 12), and the first five values are heading sin/cos, size fraction, boost margin, and a log-scaled points percentile. The remaining values are three radial histograms (food density, hazard clearance, and wall distance) computed in a 360-degree bubble around the head. Sensor scanning uses both the pellet grid and the collision grid with work caps (`CFG.sense.maxPelletChecks` and `CFG.sense.maxSegmentChecks`), so if you expand inputs you should update both `CFG.brain.inSize` and any downstream UI that expects a fixed sensor size.

## Baseline Bot Strategies

Baseline bots (`src/bots/baselineBots.ts`) are scripted entities that fill the arena. They now employ "Life Stage" strategies based on their length:

* **Small (< 25)**: "Coward" mode. Prioritizes high clearance and clamps food attraction to avoid kamikaze deaths.
* **Medium (25-80)**: "Hunter" mode. Actively intercepts nearby snakes and boosts to attack if safe.
* **Large (> 80)**: "Bully" mode. Seeks high density to block paths and cause accidents.

Bot respawning is controlled by `CFG.baselineBots.respawnDelay` (default 3.0s), ensuring a steady population without instant flooding.

## Neural controllers and evolution

Neural architecture and genetics are centralized in `src/mlp.ts` plus the brain registry and builders under `src/brains/` (`registry.ts`, `stackBuilder.ts`, `graph/schema.ts`). `buildArch(settings)` uses the active graph spec (`CFG.brain.graphSpec`) when set (from the graph editor or JSON import), otherwise it falls back to the legacy stack builder. Graph specs must match `CFG.brain.inSize` and `CFG.brain.outSize` or they are ignored. `Genome` stores weights in a `Float32Array` with `toJSON()`/`fromJSON()` for persistence compatibility. Keep `archKey()` stable if you care about loading older populations, and be mindful that `Genome.toJSON()` converts typed arrays into plain arrays for storage.

Evolution happens in `World._endGeneration()`, which computes fitness via `Snake.computeFitness()` (using `CFG.reward` weights), sorts the population, saves the best genome into the Hall of Fame, and breeds a new population using elite preservation plus tournament selection, crossover, and mutation. Recurrent nodes (GRU/LSTM/RRU) use block-wise or unit-wise crossover via `CFG.brain.gruCrossoverMode`, and share the recurrent mutation rate/std (`CFG.brain.gruMutationRate` / `CFG.brain.gruMutationStd`) for their parameter ranges. Any change to these operators should consider both feed-forward and recurrent blocks.

## Rendering, theme, and particles

Rendering is split into a fast path and a legacy path. The serialized-buffer path (`renderWorldStruct` in `src/render.ts`) draws from the binary frame buffer produced by either the server or the worker, uses `THEME` and `getPelletColor`/`getPelletGlow` from `src/theme.ts`, assigns snake colors with `hashColor` in `src/utils.ts` unless the gold skin flag is set, and adds speed/boost-based glow plus boost trails. The legacy path (`renderWorld`) draws directly from a `World` instance and includes extra overlays and particle rendering, which is useful for debugging but is not the default in worker or server mode.

`src/particles.ts` implements a pooled particle system with additive blending and is updated inside `World.update()` and rendered inside `renderWorld`. Those particles are not serialized into the worker buffer; the fast path instead uses lightweight render-side boost trails driven by the serialized boost flag and speed estimate. `src/theme.ts` is the single source of truth for palette and glow colors, so keep visual changes centralized there rather than scattering hard-coded colors across the renderer.

## UI, settings, and visualization panels

`index.html` defines the tabbed control panel (Settings, Visualizer, Stats, Hall of Fame) plus the God Mode log panel and the join overlay. `styles.css` implements the panel layout, sliders, tab buttons, the join overlay, and simple entry transitions. `src/main.ts` wires these DOM elements to the worker or server connection, holds a `currentFrameBuffer`, and uses a `proxyWorld` to expose minimal world-like methods (`toggleViewMode`, `resurrect`) to the UI and Hall of Fame code. The God Mode interactions (click to select, right-click to kill, drag to move) depend on parsing the buffer and converting screen coordinates to world coordinates using the camera values embedded in the frame header. The Settings lock hides `#settingsControls` to keep sliders out of reach, and the join overlay requires a nickname before player control is enabled.

The settings system is in `src/settings.ts`, which constructs grouped sliders from `SETTING_SPECS` and uses `data-path` attributes to map slider values into `CFG` via `setByPath` from `src/utils.ts`. Sliders marked `requiresReset` only apply on world reset; live sliders call back to `src/main.ts`, which posts incremental updates to the worker. The top-level core sliders (snake count, sim speed, layer counts, neuron sizes) are wired directly in `src/main.ts` and must stay aligned with `buildArch()` in `src/mlp.ts` and defaults in `src/config.ts`. Brain layouts are edited via the unified graph editor in the Settings tab (nodes/edges/outputs + templates), with optional JSON import/export for advanced edits.

The diagram is interactive: drag nodes to reposition (layout overrides are UI-only), toggle Connect mode to add edges by clicking start/target nodes, and use the inspector to edit node/edge/output fields.

Connect mode auto-assigns Split/Concat ports, and Full screen uses a backdrop that leaves the right-hand control panel visible.

The diagram toolbar supports Add node/output, Delete, Auto layout, and Full screen.

Saved presets load from the server DB, and the current graph draft can be applied to reset the world.

Graph preset lists come from `/api/graph-presets` and stay empty in worker mode.

Advanced JSON controls (Load JSON into editor, Copy current graph, Export JSON) live under the optional details panel.

`src/main.ts` persists the applied graph spec to localStorage (`slither_neuroevo_graph_spec`) and reloads it on startup, falling back to the default template if invalid.

Visualization helpers live in `src/BrainViz.ts`, `src/FitnessChart.ts`, and `src/chartUtils.ts`. The Brain Visualizer renders activation heat strips when the worker or server sends `stats.viz` data (enabled via the Visualizer tab; `src/main.ts` posts a `viz` message to toggle streaming). The Stats tab uses a chart selector to render fitness, species diversity, and network complexity from `fitnessHistory` entries sent by the worker or server (min/avg/max plus species/complexity metrics).

## Persistence and Hall of Fame

Persistence utilities are in `src/storage.ts`, which provides a small `Storage` wrapper and explicit population save/load helpers keyed by `slither_neuroevo_pop`. The Hall of Fame in `src/hallOfFame.ts` stores the top 50 snakes by fitness in `slither_neuroevo_hof` and is populated from `World._endGeneration()`. In the UI, `window.spawnHoF` is defined in `src/main.ts` and calls `proxyWorld.resurrect()` so the worker can spawn the saved genome, which also triggers follow mode and re-centers the camera on the resurrected snake.

Import/export is exposed in the Settings tab and uses the worker protocol in worker mode or the server HTTP endpoints in server mode. `src/main.ts` requests an export payload from the worker, adds HoF data, and downloads a JSON file; in server mode it posts `/api/save` then fetches `/api/export/latest` before downloading. Imports validate the JSON, update the Hall of Fame store, persist population JSON in localStorage, and send the genomes to the worker for an in-place reset; in server mode it posts `/api/import`. Server-side persistence (`server/persistence.ts`) stores population snapshots plus graph presets in SQLite (`data/slither.db`); `server/httpApi.ts` exposes `/api/save`, `/api/export/latest`, `/api/import`, `/api/graph-presets` (list/save), and `/api/graph-presets/:id` (load) for DB-backed workflows, while export still writes JSON to the client file system. Diagram layout overrides are not persisted.

### Scalable server persistence (Chunked blob format)

Previous versions of the server stored population snapshots as a single massive JSON string in SQLite. As population sizes and neural complexity grew, this approach hit the V8 engine's hard string length limit (approximately 512MB), causing the server to crash when saving Generation 100+ with 300 snakes.

To resolve this, the persistence layer (`server/persistence.ts`) now employs a **Chunked Binary Serialization** strategy:

1. **Genome Stripping**: When a snapshot is saved, the massive `genomes` array is removed from the metadata payload. The lightweight metadata (stats, settings, hall of fame) is saved as normal JSON in the `payload_json` column.
2. **Binary Serialization**: The genomes are handed off to `src/persistence/chunked.ts`, which serializes them into a compact binary format. This format is broken into 512MB chunks (if necessary) to respect Node buffer limits.
3. **Compression**: The binary chunks are compressed using Gzip (via Node's `zlib`).
4. **Blob Storage**: The resulting compressed buffer is stored in a dedicated `genomes_blob` BLOB column in the `population_snapshots` table.

This architecture ensures that the application can scale to thousands of complex agents without memory crashes. When loading a snapshot (`loadLatestSnapshot`), the server strictly reverses this process: reading the blob, gunzipping, parsing the binary chunks, and re-attaching the genomes to the JSON payload before passing it to the simulation controller. Existing database migrations (`ensureSnapshotColumns`) handle the schema update automatically.

## Rust & WASM Toolchain

The high-performance numerics are backed by a Rust crate located in `wasm/`. This crate compiles to a WebAssembly module that exposes SIMD-accelerated kernels (`dense_forward`, `lstm_step`, etc.) to the JavaScript runtime.

**Build Pipeline**:
The build process is automated via `scripts/build-wasm.mjs`. Use `npm run build` to invoke the pipeline, which:

1. Calls `cargo build --target wasm32-unknown-unknown --release`.
2. Optimizes the output binary using `wasm-opt` (if available) or internal shrinking flags.
3. Copies the resulting `.wasm` file to the `public/` directory for Vite consumption.

**Safety Invariants**:
Because the WASM module operates on raw pointers passed from JavaScript (`SharedArrayBuffer` views), memory safety is manual and critical.

* **Unsafe Blocks**: All raw pointer arithmetic in `lib.rs` is wrapped in explicit `unsafe {}` blocks. Every such block MUST be accompanied by a `/// # Safety` documentation comment explaining the contract (e.g., "Pointers must be valid for `len` elements").
* **Slice Copying**: Manual `for` loops that copy data byte-by-byte are banned. Use `slice.copy_from_slice()` instead, as it compiles to efficient `memcpy` intrinsics and allows the Rust compiler to elide bounds checks where possible.
* **Linting**: The CI pipeline enforces `cargo clippy` and `cargo fmt`. You can run these locally with `npm run lint:rust` and `npm run format:rust`.
* **Testing**: `npm run test:rust` acts as a compile-check for the WASM target (via `--no-run`), while `npm test` runs the actual behavioral integration tests (`server/mtParity.test.ts`) using the verified binary.

## Utilities and configuration

`src/config.ts` owns the full configuration surface in `CFG_DEFAULT`, and `resetCFGToDefaults()` rebuilds `CFG` by deep-cloning via JSON. This means new config entries should remain JSON-serializable and should be added to both `CFG_DEFAULT` and the UI slider specs if you want them exposed. `src/utils.ts` provides common math helpers, random utilities, and the color hashing used for snake skins. These helpers are used in hot loops, so prefer reusing them rather than introducing extra per-frame allocations.

## Documentation expectations

* `README.md` is for users and QA: explain sliders, brain types, presets, and troubleshooting. Keep dev architecture and testing details out of README.
* `AGENTS.md` is the dev reference: include system architecture, buffer contracts, and regression pitfalls.
* Slider names and meanings in README should mirror `src/settings.ts` and `src/config.ts`. If you change a slider label or path, update docs accordingly.
* Add TSDoc-style documentation for every function, class, class field, and module-level variable (including tests and server/util scripts), plus inline comments for behavior not covered by the docblocks. Linting is configured in `eslint.config.cjs` and enforced via `npm run lint`.

## Recent additions and footguns

* Fast-path visuals include speed/boost-based glow and boost trail particles in `renderWorldStruct`. The buffer contract is: header (7 floats), alive snakes (8 + 2\*ptCount floats), then pellets (count + 5 floats: x, y, value, type, colorId). Update serializer, render, and tests together if this changes.
* Starfield/grid draw in the fast renderer; camera/zoom must come from the worker buffer (no main-thread overrides).
* Fitness history now ships min/avg/max from the worker or server when it grows. Keep histories finite and capped when syncing to the UI.
* `bestPointsThisGen` must be initialized before any sensor pass; NaNs here made first-generation snakes vanish. Preserve that initialization when refactoring world state or sensors.
* Graph editor ports are 0-based; Split output sizes must sum to the input size, and total output size must match `CFG.brain.outSize` (turn+boost). Diagram layout overrides are UI-only.
* `data/slither.db` is local server state and should not be committed; it will exceed GitHub size limits if tracked.
* `better-sqlite3` is a native dependency; Windows installs need C++ build tools and a Windows SDK.

## Tests and verification

The automated tests are plain Vitest suites under `src/*.test.ts` and `server/*.test.ts`. Coverage spans module-level behaviors (brains, sensors, world, render, serializer) plus server integration and persistence guards (first-tick world safety, render-from-serialized buffers, main/worker history sync, protocol validation). Run them with `npm test` (which maps to `vitest run`) or the category scripts (`npm run test:unit`, `npm run test:integration`, `npm run test:system`, `npm run test:acceptance`, `npm run test:regression`, `npm run test:performance`, `npm run test:security`) powered by `scripts/run-tests.ts`. Keep in mind these are Node-based tests rather than browser integration tests.

## Build, dev, and CI workflows

The normal workflow is:

```bash
npm install
npm run server
npm run dev
```

This runs Vite with ES module support and serves the app from a local server (opening `index.html` directly will not work). For production builds use `npm run build` and `npm run preview`. Use `npm run server` (or `npm run server:dev`) to launch the Node simulation server. `play.bat` automates the Vite dev server on Windows and uses `npm run dev -- --open --force` so the browser opens automatically; `play.sh` does the same on POSIX and only auto-opens when `xdg-open` is available. In CI, `.github/workflows/node.js.yml` runs install, build, typecheck, and test across Node 20/22/24.

## Project-specific conventions and gotchas

Performance is a constant concern in this codebase. Hot paths avoid allocations and prefer typed arrays (`Float32Array` for network weights and serialization buffers, and typed arrays plus an object list in `FlatSpatialHash`), so when adding new per-frame data keep GC pressure low. The worker buffer layout is a hard contract: modify it only if you also update `renderWorldStruct` and the God Mode parsing in `src/main.ts`. There is also a legacy render path (`renderWorld`) that references a `drawSnake` helper not defined in `src/render.ts`, which is a signal that the non-worker renderer is not the current focus; if you revive it, audit that path carefully and supply any missing drawing helpers.

Keep the sensors and brain input size aligned. If you change `CFG.sense.bubbleBins` or adjust the sensor vector layout, update `CFG.brain.inSize` and any code that assumes a fixed input length (including BrainViz or any debug panels). Similarly, changes to genetic operators or architecture keys can invalidate saved genomes in localStorage, so consider how `Genome.toJSON()` and `archKey()` are used before altering their output.

If any section here feels unclear or you want deeper coverage (for example, the collision math, the exact fitness weighting, or how God Mode parsing walks the buffer), tell me which part to expand so we can iterate.

## TypeScript policy (in-progress conversion)

* Keep runtime behavior and performance identical; types must not alter logic or hot-loop allocations.
* Use strict typechecking (`tsconfig.json` with `noEmit`) and convert files in dependency order; server overrides live in `server/tsconfig.json`.
* Prefer shared protocol types under `src/protocol/` for worker/main message contracts.

## Markdown policy

* When writing markdown follow the style and formatting rules in markdown-rules/rules.md
