# Implementation Plan

## Revision notes

- Clarified baselineBotIndex identity, deterministic seed derivation rules,
  and respawn semantics to avoid conflating snakeId with bot identity.
- Resolved import/export consistency and stats-vs-frame-header semantics, with
  mandatory totals fields (no backward-compatibility requirements).
- Tightened skin flag equality/rollback expectations and expanded test/AC
  mappings and observability guidance.
- Locked Stats panel totals to always show (no debug gating).
- Observability updated to hybrid logging: warnings/errors always on, verbose
  logs debug-gated.
- Rollout gating updated: baseline bots still gated by count; skin=2 emission
  not gated by bot count.
- Locked implementation choices: NullBrain required, baselineBotIndex stored on
  Snake, RNG helper in `src/rng.ts`, hash-based seed derivation required.
- Locked persistence decisions: add localStorage and SQLite persistence for
  baseline bot settings with explicit contract steps.

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
- Baseline bots use a stable baselineBotIndex (0..count-1) that is distinct
  from snakeId and remains stable across respawns.
- Per-bot seed is derived via the `src/rng.ts` hash from
  `(baseSeed, baselineBotIndex)`; generation is included only when
  randomizeSeedPerGen is enabled.
- Baseline bots are excluded from fitness, elite selection, HoF, and player
  assignment.
- Baseline bots use external control only; NN inference is not executed for
  them.
- Baseline bots respawn deterministically after death using the same
  baselineBotIndex and a reset state machine for the current generation.

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
- Stats semantics (AGENTS.md “Runtime architecture and data flow”): keep frame
  header counts for rendering while emitting population-only counts in stats
  payloads, with explicit total fields to avoid ambiguity.
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
- DEC-002 (superseded by DEC-007): Use external control for baseline bots and
  prevent NN inference execution; attach a NullBrain to avoid unnecessary
  allocations. Superseded to require NullBrain and lock identity
  storage and RNG location choices. Migration impact: baseline bots must use
  NullBrain and `baselineBotIndex` on `Snake`.
- DEC-003 (superseded by DEC-005): Per-bot seed derived from base seed + bot id
  (or hash), with per-generation random base seed re-roll. Superseded to avoid
  conflating snakeId with bot identity and to formalize generation
  inclusion rules. Migration impact: update seed derivation to use
  baselineBotIndex and update tests that assumed snakeId-based derivation.
- DEC-004: Extend skin flag values without changing buffer layout. Rationale:
  minimal compatibility risk and no pointer math changes. Alternative: add new
  fields to buffer (rejected: larger contract change).
- DEC-005: Introduce baselineBotIndex (0..count-1) as the stable bot identity
  for seeding and state tracking; derive per-bot seed from
  `(baseSeed, baselineBotIndex)` and include `generation` only when
  `randomizeSeedPerGen` is enabled. Rationale: deterministic behavior without
  tying identity to snakeId. Alternative: use snakeId mapping (rejected: id
  reuse on respawn breaks determinism).
- DEC-006: On respawn, reset the bot state machine and RNG stream to the
  per-bot seed for the current generation. Rationale: deterministic replay per
  generation and stable behavior across death timing. Alternative: continue
  RNG stream across respawns (rejected: makes behavior depend on death timing).
- DEC-007: Baseline bots must use a NullBrain (no NN weights built) and store
  `baselineBotIndex` directly on `Snake`. RNG helpers live in `src/rng.ts`, and
  seed derivation uses a required hash function (no simple addition).
  Rationale: avoids NN compute, simplifies identity access, and isolates RNG
  logic. Alternatives: external-only control without NullBrain, manager-side
  mapping, or utility-housed RNG (rejected per F1–F4 choices).
- DEC-008: Persist baseline bot settings in both localStorage and SQLite
  snapshots using explicit schema changes. Rationale: bot settings survive
  reloads and server restarts without relying solely on exports. Alternatives:
  session-only settings or export-only persistence (rejected per G1–G2).

### Invariants

- INV-001: Frame buffer layout remains 6-float header, per-snake blocks of
  8 + 2*ptCount floats, then pellet blocks of 1 + 5*count floats.
- INV-002: Sensor vector length equals CFG.brain.inSize and stays unchanged.
- INV-003: Worker <-> main message shapes stay in sync under src/protocol/.
- INV-004: Controller timing uses fixed CFG.brain.controlDt semantics.
- INV-005: Hot-loop allocations (World.update, sensors, serialization) remain
  bounded and avoid per-tick object churn.
- INV-006: baselineBotIndex is stable across respawns and never derived from
  snakeId; baseline bot ids use a reserved range distinct from player/external.
- INV-007: Frame header `totalSnakes`/`aliveCount` reflect serialized snakes
  (including baseline bots), while stats payloads expose population-only
  metrics explicitly.
- INV-008: Baseline bot spawning and decision-making do not consume global
  RNG; a dedicated PRNG is used to avoid perturbing evolution randomness.
- INV-009: Skin flag comparisons are strict (`skin === 1`/`skin === 2`) and
  unknown values fall back to default rendering.
- INV-010: RNG helpers live in `src/rng.ts` and all baseline bot seed derivation
  uses the hash function defined there (no simple addition).
- INV-011: Baseline bot settings are persisted in localStorage under a dedicated
  key and in SQLite snapshot columns, and are restored on startup/reset.

## Alternatives considered

- Separate world for baseline bots: avoids population contamination but
  duplicates physics and rendering work; rejected for perf and complexity.
- Server-only baseline bots: easier to implement but breaks worker fallback
  parity; rejected to preserve worker/server symmetry.
- Use color strings only (no skin flag): simpler but cannot drive distinct eye
  rendering in fast path; rejected due to renderer contract.

## Dependencies and sequencing

- Stage 01 (`docs/todo/01-baseline-bot-settings-ui.md`) provides config and UI
  controls, expands settings update types, and aligns import/export handling
  for new settings. No runtime behavior changes.
- Stage 02 (`docs/todo/02-baseline-bot-runtime.md`) adds baseline bot controller
  logic, spawn lifecycle, RNG separation, and stats exclusion. Depends on Stage
  01 settings keys and types.
- Stage 03 (`docs/todo/03-bot-rendering-buffer.md`) adds skin flag semantics,
  renderer changes, and God Mode parsing alignment. Depends on Stage 02 bot
  identity flags and Stage 01 settings.

Merge prerequisites by stage:

- Stage 01: no prerequisites.
- Stage 02: requires Stage 01 (`docs/todo/01-baseline-bot-settings-ui.md`)
  merged for settings paths and import behavior.
- Stage 03: requires Stage 02 (`docs/todo/02-baseline-bot-runtime.md`) and
  Stage 01 (`docs/todo/01-baseline-bot-settings-ui.md`) merged for bot identity
  and settings gating.

## Data model changes overview

- CFG additions: new baseline bot settings (count, seed, per-generation random
  seed toggle). Defaults keep behavior unchanged.
- SettingsUpdate path union: add new paths for baseline bot controls and seed.
- Export/import: baseline bot settings are carried in export payload fields
  (`settings` and `updates`). Imports apply these fields when present in both
  worker and server modes; missing fields default to CFG_DEFAULT. Unknown
  settings paths cause the import to fail with a clear error.
- localStorage: add `slither_neuroevo_baseline_bot_settings` to persist bot
  settings across reloads (separate from `slither_neuroevo_pop`).
- SQLite: add snapshot columns to persist settings/updates with population
  snapshots (see Stage 01 for schema details).
- Stats payloads: add explicit population-only and total counts to avoid
  confusion with frame header counts. Required fields:
  - `alive` = population-only alive count (existing UI label uses this).
  - `aliveTotal` = total alive count (population + baseline bots).
  - `baselineBotsAlive`/`baselineBotsTotal` = bot-only counts.
  - Expand/migrate/contract: additive only; keep `alive` semantics unchanged
    and require totals in worker + server emitters.
- Frame buffer: skin flag domain expanded (0 default, 1 gold, 2 robot). Layout
  unchanged. Expand/migrate/contract plan:
  - Expand: update renderer and serializer to accept 2.
  - Migrate: start writing 2 for baseline bots after the renderer supports it.
  - Contract: not required; keep support for 0/1/2.
- localStorage persistence (baseline bot settings):
  - Expand: write `slither_neuroevo_baseline_bot_settings` on change.
  - Migrate: read and apply on startup/reset; validate schema.
  - Contract: remove any legacy in-memory-only path; treat invalid stored
    values as errors and require user reset.
- SQLite persistence (baseline bot settings):
  - Expand: add snapshot columns and dual-write settings to both columns and
    payload JSON.
  - Migrate: read from columns first, fall back to payload JSON.
  - Contract: stop reading settings from payload JSON (columns required).

## State machine designs

### Connection and fallback flow (unchanged, documented for invariants)

State table

| State | Description | Invariants |
| --- | --- | --- |
| Connecting | Attempting WS connection, worker may be idle | Join overlay visible if no worker |
| Server | WS connected, server streaming frames | Worker stopped; join overlay visible |
| Worker | Worker active, local frames | Join overlay hidden; no WS control |
| WorkerFallbackPending | WS not connected, fallback timer running | No duplicate worker start |
| ReconnectPending | Backoff timer scheduled | Single reconnect timer active |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| connect(url) | any | Connecting | wsClient available | schedule fallback | single fallback timer |
| wsConnected | Connecting | Server | none | stop worker, reset stats | join overlay visible |
| wsDisconnected | Server | ReconnectPending | none | schedule fallback + reconnect | no double worker |
| reconnectTimer | ReconnectPending | Connecting | wsClient available | connect + backoff | single timer |
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
| SeedSet | Base seed selected | per-bot seed uses baselineBotIndex |
| BotsSpawned | Baseline bots appended to World.snakes | population order preserved |
| Running | Bots controlled each tick | NN inference not executed for bots |
| RespawnPending | Bot died, awaiting respawn | baselineBotIndex preserved |
| GenEndPending | EndGeneration triggered | bots excluded from fitness |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| generationStart | SeedPending | SeedSet | randomize toggle on/off | choose base seed | no shared RNG |
| spawnBots | SeedSet | BotsSpawned | count > 0 | append bots | population order |
| tick | BotsSpawned | Running | none | compute bot actions | external control only |
| botDied | Running | RespawnPending | death detected | mark respawn timer | identity preserved |
| respawnReady | RespawnPending | Running | timer elapsed | respawn bot | deterministic seed |
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
- Protocol errors: unknown settings paths are treated as fatal for imports
  (no partial apply); other protocol errors are rejected with a clear error.
- Import errors: missing settings fields fall back to CFG_DEFAULT; invalid
  settings paths fail the import.
- Fatal errors: buffer layout mismatch detected in tests only (CI gate).

## Performance considerations

- Bot controller uses preallocated buffers and avoids per-tick allocations.
- Bot action computation happens once per tick per bot, reused for substeps.
- No additional fields in serialized buffer to avoid extra memory copies.

## Security and privacy considerations

- No new secrets; seed values are non-sensitive numeric data.
- Export/import files include bot settings; do not log file contents.
- Avoid logging raw settings payloads at info level; use debug gating.
- Use a local PRNG helper in `src/rng.ts`; no new dependencies to avoid
  supply-chain risk.

## Observability

- Error/warn logs are always emitted; verbose logs are debug-gated.
- Proposed log events (debug-gated unless noted):
  - bot.seed.selected { baseSeed, generation, randomize }
  - bot.spawned { count, idRangeStart, idRangeEnd }
  - bot.stats.filtered { excludedCount }
  - bot.controller.error { snakeId, state, error } (always on, warn/error)
- Debug toggle: CFG.debug.baselineBots (default false).

## Debug playbook

- Worker mode: set baseline bot count > 0, enable debug toggle, and confirm
  `bot.spawned`/`bot.seed.selected` logs; verify `stats.alive` matches
  population count while frame header `aliveCount` includes bots.
- Server mode: start server + UI, enable bots, and confirm controller assigns
  only player snakes (no bot ids) while bot logs appear server-side.
- Determinism check: export a snapshot, reload, re-import, and verify bot
  motion matches the same generation seed with randomize disabled.

## Rollout plan

- Merge-safe gating: baseline bots are enabled only when count > 0 (default 0).
- Skin flag behavior: `skin === 2` is not gated by bot count; it is emitted
  whenever a snake is marked as robot/baseline, regardless of other settings.
- Rollback: revert bot modules and config keys; no mixed-version compatibility
  is required for this repo. Buffer skin flag value 2 falls back to default
  color in the current renderer (checks `skin === 1`); if reverting to any
  renderer that treats non-zero as gold, robots could appear gold. Mitigation:
  keep `skin === 2` usage limited to baseline bots and HoF retains `skin === 1`.

## Acceptance criteria

- AC-001: Baseline bots spawn in worker and server with count added to NPCs;
  population count remains unchanged.
- AC-002: Baseline bots never execute NN inference and never enter fitness,
  elite, or HoF selection.
- AC-003: Bot RNG is independent from sim RNG; per-bot seed derives from base
  seed + baselineBotIndex; generation is included only when the randomize
  toggle is enabled.
- AC-004: Stats/fitness history exclude baseline bots in worker and server and
  explicitly expose population-only vs total counts.
- AC-005: Baseline bots are never assigned to player controllers; join overlay
  behavior remains unchanged.
- AC-006: Skin flag renders metallic robot bots with robot eyes; God Mode
  parsing remains correct.
- AC-007: Import/export retains new bot settings when present; missing fields
  default to CFG_DEFAULT and unknown settings paths fail the import with an
  explicit error.
- AC-008: Baseline bot settings persist across reloads (localStorage) and
  server restarts (SQLite snapshots) with schema validation.
