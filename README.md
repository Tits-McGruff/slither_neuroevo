# Slither Neuroevolution

A browser-based neuroevolution sandbox inspired by Slither.io. Populations of snakes evolve neural networks, learn to seek food, avoid hazards, and compete across generations. This README is written for users and QA testers who want to run the sim, understand the UI, and explore behavior.

## Quick start

You need Node.js installed.

```bash
npm install
npm run dev
```

Open the local URL printed by Vite (usually `http://localhost:5173`).

Note: This project uses ES modules, so opening `index.html` directly in a file browser will not work.

### Server mode (optional)

Run the simulation server in a second terminal to enable multiplayer join/spectate and DB-backed graph presets.

```bash
npm run server
npm run dev
```

The UI will connect over WebSocket and show **SERVER** in the status pill. If the server is down, the app falls back to a local Web Worker.
Server mode also enables DB-backed graph presets in the Brain graph panel.
<!-- Added to reflect server-only graph preset storage in server/httpApi.ts and main.ts. -->
By default the client connects to `ws://localhost:5174`; use `?server=ws://host:port` to point at a different server.
<!-- Added to document resolveServerUrl query parameter support in src/net/wsClient.ts. -->

## Controls

- `V`: Toggle camera mode between Overview and Follow.
- Left click: Select a snake (God Mode selection).
- Right click: Kill the selected snake (God Mode).
- Left click + drag: Move a selected snake (God Mode).
- Mouse to steer, hold click to boost (when playing as a user snake).
- **Settings lock**: Hides all sliders and controls inside the Settings tab; unlock to edit.
- **Apply and reset**: Rebuild the world using reset-only settings.
- **Defaults**: Restore default slider values and reset.

## Join and spectate

- Enter a nickname, then **Play** to spawn a player snake (server mode only).
- **Spectate** starts the sim with no player control.
- If the server is unavailable, the join overlay is hidden and the sim runs in worker mode. <!-- Updated to match src/main.ts hiding the overlay when worker mode is active. -->
- When a server connection is established, the client auto-spectates in overview mode until you join.
<!-- Added to reflect wsClient onConnected sending spectator + overview in src/main.ts. -->
- Player control begins after the server sends an assignment; the overlay hides once assigned.
<!-- Added to reflect assign handling and overlay hide in src/main.ts. -->

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
- **Simulation speed**: Multiplier for physics time. Higher values run faster but reduce visual clarity.
- **AI hidden layers**: How many MLP hidden layers to use (1–5). More layers increase capacity.
- **Neurons layer 1–5**: The size of each hidden layer. Only layers up to the selected count are active.

### World and food

- **World radius**: Arena size. Larger maps spread snakes and food farther apart.
- **Pellet target count**: Total pellets kept in the arena. More pellets means faster growth.
- **Pellet spawn per second**: Refill rate when pellets are eaten or removed.
- **Food value per pellet**: How much points and growth one pellet provides.
- **Growth per food**: How many body segments a pellet adds.

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
<!-- Updated label to match settings UI in src/settings.ts. -->
- **Recurrent mutation std (GRU/LSTM/RRU)**: Mutation strength for recurrent weights.
<!-- Updated label to match settings UI in src/settings.ts. -->
- **Recurrent crossover mode (0 block, 1 unit)**: 0 = block, 1 = unit-wise crossover.
<!-- Updated label to match settings UI in src/settings.ts. -->
- **GRU init update gate bias**: Sets default memory persistence.
- **LSTM init forget gate bias**: Sets default memory persistence for LSTM.
- **RRU init gate bias**: Sets default gating bias for RRU.

## Brain graph editor

The Brain graph panel lets you build any ordering or combination of MLP/GRU/LSTM/RRU/Dense/Split/Concat, including splits and skip connections. Changes require **Apply graph** and then **Apply and reset**.

- **Templates**: Quick starting points (Linear MLP, MLP → GRU → MLP, Skip + concat, Split + parallel heads).
- **Nodes**: Each node has an id and a type. Input is fixed to the sensor size. Dense/MLP use input/output sizes, GRU/LSTM/RRU use hidden size, Split uses a comma list of output sizes (must sum to its input size).
- **Edges**: Connect nodes. `fromPort` picks an output on a multi-output node (Split). `toPort` sets input order for multi-input nodes (Concat). Ports are 0-based; leave blank for default ordering.
<!-- Kept wording but added below to clarify connect-mode auto-assignment. -->
- **Outputs**: Defines which node outputs become the final brain output. If the output comes from a Split, set `port`. The summed output size must equal 2 (turn + boost).
- **Diagram**: Visualizes the current editor graph left → right. Use **Full screen** to bring it forward while editing.
- **Diagram overlay**: Full screen dims the arena while keeping the right-side control panel visible.
<!-- Added to describe graph-diagram fullscreen/backdrop behavior in styles.css. -->
- **Diagram editing**:
  - **Select**: Click a node/edge/output to edit it in the inspector.
  - **Connect**: Click **Connect**, then click a start node and a target node to add an edge (Split/Concat ports auto-assign). <!-- Clarifies connect mode behavior from graph editor logic. -->
  - **Move**: Drag nodes to reposition the diagram (visual layout only).
  - **Toolbar**: **Add node**, **Add output**, **Delete**, **Auto layout** (clears manual positions), **Full screen**. <!-- Matches current diagram toolbar buttons in index.html. -->
- **Saved presets**: Enter a name and **Save preset** to store in the server database; in worker mode the list stays empty.
<!-- Added the save workflow from the graph preset UI in index.html. -->
- **Preset loading**: Click a saved preset entry to load it into the editor (you still need Apply graph).
<!-- Added to reflect refreshSavedPresets/load handler in src/main.ts. -->
- **Layout persistence**: Diagram positions are UI-only and reset after refresh or Auto layout.
- **Graph storage**: The applied graph spec is saved in browser localStorage; **Reset graph** reloads the applied spec or the default template.
- **Advanced JSON**: Use **Load JSON into editor** to import, **Copy current graph** to populate the JSON editor, and **Export JSON** to download a file.
<!-- Added to document the Advanced JSON controls in index.html. -->

### Misc

- **Frame dt clamp**: Max time step per physics update (stability guard).

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

- Use GRU memory: 1
- GRU hidden size: 24–48
- GRU mutation rate: 0.01–0.03
- GRU mutation std: 0.12–0.25
- Brain control dt: 0.010–0.020

## Visualizer and Hall of Fame

- **Brain Visualizer**: Shows the focused snake’s network activations. If you don’t see anything, switch to follow mode or select a snake.
- **Visualizer streaming**: Data is only requested while the Visualizer tab is active. <!-- Reflects viz toggle behavior in src/main.ts. -->
- **Fitness Stats**: Shows min/avg/max over generations.
- **Hall of Fame**: Lets you resurrect top genomes; imports/exports are stored in browser storage.

## Troubleshooting

- **No snakes visible**: Click Apply and reset; reduce world radius or increase snake count.
- **Sim too slow**: Reduce NPC snakes, pellet target count, or world radius.
- **Windows install fails**: Server dependencies use `better-sqlite3`; install Visual Studio C++ build tools plus a Windows SDK, then re-run `npm install`.
<!-- Added to document native build requirements triggered by better-sqlite3 on Windows. -->
- **Visualizer empty**: Ensure a snake is focused (Follow mode) and wait a tick.
- **Join disabled**: The server is not connected; worker mode does not allow player control. <!-- Matches join flow requiring server connection. -->
- **Snakes die instantly**: Lower hit scale or increase skip segments near head.
- **Server install fails on Windows**: Use Node 20 LTS or install the Visual Studio C++ build tools + Windows SDK (for `better-sqlite3`).
