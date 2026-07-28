# Agent instructions for slither_neuroevo

## Project scope and layout

Slither Neuroevolution is a browser-based neuroevolution sandbox. The browser
is a rendering and control client. On the current branch, Node/TypeScript still
owns the authoritative simulation; this is the temporary reference
implementation during the approved forward migration. The approved target has
one Rust-owned authoritative game, with Node limited to a thin HTTP,
WebSocket, static-file, routing, and SQLite-metadata interface. Loopback is the
default, and deliberate use from other devices on the owner's trusted home LAN
is supported. This repository does not provide authentication, TLS, or a
hardened public deployment mode.

The main browser entry point is `index.html`, with UI behavior in `src/main.ts`,
rendering in `src/render.ts`, and styling in `styles.css`. Server startup is in
`server/index.ts`; orchestration is in `server/simServer.ts`; the fixed-step
engine is `src/sim/SimCore.ts`; and the authoritative model is `src/world.ts`.
The native neural kernels live under `native/`.

`package.json` and `package-lock.json` define the Node toolchain. TypeScript is
checked through `tsconfig.json`, ESLint through `eslint.config.cjs`, and Vite
through `vite.config.ts`. SQLite state defaults to the ignored
`data/slither.db`. `server/config.toml` is also ignored: `server/config.ts`
creates it from current defaults on first startup when it is absent. Do not
describe it as a tracked source file.

Active plans live in `docs/todo/`.
`docs/todo/rust-authoritative-runtime-plan.md`, revision
`2026-07-29-draft-4`, is the owner-approved implementation plan.
`docs/todo/rust-authoritative-runtime-implementation-log.md` is its short
factual execution record. `docs/todo/project-recovery-plan.md`,
`docs/todo/native_refactor_plan.md`, and `docs/todo/archive/` are superseded
historical material and must not direct implementation. In particular, the
old claim that the owner selected kernel-only Rust is false. Durable
architecture choices live in `docs/decisions/`.

## Current reference runtime flow

Until the approved Rust cutover passes its gates, the selected TypeScript
reference/production flow is:

```text
browser control
  -> Protocol 2 WebSocket validation
  -> SimServer command queue
  -> fixed-step boundary
  -> SimCore scheduler
  -> World.step
  -> serial brain or canonical BrainPool
  -> movement, food, collision, and evolution
  -> binary frame and JSON stats
```

`server/wsHub.ts` owns WebSocket connection state. `server/protocol.ts` is the
source of truth for Protocol 2 JSON messages. `server/controllerRegistry.ts`
owns external player/bot assignments and rate limits. `server/httpApi.ts`
provides health, persistence export/import, Hall of Fame, resurrection, and
graph-preset endpoints.

The browser has no local World or simulation worker. Do not reintroduce a
browser fallback, optimistic authoritative state, or a second simulation loop.
On disconnect, the UI reconnects and waits for server frames.

The migration keeps this path as a selectable test oracle while Rust replaces
it subsystem by subsystem. It is not an automatic fallback for the
Rust-authoritative runtime.

## Fixed-step scheduling and World ordering

`SimCore.update()` converts elapsed wall time and `simSpeed` into zero or more
complete fixed steps. `simSpeed` changes the requested rate of fixed-step
execution; it never enlarges a World delta. A per-pump cap may discard and
report wall-clock debt, but committed World state never skips a step or uses a
partial step.

`World.step()` is the sole production control/physics pipeline. It samples all
due controls from one observation boundary before movement, then advances
physics with collision-only substeps. Keep external, serial-neural, and pooled
population controls aligned to that ordering.

## Determinism and run identity

Authoritative randomness uses the versioned streams in `src/rng.ts`. World,
evolution, observer, and durable baseline-bot streams are derived from the
normalized run seed. Cosmetic rendering randomness must not advance those
streams.

Reset rebuilds generation one with the same seed and a new run ID. New Run
uses system entropy for a different seed and creates a new run ID. Both become
current only after their required run-start checkpoint commits.

Exact replay is required only for the same source revision, RNG/snapshot
versions, graph and settings, backend build, target architecture, supported
environment, completed-step count, and ordered action log. Compare JS and
native kernels with explicit numeric tolerances; do not promise bit-identical
long-horizon results across backends or platforms.

## Approved Rust-authoritative migration and transitional native backend

The kernel-only boundary is superseded. The approved destination is one
Rust-owned authoritative game: persistent world state, fixed-step scheduling,
sensors, heterogeneous neural inference, recurrent state, movement, food,
collisions, controllers, evolution, generation transitions, RNG/allocator
state, checkpoint construction, and binary frame packing all belong in Rust.
Node remains the thin LAN/API/file/SQLite-metadata interface, and browser
TypeScript remains the renderer, UI, camera presentation, and input collector.
See `docs/todo/rust-authoritative-runtime-plan.md` and
`docs/decisions/0002-rust-authoritative-runtime.md`.

The current reference backend still exposes Dense, MLP, GRU, LSTM, and RRU
kernels from `native/src/simd_kernels.rs` through `native/src/lib.rs`. Keep it
working for characterization and differential tests, but do not extend the
per-snake/per-layer N-API boundary as the final architecture.

Normal startup selects the native backend. `src/brains/nativeBridge.ts`
validates every required export and a source-derived build identifier before
brains are constructed. A missing or incompatible addon fails normal startup
with build instructions. `--backend js` is an explicit diagnostic mode, not a
silent fallback.

Backend and threading are independent axes. `--mt` requests the canonical
worker pool; `--mt-workers N` requests a bounded count. Native single-thread,
native MT, JS diagnostic single-thread, and JS diagnostic MT are all tested.
Enabling or disabling MT must not change the selected math backend.

`server/brainPool.ts` is the sole production pool,
`server/brainPoolProtocol.ts` is its parent/worker contract, and
`server/worker/inferWorker.ts` is the sole inference worker. Shared typed
buffers hold inputs, outputs, population weights, and slot indices. Completion
and reset use tagged messages and promises, not a second Atomics-based pool.

Population slots are stable recurrent-state identities. Workers own slots by a
deterministic modulo rule for the entire pool epoch; shuffled or shrinking
batches do not migrate GRU/LSTM/RRU state. Pool lifecycle and population
weights have separate monotonic epochs. A new weight epoch is usable only after
every worker acknowledges rebinding and recurrent-state reset.

If a requested worker fails, times out, exits, or violates the protocol, the
in-flight authoritative step is rejected. No successful frame, stats, or
checkpoint is published, and the server does not fall back mid-generation.
Health/status remain available. Recovery requires an explicit Reset, New Run,
or process restart from a valid checkpoint boundary.

Build and verify the addon from the repository root with:

```powershell
npm --prefix native run build
cargo test --manifest-path native\Cargo.toml --release
cargo fmt --manifest-path native\Cargo.toml -- --check
cargo clippy --manifest-path native\Cargo.toml -- -D warnings
```

When the local npm shim is broken, use the compiled napi-rs CLI from `native/`:

```powershell
node .\node_modules\@napi-rs\cli\dist\cli.js build --platform --release
```

Every public N-API entry validates positive dimensions, checked arithmetic,
array lengths, scratch/state sizes, and unsupported writable aliasing before
raw-pointer code. Keep unsafe scopes narrow and document their exact valid
ranges and non-overlap requirements. Supported native targets are x86_64
Windows MSVC and x86_64 Linux GNU; there is no WASM path or non-x86_64 native
fallback.

## Binary frames and rendering

`src/serializer.ts` and `src/protocol/frame.ts` define the hard binary frame
contract. A `Float32Array` contains:

1. Seven header floats: generation, total snakes, alive count, world radius,
   camera X, camera Y, and zoom.
2. Each alive snake: eight floats for ID, radius, skin, head X/Y, direction,
   boost, and body-point count, followed by `pointCount * 2` body coordinates.
3. One pellet count followed by five floats per pellet: X, Y, value, type, and
   color ID.

`src/render.ts` and God Mode parsing in `src/main.ts` walk this layout. Change
serializer, frame helpers, renderer, selection parsing, and tests together.
Prefer extending the compact buffer over cloning the World into the browser.

## Sensors and neural controllers

The only supported sensor layout is v3. Its input length is
`19 + 4 * bubbleBins`, where `bubbleBins` is at least 8. Keep
`CFG.brain.inSize`, `src/protocol/sensors.ts`, sensor construction, baseline
bots, graph validation, and visualizer expectations aligned.

`points_delta_norm` is score change accumulated since that snake's previous
delivered sensor sample, or since construction for its first sample. Unsampled
control intervals accumulate. `Snake.sampleSensors()` owns this stateful
boundary; `computeSensors()` remains pure. External, serial-neural, and pooled
paths must all sample through the same method. Baseline-bot strategy probes are
not delivered observations and must remain pure.

Neural graph construction is centralized in `src/mlp.ts` and `src/brains/`.
Graph ports are zero-based. Split sizes must sum to their input, Concat port
ordering is explicit, and total outputs must equal the two turn/boost values.
Architecture keys and parameter counts are persistence compatibility
boundaries.

## UI and Protocol 2 controls

The welcome message supplies the active seed, run ID, config revision/hash,
authoritative settings, sensor spec, serializer version, and honest inference
mode. The status pill must show the server, seed, active backend, and active
worker count.

Live settings use one atomic `settings` request and apply only at a pre-step
server boundary. The shared metadata in
`src/protocol/settingDefinitions.ts` defines type, range, and whether a path is
live or reset-only. The browser updates its displayed state from
`settingsApplied`, not from an optimistic local write. Reset-only controls and
graph changes use `reset` through Apply and reset.

God Mode kill uses the normal death path. Move translates the entire body by
one bounded delta and rebuilds spatial state. Logs are based on
`godModeResult`. New Run is acknowledged only after its new generation-one
checkpoint is durable.

## Persistence and export

The current TypeScript reference stores checkpoints as versioned parent
metadata plus one `snapshot_genomes` child row per dense population slot in a
single SQLite transaction. Its browser export/import path still materializes
population JSON. Treat both as compatibility/reference evidence, not the
approved destination.

The old `genomes_blob` format is read-only compatibility. Its bounded reader
warns that a legacy load may still allocate the combined population; never
rewrite or delete a user database merely to migrate it.

A resumable checkpoint is an exact generation-boundary population checkpoint,
not a mid-tick world save. It captures the evolved population, generation,
simulation step, seed/run/config identity, authoritative RNG and allocator
state, and the zero-recurrent-state boundary before spawn, pellets, focus,
sensors, or inference.

New checkpoint-v3 population payloads are immutable managed files containing
packed binary data with per-payload raw or shuffled-Zstandard encoding.
SQLite stores only metadata, current pointers, compact history, graph/config
records, Hall-of-Fame indexes, and file references. Export is one ordinary
direct browser download of a self-contained archive; import uploads the
original file directly. Browser JavaScript never parses or reconstructs the
population. Keep current/legacy readers until the owner's real databases and
save files have been inventoried and migrated within the approved limits.

Ordinary exact checkpoints remain generation-boundary saves, not mid-round
world snapshots. Normal startup eventually uses the latest valid retained
managed checkpoint and the approved recovery-branch rule. During migration,
the current flags and readers remain available only as documented by the
active stage.

## Local and trusted-LAN setup

The default workflow is local loopback use:

```powershell
npm install
npm --prefix native run build
npm run server
npm run dev
```

Open the Vite URL, normally `http://localhost:5173`. Opening `index.html`
directly does not work. `play.bat` and `play.sh` install missing dependencies,
build the mandatory native addon, start the simulation server and Vite, and
write PID/log files in the repository root.

Trusted home-LAN use is supported by setting `host` and `uiHost` to
`0.0.0.0` or an explicit LAN interface and setting `publicWsUrl` when the
browser cannot derive the simulation-server host from the UI hostname. Both
launchers must retain non-loopback address discovery and print usable UI,
server, and WebSocket network URLs. Vite HMR must remain connectable from
another LAN device. CORS enables those browser/server address combinations; it
is not authentication. Never equate trusted-LAN routing with public-internet
hardening, and do not advertise router port forwarding or untrusted-network
exposure as safe.

`server/config.ts` resolves defaults, the generated TOML file, environment
overrides, and CLI flags. Important experiment/runtime flags include
`--backend native|js`, `--mt`, `--mt-workers N`, `--seed N`, `--fresh`,
`--resume latest|N`, `--checkpoint-every N`, and `--db-path PATH`. A configured
seed conflicts with resume and therefore requires `--fresh`.

LAN-related overrides are `--host`, `--ui-host`, and `--public-ws-url`, with
matching `HOST`, `UI_HOST`, and `PUBLIC_WS_URL` environment variables.
`publicWsUrl` is the browser's configured route to the simulation server; its
legacy name does not imply public-internet safety.

## Tests and CI

Primary test layers are explicit and non-overlapping in
`scripts/test-categories.ts`:

- unit: small pure/module contracts;
- component: multi-module behavior without full server boundaries;
- integration: real boundaries such as WebSocket, persistence, or pool use;
- system: process/server lifecycle behavior;
- acceptance: owner-visible end-to-end contract;
- regression: named historical failures;
- performance: measured budgets, informational in CI while history matures;
- security: input limits, protocol rejection, and boundary hardening.

`native-required` is an additive overlay. It must load the source-identified
addon and execute native plus MT contracts; it must not skip because native is
missing. Network suites may skip only when
`SLITHER_SKIP_NETWORK_TESTS=1` is explicitly set, and they emit a visible
warning.

Use direct commands when an npm/PowerShell wrapper obscures completion:

```powershell
node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts all --reporter=dot
node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts native-required --reporter=dot
node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false
node .\node_modules\eslint\bin\eslint.js .
node .\node_modules\vite\bin\vite.js build
cargo test --manifest-path native\Cargo.toml --release
```

CI is `.github/workflows/CI.yml`. Its Ubuntu/Windows and Node 22/24 matrix
builds native once in each job, verifies the addon identity, runs the native/MT
overlay and every primary JavaScript layer, then runs Vite, TypeScript, and
ESLint. A separate Rust job enforces rustfmt and Clippy.

## Coding and documentation rules

- Preserve hot-path typed arrays and avoid per-frame allocation unless a
  measurement justifies it.
- Add TSDoc-style documentation for functions, classes, class fields, and
  module-level variables in `src/`, `server/`, scripts, and tests.
- Keep shared wire types under `src/protocol/` or `server/protocol.ts`; do not
  recreate browser-worker message surfaces.
- `README.md` is for users and QA. `AGENTS.md` is the developer reference.
  `docs/API-instructions.md` is the local external-client contract.
- Keep README slider names aligned with
  `src/protocol/settingDefinitions.ts`. Do not document removed v2 sensor or
  frame-delta controls.
- Use ordinary CommonMark with blank lines around lists and fenced blocks.
- Never commit `data/slither.db`, generated native binaries, PID/log files, or
  `server/config.toml`.
- Preserve `bestPointsThisGen` initialization before the first sensor pass.
- Treat `populationSlot`, snake-array index, visible snake ID, baseline-bot
  slot, and external controller ID as different identities.
