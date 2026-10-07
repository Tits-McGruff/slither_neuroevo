# Slither Neuroevolution

A browser-based neuroevolution sandbox inspired by Slither.io. Populations of snakes evolve neural networks, learn to seek food, avoid hazards, and compete across generations. This README is written for users and QA testers who want to run the sim, understand the UI, and explore behavior.

## Key Features

- **Remote browser client**: The browser renders server frames and sends controls; it does not run a second game.
- **Rust-authoritative runtime**: The complete authoritative game—world state, sensing, differently weighted brains, movement, collision, evolution, and frame packing—runs in Rust behind a thin Node interface.
- **Explicit reference runtime**: The former TypeScript `SimCore`/`World` remains available only through `npm run server:reference` as a test oracle; production never falls back to it.
- **Deep Evolution**: Supports MLP, GRU, LSTM, and RRU architectures with complex genetic operators and a modular graph editor.
- **Deterministic run controls**: Reset repeats a seed; New Run starts and checkpoints a different seed.
- **Bounded persistence**: Managed immutable checkpoint files hold packed binary population data; SQLite holds small metadata/history/indexes. Browser import/export uses direct file upload/download without population-sized JavaScript objects.

## Contents

- [Quick start](#quick-start)
- [Server startup and recovery](#server-startup-and-recovery)
- [Saves, storage, and diagnostics](#saves-storage-and-diagnostics)
- [Measured workloads and limits](#measured-workloads-and-limits)
- [Debian service, updates, and backups](#debian-service-updates-and-backups)
- [Configuration and architecture](#configuration-and-architecture)
- [Home-LAN access](#home-lan-access)
- [Controls](#controls) and [joining a game](#join-and-spectate)
- [Slider guide](#slider-guide) and [brain graph editor](#brain-graph-editor)
- [Import and export](#import-and-export)
- [Preset recipes](#preset-recipes-qa-friendly)
- [Test suites](#test-suites) and [troubleshooting](#troubleshooting)

## Quick start

### Prerequisites

- **Node.js**: v24 or newer
- **Rust**: Required for compiling the game engine. Install via [rustup.rs](https://rustup.rs).
- **Windows build tools**: Visual Studio C++ build tools and a Windows SDK are required by native dependencies.
- **Native platforms**: x86_64 Windows MSVC and x86_64 Linux GNU.

### Running

Install dependencies and build the game engine:

```bash
npm install
npm --prefix native run build
```

Start the simulation server in one terminal:

```bash
npm run server
```

Start the browser development server in a second terminal:

```bash
npm run dev
```

`npm install` installs both root and native-package dependencies. Normal server
startup requires the addon; it does not silently fall back to JavaScript. The
root `npm run build` command builds both the addon and production client. To
build only the addon, use the command above or run this from `native/`:

```bash
cd native
npm run build
```

The convenience launchers install missing dependencies, build the required
addon, and write logs/PID files in the repository root:

- Windows: `play.bat`
- Linux: `play.sh`

On Windows, open the local URL printed by Vite (usually `http://localhost:5173`).
On Linux, `play.sh` starts the Rust server with the built browser at its printed
server URL. If an existing database cannot resume, it does not move or replace
that database; it keeps the health-only server available for diagnosis and
never starts a new experiment automatically over it.

Note: This project uses ES modules, so opening `index.html` directly in a file browser will not work.

## Server startup and recovery

Normal startup is Rust-authoritative and supports durable fresh-run and
managed-checkpoint restart paths:

```powershell
npm --prefix native run build
npm run build:client
npm run server -- --fresh --db-path ./data/rust-experiment.sqlite
npm run server -- --resume latest --db-path ./data/rust-experiment.sqlite
```

### Choose how to start

For a new experiment, start once with `--fresh`, then reuse that database with
one of the resume options:

| Option | Behavior |
|---|---|
| `--fresh` | Starts and durably records a new run without deleting older checkpoints. |
| `--resume latest` | Validates the current checkpoint. If needed, recovers from the newest valid retained boundary under a labelled recovery branch. |
| `--resume <checkpoint-sha256>` | Selects that exact retained checkpoint, including an older generation or prior run. Requires the producing build identity and never substitutes another checkpoint. |

- Selecting a checkpoint other than the active current one creates a separate
  branch after validation. The source's later history is preserved.
- After a rebuild, `--resume latest` can continue a checkpoint when its state
  versions, target, release profile, settings schema, and math backend remain
  compatible. That continuation is recorded as a new branch before activation.
- If latest startup finds no valid retained boundary, the process exposes a
  failing health endpoint and refuses game WebSockets. It does not start a new
  game over the failed experiment.

The browser status pill labels converted saves and selected/imported branches;
its tooltip shows their source details.

### Open an older database

`--resume latest` also accepts compatible TypeScript reference databases:

- Current per-genome checkpoint rows.
- Older combined `genomes_blob` populations.
- Format-zero populations embedded in parent JSON.

Rust reads the population from SQLite incrementally, preserves compatible
settings and an ASCII-safe graph, and writes a new generation-one checkpoint
without changing the source rows. Later restarts use that managed checkpoint.

This is a **population conversion**, not exact continuation of the old game.
Health and welcome messages retain a `legacyConversion` notice with the source
row, format, generation, and seed when available. The source seed is provenance;
the new run uses its own seed for missing state. Exports and subsequent imports
retain that population-only classification.

### Calculation workers

The server defaults to five Rust calculation workers on new
configurations. Use `--rust-workers N` (1–7, or `RUST_WORKERS=N`) to override its persistent worker pool;
it parallelizes sensing and brain evaluation while keeping brain-state and
physics commits ordered. This is separate from the reference server's
`--mt-workers` option.

### World resource limits

New runs, Apply and reset, New Run and legacy population conversions admit
bounded storage for **1,000,000 total body points** and **250,000 total pellets**.
The per-snake **Max length** setting remains separate from the aggregate body
limit. The ambient target can reach 25,000 pellets while leaving room for normal
corpse and boost pellets. State admission charges the declared world and frame
storage before activation; exceeding a runtime resource ceiling rejects the
complete step. Existing exact checkpoints retain their originally admitted
limits. Apply and reset creates a new boundary with the current allowances.

## Saves, storage, and diagnostics

The server prints a browser URL and supports Protocol 2 players and bots,
including steering, sensors, disconnect, and reclaim. For another device on
your trusted home network, see [Home-LAN access](#home-lan-access).

### Save and run controls

| Control | What it does |
|---|---|
| **Pin checkpoint** | Permanently protects the exact current managed checkpoint. It does not download a save. |
| **Export** | Downloads the current checkpoint, complete compact history, and run-scoped Hall of Fame as one `.slither-save` file. Only one export runs at a time; another request receives `409` until it finishes or is cancelled. |
| **Import** | Uploads the original `.slither-save` or older browser-exported `.json` file and shows upload progress. The live game changes only after the replacement commits. |
| **Apply and reset** | Applies reset-only settings and the selected graph, then records generation one with the same seed and a new run ID. |
| **New Run** | Records generation one with a different seed and a new run ID. |

Reset, New Run, and successful import keep existing WebSockets open. Clients
discard old assignments and join the replacement run without stale controller
tokens. When an imported save has later local history, the browser offers a
new-run branch and preserves that later history.

Exact saves restore the complete experiment. Older JSON saves restore a
compatible population into a new generation-one run; they cannot restore
missing history or random-state continuation. See [Import and export](#import-and-export)
for compatibility details.

### Storage admission and retention

Checkpoints live beside the database in its `.checkpoints` directory.

- **Before writing:** checkpoint, import, export, and pin operations account
  for existing work files, new source/candidate/final files, SQLite/WAL space,
  and a **1 GiB operating reserve**. A rejection shows the byte calculation.
- **Automatic retention:** cleanup runs at startup and after each durable
  generation save. It keeps the latest eight checkpoints plus configured
  milestones and prior-run anchors, subject to the configured byte budget.
- **Protected data:** cleanup removes only unpinned managed files. Compact
  generation history and Hall-of-Fame records remain intact.
- **Interrupted writes:** managed restart verifies retained checkpoints and
  referenced Hall-of-Fame objects before removing exact final files left
  unreferenced by interrupted publication. Unknown files and links are left alone.

Cleanup is recorded in SQLite before files are removed. Health reports the last
cleanup's file/byte counts and the latest, recent, milestone, prior-run-anchor,
pinned, and planned-prune retention classes.

### Charts and Hall of Fame

- **Fitness, species, and weight charts** use the newest 120 compact persisted
  generation summaries. They survive restart, recovery, and import.
- **Hall of Fame** reads bounded best-first records from `/api/hof`.
- **Spawn** sends only the selected entry identity. Rust loads its managed
  weights, validates and places the new snake, then publishes it with independent
  control. Neural weights never pass through browser JavaScript or Node's main thread.
- **Visualizer** captures activations only while a browser is viewing its tab.
- **God Mode** moves the complete body within bounds; kill follows the normal
  death, corpse-pellet, random-stream, ID, and controller lifecycle paths.

### Health and diagnostics

`/api/health` reports recovery provenance and compact measurements without
serializing the world or loading checkpoint populations into Node:

| Area | Reported values |
|---|---|
| Simulation | Step mean/p95/p99/max, simulated-to-wall time, frame bytes, and checkpoint timing. |
| Controls | Separate player/trainer action and lifecycle latency; Rust-confirmed assignment, reclaim, action, and disconnect counts. |
| Node process | Event-loop delay and process memory. |
| `nativeQueues` | Inbound/output occupancy, lifetime peaks, and configured limits as exact sixteen-digit hexadecimal counters. |
| `outbound` | Reliable WebSocket queues, pending frames, connection peaks/limits, and lifetime frame-replacement/reliable-failure counts that survive disconnects. |
| Storage | SQLite, WAL, free-page, temporary-file, free-disk, quota, and operating-reserve byte counts; retention and cleanup results. |

Percentiles are conservative fixed-histogram upper bounds. Reporting does not
retain a per-step series or authoritative game arrays. For the external-client
contract, see [API instructions](docs/API-instructions.md).

<details>
<summary>QA diagnostic: player input during frame and sensor suppression</summary>

With that server running, a short real-boundary diagnostic exercises a
spectator, an observation-driven Protocol 2 bot with disconnect/token reclaim,
and a UI-class player driven by the production independent latest-action pump.
The player pauses all inbound frame and sensor consumption for 1.5 seconds,
sends a turn change and boost release during the pause, then requires inbound
recovery while the telemetry endpoint confirms that Rust applied the actions:

```powershell
npm run probe:stage6-runtime -- --ws-url ws://127.0.0.1:5174 --duration-seconds 30
```

Use `--player-suppression-seconds N` to change the receive pause. Add
`--require-frame-replacement` on a deliberately constrained or remote link to
require at least one server-side superseded display frame, and add
`--require-generation-transition` with a duration long enough for a complete
round. The probe is a wire-compatible diagnostic client; it does not replace
the required unchanged owner trainer or a real browser on another trusted-LAN
device.

</details>

### Settings and graph changes

- **Reset-only settings** include population and baseline-bot counts. Apply
  them with **Apply and reset** within the existing UI limits.
- **Live settings** apply atomically at the next Rust step boundary and remain
  active through later Reset or New Run operations.
- **Graphs** are sent directly to Rust for independent validation and compilation
  before population allocation. Both the default stack and custom graphs are supported.
- **Named presets** are saved, listed, and loaded through the isolated SQLite
  metadata worker.

Use `npm run server:reference` only for the retained TypeScript comparison
implementation.

## Measured workloads and limits

The retained Rust measurements use Oxygen's Ryzen 7 2700/Debian host at
1x simulation speed, with 3,500 target pellets and ten baseline bots.
Population snakes have individually owned neural weights. The
large graph has 402,914 parameters per snake; the P3 startup probe uses a
deterministic packed-weight capacity fixture. The default configuration starts
with five calculation workers. The table lists the worker counts used for
the server timing or capacity results; use `--rust-workers N` to select one.

| Case | Population | Sensors per snake | Brain | Server workers | Completed measurements |
|---|---:|---:|---|---:|---|
| P0 | 55 | 83, with 16 angular bins | Default graph | 5 | Server timing, LAN steering and desktop drawing |
| P1 | 300 | 83, with 16 angular bins | Default graph | 5 or 6 | Server timing, thirty-minute loaded player/trainer soak, LAN steering and desktop drawing |
| P2 | 55 | 147, with 32 angular bins | Large custom graph | 5 | Server timing, checkpoint/export overlap, LAN steering and desktop drawing |
| P3 | 300 | 147, with 32 angular bins | Large custom graph | 6 | Large-population persistence and startup-capacity checks; real-time performance remains unqualified |

### Server and desktop results

- **P1 loaded soak:** five workers completed 107,216 steps over thirty minutes
  at 0.9927 simulated/wall time with zero discarded scheduler time.
- **P1 worker comparison:** ten-minute windows with two real trainer actors
  passed with five and six workers. Both had a 16.667 ms step-p99 upper bound
  and zero discarded time. Four workers missed the timing target.
- **P2 checkpoint/export overlap:** five workers achieved at least 0.9809
  simulated/wall time, zero discarded time, and a maximum checkpoint barrier
  below one second.
- **LAN steering:** 200 attempts per player/bot route in each P0/P1/P2 case
  gave p95 upper bounds at most 42.6 ms, including unknown responses in the ranking.
- **Desktop drawing:** separate sixty-second foreground samples had p95
  intervals of 16.8–16.9 ms on a Ryzen 7 5800X/RTX 4080 with Chromium 154.
  P2 also recorded a 1.55-second stall while opening the graph/settings panel.

### Capacity and storage

For the full-size P3 legacy-conversion fixture, a 1280 MiB checkpoint budget
rejected startup before publishing a current checkpoint. A 1986 MiB budget
admitted one generation-one checkpoint and its subsequent managed restart.
The fixture preserved all legacy source rows. Actual admission depends on the
protected checkpoints, new payload size and available disk space; keep the
normal 4096 MiB default unless a deliberate storage budget requires otherwise.

The dense-world desktop drawing checks also pass in follow and overview modes
with more than 200,000 body segments. That fixture remains a capacity case
whose server simulation is slower than real time.

### Browser archive memory

Ordinary downloads and original-file uploads passed for small, 71 MiB, and
roughly 393 MiB saves:

| Measurement | Observed large-minus-small increase |
|---|---:|
| Export JavaScript heap peak | 4.95 MiB |
| Export heap after garbage collection | 0.40 MiB |
| Combined browser-process private memory | 150.6 MiB |

Unattended Debian service operation and committed-checkpoint restart are verified.

### Laptop follow-up

Nitrogen (Surface Laptop 4) connects and plays over the LAN, but its measured
display intervals miss the selected performance target, especially in overview.
The owner has deferred laptop performance and the remaining laptop workload
checks until after this migration. These results do not establish the same
display performance on every device. The measured scope, remaining limitations
and links to raw reports are in the
[factual implementation log](docs/todo/rust-authoritative-runtime-implementation-log.md).

## Debian service, updates, and backups

### Install and start the service

The checked-in service runs the Rust server in the foreground so systemd owns
the real process and can restart a crash. Build once, optionally copy the
environment example, then install the per-user unit:

```bash
npm ci
npm run build
cp server/systemd.env.example server/systemd.env
sh scripts/install-systemd-user.sh
systemctl --user start slither-neuroevo.service
systemctl --user status slither-neuroevo.service
```

The unit waits five seconds before a failed-process restart and stops after
three starts within two minutes. It does not restart a clean manual stop.
`journalctl --user -u slither-neuroevo.service -f` follows its logs. For a
server that must survive logout and start before the user logs in, an
administrator must enable user lingering once with
`sudo loginctl enable-linger "$USER"`. The installer warns when it is disabled.
The manual `play.sh`/`shutdown.sh` pair remains useful for diagnosis, but do not run
it at the same time as the systemd service.

### Recover from a calculation fault

A caught Rust calculation fault keeps the process alive and reports the fault
at `/api/health`; it does not trigger an automatic service restart. Inspect the
health response and logs, then use `systemctl --user restart slither-neuroevo.service`
to resume from the latest valid committed checkpoint. The interrupted round's
unsaved progress is lost.

### Update the application

1. Stop the service.
2. Update the checkout.
3. Run `npm ci` and `npm run build`.
4. Start the service again.

The service start command never installs dependencies or rebuilds files.

### Back up the complete server

A complete backup must include SQLite and the immutable files beside it; a
copy of the `.db` file alone is incomplete. This command is safe while the
server is running. It takes an online SQLite snapshot, copies exactly the
checkpoint and Hall-of-Fame objects referenced by that snapshot, verifies
their byte counts and SHA-256 hashes, and retries if pruning races the copy.
The next backup in the same parent directory removes this tool's abandoned
partial sets after 24 hours when their creating process has exited:

```bash
npm run backup:production -- \
  --db-path ./data/rust-authority.db \
  --output ./backups/slither-2026-09-27
```

### Restore a backup

Restore only while the server is stopped and to an absent target path. The
restore command validates every file and refuses to overwrite an existing
database or managed directory:

```bash
npm run restore:production -- \
  --backup ./backups/slither-2026-09-27 \
  --db-path ./data/restored.db
```

To start a new restored copy from a retained prior run's latest checkpoint,
add `--checkpoint-id` with its 64-character ID. The command selects that
run only in the new copy; it rejects pruned checkpoints and older boundaries
that are not their run's current pointer. Start the copy with `--resume latest`.

Start the restored copy with `SLITHER_DB_PATH=./data/restored.db` or an
equivalent `server/systemd.env` setting. Keep portable `.slither-save` exports
as an additional one-experiment backup, not as a replacement for the complete
server backup set.

### Reclaim space from a legacy database

After migrating old population rows, reclaim their unused SQLite pages only
while the server is stopped. This explicit maintenance command creates and
validates a separate complete backup before running SQLite `VACUUM`; the
backup directory must be new, and the source volume needs room for a temporary
database copy. It leaves the backup in place:

```bash
npm run compact:legacy -- \
  --db-path ./data/rust-authority.db \
  --backup ./backups/pre-compact-2026-09-28 \
  --offline
```

## Configuration and architecture

### Runtime ownership

This application uses a pure client/server model. The browser renders binary
frames and submits controls over Protocol 2 WebSocket messages. There is no
browser-local World, offline mode, or local simulation worker. Rust owns the
game; Node only routes HTTP/WebSocket/file traffic and small SQLite metadata.
The TypeScript game remains a selected test oracle, not a production fallback.

Loopback is the default, and deliberate use from a phone or another computer
on the same trusted home LAN is supported. The project has no accounts,
authentication, authorization, or TLS, so do not expose it through router port
forwarding or run it on an untrusted network.

### Configuration file and overrides

On first server startup, `server/config.ts` creates the ignored
`server/config.toml` file from current defaults. Useful fields include the
server/UI bind addresses and ports, `publicWsUrl`, checkpoint interval,
worker settings and Rust calculation-worker count. `publicWsUrl` is simply the
WebSocket address the webpage should use when the simulation server is not at
the same hostname as the UI; despite the legacy word “public,” it does not make
the service safe for the public internet. Normal defaults are native,
five calculation workers and resume-latest. Use `--rust-workers N` to override
the worker count, `--fresh` for a new durable run, or
`--resume latest|sha256:<checkpoint-id>` for managed recovery. Reference-only
backend and Node-MT flags belong to `npm run server:reference`.

### Checkpoint byte budget

`checkpointBudgetMiB` defaults to 4096 MiB and bounds unpinned automatic
checkpoints and the managed directory plus SQLite/WAL during checkpoint
publication. Set it in `server/config.toml`, with `CHECKPOINT_BUDGET_MIB`, or
with `--checkpoint-budget-mib N` (1280–65536 MiB). Explicitly pinned checkpoints
and downloaded exports are outside this cap. A budget too small for the
protected current and prior-run anchors stops the next durable transition
instead of deleting them.

## Home-LAN access

On the Windows computer running Slither Neuroevolution:

1. Start the server once so `server/config.toml` exists, then stop it.
2. In that file, set `host` and `uiHost` to `"0.0.0.0"`.
3. Set `publicWsUrl` to your computer's home-network address, for example
   `"ws://192.168.1.25:5174"`.
4. Run `play.bat`. It prints one or more **UI Network** addresses. Open one of
   those addresses on the phone or other computer while both devices are on
   the same trusted home network.
5. If Windows asks about Node.js network access, allow it on **Private
   networks**. If no prompt appears and the page cannot connect, allow inbound
   TCP ports 5173 and 5174 on the Windows Private firewall profile.

Your address will usually start with `192.168.`, `10.`, or `172.16` through
`172.31`. It may change after a router or computer restart; update
`publicWsUrl` to the address printed by the launcher when needed. Do not
configure router port forwarding for these ports.

## Test suites

`npm test` builds/tests native code and runs the JavaScript suite. After the
addon is built, `npm run test:js` runs the explicit complete JavaScript
manifest. Focused commands are:

- `test:unit`: small pure/module contracts.
- `test:component`: multi-module behavior without a full server boundary.
- `test:integration`: real WebSocket, persistence, worker-pool, and other
  subsystem boundaries.
- `test:system`: server/process lifecycle behavior.
- `test:acceptance`: owner-visible end-to-end contract.
- `test:regression`: named historical failures.
- `test:performance`: measured budgets; currently informational in CI.
- `test:security`: protocol/input and resource-boundary hardening.
- `test:native-required`: additive native and multi-thread contracts that fail
  rather than skip when the addon is unavailable.

Server panic tests use a separate release addon with test hooks. The full,
integration and required-native commands prepare it under `native/target` and
reuse it while its source identity matches. Normal server startup requires the
production addon and rejects the test build.

## Controls

- `V`: Toggle between Play and Spectate camera modes.
- Left click: Select a snake (God Mode selection).
- Right click: Kill the selected snake (God Mode).
- Left click + drag: Move a selected snake (God Mode).
- Mouse to steer, hold click to boost (when playing as a user snake).
- **Settings lock**: Hides all sliders and controls inside the Settings tab; unlock to edit.
- **Apply and reset**: Rebuild the world using reset-only settings. Reset keeps
  the active seed, creates a new run ID, and durably records generation one.
- **Defaults**: Restore default slider values and perform the same-seed reset.
- **New Run (new seed)**: Start generation one with a different seed after its
  run-start checkpoint commits. Older checkpoints are retained.

## Join and spectate

- Enter a nickname, then **Play** to spawn a player snake (server mode only).
- **Spectate** starts the sim with no player control.
- If the server is unavailable, the UI stays in the connecting state and does not run a local sim.
- When a server connection is established, the client auto-spectates and shows the join overlay.
- **Spectate** switches the camera to overview; **Play** switches to follow after assignment.
- Player control begins after the server sends an assignment; the overlay hides once assigned.
- Your nickname is saved in browser storage and restored on reload.

## Understanding the brain (MLP vs GRU)

### MLP (feed-forward)

An MLP uses only the current sensor inputs. It reacts quickly but has no memory. Expect twitchier, reflex-like behavior that can be strong for fast foraging but weaker at long-term planning.

### GRU (memory)

A GRU adds a hidden state that persists across time steps. This gives the snake short-term memory: smoother steering, better wall avoidance, and more stable pursuit or escape behavior. GRU brains can be more sensitive to mutation and may need gentler mutation settings.

### Practical effect

- **MLP**: quick reactions, simpler strategies, faster training.
- **GRU**: smoother motion, memory of recent events, better long arcs or deliberate turning.
- **LSTM/RRU**: alternate memory cells with their own hidden-size sliders.

## Slider guide

Most sliders are **live** (apply immediately). Some are **reset-only** (require Apply and reset). The UI marks this next to each slider.

### Core sliders

- **NPC snakes**: Total population size. Higher values make the sim more chaotic and slower.
- **Simulation speed**: Requested wall-clock rate for complete fixed steps. It
  never enlarges a physics step; an overloaded machine may achieve less than
  the requested multiplier and report dropped scheduler debt.
- **AI hidden layers**: How many MLP hidden layers to use (1–5). More layers increase capacity.
- **Neurons layer 1–5**: The size of each hidden layer. Only layers up to the selected count are active.

### World and food

- **World radius**: Arena size. Larger maps spread snakes and food farther apart.
- **Pellet target count**: Total pellets kept in the arena. More pellets means faster growth.
- **Pellet spawn per second**: Refill rate when pellets are eaten or removed.
- **Food value per pellet**: How much points and growth one pellet provides.
- **Growth per food**: How many body segments a pellet adds.
- **Edge food falloff**: Toggles the radial fade so ambient food density tapers toward the arena edge.
- **Edge fade start**: Where the edge fade begins (fraction of radius; gentle early, sharper near the wall).
- **Edge fade sharpness**: Controls how quickly the fade steepens near the wall.
- **Filament contrast**: Boosts filament/void separation (higher = thinner filaments, larger voids).
- **Filament warp scale**: Strength of the domain warp that twists the web (fraction of radius).
- **Filament warp frequency**: Controls how tight the warp ripples are.
- **Filament scale (large)**: Size of the largest filament structures.
- **Filament scale (medium)**: Size of mid-scale web structures.
- **Filament scale (small)**: Size of fine filament detail.
- **Filament speckle strength**: Extra dust-like speckle blended into the filaments.

### Snake physics

- **Base speed**: Default travel speed of snakes.
- **Boost speed**: Speed while boosting (relative to base speed).
- **Turn rate**: How quickly snakes can rotate.
- **Base radius**: Base body thickness.
- **Max radius**: Maximum body thickness at large sizes.
- **Thickness scale**: How quickly thickness grows with length.
- **Thickness log divisor**: Controls how quickly thickness growth tapers off.
- **Segment spacing**: Distance between body points (affects body smoothness).
- **Start length**: Initial number of segments at spawn.
- **Max length**: Upper cap on total segments.
- **Min length**: Minimum allowed length (prevents collapse).
- **Size speed penalty**: Slows large snakes at high lengths.
- **Size boost penalty**: Reduces boost advantage for large snakes.

### Boost and mass

- **Min points to boost**: Points required before boosting is allowed.
- **Boost points cost per second**: How quickly points are spent while boosting.
- **Boost cost size factor**: Larger snakes spend points faster while boosting.
- **Length loss per point**: How much length shrinks per point spent.
- **Boost drop pellet value factor**: Value of pellets dropped while boosting.
- **Boost drop jitter**: Spread of pellets dropped behind boosting snakes.

### Collision

- **Substep max dt**: Smaller values improve collision accuracy at higher speeds.
- **Skip segments near head**: Ignores near-head body segments for collision checks.
- **Hit scale**: Collision radius multiplier (higher = more collisions).
- **Collision grid cell size**: Spatial hash resolution; too small slows, too large misses.
- **Collision neighbor range**: How many neighbor cells are checked per query.

### Sensors

- **Sensor bins**: Number of angular bins per channel (reset-only).
- **Near radius base**: Base near sensing radius.
- **Near radius scale**: Size-based near radius increase.
- **Near radius min**: Minimum near sensing radius.
- **Near radius max**: Maximum near sensing radius.
- **Far radius base**: Base far sensing radius.
- **Far radius scale**: Size-based far radius increase.
- **Far radius min**: Minimum far sensing radius.
- **Far radius max**: Maximum far sensing radius.
- **Food saturation K**: Saturation constant for food density.
- **Max pellet checks**: Work cap for pellet sampling.
- **Max segment checks**: Work cap for segment sampling.
- Sensor model note: v3 observations include nearest-pellet distance and direction (`nearest_food_dir_sin/cos`) in addition to binned food/hazard/wall/head channels.

### Baseline bots

- **Baseline bot count**: Number of scripted opponents rebuilt on reset.
- **Respawn delay (sec)**: Live delay before a dead baseline bot returns.
- **Randomize base seed per generation**: Derives a different deterministic
  baseline-bot seed for each generation.
- **Baseline bot base seed**: Non-negative base seed for scripted bots.
- **Randomize base seed**: Chooses a new valid value in the settings UI; Apply
  and reset is still required before that reset-only seed becomes active.

### Evolution

- **Generation duration seconds**: Length of each generation.
- **Elite fraction**: Portion of top genomes preserved unchanged.
- **Mutation rate**: Probability of mutating each weight.
- **Mutation std**: Strength of weight perturbations.
- **Crossover rate**: Chance that offspring blends parents (vs clone).

### Observer and camera

- **Focus recheck seconds**: How often the focus snake is re-evaluated.
- **Focus switch margin**: Higher values resist switching to a new leader.
- **Early end min seconds**: Minimum time before early stop is allowed.
- **Early end alive threshold**: Stop early when alive count drops below this.
- **Overview padding**: Extra zoom-out in overview mode.
- **Follow zoom lerp**: Camera smoothing in follow mode.
- **Overview zoom lerp**: Camera smoothing in overview mode.
- **Overview extra margin**: Extra radius beyond the arena in overview.

### Rewards

- **Points per food**: Score gain for eating.
- **Points per kill**: Score gain for kills.
- **Points per second alive**: Passive score while alive.
- **Fitness survival per second**: Fitness weight for time alive.
- **Fitness per food**: Fitness weight for eating.
- **Fitness per grown segment**: Fitness weight for growth.
- **Fitness per kill**: Fitness weight for kills.
- **Fitness points normalization weight**: Fitness contribution from total points.
- **Fitness top points bonus**: Extra fitness for top scorers in a generation.

### Brain and memory

- **GRU hidden size**: Memory width; bigger = more capacity, more parameters.
- **LSTM hidden size**: LSTM memory width.
- **RRU hidden size**: RRU memory width.
- **Brain control dt**: How often the brain updates relative to physics.
- **Recurrent mutation rate (GRU/LSTM/RRU)**: Mutation rate applied to recurrent weights.
- **Recurrent mutation std (GRU/LSTM/RRU)**: Mutation strength for recurrent weights.
- **Recurrent crossover mode (0 block, 1 unit)**: 0 = block, 1 = unit-wise crossover.
- **GRU init update gate bias**: Sets default memory persistence.
- **LSTM init forget gate bias**: Sets default memory persistence for LSTM.
- **RRU init gate bias**: Sets default gating bias for RRU.

## Brain graph editor

The Brain graph panel lets you build any ordering or combination of MLP/GRU/LSTM/RRU/Dense/Split/Concat, including splits and skip connections. Changes require **Apply graph** and then **Apply and reset**. When a custom graph is active, the stack sliders (hidden layers + neurons) are disabled and ignored.

- **Templates**: Quick starting points (Linear MLP, MLP → GRU → MLP, Skip + concat, Split + parallel heads).
- **Nodes**: Each node has an id and a type. Input is fixed to the sensor size. Dense/MLP/GRU/LSTM/RRU input sizes are inferred from wiring and shown read-only. Split uses a comma list of output sizes (must sum to its input size).
- **Edges**: Connect nodes. `fromPort` picks an output on a multi-output node (Split). `toPort` sets input order for multi-input nodes (Concat). Ports are 0-based; leave blank for default ordering.
- **Outputs (simple)**: Pick an output node and optionally **Split into 2 outputs**. A single node with size 2 drives turn + boost. A split uses port 0 → turn and port 1 → boost.
- **Outputs (advanced)**: Expand **Advanced outputs** to map multiple output refs manually. The summed output size must equal 2 (turn + boost).
- **Diagram**: Visualizes the current editor graph left → right. Use **Full screen** to bring it forward while editing.
- **Diagram overlay**: Full screen dims the arena while keeping the right-side control panel visible.
- **Diagram editing**:
  - **Select**: Click a node/edge/output to edit it in the inspector.
  - **Connect**: Drag from the small handle on a node to another node (Split/Concat ports auto-assign).
  - **Move**: Drag nodes to reposition the diagram (visual layout only).
  - **Toolbar**: **Add node**, **Add output**, **Delete**, **Auto layout** (clears manual positions), **Full screen**.
- **Saved presets**: Enter a name and **Save preset** to store it in the server database.
- **Preset loading**: Click a saved preset entry to load it into the editor (you still need Apply graph).
- **Layout persistence**: Diagram positions are UI-only and reset after refresh or Auto layout.
- **Graph storage**: The applied graph spec is saved in browser localStorage; **Reset graph** reloads the applied spec or the default template.
- **Advanced JSON**: Use **Load JSON into editor** to import, **Copy current graph** to populate the JSON editor, and **Export JSON** to download a file.

## Import and export

### Export an exact save

Click **Export** to start one direct `/api/export/latest` download of a flat
`.slither-save` archive:

- The save contains the leased checkpoint, complete compact history, and
  run-scoped Hall of Fame.
- Large population, recurrent-state, and Hall-of-Fame weight entries use
  whichever is smaller: raw storage or lossless compression.
- The archive keeps the best **50 unique unpinned winner genomes**, plus any
  pinned winners, while preserving the complete compact generation history.
- Rust validates and packs the archive, Node streams it, and browser JavaScript
  never reads or rebuilds its population.

### Import an exact or older save

Click **Import** and select the original file. It is uploaded directly as the
request body; browser JavaScript never reads or reconstructs its contents.

| File | Restored behavior |
|---|---|
| `.slither-save` | Rust privately validates the exact-save roles and restores the saved experiment. |
| Older browser `.json` | Rust incrementally parses bounded genomes from disk and starts a new generation-one population lineage. Missing Rust history, allocator, and random-stream state cannot be restored. |

SQLite commits the replacement and active-run pointer before the running game
switches. **A rejected upload leaves the prior game current.**

#### Legacy conversion notices

For older JSON saves, compatible settings and ASCII-identified graphs are
applied. A source seed is retained as source information when present; the new
run uses its own seed for missing state.

Checkpoints retain a `legacyConversion` record, and exports remain classified
as `legacy-population-import` through later generations and restarts.
**Reset** and **New Run** start a fresh lineage and clear that record.

#### Connected clients and existing history

- Successful replacement keeps browser/trainer sockets connected, invalidates
  every old assignment, and requires a fresh ordered join.
- An older save cannot silently overwrite later generations from the same run.
  The page offers an explicit new-run branch, records the source run/generation/
  checkpoint, and preserves the original future.

### What an automatic checkpoint preserves

Automatic restart checkpoints capture an exact **generation boundary**:

- Evolved population and generation.
- Experiment configuration and seed.
- Random-number and deterministic allocator state.

The boundary is saved before the new generation is spawned. Snake positions,
pellets, and recurrent activations are reconstructed from that boundary;
a checkpoint does not resume the middle of a tick.

| Startup choice | Replay scope |
|---|---|
| `--fresh` | Durably records a new run without deleting older snapshots. |
| `--resume <snapshot-id>` | Selects a specific valid checkpoint; exact replay remains tied to the producing build. |
| `--resume latest` | Uses the latest valid checkpoint. A newer compatible build can create a labelled continuation branch without claiming exact replay of the old binary. |

See [Server startup and recovery](#server-startup-and-recovery) for selection
and recovery behavior.

### TypeScript reference compatibility

The explicit `npm run server:reference` runner retains JSON export/import for
comparison. Its JSON exports are portable population files, rather than
automatic exact-resume checkpoints.

Rust's resume-latest path converts compatible per-genome SQLite rows, combined
`genomes_blob` rows, and format-zero parent-JSON populations into new generation-one
runs while preserving the source rows.

Reference JSON import resets that reference simulation to the file contents.
Older graphs may be incompatible with the current v3 sensor layout. **Keep the
database intact** and use a save from a compatible graph/sensor build when
input sizes differ.

## Preset recipes (QA-friendly)

### Fast iteration

Use this to quickly see visible evolution.

- NPC snakes: 30–60
- World radius: 1600–2200
- Generation duration: 20–40
- Mutation rate: 0.05–0.12
- Mutation std: 0.35–0.60
- Elite fraction: 0.10–0.20

### Survival-focused

Encourages long-lived snakes.

- Points per second alive: 1.0–2.5
- Fitness survival per second: 1.5–3.0
- Points per kill: 10–30
- Fitness per kill: 10–30

### Aggressive combat

Encourages hunting and kills.

- Points per kill: 80–150
- Fitness per kill: 100–200
- Points per food: 1–2
- Fitness per food: 2–5

### Foraging/exploration

Encourages food-seeking behavior.

- Points per food: 3–6
- Fitness per food: 10–20
- Pellet target count: 3000–8000
- Pellet spawn per second: 200–600

### Memory-heavy (GRU)

Use GRU for smoother, more deliberate behavior.

- Start from the **MLP → GRU → MLP** graph template.
- GRU hidden size: 24–48
- GRU mutation rate: 0.01–0.03
- GRU mutation std: 0.12–0.25
- Brain control dt: 0.010–0.020

## Visualizer and Hall of Fame

- **Brain Visualizer**: Shows the focused snake’s network activations. If you don’t see anything, switch to follow mode or select a snake.
- **Visualizer streaming**: Data is only requested while the Visualizer tab is active.
- **Fitness Stats**: Switch between Fitness History (min/avg/max), Species Diversity, and Network Complexity.
- **Hall of Fame**: Lets you resurrect top genomes; compact entries and packed winner weights are retained by the server and included in exports.

## Troubleshooting

- **No snakes visible**: Click Apply and reset; reduce world radius or increase snake count.
- **Sim too slow**: Reduce NPC snakes, pellet target count, or world radius.
- **Visualizer empty**: Ensure a snake is focused (Follow mode) and wait a tick.
- **Join disabled**: The local server is not connected yet.
- **Snakes die instantly**: Lower hit scale or increase skip segments near head.
- **Install fails on Windows**: Use Node 24+ and install the Visual Studio C++
  build tools plus a Windows SDK for `better-sqlite3` and the native addon,
  then re-run `npm install`.
- **Native startup failure**: Run `npm --prefix native run build`. The normal
  server does not fall back to JavaScript; use `npm run server:reference --
  --backend js` only for deliberate reference diagnosis.
- **Worker failure**: The server faults the run instead of switching backends
  or publishing a partial step. Use Apply and reset, New Run, or restart from a
  valid checkpoint after addressing the reported cause.
- **Import input size mismatch**: Re-export from a build with the same v3
  sensor size and graph parameter count; do not delete the database as a first
  troubleshooting step.
