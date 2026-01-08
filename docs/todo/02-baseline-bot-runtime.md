# Stage 02: Baseline Bot Runtime and Stats Exclusion

## Revision notes

- Introduced baselineBotIndex identity, deterministic seed derivation rules,
  and explicit respawn semantics.
- Clarified stats vs frame header semantics and required population-only vs
  total counts in stats payloads (no backward-compatibility requirements).
- Expanded touch-point checklists and test mappings for determinism and
  respawn behavior.
- Aligned data model notes with Stage 01 persistence decisions (settings are
  persisted; runtime bot state remains ephemeral).

## A) Delta vs AGENTS.md

- Changes: add baseline bot controller module, spawn baseline bots after the
  population, exclude bots from fitness/elite/HoF and stats aggregation, and
  prevent controller assignment to baseline bots.
- Unchanged: worker/server handshake, buffer layout, sensor sizing, graph
  editor, and import/export payload shapes (except settings values).
- Contract touch points: World update loop (AGENTS.md “Simulation core”),
  controller registry assignment (AGENTS.md “Runtime architecture and data
  flow”), and stats collection paths in worker/server.

## B) Scope; non-goals; assumptions; constraints and invariants

- Relevant decisions: DEC-001, DEC-002, DEC-003 (superseded), DEC-005, DEC-006,
  DEC-007.
- Relevant invariants: INV-002, INV-003, INV-004, INV-005, INV-006, INV-007,
  INV-008, INV-010.
- Scope: runtime bot control, RNG separation, spawn lifecycle, stats exclusion.
- Non-goals: rendering changes, buffer format changes.
- Assumptions: baselineBotIndex is stable and independent of snakeId; respawn
  delay is fixed (constant) to avoid extra UI controls; bot spawn uses the
  dedicated `src/rng.ts` PRNG to avoid consuming global randomness.

## C) Architecture overview (stage-local)

- Introduce a baseline bot controller module (`src/bots/baselineBots.ts`) that
  owns per-bot state machines, baselineBotIndex identity, per-bot RNG streams,
  and action buffers.
- Extend `World` to:
  - Track baseline bot snakes separately (e.g., `baselineBots: Snake[]`).
  - Spawn baseline bots after `_spawnAll()` to keep population order.
  - Assign baseline bot ids from a reserved range distinct from population and
    external controller ids (new `BASELINE_BOT_ID_START` constant, set above
    the HoF/random id range and `EXTERNAL_SNAKE_ID_START`).
  - Derive per-bot seeds from `(baseSeed, baselineBotIndex)` and, when
    `randomizeSeedPerGen` is enabled, from `(baseSeed, generation)` first.
  - Seed formula: `genSeed = randomize ? hash(baseSeed, generation) : baseSeed;
    botSeed = hash(genSeed, baselineBotIndex)`; respawns reset to `botSeed`.
  - Compute baseline bot actions once per tick and feed them to
    `Snake.update(world, dt, control)` to avoid NN inference.
  - Respawn baseline bots deterministically after death using the same
    baselineBotIndex and a reset state machine.
  - Exclude baseline bots from `bestPointsThisGen`, fitness computation,
    fitnessHistory aggregation, and HoF selection.
- Update `ControllerRegistry` to exclude baseline bots from assignment by
  adding a `controllable` flag in the `getSnakes` dependency.
- Ensure worker and server stats use population-only counts for fitness and
  chart history while emitting required total counts to avoid confusion with
  frame header totals.
  - Rendering loops still use frame header `totalSnakes`/`aliveCount` (includes
    baseline bots); UI labels and charts use stats `alive` (population-only).
  - Always surface `aliveTotal` and `baselineBotsAlive` in the Stats panel to
    make totals explicit (no debug gating).

## D) Alternatives considered with tradeoffs

- Filter baseline bots via id ranges only in ControllerRegistry: rejected
  because it relies on hard-coded ranges and does not convey intent in type
  signatures.
- Build a separate World for bots: rejected due to duplicate physics and render
  work and increased memory pressure.

## E) Planned modules and functions

### Planned modules/files likely to change

- `src/world.ts`
- `src/snake.ts`
- `src/worker.ts`
- New: `src/rng.ts` (seed hashing helper + PRNG)
- New: `src/brains/nullBrain.ts` (NullBrain implementation)
- `server/simServer.ts`
- `server/controllerRegistry.ts`
- `src/protocol/messages.ts` (stats payload types if alive counts change)
- New: `src/bots/baselineBots.ts`
- Tests: `src/world.test.ts`, `src/snake.test.ts`, `src/worker.test.ts`,
  `server/controllerRegistry.test.ts`

### Planned new module: `src/bots/baselineBots.ts`

Proposed types and signatures (illustrative):

```ts
export type BotState = 'roam' | 'seek' | 'avoid' | 'boost';

export interface BaselineBotSettings {
  count: number;
  seed: number;
  randomizeSeedPerGen: boolean;
}

export interface BotAction {
  turn: number;
  boost: number;
}

export class BaselineBotManager {
  constructor(settings: BaselineBotSettings);
  resetForGeneration(gen: number): void;
  registerBot(index: number, snakeId: number): void;
  markDead(index: number): void;
  update(world: World, dt: number): void;
  getActionForSnake(snakeId: number): BotAction | null;
  getActionByIndex(index: number): BotAction | null;
}

/** Deterministic seed derivation for baseline bots. */
export function deriveBotSeed(
  baseSeed: number,
  generation: number,
  baselineBotIndex: number,
  randomizeSeedPerGen: boolean
): number;

I/O contract for `deriveBotSeed`:
- Inputs: finite numbers; `baselineBotIndex` must be in `[0, count)`.
- Output: unsigned 32-bit integer seed; stable for identical inputs.
- Errors: out-of-range indices return a clamped seed and emit debug log.

### Planned new module: `src/brains/nullBrain.ts`

```ts
import type { Brain } from './types.ts';

export class NullBrain implements Brain {
  reset(): void;
  forward(inputs: Float32Array): Float32Array;
}
```

I/O contract for `NullBrain`:
- `reset()` is a no-op.
- `forward()` returns a cached zeroed `Float32Array` of size `CFG.brain.outSize`
  and does not allocate per call.
- Should never be called for baseline bots in steady state; if called, it does
  not throw (keeps simulation stable).
```

Usage example (illustrative):

```ts
this.botManager.resetForGeneration(this.generation);
this._spawnBaselineBots(); // assigns baselineBotIndex + snakeId
this.botManager.update(this, stepDt);
const action = this.botManager.getActionForSnake(sn.id);
if (action) sn.update(this, dt, action);
```

Validation and error behavior:

- `BaselineBotManager` clamps `count` to `>= 0` and treats non-finite seeds as
  `0`.
- `deriveBotSeed` returns a 32-bit unsigned integer; invalid inputs are
  normalized to 0 before hashing.
- `registerBot` ignores duplicate baselineBotIndex values and logs a debug
  warning; `markDead` is idempotent for already-dead bots.

### Planned changes in `src/world.ts`

- New fields:
  - `baselineBots: Snake[]`
  - `botManager: BaselineBotManager`
- New methods:
  - `_spawnBaselineBots(): void` (append bots after population)
  - `_resetBaselineBotsForGen(): void` (seed selection and manager reset)
- Track `baselineBotIndex` on `Snake` to keep identity stable across respawns.
- Respawn policy: when a baseline bot dies, schedule a respawn after a fixed
  delay (e.g., 0.5s) and re-register the same baselineBotIndex with a fresh
  snake id from the baseline bot reserved range.
- Update `_endGeneration()` to compute fitness using population-only snakes.
- Update `update()` to compute `bestPointsThisGen` from population-only snakes.

### Planned changes in `src/snake.ts`

- Add a control mode flag and install a NullBrain for baseline bots to avoid
  NN inference and weight allocation.

```ts
type ControlMode = 'neural' | 'external-only';
```

- Define a minimal NullBrain implementation that never allocates and never
  runs forward (required for baseline bots).
- Require an RNG parameter for baseline bot spawn position/heading so baseline
  bots never consume global `Math.random`.

### Planned changes in `server/controllerRegistry.ts`

- Extend `ControllerRegistryDeps.getSnakes` to return `{ id, alive, controllable }`.
- Update `pickAvailableSnake` and `isSnakeAssignable` to require
  `controllable === true`.

## F) Data model changes; data flow; migration strategy; backward compatibility

- New runtime-only fields: `Snake.controlMode` and `Snake.baselineBotIndex`.
- No additional persistence schema changes in this stage; baseline bot runtime
  state (snakes/controllers) is not exported.
- Stats payload: keep `alive` as population-only and add required fields
  `aliveTotal`, `baselineBotsAlive`, and `baselineBotsTotal` to surface totals.
- Expand/migrate/contract for stats payload:
  - Expand: add required fields to worker + server stats emitters and protocol
    types while keeping `alive` semantics unchanged for existing UI.
  - Migrate: update `src/main.ts` to show totals unconditionally.
  - Contract: keep fields (no removal planned).
- Settings persistence is handled in Stage 01 (localStorage + SQLite); this
  stage must not introduce new DB/localStorage keys.

## G) State machine design

### Baseline bot controller

State table

| State | Description | Invariants |
| --- | --- | --- |
| roam | Wander and avoid walls | action computed from local heading |
| seek | Turn toward local food cluster | uses sensor-derived density |
| avoid | Avoid nearby hazards or walls | turn bias away from threat |
| boost | Short boost burst | boost only if points allow |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| noFoodSeen | seek | roam | low food density | clear target | deterministic per seed |
| hazardNear | any | avoid | hazard threshold | set avoid timer | safe turn |
| avoidTimeout | avoid | roam | timer elapsed | clear avoid | no stuck state |
| foodCluster | roam | seek | density threshold | set target | bounded turn |
| boostTrigger | roam/seek | boost | points ok + random | set boost timer | boost bounds |
| boostTimeout | boost | roam | timer elapsed | clear boost | no lingering boost |

### Generation seed selection

State table

| State | Description | Invariants |
| --- | --- | --- |
| baseSeedStatic | Base seed fixed | per-bot seed uses baselineBotIndex |
| baseSeedRandomized | Base seed rolled per gen | per-bot seed uses generation |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| genStart | any | baseSeedStatic | randomize off | genSeed = baseSeed | deterministic |
| genStart | any | baseSeedRandomized | randomize on | genSeed = hash(baseSeed, gen) | independent RNG |

### Baseline bot lifecycle and respawn

State table

| State | Description | Invariants |
| --- | --- | --- |
| alive | Bot snake active | baselineBotIndex stable |
| deadPending | Bot died, awaiting respawn | RNG resets to per-gen seed on respawn |
| respawning | New snake spawn | id assigned from bot id range |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| botDied | alive | deadPending | death detected | start respawn timer | identity preserved |
| respawnReady | deadPending | respawning | timer elapsed | spawn snake + register | deterministic spawn RNG |
| respawned | respawning | alive | spawn complete | reset state machine | stable seed |

## H) Touch points checklist

- Worker <-> main messages: unchanged message types, but settings values are
  consumed in worker; verify both ends still use `setByPath` with new keys.
- Server controller assignment: update `server/controllerRegistry.ts` and
  `server/simServer.ts` getSnakes shape together.
- Stats payload changes (population vs total counts): update together:
  - `src/protocol/messages.ts` (FrameStats)
  - `server/protocol.ts` (StatsMsg)
  - `server/simServer.ts` (emit required fields)
  - `src/worker.ts` (emit required fields)
  - `src/net/wsClient.ts` + `src/main.ts` (consume fields)
  - Tests: `src/worker.test.ts`, `server/protocol.test.ts`
- No buffer or sensor changes in this stage.

## I) Error handling

- Bot controller exceptions: catch and disable bot updates for the current
  generation; log `bot.controller.error` with snakeId.
- Invalid seed/count values: clamp to defaults in `BaselineBotManager`.
- Respawn failures (no available id range): log and keep bot in deadPending
  until next tick; do not crash the loop.

## J) Performance considerations

- Per-bot state stored in fixed arrays or Maps sized to bot count.
- Compute bot actions once per tick and reuse across substeps.
- Avoid allocations in `World.update` and `BaselineBotManager.update`.

## K) Security and privacy considerations

- No new data at rest; avoid logging seeds at info level.
- Avoid adding external RNG dependencies; implement PRNG locally to minimize
  dependency risk.

## L) Observability

- Error/warn logs are always emitted; verbose logs are debug-gated.
- Debug-gated logs:
  - `bot.spawned` { count, generation }
  - `bot.seed.selected` { baseSeed, generation, randomized }
  - `bot.stats.filtered` { excludedCount }
  - `bot.respawn` { baselineBotIndex, snakeId, delayMs }
- Always-on warn/error logs:
  - `bot.controller.error` { snakeId, state, error }
  - `bot.respawn.failed` { baselineBotIndex, reason }
- Debug toggle: CFG.debug.baselineBots.

## Debug playbook

- Spawn + respawn: set bot count to 1, kill the bot via God Mode, and confirm
  the respawn log and stable baselineBotIndex mapping.
- Stats check: compare frame header `aliveCount` against stats `alive` and
  `aliveTotal` to confirm population-only vs total semantics.
- Determinism check: run two worker sessions with the same seed and confirm
  bot motion matches for the same generation when randomize is disabled.

## M) Rollout and rollback plan (merge-safe gating)

- Gating: baseline bots active only when `CFG.baselineBots.count > 0`.
- Rollback: remove baseline bot manager and control flags; population order
  remains intact and stats revert to pre-bot behavior.
- Compatibility: if rolling back server/worker code, new UI must tolerate
  missing `aliveTotal`/`baselineBotsAlive` fields (defaults apply).

## N) Testing plan

- `src/world.test.ts`
  - Add test: baseline bots append after population and do not affect
    `population.length` (`it('appends baseline bots after population')`).
  - Add test: `bestPointsThisGen` computed from population-only snakes
    (`it('excludes baseline bots from bestPointsThisGen')`).
  - Add test: `_endGeneration` ignores baseline bots for fitness and HoF
    (`it('excludes baseline bots from fitness and hof')`).
  - Add test: baselineBotIndex stable across respawn and does not equal
    snakeId; respawn resets controller state
    (`it('baselineBotIndex stable across respawn')`).
  - Add test: `deriveBotSeed` uses `(baseSeed, baselineBotIndex)` and includes
    generation only when randomize is enabled
    (`it('deriveBotSeed includes generation only when enabled')`).
  - Add negative test: bot respawn does not stall (bot count returns to target
    within the fixed delay)
    (`it('respawns baseline bots within delay')`).
- `src/snake.test.ts`
  - Add test: external control path does not call brain.forward
    (`it('external control bypasses brain forward')`).
  - Add test: snake spawn uses provided RNG when supplied (baseline bots)
    (`it('uses provided rng for spawn')`).
- `src/brains/nullBrain.test.ts` (new file)
  - Add test: `NullBrain.forward` returns a stable zeroed buffer of size
    `CFG.brain.outSize` without allocating per call
    (`it('NullBrain returns stable zero buffer')`).
- `server/controllerRegistry.test.ts`
  - Add test: `pickAvailableSnake` ignores non-controllable baseline bots
    (`it('skips non-controllable snakes')`).
- `src/worker.test.ts`
  - Add test: worker stats exclude baseline bots from `alive` and
    `fitnessHistory` updates, and include required total fields
    (`it('stats exclude baseline bots and include totals')`).
  - Add test: frame header `aliveCount` includes baseline bots while
    `stats.alive` excludes them
    (`it('frame header includes bots while stats do not')`).
- `server/protocol.test.ts`
  - Add test: StatsMsg requires `aliveTotal`/`baselineBotsAlive`/`baselineBotsTotal`
    fields and rejects missing totals
    (`it('stats requires total fields')`).
- Validation commands:
  - `npm run test:unit`
  - `npm run test:integration`
  - `npm test`
  - CI still runs `npm run build` and `npm run typecheck`; stage changes must
    keep them green.
- AC mapping:
  - AC-001 -> `src/world.test.ts` / `baseline bots append after population`.
  - AC-002 -> `src/snake.test.ts` / `external control bypasses brain` and
    `src/world.test.ts` / `HoF ignores baseline bots` and
    `src/brains/nullBrain.test.ts` / `NullBrain returns stable zero buffer`.
  - AC-003 -> `src/world.test.ts` / `deriveBotSeed includes generation only
    when enabled`.
  - AC-004 -> `src/worker.test.ts` / `stats exclude baseline bots` and
    `server/protocol.test.ts` / `stats requires total fields`.
  - AC-005 -> `server/controllerRegistry.test.ts` / `skip baseline bots`.

## O) Compatibility matrix

- Server mode: changed, ok (controller registry excludes baseline bots;
  verified by server tests).
- Worker fallback: changed, ok (bot controller runs in worker; verified by
  worker tests).
- Join overlay: unchanged, ok.
- Visualizer streaming: unchanged, ok.
- Import/export: unchanged for runtime state; settings persistence is handled
  in Stage 01 (bots still excluded from population export).

## P) Risk register

- Risk: population order assumptions broken if bots are inserted mid-array.
  Mitigation: enforce append-only in `_spawnBaselineBots` and test ordering.
- Risk: stats drift if bots included; mitigate with explicit population-only
  aggregation and unit tests.
