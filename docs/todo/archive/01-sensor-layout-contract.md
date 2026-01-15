# Stage 01: Sensor Layout Contract

## Revision notes

- 2025-01-13: Created stage doc from the monolithic draft, focusing on a shared layout contract without changing sensor outputs.
- 2025-01-13: Removed backward-compatibility framing while keeping the staged rollout sequence.

## Scope

- Introduce a shared sensor layout helper that describes the current layout and label order.
- Wire the helper into sensorSpec generation so server/client metadata is consistent.
- Establish layout selection metadata without changing sensor outputs.

## Non-goals

- No changes to sensor values, bin mapping, or channel math.
- No UI slider additions or config shape changes.
- No baseline bot or test expectation changes beyond using the helper for labels and offsets.

## Assumptions

- `src/sensors.ts` continues to compute the current layout values until Stage 04.
- Server sensorSpec is the only protocol-facing sensor metadata (AGENTS.md "Runtime architecture and data flow").

## Compatibility intent

- No migration is provided; old saves/exports are treated as invalid once v2 is active.

## Constraints and invariants

- INV-SEN-001, INV-SEN-002, INV-SEN-006 from `docs/todo/sensors-v2-plan.md` apply, grounded in AGENTS.md "Simulation core: World, Snake, sensors, and physics" and "Runtime architecture and data flow."
- The layout helper is the only allowed source for counts/offsets once introduced.

## Delta architecture overview

- New shared helper module provides layout constants and sensorSpec label order.
- `server/index.ts` uses helper to build `sensorSpec`.
- No changes to `buildSensors` logic in this stage.

## Symmetry checklist

- Worker: no direct changes (sensors still computed via `src/sensors.ts`).
- Main: no changes; welcome `sensorSpec` should include optional `layoutVersion`.
- Server: `server/index.ts` must use the helper for `sensorSpec`.
- UI: no changes.
- Desync failure mode: if any consumer bypasses the helper, `server/integration.test.ts` can detect `sensorCount` drift and bots/tests will read incorrect offsets.

## Touch-point checklist (hard contracts)

- `src/protocol/sensors.ts` provides `getSensorLayout` and `getSensorSpec`.
- `server/index.ts` uses `getSensorSpec` for welcome payload.
- `src/bots/baselineBots.ts` and tests read offsets from the helper, not literals.
- `src/sensors.test.ts` validates layout counts and offsets for the current layout.

```ts
const layout = getSensorLayout(CFG.sense.bubbleBins, 'legacy');
const spec = getSensorSpec(layout);
// spec.sensorCount and spec.order must match buildSensors output.
```

```ts
// server/index.ts
const layout = getSensorLayout(CFG.sense.bubbleBins, 'legacy');
const sensorSpec = getSensorSpec(layout);
const welcome = { type: 'welcome', sensorSpec, /* ... */ };
```

## Consumer list and desync impact

- Must use the helper:
  - `src/sensors.ts`
  - `src/bots/baselineBots.ts`
  - `src/sensors.test.ts`
  - `src/bots/baselineBots.test.ts`
  - `server/index.ts`
  - `server/integration.test.ts`
- If any consumer is out of date:
  - Baseline bots misinterpret bins, causing erratic avoidance behavior.
  - Server sensorSpec count/order mismatches the sensor arrays, triggering integration test failures.

## Alternatives considered

- Keep labels hard-coded in `server/index.ts`. Tradeoff: continues drift risk for bots/tests and sensorSpec.
- Introduce layout helper only in sensors, not server. Tradeoff: sensorSpec could still desync.

## Merge prerequisites

- None.

## Stage outputs

- New layout helper module exists and is imported by `server/index.ts`.
- sensorSpec order and count derived from the helper.
- Tests validate layout counts for the current layout.

## Planned modules and functions

- New module: `src/protocol/sensors.ts`
- Types:
  - `type SensorLayout = { layoutVersion: 'legacy' | 'v2'; bins: number; scalarCount: number; channelCount: number; inputSize: number; offsets: { food: number; hazard: number; wall: number; head: number | null }; order: string[] }`
- Functions:
  - `export function getSensorLayout(bins: number, layoutVersion: 'legacy' | 'v2' = 'legacy'): SensorLayout`
  - `export function getSensorSpec(layout: SensorLayout): { sensorCount: number; order: string[]; layoutVersion?: string }`
- Notes:
  - For the current layout, `inputSize = 5 + 3 * bins`, offsets match existing `src/sensors.ts` and `src/bots/baselineBots.ts`.
  - `layoutVersion` is optional in sensorSpec for debugging and layout selection.

## Data model and data flow

- `server/index.ts` should call `getSensorLayout` with `CFG.sense.bubbleBins` and pass `getSensorSpec(layout)` into the welcome payload.
- `src/net/wsClient.ts` and `server/protocol.ts` types should accept `layoutVersion?: string` on sensorSpec (optional field for debugging and selection).

## Error handling strategy

- If bins is non-finite or < 1, clamp to the minimum (8) and log `sensors.layout.invalid_bins`.
- If layoutVersion is unknown, fallback to the default layoutVersion and log `sensors.layout.invalid_version`.

## Expected failure modes

- `sensorSpec.sensorCount` mismatches actual sensor length because a consumer bypassed the helper (caught in `server/integration.test.ts`).
- Baseline bots read wrong offsets if they keep using `5 + 3 * bins` literals (caught in `src/bots/baselineBots.test.ts`).
- UI shows stale sensorSpec order if `server/index.ts` is not updated (manual verification in welcome payload logs).

## Performance considerations

- Helper must be pure and allocation-light (single `order` array per call).
- Do not build layout per-snake; use once per reset/init.

## Security and privacy

- No new data collection; keep sensorSpec metadata minimal.
- Avoid logging raw sensor values in this stage.

## Observability and debugability

- Add optional debug log when generating sensorSpec:
  - `sensors.layout.spec_built` with fields `{ layoutVersion, bins, inputSize }`.

## Rollout

- Rollout: helper is additive and does not change behavior.

## Tests and validation

- Modify existing tests:
  - `server/integration.test.ts` to allow (but not require) `layoutVersion` on sensorSpec.
- Add new tests:
  - `src/sensors.test.ts` new case for `getSensorLayout` current-layout counts.
- Commands:
  - `npm test`
  - `npm run test:unit` (covers `src/*.test.ts`)

## Acceptance criteria mapping

- AC-SEN-001: `src/sensors.test.ts`, `server/integration.test.ts`.
