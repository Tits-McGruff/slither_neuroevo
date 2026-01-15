# Sensor v2 Plan

## Revision notes

- 2025-01-13: Split the draft into merge-safe stages, added stable IDs, and aligned constraints and touch points with AGENTS.md and current repo files.
- 2025-01-13: Removed backward-compatibility framing and focused on the agreed "no migration" stance.

## Summary

Deliver the agreed v2 sensor layout with collision-accurate clearance, centered binning, and a shared layout contract used by sensors, bots, tests, and server sensorSpec. Roll out in staged increments that preserve CI stability, with an explicit compatibility break and user-visible reset guidance for input-size changes.

## Stage files

- [`01-sensor-layout-contract.md`](01-sensor-layout-contract.md) - introduce a shared layout contract and sensorSpec metadata wiring without changing outputs.
- [`02-sensor-v2-implementation.md`](02-sensor-v2-implementation.md) - add the v2 computation path (centered binning, rNear/rFar, new channels) behind the layout guard.
- [`03-sensor-config-ui.md`](03-sensor-config-ui.md) - expose all sensor config values in the UI and settings allow-list, enforce reset-only behavior for shape changes.
- [`04-sensor-integration-and-flip.md`](04-sensor-integration-and-flip.md) - switch default layout to v2, update bots/server/tests, add regression tests, and document the compatibility break.

## Scope

- In: sensor layout v2, collision-aligned clearance math, shared layout contract, settings/UI exposure for sensor config, bots/server/test alignment, and deterministic validation tests.
- Out: preserving old population saves/exports, changing rendering/serialization buffers, or altering reward/fitness logic beyond sensor inputs.

## Non-goals

- No new training pipeline or brain architecture changes beyond input sizing.
- No new network protocol beyond sensorSpec metadata required for validation.
- No changes to the binary frame format or render pipeline (AGENTS.md "Binary frame format and rendering pipeline").

## Assumptions

- `src/sensors.ts` is the single source of sensor computation for both server and worker (AGENTS.md "Runtime architecture and data flow").
- Baseline bots continue to consume food/hazard/wall bins only (`src/bots/baselineBots.ts`).
- `CFG.brain.inSize` remains the authoritative input size for brain construction and graph spec validation (AGENTS.md "Neural controllers and evolution").

## Compatibility intent

- No migration is provided. Old saves/exports are invalid once the v2 input size is active.
- If input-size mismatch is detected during import/reset, clear local storage or delete `data/slither.db` and retry.

## Constraints and invariants

- INV-SEN-001: Sensor vector length must equal `CFG.brain.inSize` (AGENTS.md "Simulation core: World, Snake, sensors, and physics").
- INV-SEN-002: Worker/main/server symmetry is mandatory; sensorSpec and sensor outputs must align across worker and server modes (AGENTS.md "Runtime architecture and data flow").
- INV-SEN-003: Hot-path sensors must reuse typed buffers and avoid per-tick allocations (AGENTS.md "Project-specific conventions and gotchas").
- INV-SEN-004: Hazard clearance must use the same collision geometry as `World._resolveCollisionsGrid` (`src/world.ts`) including `hitScale`.
- INV-SEN-005: Sensor outputs must stay within `[-1, 1]` and avoid NaN/Infinity values.
- INV-SEN-006: Settings updates must be registered in `src/protocol/settings.ts` or server validation will reject them (`server/protocol.ts` uses `SETTINGS_PATHS`).

## Touch-point checklist (hard contracts)

- Layout contract: all consumers use the shared layout helper (Stage 01).
- Sensor math: clearance and bin mapping match collision geometry and centered bins (Stage 02).
- Settings: all `sense.*` paths are in `SETTING_SPECS` and `SETTINGS_PATHS` (Stage 03).
- Protocol: server `sensorSpec` order/count matches layout helper (Stage 01/04).
- Tests: each contract has a failing test when desynced (Stage 02/04).

```ts
export type SensorLayout = {
  layoutVersion: 'legacy' | 'v2';
  bins: number;
  scalarCount: number;
  channelCount: number;
  inputSize: number;
  offsets: {
    food: number;
    hazard: number;
    wall: number;
    head: number | null;
  };
  order: string[];
};
```

## Decisions

- DEC-SEN-001: Drop `forwardClearDelta`; no temporal delta features in the v2 layout.
- DEC-SEN-002: Hazard clearance includes enemy head segments and uses `(snake.radius + other.radius) * CFG.collision.hitScale`.
- DEC-SEN-003: Head-pressure is head-only; body segments never contribute.
- DEC-SEN-004: A shared layout helper is the single source of truth for counts, offsets, and sensorSpec labels.
- DEC-SEN-005: Bin count is UI-adjustable but reset-only, because it changes input size.
- DEC-SEN-006: Compatibility is intentionally broken for old saves/exports; mismatches must be rejected or cleared.

## Key decisions and invariants registry

- Decisions: DEC-SEN-001 through DEC-SEN-006 define the fixed design choices for this plan.
- Invariants: INV-SEN-001 through INV-SEN-006 define non-negotiable contracts for sizing, symmetry, and hot-path behavior.
- No superseded entries; if a decision changes later, add a new DEC-SEN-### and mark the older one as superseded.

## Delta architecture overview

- Add a shared sensor layout contract (new helper module) consumed by:
  - `src/sensors.ts`
  - `src/bots/baselineBots.ts`
  - `src/sensors.test.ts` and `src/bots/baselineBots.test.ts`
  - `server/index.ts` sensorSpec generation
- Replace legacy sensor math with v2 layout and channels while keeping the sensor API shape (`buildSensors` signature) stable.
- Expand `CFG.sense` with explicit rNear/rFar and food saturation parameters, and derive input size from the layout helper.

## Planned modules and functions (summary)

- `src/protocol/sensors.ts`
  - `getSensorLayout(bins: number, layoutVersion: 'legacy' | 'v2'): SensorLayout`
  - `getSensorSpec(layout: SensorLayout): { sensorCount: number; order: string[]; layoutVersion?: string }`
- `src/sensors.ts`
  - `buildSensors(world: WorldLike, snake: SnakeLike, out?: Float32Array | null): Float32Array`
  - Index conventions:
    - Scalars first, then channel bins in layout order.
    - All outputs normalized to `[-1, 1]`.
    - Distances use world units (`CFG.worldRadius` scale), normalized by rNear/rFar per channel.
  - Clamping rules:
    - Ratios clamped to `[0, 1]` before mapping to `[-1, 1]`.
    - Non-finite inputs fall back to safe defaults (see Error handling strategy).

## Data model and data flow

- Sensor layout contract defines:
  - scalar count, channel count, offsets, and `inputSize`.
  - label order for server `sensorSpec` (`server/index.ts`).
- `CFG.brain.inSize` must be derived from the layout helper and current `CFG.sense.bubbleBins`.
- UI sliders update `CFG.sense` via `src/settings.ts` and `src/protocol/settings.ts`, with reset-only semantics enforced by `requiresReset` and the existing reset flow.
- In worker mode, reset occurs via `init` in `src/worker.ts`; in server mode via `reset` messages (`server/protocol.ts`).
  - Reset order: apply pending `CFG.sense` updates -> recompute layout -> set `CFG.brain.inSize` -> rebuild world/population.
  - Reset clears in-memory population; localStorage and DB data are cleared only if a mismatch is detected (Stage 04).

## Data model changes overview

- CFG additions: `sense.layoutVersion`, `sense.bubbleBins`, rNear/rFar params, `sense.foodKBase`, `sense.maxPelletChecks`, `sense.maxSegmentChecks`.
- SettingsUpdate allow-list: add `sense.*` paths to `SETTINGS_PATHS` to permit server resets.
- sensorSpec shape: `{ sensorCount, order, layoutVersion? }` derived from layout helper.
- Input sizing: `CFG.brain.inSize` derived from layout helper and bins; graph spec validation must use the derived size.
- Persistence: no migration; input-size mismatches invalidate existing population data.

## Alternatives considered

- Keep single-radius sensing (legacy bubble). Tradeoff: cannot decouple near-term avoidance from far-term routing.
- Drop head-pressure channel. Tradeoff: loses explicit head-to-head information.

## Error handling strategy

- Clamp invalid or non-finite config values (bins, radii, KFood) to safe defaults.
- If derived input size mismatches `CFG.brain.inSize`, log a layout mismatch and block reset/import until corrected.
- If settings updates include unknown paths or non-finite values, ignore and surface a UI hint (existing patterns in `src/settings.ts`).

## Performance considerations

- Reuse scratch `Float32Array` buffers for bins and avoid per-tick allocations (aligns with `src/sensors.ts` current approach).
- Respect `maxPelletChecks` and `maxSegmentChecks` caps to avoid pathological loops.
- Avoid redundant `sqrt` calls by only applying when updating per-bin minima.

## Security and privacy

- No external data collection or network calls beyond existing server WS.
- Avoid logging per-tick sensor arrays; use aggregate stats only in debug mode.
- Keep local persistence handling consistent with current `localStorage`/SQLite usage.

## Observability and debugability

- Add a debug toggle (config and UI) to emit periodic per-channel stats:
  - `sensors.debug.stats`: `{ bins, min, max, mean }`
  - `sensors.debug.layout`: `{ layoutVersion, inputSize, bins }`
- Local debug playbook:
  - `npm run server` + `npm run dev`, enable the debug toggle, verify logs and UI slider behavior.
  - Use the Visualizer tab for sanity checks on head pressure and clearance patterns.

## Dependencies and sequencing

- Stage 01: no prerequisites.
- Stage 02: requires Stage 01 (layout helper and sensorSpec wiring).
- Stage 03: requires Stage 01 (settings allow-list and UI wiring).
- Stage 04: requires Stages 01-03 (layout helper, v2 path, UI exposure).

## Rollout

- Rollout via staged plan: layout helper first, then v2 implementation, then UI exposure, then default switch.

## Acceptance criteria and test mapping

- AC-SEN-001: Sensor length/order matches layout helper, bots, and server sensorSpec.
  - Tests: `src/sensors.test.ts` (`describe('layout helper')`), `src/bots/baselineBots.test.ts` (`describe('BaselineBotManager AI')`), `server/integration.test.ts` (`assigns a player and streams sensors`).
  - Commands: `npm test`, `npm run test:integration`, `npm run test:unit`.
- AC-SEN-002: Centered bin mapping is stable at `-pi`, `0`, `pi` boundaries.
  - Tests: add to `src/sensors.test.ts` (`it('centered bin mapping')`).
  - Commands: `npm test`, `npm run test:unit`.
- AC-SEN-003: Lethal clearance matches collision geometry and includes enemy head segments.
  - Tests: add to `src/sensors.test.ts` (`it('lethal clearance matches hitScale')`).
  - Commands: `npm test`, `npm run test:unit`.
- AC-SEN-004: Wall clearance subtracts `snake.radius` and normalizes by `rNear`.
  - Tests: add to `src/sensors.test.ts` (`it('wall clearance uses rNear')`).
  - Commands: `npm test`, `npm run test:unit`.
- AC-SEN-005: Food potential is deterministic with fixed pellets and capped checks.
  - Tests: add to `src/sensors.test.ts` (`it('food potential deterministic')`).
  - Commands: `npm test`, `npm run test:unit`.
- AC-SEN-006: Head pressure is head-only and uses min distance per bin.
  - Tests: add to `src/sensors.test.ts` (`it('head pressure ignores bodies')`).
  - Commands: `npm test`, `npm run test:unit`.
- AC-SEN-007: Reset-only settings do not apply live; reset rebuilds input size and world.
  - Tests: `src/settings.test.ts` (`hookSliderEvents triggers live updates`), `src/worker.test.ts` (`applies updateSettings messages to CFG`).
  - Commands: `npm test`, `npm run test:unit`.
- AC-SEN-008: Outputs remain in `[-1, 1]` with no NaNs in empty scenes.
  - Tests: `src/sensors.test.ts` (`reports clear hazard and wall bins`), `src/world.test.ts` (finite sensors first tick).
  - Commands: `npm test`, `npm run test:unit`.
