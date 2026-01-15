# Stage 04: Integration, Default Flip, and Regression Coverage

## Revision notes

- 2025-01-13: Created stage doc to flip the default to v2 and align bots/server/tests.
- 2025-01-13: Removed backward-compatibility framing while keeping explicit reset guidance.

## Scope

- Switch default sensor layout to v2.
- Update baseline bots and server sensorSpec order to v2 layout.
- Update tests to reflect the new default layout and add regression coverage.
- Document and enforce compatibility break for old populations/exports.

## Non-goals

- No changes to rendering, serialization, or worker protocol beyond sensorSpec metadata.
- No changes to reward or evolution logic.

## Assumptions

- Stages 01-03 are merged and provide layout helper, v2 computation path, and UI exposure.
- `CFG.brain.inSize` is derived from the layout helper for v2.

## Compatibility intent

- No migration is provided; old saves/exports are invalid once v2 is the default.

## Staging intent

- This stage switches the default layout to v2 and updates all consumers.
- Old populations/exports are rejected or cleared when input size mismatches.

## Constraints and invariants

- INV-SEN-001 through INV-SEN-006 apply, anchored in AGENTS.md "Simulation core: World, Snake, sensors, and physics" and "Runtime architecture and data flow."
- `server/integration.test.ts` must continue to assert sensorCount matches sensorSpec.

## Delta architecture overview

- Default layoutVersion set to `v2`.
- Baseline bots use layout helper offsets aligned with v2.
- Server sensorSpec labels and count match v2 layout helper.

## Symmetry checklist

- Worker: sensors now emit v2 by default; ensure worker stats and sensor buffers align.
- Main: graph spec validation and UI now expect v2 input size.
- Server: `sensorSpec` order/count and sensor message lengths must match v2.
- UI: reset-only layout changes should display warnings and rebuild world state.
- Desync failure mode: server sensorSpec count mismatch causes `server/integration.test.ts` failure.

## Touch-point checklist (hard contracts)

- `src/config.ts` default `sense.layoutVersion` is `v2`.
- `src/sensors.ts` uses the v2 layout by default after this stage.
- `src/bots/baselineBots.ts` reads offsets from the layout helper, not literals.
- `server/index.ts` builds sensorSpec from the layout helper and reports the v2 order.
- `src/sensors.test.ts` and `server/integration.test.ts` fail on count or order drift.

```ts
const layout = getSensorLayout(CFG.sense.bubbleBins, 'v2');
const { food, hazard, wall } = layout.offsets;
// Baseline bots read bins using offsets instead of 5 + 3 * bins.
```

## Alternatives considered

- Keep a layoutVersion toggle for emergency debugging. Tradeoff: ongoing maintenance burden.
- Hard-remove the layout toggle immediately. Tradeoff: no quick revert in production.

## Merge prerequisites

- Requires Stages 01-03 (`docs/todo/01-sensor-layout-contract.md`, `docs/todo/02-sensor-v2-implementation.md`, `docs/todo/03-sensor-config-ui.md`).

## Stage outputs

- Default layout is v2 for sensors, bots, and server sensorSpec.
- Graph spec validation and input sizing reflect v2 by default.
- Tests cover default v2 layout and sensorSpec alignment.

## Planned modules and functions

- `src/config.ts`
  - Set `sense.layoutVersion` default to `v2`.
  - Set `sense.bubbleBins` default to 16.
  - Ensure `brain.inSize` default aligns with the v2 helper.
- `src/bots/baselineBots.ts`
  - Use layout helper offsets instead of `5 + 3 * bins`.
- `server/index.ts`
  - Build `sensorSpec` labels via layout helper (v2 order).
- `src/main.ts`
  - On import or reset, reject payloads where input size mismatches current layout.
  - Functions impacted:
    - `buildSensorSpec(): SensorSpec`
    - `BaselineBotManager.computeAction(...)` (reads offsets from layout helper)

## Data model and data flow

- `sensorSpec` order changes to match v2 layout.
- `CFG.brain.inSize` and layout helper are the source of truth for input size.
- Imports/exports do not attempt migration; mismatched input size yields a visible error.
- Graph spec validation in `src/main.ts` must reject mismatched input sizes after the default flip.
  - User-facing guidance: clear `localStorage` keys or delete `data/slither.db` if incompatibilities persist.

## Error handling strategy

- On import mismatch, show a clear UI error and do not apply the population.
- On reset with invalid config, revert to defaults and log `sensors.layout.reset_fallback`.

## Expected failure modes

- Default v2 switch breaks bots if offsets are still derived from `5 + 3 * bins` (caught by `src/bots/baselineBots.test.ts`).
- `sensorSpec` order/count mismatch between server and client (caught by `server/integration.test.ts`).
- Graph spec validation rejects layouts because `CFG.brain.inSize` not updated (caught during reset in manual debug playbook).

## Performance considerations

- Monitor v2 performance for additional `sqrt` calls; keep segment checks capped.
- Ensure scratch buffers are reused after the default switch.

## Security and privacy

- No new external data exposure.
- Avoid persisting debug logs.

## Observability and debugability

- Add a one-time log on default flip:
  - `sensors.layout.default_v2_enabled` with `{ bins, inputSize }`.
- Update the debug playbook to include clearing `localStorage` and `data/slither.db` when mismatches occur.

## Local debug playbook

- `npm run server`
- `npm run dev`
- Open Settings -> Sensors, confirm bins and rNear/rFar values.
- Reset the world, confirm sensorSpec order/count and sensor length match `CFG.brain.inSize`.

## Rollout

- Rollout: flip default to v2 after stage tests pass.

## Tests and validation

- Modify tests:
  - `src/sensors.test.ts` expectations for default layout size and scalar ordering.
  - `src/bots/baselineBots.test.ts` for new offsets and layout size.
  - `server/integration.test.ts` to expect v2 sensorCount and labels.
- Add tests:
  - `src/sensors.test.ts` case asserting default layout is v2.
  - `src/world.test.ts` case ensuring sensors remain finite after reset with v2 layout.
- Commands:
  - `npm test`
  - `npm run test:integration`
  - `npm run test:unit`

## Acceptance criteria mapping

- AC-SEN-001: `src/sensors.test.ts`, `src/bots/baselineBots.test.ts`, `server/integration.test.ts`.
- AC-SEN-008: `src/sensors.test.ts`, `src/world.test.ts`.
