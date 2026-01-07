# Stage 02: Baseline Bot Runtime and Stats Exclusion

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

- Relevant decisions: DEC-001, DEC-002, DEC-003.
- Relevant invariants: INV-002, INV-003, INV-004, INV-005.
- Scope: runtime bot control, RNG separation, spawn lifecycle, stats exclusion.
- Non-goals: rendering changes, buffer format changes.

## C) Architecture overview (stage-local)

- Introduce a baseline bot controller module (`src/bots/baselineBots.ts`) that
  owns per-bot state machines, per-bot RNG, and action buffers.
- Extend `World` to:
  - Track baseline bot snakes separately (e.g., `baselineBots: Snake[]`).
  - Spawn baseline bots after `_spawnAll()` to keep population order.
  - Compute baseline bot actions once per tick and feed them to
    `Snake.update(world, dt, control)` to avoid NN inference.
  - Exclude baseline bots from `bestPointsThisGen`, fitness computation,
    fitnessHistory aggregation, and HoF selection.
- Update `ControllerRegistry` to exclude baseline bots from assignment by
  adding a `controllable` flag in the `getSnakes` dependency.
- Ensure worker and server stats use population-only counts for fitness and
  chart history; alive counts in UI should also reflect population-only unless
  explicitly desired.

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
  attachBot(snakeId: number): void;
  detachBot(snakeId: number): void;
  update(world: World, dt: number): void;
  getAction(snakeId: number): BotAction | null;
}
```

Usage example (illustrative):

```ts
this.botManager.resetForGeneration(this.generation);
this._spawnBaselineBots();
this.botManager.update(this, stepDt);
const action = this.botManager.getAction(sn.id);
if (action) sn.update(this, dt, action);
```

### Planned changes in `src/world.ts`

- New fields:
  - `baselineBots: Snake[]`
  - `botManager: BaselineBotManager`
- New methods:
  - `_spawnBaselineBots(): void` (append bots after population)
  - `_resetBaselineBotsForGen(): void` (seed selection and manager reset)
- Update `_endGeneration()` to compute fitness using population-only snakes.
- Update `update()` to compute `bestPointsThisGen` from population-only snakes.

### Planned changes in `src/snake.ts`

- Add a control mode or brain policy flag to avoid NN inference for baseline
  bots. Example:

```ts
type ControlMode = 'neural' | 'external-only';
```

- If using a NullBrain, define a minimal Brain that never allocates and never
  runs forward.

### Planned changes in `server/controllerRegistry.ts`

- Extend `ControllerRegistryDeps.getSnakes` to return `{ id, alive, controllable }`.
- Update `pickAvailableSnake` and `isSnakeAssignable` to require
  `controllable === true`.

## F) Data model changes; data flow; migration strategy; backward compatibility

- New runtime-only fields: `Snake.controlMode` (or `Snake.role`).
- No persistence schema changes; baseline bots are not exported.
- Stats payload: if `alive` becomes population-only, update the type and tests
  to document this change; keep backward compatibility by defaulting to total
  when field is missing in older payloads.
- No DB or localStorage changes.

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
| baseSeedStatic | Base seed fixed | per-bot seed derived from base |
| baseSeedRandomized | Base seed rolled per gen | seed uses bot RNG only |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| genStart | any | baseSeedStatic | randomize off | keep seed | deterministic |
| genStart | any | baseSeedRandomized | randomize on | roll seed | independent RNG |

## H) Touch points checklist

- Worker <-> main messages: unchanged message types, but settings values are
  consumed in worker; verify both ends still use `setByPath` with new keys.
- Server controller assignment: update `server/controllerRegistry.ts` and
  `server/simServer.ts` getSnakes shape together.
- No buffer or sensor changes in this stage.

## I) Error handling

- Bot controller exceptions: catch and disable bot updates for the current
  generation; log `bot.controller.error` with snakeId.
- Invalid seed/count values: clamp to defaults in `BaselineBotManager`.

## J) Performance considerations

- Per-bot state stored in fixed arrays or Maps sized to bot count.
- Compute bot actions once per tick and reuse across substeps.
- Avoid allocations in `World.update` and `BaselineBotManager.update`.

## K) Security and privacy considerations

- No new data at rest; avoid logging seeds at info level.

## L) Observability

- Debug logs (gated):
  - `bot.spawned` { count, generation }
  - `bot.seed.selected` { baseSeed, generation, randomized }
  - `bot.stats.filtered` { excludedCount }

## M) Rollout and rollback plan (merge-safe gating)

- Gating: baseline bots active only when `CFG.baselineBots.count > 0`.
- Rollback: remove baseline bot manager and control flags; population order
  remains intact and stats revert to pre-bot behavior.

## N) Testing plan

- `src/world.test.ts`
  - Add test: baseline bots append after population and do not affect
    `population.length`.
  - Add test: `bestPointsThisGen` computed from population-only snakes.
  - Add test: `_endGeneration` ignores baseline bots for fitness and HoF.
- `src/snake.test.ts`
  - Add test: external control path does not call brain.forward.
- `server/controllerRegistry.test.ts`
  - Add test: `pickAvailableSnake` ignores non-controllable baseline bots.
- `src/worker.test.ts`
  - Add test: worker stats exclude baseline bots from `alive` and
    `fitnessHistory` updates.
- Validation commands:
  - `npm run test:unit`
  - `npm run test:integration`
  - `npm test`
- AC mapping:
  - AC-001 -> `src/world.test.ts` population/bot count tests.
  - AC-002 -> `src/snake.test.ts` external control test + HoF exclusion test.
  - AC-003 -> `src/world.test.ts` seed derivation test.
  - AC-004 -> `src/worker.test.ts` stats exclusion test.
  - AC-005 -> `server/controllerRegistry.test.ts` controllable filter test.

## O) Compatibility matrix

- Server mode: changed, ok (controller registry excludes baseline bots;
  verified by server tests).
- Worker fallback: changed, ok (bot controller runs in worker; verified by
  worker tests).
- Join overlay: unchanged, ok.
- Visualizer streaming: unchanged, ok.
- Import/export: unchanged, ok (baseline bots not persisted).

## P) Risk register

- Risk: population order assumptions broken if bots are inserted mid-array.
  Mitigation: enforce append-only in `_spawnBaselineBots` and test ordering.
- Risk: stats drift if bots included; mitigate with explicit population-only
  aggregation and unit tests.
