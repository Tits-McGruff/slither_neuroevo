# Multiplayer + storage + extensible brains plan

This plan is intentionally verbose so future-me has full context. It covers
high-level architecture, low-level design, and a phased execution path.

## Goals

- Run the simulation in a server-authoritative loop so multiple clients can
  connect (humans + external bots).
- Expose per-snake sensor data and accept control actions over a public-ish API.
- Keep the core sim fast, deterministic, and compatible with evolution.
- Support multiple brain types (MLP, GRU, LSTM, RRU) and flexible layouts
  (branches, skips, reordering).
- Persist key data (Hall of Fame, population snapshots, sessions) without
  putting the database on the hot path.

## Non-goals (initially)

- Full auth system, payments, or production-grade security.
- Global scale or massive concurrency.
- Guaranteed backward compatibility for every future brain layout.

## Decision checklist (resolve before implementation)

- [x] Single world session (one world per server instance).
- [x] Bot access model: open (no keys).
- [x] Persistence scope: include player profiles.
- [x] Sensor payload shape: raw floats (with schema metadata in handshake).
- [x] Tick rate: configurable (server config/CLI).
- [x] Action handling: keep last-known action for N ticks, then neutral + optional AI fallback.
- [x] Frame delivery: rate-limit for UI clients (drop excess).
- [x] Keep worker fallback in client (for now).
- [x] DB choice now: SQLite only (clean interface for later swap).
- [x] Brain evolution scope: weight-only mutation (for now).

## Configuration defaults (initial, configurable)

- tickRateHz: 60
- uiFrameRateHz: 30 (rate-limit UI frames; drop excess)
- actionTimeoutTicks: 10 (keep last-known for 10 ticks, then neutral)
- actionTimeoutFallback: optional AI fallback after timeout (for bot/player controllers)
- maxActionsPerTick: 1 (action contains both turn + boost)
- maxActionsPerSecond: 120 (soft cap; drop extras)
- checkpointEveryGenerations: 1 (configurable + manual save)
- playerNameMaxLen: 24 (trim + sanitize)

## High-level architecture

Clients (browser UI and external bots) connect to a Node server via WebSocket.
The server runs the authoritative world loop, applies actions, and sends
frame data and per-player sensors. The database is only for persistence
(checkpoints, Hall of Fame, optional player profiles), not for real-time state.

Rough layout:

- Browser UI (Vite client)
  - Renders frames
  - Sends player actions
  - Receives sensors when in bot/human control mode

- Bot clients (Python/JS/etc.)
  - Connect via WebSocket
  - Receive sensors and world meta
  - Send action commands

- Node server (authoritative sim)
  - world update loop
  - action queue / controller manager
  - serializer for frames
  - WebSocket hub
  - persistence module

- SQLite database (file-based)
  - Hall of Fame entries
  - population snapshots
  - optional player/bot registry

## Tick data flow

1) Server tick T starts.
2) For each snake:
   - Sensors computed and cached (already happens today in Snake.update).
   - Controller chosen:
     - Brain controller: uses MLP/GRU forward.
     - Player/bot controller: uses last received action for that snake.
3) World updates with FIXED_DT (server authoritative).
4) Frame buffer is serialized (Float32Array) for UI rendering clients.
5) Per-client sensors are packaged and sent (only to the client who owns
   a snake, not broadcast).
6) Client actions arrive asynchronously and are queued for the next tick.

Key rule: if actions are late, we use the last known action (or neutral).
This keeps the sim stable and avoids stalling the loop.

## Network protocol (draft)

WebSocket messages are JSON for control and metadata, binary for frame buffers.

Client -> server (JSON):

- hello: { type: "hello", clientType: "ui" | "bot", version: 1 }
- join: { type: "join", mode: "spectator" | "player", name?: string }
- action: { type: "action", tick: number, snakeId: number, turn: number, boost: number }
- ping: { type: "ping", t: number }

Server -> client (JSON):

- welcome: { type: "welcome", sessionId, tickRate, worldSeed, cfgHash }
- assign: { type: "assign", snakeId, controller: "player" | "bot" }
- sensors: { type: "sensors", tick, snakeId, sensors: number[], meta: { x, y, dir } }
- stats: { type: "stats", tick, gen, alive, fps, fitnessData? }
- error: { type: "error", message }

Server -> client (binary):

- frame buffer (ArrayBuffer) serialized by existing serializer

Notes:

- UI clients get frames at render rate; bots can request lower frequency if
  needed (optional "subscription" message later).
- Tick numbers make it easier to detect stale actions.
- Each action message carries both `turn` and `boost` so one action per tick is enough.
- We can keep this protocol minimal and extend later without breaking.

## Server module layout (proposed)

- server/index.ts
  - bootstraps HTTP + WebSocket
  - spins sim loop

- server/simServer.ts
  - owns world instance
  - tick loop and action dispatch
  - sensor publishing

- server/controllerRegistry.ts
  - maps snakeId -> controller (brain, player, bot)
  - stores last action for each controlled snake

- server/persistence.ts
  - SQLite connection
  - read/write population snapshots, HoF entries

- server/wsHub.ts
  - manages client connections
  - routes messages to simServer

- shared core (reuse existing)
  - src/config.ts
  - src/world.ts
  - src/snake.ts
  - src/mlp.ts (and future brains)
  - src/sensors.ts
  - src/serializer.ts
  - src/protocol/*

Important: server must not import any browser-only modules (render, DOM).

## Persistence strategy (why minimal on hot path)

DB writes are slower and can block event loop if used in the tick loop.
The world state should stay in memory; DB writes happen at safe boundaries
(end of generation, manual save, periodic checkpoint). This preserves sim
performance and keeps multiplayer stable.

Suggested persistence events:

- End of generation: save top genomes and summary stats
- Manual save/export: dump entire population
- Periodic snapshot (every N generations)

Optional extras (if desired):

- Player profiles (name, createdAt)
- Bot registry (api keys, allowed rate)
- Match history or leaderboards

## SQLite schema sketch

Minimal tables:

- hof_entries
  - id INTEGER PRIMARY KEY
  - created_at INTEGER
  - gen INTEGER
  - seed INTEGER
  - fitness REAL
  - points REAL
  - length REAL
  - genome_json TEXT

- population_snapshots
  - id INTEGER PRIMARY KEY
  - created_at INTEGER
  - gen INTEGER
  - payload_json TEXT

Optional tables if multiplayer identity is needed:

- players
  - id TEXT PRIMARY KEY (uuid)
  - name TEXT
  - created_at INTEGER

- api_keys
  - key TEXT PRIMARY KEY
  - owner_id TEXT
  - created_at INTEGER
  - revoked_at INTEGER

- sessions
  - id TEXT PRIMARY KEY
  - created_at INTEGER
  - seed INTEGER
  - cfg_json TEXT

## Migration plan from current client-only sim

Step A: isolate core sim

- Ensure world/snake/mlp/sensors/serializer have no DOM or window usage.
- Create server-specific entry that imports only these modules.

Step B: create server process

- Add a new Node entry point in /server
- Add a WS server (ws or uWebSockets.js) and basic HTTP for health
- Start loop with FIXED_DT and stream frames

Step C: client connects to server

- Add a WS client in main.ts
- Render frames from server instead of worker
- Keep worker path as a fallback until stable

Step D: controller override

- Add a controller interface so a snake can be driven by:
  - local brain (default)
  - player action (external)
- Expose sensors for assigned snakes only

Step E: persistence

- Add SQLite writes on generation end
- Implement export/load endpoints

## Brain interface and registry

Introduce a minimal Brain API so different architectures are plug-in modules:

- Brain
  - forward(input: Float32Array): Float32Array
  - reset(): void
  - toJSON(): BrainJSON
  - fromJSON(json: BrainJSON): Brain
  - paramLength(): number

- BrainFactory
  - build(arch: BrainArch): Brain

Current MLP/GRU become implementations behind this interface.

## Flexible brain layout (graph-based)

Define a small DAG format to describe arbitrary layouts:

- Node types: MLP, GRU, LSTM, RRU, Dense, Concat, Split
- Edge types: forward connection with optional weight matrix

Example layout:

- input -> MLP -> Split
  - Split[0] -> GRU -> Dense -> output
  - Split[1] -> MLP -> output

Compiler steps:

1) Validate graph (no cycles, shapes compatible).
2) Allocate parameter slices in a single Float32Array.
3) Build an execution list (topological order).
4) Run forward pass by iterating nodes and reading/writing buffers.

Mutation/crossover:

- Keep genome as a single float array
- Each node knows its parameter slice
- Mutate all slices uniformly for now
- Later: support structural mutations (add/remove nodes)

## External bot API design

Goal: let bots run their own heavy ML outside the sim.

- Bot registers via WS or HTTP
- Server assigns a snakeId to each bot
- Server sends sensors for that snake each tick
- Bot returns action { turn, boost } for the next tick

Reliability rules:

- If no action arrives by tick deadline, reuse last action
- After `actionTimeoutTicks`, drop to neutral and optionally hand control back to AI
- If bot disconnects, reassign snake to AI brain

## Performance targets

- Server tick: configurable (default 60 Hz)
- Max frame serialization: under 2 ms per tick (goal)
- WS broadcast: send binary buffer to UI clients only
- Sensors: send only to owning client to reduce bandwidth

## Tests and verification

- Unit tests for controller override and action timing
- Integration test: server headless sim runs 300 ticks without drift
- Protocol tests: sensors and actions align with tick numbers

## Milestones and order of work

Phase 1: Server skeleton

- [ ] Create /server entry
- [ ] Run world loop in Node
- [ ] Broadcast frames over WS

Phase 2: Client reroute

- [ ] UI receives frames from WS
- [ ] Remove worker dependency (keep fallback)

Phase 3: Controller override

- [ ] Player/bot actions control a snake
- [ ] Sensors sent per player

Phase 4: Persistence

- [ ] SQLite schema + save/load
- [ ] Import/export through server endpoints

Phase 5: Brain registry

- [ ] MLP/GRU behind Brain interface
- [ ] Add LSTM (or RRU) as new module

Phase 6: Graph-based layouts

- [ ] DAG format + compiler
- [ ] Update arch config and genome mapping

## Open questions

- [x] Do we want multi-session support (more than one world at a time)? (No, single world per instance)
- [x] Should sensors be normalized or raw? (Raw + schema metadata)
- [x] Do we want a bot sandbox or rate limiting? (Configurable soft limits)
- [x] How often to checkpoint populations? (Configurable + manual save)
