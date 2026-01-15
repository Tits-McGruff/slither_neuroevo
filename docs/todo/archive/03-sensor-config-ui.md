# Stage 03: Sensor Config and UI Exposure

## Revision notes

- 2025-01-13: Created stage doc to expose sensor config values in the UI and settings allow-list, with reset-only enforcement for input-shape changes.

## Scope

- Add UI sliders for all `CFG.sense` fields used by v2, including bins and radius parameters.
- Register new settings paths for validation and server reset handling.
- Define reset-only behavior for any setting that changes input size.

## Non-goals

- No default switch to v2 layout yet.
- No baseline bot logic changes.
- No new protocol messages beyond settings updates already supported.

## Assumptions

- Settings are wired via `SETTING_SPECS` in `src/settings.ts`.
- Live-update paths are validated by `SETTINGS_PATHS` in `src/protocol/settings.ts`.

## Compatibility intent

- No migration is provided; old saves/exports are treated as invalid once v2 is active.
- UI and settings changes are additive and must not alter default simulation behavior.
- Reset-only settings must not apply live and must require explicit reset.

## Constraints and invariants

- INV-SEN-001 and INV-SEN-006 apply, grounded in AGENTS.md "UI, settings, and visualization panels" and "Documentation expectations."
- UI slider labels must mirror `CFG` paths and `README.md` (AGENTS.md "Documentation expectations").

## Delta architecture overview

- Add a new "Sensors" group in `SETTING_SPECS`.
- Extend settings allow-list for sensor paths.
- Enforce reset-only semantics for bin count and layout version toggles.

## Symmetry checklist

- Worker: `updateSettings` applies new sensor paths (validate in `src/worker.test.ts`).
- Main: reset-only controls must trigger a reset flow (not live updates).
- Server: reset message validation must allow new sensor paths (`SETTINGS_PATHS`).
- UI: sliders must render with correct labels, ranges, and `requiresReset` flags.
- Desync failure mode: missing allow-list entries cause server reset rejection.

## Touch-point checklist (hard contracts)

- `src/settings.ts` includes "Sensors" group with correct `path` values.
- `src/protocol/settings.ts` includes matching `sense.*` entries.
- `src/settings.test.ts` asserts sliders exist and reset-only flags are set.
- `server/protocol.ts` rejects unknown settings paths in reset messages.

```ts
// settings.ts
{ group: "Sensors", path: "sense.bubbleBins", label: "Sensor bins", min: 8, max: 32, step: 1, decimals: 0, requiresReset: true }

// settings.ts allow-list (protocol)
export const SETTINGS_PATHS = [
  // ...
  'sense.bubbleBins',
  'sense.rNearBase',
  'sense.rNearScale'
];
```

## Alternatives considered

- Expose only a subset of sensor settings (bins + rNear/rFar). Tradeoff: less tuning control.
- Hide layout version from UI and keep it config-only. Tradeoff: less explicit reset guidance.

## Merge prerequisites

- Requires Stage 01 (`docs/todo/01-sensor-layout-contract.md`) for shared layout and sensorSpec wiring.

## Stage outputs

- "Sensors" group appears in settings UI with all `sense.*` sliders.
- `SETTINGS_PATHS` includes all `sense.*` entries so server resets accept them.
- Reset-only sliders do not emit live updates.

## Planned modules and functions

- `src/settings.ts`
  - Add `SettingSpec` entries under a "Sensors" group for:
    - `sense.bubbleBins` (requiresReset = true)
    - `sense.layoutVersion` (requiresReset = true; use checkbox if boolean)
    - `sense.rNearBase`, `sense.rNearScale`, `sense.rNearMin`, `sense.rNearMax`
    - `sense.rFarBase`, `sense.rFarScale`, `sense.rFarMin`, `sense.rFarMax`
    - `sense.foodKBase`
    - `sense.maxPelletChecks`, `sense.maxSegmentChecks`
  - Ensure labels, min/max/step mirror expected units (world units or ratios).
- `src/protocol/settings.ts`
  - Add the same `sense.*` paths to `SETTINGS_PATHS`.
- `src/main.ts`
  - Ensure reset-only controls trigger a reset message and do not live-update.
  - Surface validation errors in existing settings UI hint patterns.
  - Functions impacted:
    - `buildSettingsUI(container: HTMLElement): void`
    - `hookSliderEvents(root: HTMLElement, onLiveChange: (input: HTMLInputElement) => void): void`
    - `updateCFGFromUI(root: HTMLElement): void`

```ts
// Suggested slider ranges (world units)
{ path: "sense.bubbleBins", min: 8, max: 32, step: 1, requiresReset: true }
{ path: "sense.rNearBase", min: 200, max: 900, step: 10, requiresReset: true }
{ path: "sense.rNearScale", min: 0, max: 600, step: 10, requiresReset: true }
{ path: "sense.rNearMin", min: 150, max: 900, step: 10, requiresReset: true }
{ path: "sense.rNearMax", min: 200, max: 1200, step: 10, requiresReset: true }
{ path: "sense.rFarBase", min: 400, max: 2000, step: 20, requiresReset: true }
{ path: "sense.rFarScale", min: 0, max: 1200, step: 20, requiresReset: true }
{ path: "sense.rFarMin", min: 400, max: 2200, step: 20, requiresReset: true }
{ path: "sense.rFarMax", min: 600, max: 3000, step: 20, requiresReset: true }
{ path: "sense.foodKBase", min: 0.5, max: 12.0, step: 0.1, requiresReset: false }
{ path: "sense.maxPelletChecks", min: 100, max: 3000, step: 50, requiresReset: false }
{ path: "sense.maxSegmentChecks", min: 200, max: 4000, step: 50, requiresReset: false }
```

## Data model and data flow

- Reset-only semantics:
  - In worker mode, `reset` triggers `init` with updated `CFG` and rebuilds the world.
  - In server mode, `reset` sends `updates` and `settings` to the server which rebuilds the world.
- Settings paths must be allow-listed or server updates will be rejected (`server/protocol.ts`).
- `hookSliderEvents` relies on the `requiresReset` dataset attribute; ensure new sliders populate it.
  - For reset-only sliders, no live `updateSettings` should be sent until reset.

## UI behavior notes

- Reset-only sliders show the standard "requires reset" hint and do not trigger live updates.
- On apply/reset, the world rebuilds with the updated sensor layout and input size.

## Error handling strategy

- Reject invalid numeric settings (non-finite, out of range) in `updateCFGFromUI`.
- For reset-only changes, show a UI hint that a reset is required before changes take effect.
- Add a server-side negative test to ensure invalid sensor paths are rejected in `parseClientMessage`.
- Ignore settings updates with missing `path` or `value` fields and log `settings.update.invalid_payload`.

## Expected failure modes

- Reset-only sliders still emit live updates because `requiresReset` is missing (caught by `src/settings.test.ts`).
- Server rejects sensor updates because `SETTINGS_PATHS` is missing a `sense.*` entry (caught by `server/protocol.test.ts`).
- UI sliders show incorrect ranges for rNear/rFar (manual UI check after reset).

## Performance considerations

- Settings UI changes should not introduce new per-frame work.
- No new allocations in hot loops.

## Security and privacy

- Avoid logging user config values beyond aggregate debug info.
- No new storage or export paths.

## Observability and debugability

- Add a UI "Sensors debug" toggle mapped to `CFG.sense.debug` with a tooltip describing log output.

## Rollout

- Rollout is additive and can ship without behavior change.

## Tests and validation

- Modify tests:
  - `src/settings.test.ts` to assert new sensor sliders are created.
  - `src/worker.test.ts` to ensure `updateSettings` can apply new sensor paths.
- Add tests:
  - `server/protocol.test.ts` to reject reset updates with unknown `sense.*` paths.
- Add tests:
  - `src/settings.test.ts` case asserting `requiresReset` data attribute for bins/layout.
- Commands:
  - `npm test`
  - `npm run test:unit`

## Acceptance criteria mapping

- AC-SEN-007: `src/settings.test.ts`, `src/worker.test.ts`.
