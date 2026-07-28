# Slither Neuroevolution

A browser-based neuroevolution sandbox inspired by Slither.io. Populations of snakes evolve neural networks, learn to seek food, avoid hazards, and compete across generations. This README is written for users and QA testers who want to run the sim, understand the UI, and explore behavior.

## Key Features

- **Remote browser client**: The browser renders server frames and sends controls; it does not run a second game.
- **Rust-authoritative migration**: The approved implementation is moving the complete authoritative game—world state, sensing, differently weighted brains, movement, collision, evolution, and frame packing—into Rust behind a thin Node interface.
- **Current reference runtime**: Until cutover passes its correctness, Debian-VM, LAN-browser, RL-trainer, persistence, and recovery gates, the existing TypeScript `SimCore`/`World` remains available as the selected reference and test oracle.
- **Deep Evolution**: Supports MLP, GRU, LSTM, and RRU architectures with complex genetic operators and a modular graph editor.
- **Deterministic run controls**: Reset repeats a seed; New Run starts and checkpoints a different seed.
- **Bounded persistence target**: Managed immutable checkpoint files hold packed binary population data; SQLite holds small metadata/history/indexes. Browser import/export becomes direct file upload/download without population-sized JavaScript objects.

## Quick start

### Prerequisites

- **Node.js**: v22 or newer
- **Rust**: Required for compiling the native acceleration layer. Install via [rustup.rs](https://rustup.rs).
- **Windows build tools**: Visual Studio C++ build tools and a Windows SDK are required by native dependencies.

### Running

The simulation runs in a server-authoritative mode. You must start the simulation server and the Vite dev server separately (or use the convenience launchers).

```bash
npm install
npm --prefix native run build
npm run server
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
addon, start both services, and write logs/PID files in the repository root:

- Windows: `play.bat`
- macOS/Linux: `play.sh`

Open the local URL printed by Vite (usually `http://localhost:5173`).

Note: This project uses ES modules, so opening `index.html` directly in a file browser will not work.

### Architecture

This application uses a pure client/server model. The browser renders binary
frames and submits controls over Protocol 2 WebSocket messages. There is no
browser-local World, offline mode, or local simulation worker. The current
branch still runs the authoritative game in Node/TypeScript while the approved
Rust-owned engine is implemented and tested beside it. After cutover, Rust
will own the game and Node will only route HTTP/WebSocket/file traffic and
small SQLite metadata. The TypeScript game will remain a selected test oracle
during stabilization, not an automatic production fallback.

Loopback is the default, and deliberate use from a phone or another computer
on the same trusted home LAN is supported. The project has no accounts,
authentication, authorization, or TLS, so do not expose it through router port
forwarding or run it on an untrusted network.

On first server startup, `server/config.ts` creates the ignored
`server/config.toml` file from current defaults. Useful fields include the
server/UI bind addresses and ports, `publicWsUrl`, checkpoint interval,
native/JS diagnostic backend, and worker settings. `publicWsUrl` is simply the
WebSocket address the webpage should use when the simulation server is not at
the same hostname as the UI; despite the legacy word “public,” it does not make
the service safe for the public internet. Normal defaults are native,
single-threaded inference and resume-latest. Use `--mt` (and optionally
`--mt-workers N`) for native MT, `--backend js` only for diagnosis, `--fresh`
for a new durable run, or `--resume <snapshot-id>` for one compatible
checkpoint.

### Open it from a phone or another home computer

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
- `test:acceptance`: owner-visible end-to-end contracts.
- `test:regression`: named historical failures.
- `test:performance`: measured budgets; currently informational in CI.
- `test:security`: protocol/input and resource-boundary hardening.
- `test:native-required`: additive native and multi-thread contracts that fail
  rather than skip when the addon is unavailable.

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
- **Sensors debug logs**: Enables sensor debug logging.
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

Population import/export lives in the Settings tab and writes a JSON file that includes the population, applied settings, the active graph spec, and Hall of Fame entries. The server streams its snapshot without constructing one population-sized JSON string; the browser then assembles the download so it can add UI settings and Hall of Fame entries. Imports replace the population and settings, but an imported seed is retained as file metadata; it does not silently change the active run's seed.

Automatic restart checkpoints are exact generation-boundary population
checkpoints. They preserve the evolved population, generation, experiment
configuration, seed, random-number state, and deterministic allocator state
before the new generation is spawned. They are not arbitrary mid-tick world
saves: transient snake positions, pellets, and recurrent activations are
reconstructed from the saved generation-start boundary instead of being
restored from the middle of a tick. Normal startup resumes the latest valid
checkpoint; use `--fresh` to start and durably record a new run without
deleting older snapshots, or `--resume <snapshot-id>` to select a specific
valid checkpoint.

The **Export** button first creates a population-export snapshot, then streams
JSON one genome at a time to the browser. A population export is portable but
is not selected for automatic exact resume. Current resumable checkpoints use
per-genome SQLite rows; the older combined `genomes_blob` format remains
read-only compatibility.

Imports reset the simulation to the file contents under the active run identity.
Imports from older builds may be incompatible with the current v3 sensor
layout. Keep the database intact and use an export produced by a compatible
graph/sensor build when input sizes differ.

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
- **Hall of Fame**: Lets you resurrect top genomes; Hall of Fame entries are stored in browser storage and included in exports.

## Troubleshooting

- **No snakes visible**: Click Apply and reset; reduce world radius or increase snake count.
- **Sim too slow**: Reduce NPC snakes, pellet target count, or world radius.
- **Visualizer empty**: Ensure a snake is focused (Follow mode) and wait a tick.
- **Join disabled**: The local server is not connected yet.
- **Snakes die instantly**: Lower hit scale or increase skip segments near head.
- **Install fails on Windows**: Use Node 22+ and install the Visual Studio C++
  build tools plus a Windows SDK for `better-sqlite3` and the native addon,
  then re-run `npm install`.
- **Native startup failure**: Run `npm --prefix native run build`. JavaScript is
  available only as the explicit `--backend js` diagnostic mode.
- **Worker failure**: The server faults the run instead of switching backends
  or publishing a partial step. Use Apply and reset, New Run, or restart from a
  valid checkpoint after addressing the reported cause.
- **Import input size mismatch**: Re-export from a build with the same v3
  sensor size and graph parameter count; do not delete the database as a first
  troubleshooting step.
