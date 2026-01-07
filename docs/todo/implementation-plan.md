# Implementation Plan

Add state-machine baseline bots that run alongside the evolving population,
controlled by deterministic seeds, visually distinct (metallic + robot eyes),
and excluded from fitness/elite/HoF. The plan is split into mergeable stages
that keep CI green and preserve the binary frame contract and worker/server
protocol symmetry described in AGENTS.md.

## Scope

- In: Baseline bot settings (count + seed controls), per-bot seeded state
  machine controller, spawn/ID management, stats/fitness exclusion,
  serialization/rendering skin flag, God Mode parsing compatibility, and
  targeted tests.
- Out: Changes to neural architecture, sensor layout, genetic operators,
  multiplayer join flows, or DB schema beyond what is needed for settings.

## Assumptions

- Baseline bot count adds to the existing NPC count (not a replacement).
- Per-bot seed = base seed + bot id (or a stable hash of those values).
- Baseline bots are excluded from fitness, elite selection, HoF, and player
  assignment.
- Baseline bots use external control only; NN inference is not executed for
  them.

## Constraints and invariants

- Follow AGENTS.md “Binary frame format and rendering pipeline” and keep the
  Float32Array layout unchanged (header + snake blocks + pellet blocks).
- Follow AGENTS.md “Simulation core” on sensor sizing and control timing
  invariants (CFG.brain.inSize alignment, fixed controlDt semantics).
- Follow AGENTS.md “Runtime architecture and data flow” to keep worker/server
  protocol symmetry and fallback behavior intact.
- Follow AGENTS.md “Project-specific conventions and gotchas” on hot-path
  allocation avoidance and typed-array use.

## Delta architecture overview

- Runtime integration (AGENTS.md “Runtime architecture and data flow”): add a
  baseline-bot controller path in worker and server loops without altering
  handshake, join overlay behavior, or viz toggles.
- World lifecycle (AGENTS.md “Simulation core”): spawn baseline bots after the
  population and exclude them from fitness and history aggregation; ensure
  bestPointsThisGen normalization does not incorporate baseline bots.
- Rendering contract (AGENTS.md “Binary frame format and rendering pipeline”):
  extend the skin flag domain to include a robot value, and update serializer,
  renderer, and main-thread buffer parsing together.
- UI/settings (AGENTS.md “UI, settings, and visualization panels”): add
  baseline bot count and seed controls in Settings and include them in settings
  updates for worker/server resets.
- Tests (AGENTS.md “Tests and verification”): update existing Vitest suites
  that assert settings, serialization, rendering, and world lifecycle.

## Key decisions and invariants registry

### Decisions

- DEC-001: Represent baseline bots as separate, non-population snakes appended
  after the population in World.snakes. Rationale: preserves population order
  assumptions in World._endGeneration without expensive filters. Alternative:
  separate World for bots (rejected: complicates rendering and collisions).
- DEC-002: Use external control for baseline bots and prevent NN inference
  execution; optionally attach a NullBrain to avoid unnecessary allocations.
  Rationale: satisfies “no NN brain in the background” and keeps hot paths
  stable. Alternative: keep brain and rely on external control only (rejected
  because it still builds NN weights per bot).
- DEC-003: Per-bot seed derived from base seed + bot id (or hash), with optional
  per-generation random base seed re-roll. Rationale: deterministic behavior
  per generation while allowing controlled variability. Alternative: global RNG
  stream shared with world (rejected: perturbs evolution randomness).
- DEC-004: Extend skin flag values without changing buffer layout. Rationale:
  minimal compatibility risk and no pointer math changes. Alternative: add new
  fields to buffer (rejected: larger contract change).

### Invariants

- INV-001: Frame buffer layout remains 6-float header, per-snake blocks of
  8 + 2*ptCount floats, then pellet blocks of 1 + 5*count floats.
- INV-002: Sensor vector length equals CFG.brain.inSize and stays unchanged.
- INV-003: Worker <-> main message shapes stay in sync under src/protocol/.
- INV-004: Controller timing uses fixed CFG.brain.controlDt semantics.
- INV-005: Hot-loop allocations (World.update, sensors, serialization) remain
  bounded and avoid per-tick object churn.

## Alternatives considered

- Separate world for baseline bots: avoids population contamination but
  duplicates physics and rendering work; rejected for perf and complexity.
- Server-only baseline bots: easier to implement but breaks worker fallback
  parity; rejected to preserve worker/server symmetry.
- Use color strings only (no skin flag): simpler but cannot drive distinct eye
  rendering in fast path; rejected due to renderer contract.

## Dependencies and sequencing

- Stage 01 (`docs/todo/01-baseline-bot-settings-ui.md`) provides config and UI
  controls and expands settings update types. No runtime behavior changes.
- Stage 02 (`docs/todo/02-baseline-bot-runtime.md`) adds baseline bot controller
  logic, spawn lifecycle, RNG separation, and stats exclusion. Depends on Stage
  01 settings keys and types.
- Stage 03 (`docs/todo/03-bot-rendering-buffer.md`) adds skin flag semantics,
  renderer changes, and God Mode parsing alignment. Depends on Stage 02 bot
  identity flags and Stage 01 settings.

## Data model changes overview

- CFG additions: new baseline bot settings (count, seed, per-generation random
  seed toggle, and optional enable flag). Defaults keep behavior unchanged.
- SettingsUpdate path union: add new paths for baseline bot controls and seed.
- No new localStorage keys planned; seed values flow via settings updates and
  export payloads only.
- Frame buffer: skin flag domain expanded (0 default, 1 gold, 2 robot). Layout
  unchanged. Expand/migrate/contract plan:
  - Expand: update renderer and serializer to accept 2.
  - Migrate: start writing 2 for baseline bots after the renderer supports it.
  - Contract: not required; keep support for 0/1/2.

## State machine designs

### Connection and fallback flow (unchanged, documented for invariants)

State table

| State | Description | Invariants |
| --- | --- | --- |
| Connecting | Attempting WS connection, worker may be idle | Join overlay visible if no worker |
| Server | WS connected, server streaming frames | Worker stopped; join overlay visible |
| Worker | Worker active, local frames | Join overlay hidden; no WS control |
| WorkerFallbackPending | WS not connected, fallback timer running | No duplicate worker start |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| connect(url) | any | Connecting | wsClient available | schedule fallback | single fallback timer |
| wsConnected | Connecting | Server | none | stop worker, reset stats | join overlay visible |
| wsDisconnected | Server | Connecting or Worker | worker exists? | schedule fallback + reconnect | no double worker |
| fallbackTimeout | Connecting | Worker | ws not connected | start worker | join overlay hidden |
| manualStartWorker | any | Worker | worker not started | create worker | worker-only mode |

### Viz streaming toggle

State table

| State | Description | Invariants |
| --- | --- | --- |
| VizDisabled | Viz not streaming | wsClient.sendViz(false) or worker message |
| VizEnabled | Viz streaming active | wsClient.sendViz(true) or worker message |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| tabSwitch(viz) | VizDisabled | VizEnabled | tab-viz active | send viz enabled | single stream source |
| tabSwitch(other) | VizEnabled | VizDisabled | not tab-viz | send viz disabled | no redundant stream |
| disconnect | VizEnabled | VizDisabled | ws drop | clear viz payload | no stale viz |

### Baseline bot generation lifecycle

State table

| State | Description | Invariants |
| --- | --- | --- |
| SeedPending | Generation start, seed selection pending | seed derivation uses bot RNG only |
| SeedSet | Base seed selected | per-bot seed = base + bot id |
| BotsSpawned | Baseline bots appended to World.snakes | population order preserved |
| Running | Bots controlled each tick | NN inference not executed for bots |
| GenEndPending | EndGeneration triggered | bots excluded from fitness |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| generationStart | SeedPending | SeedSet | randomize toggle on/off | choose base seed | no shared RNG |
| spawnBots | SeedSet | BotsSpawned | count > 0 | append bots | population order |
| tick | BotsSpawned | Running | none | compute bot actions | external control only |
| endGeneration | Running | GenEndPending | generation end | exclude bots from fitness | stats accuracy |
| resetWorld | any | SeedPending | reset triggered | clear bot state | no stale actions |

### Import/export flow (settings + seed)

State table

| State | Description | Invariants |
| --- | --- | --- |
| Idle | No import/export active | settings in UI reflect CFG |
| ExportPending | Export requested | include baseline bot settings |
| ImportPending | Import file selected | validate settings/graph |
| ServerResetWait | Server reset in flight | wait for tick reset |
| Completed | Import/export done | UI consistent |
| Error | Import/export error | errors surfaced to user |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| exportClick | Idle | ExportPending | none | gather settings + HoF | include bot settings |
| exportDone | ExportPending | Completed | ok | download file | no partial file |
| importClick | Idle | ImportPending | file selected | parse + validate | settings apply |
| serverReset | ImportPending | ServerResetWait | server mode | send reset | tick waits |
| importDone | ImportPending | Completed | ok | update UI | no stale settings |
| error | any | Error | none | show message | recoverable |

## Error handling strategy

- Input validation: non-finite seeds, negative counts, or invalid toggles are
  rejected with UI hints and logged at debug level.
- Runtime errors: baseline bot controller exceptions are caught per tick and
  disable bot updates for that generation (recoverable on reset).
- Protocol errors: unknown settings paths are ignored with a warning; no crash.
- Fatal errors: buffer layout mismatch detected in tests only (CI gate).

## Performance considerations

- Bot controller uses preallocated buffers and avoids per-tick allocations.
- Bot action computation happens once per tick per bot, reused for substeps.
- No additional fields in serialized buffer to avoid extra memory copies.

## Security and privacy considerations

- No new secrets; seed values are non-sensitive numeric data.
- Export/import files include bot settings; do not log file contents.
- Avoid logging raw settings payloads at info level; use debug gating.

## Observability

- Proposed log events (debug gated):
  - bot.seed.selected { baseSeed, generation, randomize }
  - bot.spawned { count, idRangeStart, idRangeEnd }
  - bot.controller.error { snakeId, state, error }
  - bot.stats.filtered { excludedCount }
- Debug toggle: CFG.debug.baselineBots (default false) or console flag.

## Rollout plan

- Merge-safe gating: baseline bots are enabled only when count > 0 (default 0).
- Rollback: revert bot modules and config keys; older builds ignore unknown
  settings paths. Buffer skin flag value 2 is ignored by older renderers, so
  rollback may show default colors but remains functional.

## Acceptance criteria

- AC-001: Baseline bots spawn in worker and server with count added to NPCs;
  population count remains unchanged.
- AC-002: Baseline bots never execute NN inference and never enter fitness,
  elite, or HoF selection.
- AC-003: Bot RNG is independent from sim RNG; per-bot seed derives from base
  seed + bot id; per-generation randomization toggle works.
- AC-004: Stats/fitness history exclude baseline bots in worker and server.
- AC-005: Baseline bots are never assigned to player controllers; join overlay
  behavior remains unchanged.
- AC-006: Skin flag renders metallic robot bots with robot eyes; God Mode
  parsing remains correct.
- AC-007: Import/export retains new bot settings without breaking older files.
