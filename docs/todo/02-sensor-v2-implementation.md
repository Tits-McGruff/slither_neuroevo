# Stage 02: Sensor v2 Computation Path (Guarded)

## Revision notes

- 2025-01-13: Created stage doc to add the v2 computation path behind the layout guard while keeping the current default behavior intact.
- 2025-01-13: Removed backward-compatibility framing while preserving staged rollout.

## Scope

- Implement v2 sensor layout and channel math behind the layout helper (`layoutVersion = 'v2'`).
- Keep the current default output unchanged until Stage 04 flips the default.
- Add deterministic, geometry-focused tests for the new math.

## Non-goals

- No default switch to v2 yet.
- No UI slider exposure changes (handled in Stage 03).
- No baseline bot behavior changes (handled in Stage 04).

## Assumptions

- `buildSensors` continues to be the public API for snake sensors.
- `Snake.speed` and `Snake.boost` already exist (`src/snake.ts`) and will be used as scalars.

## Compatibility intent

- No migration is provided; old saves/exports are treated as invalid once v2 is active.

## Staging intent

- v2 is opt-in via `layoutVersion = 'v2'`.
- Default output stays unchanged until the final stage to keep CI stable.

## Constraints and invariants

- INV-SEN-001 through INV-SEN-005 apply, anchored in AGENTS.md "Simulation core: World, Snake, sensors, and physics" and "Project-specific conventions and gotchas."
- Any new arrays for v2 must reuse buffers across calls.

## Delta architecture overview

- Add a v2 branch inside `buildSensors`, selected by `layoutVersion`.
- Implement centered bin mapping, rNear/rFar scaling, and new channels.
- Introduce new sensor config fields for v2 math, but keep them inactive unless v2 is enabled.

## Symmetry checklist

- Worker: `src/sensors.ts` changes apply in worker mode automatically (shared code).
- Main: no direct changes, but graph spec validation must read the derived `CFG.brain.inSize`.
- Server: sensorSpec continues to reflect the current layout until Stage 04; v2 tests set layoutVersion explicitly.
- UI: no changes in this stage.
- Desync failure mode: v2 layout enabled without matching `CFG.brain.inSize` causes graph spec rejection and sensor length mismatches.

## Touch-point checklist (hard contracts)

- `src/sensors.ts` implements v2 under `layoutVersion = 'v2'`.
- `src/sensors.test.ts` adds deterministic tests for bin mapping and clearance math.
- `src/world.ts` collision geometry (`hitScale`) is the reference for hazard clearance.

```ts
function angleToCenteredBin(relAngle: number, bins: number): number {
  let u = (relAngle + Math.PI) / (2 * Math.PI);
  u = (u + 0.5 / bins) % 1;
  const idx = Math.floor(u * bins);
  return Math.max(0, Math.min(bins - 1, idx));
}
```

## Coordinate frame and bin mapping (edge cases)

- Use centered mapping:
  - `u = (relAngle + pi) / (2 * pi)` in `[0, 1)`
  - `u = (u + 0.5 / bins) % 1`
  - `idx = floor(u * bins)` clamped to `[0, bins - 1]`
- Example with `bins = 16`:
  - `relAngle = 0` -> `u = 0.5`, shifted to `0.53125`, `idx = 8` (forward bin).
  - `relAngle = -pi` -> `u = 0`, shifted to `0.03125`, `idx = 0`.
  - `relAngle = pi - eps` -> `u` approx `1`, shifted to `0.03125 - eps`, `idx = 0`.
- Exactly-on-boundary cases map deterministically via `floor` after shift.

## Radii derivation and units

- `sizeNorm` is `snake.sizeNorm()` (computed from length, clamped `[0, 1]`).
- `rNear` and `rFar` are recomputed per tick using world units and clamped to min/max bounds.
- Monotonicity requirement: `rNear` and `rFar` must be non-decreasing with `sizeNorm`.
- Guard: ensure `rFar >= rNear + 1` after clamps to avoid zero-width far range.

## Alternatives considered

- Replace `buildSensors` with a new `buildSensorsV2` export. Tradeoff: more code and call sites; harder to maintain symmetry.
- Inline v2 logic without the layout helper. Tradeoff: higher drift risk across bots/tests/server.

## Merge prerequisites

- Requires Stage 01 (`docs/todo/01-sensor-layout-contract.md`) for the layout helper.

## Stage outputs

- v2 computation path exists behind `layoutVersion = 'v2'`.
- Deterministic, geometry-focused tests cover v2 math.
- Default output remains unchanged until Stage 04.

## Planned modules and functions

- `src/sensors.ts`
  - Update `SnakeLike` to include `speed` and `boost`.
  - Add `buildSensors` branch:
    - `const layout = getSensorLayout(bins, layoutVersion)`
    - Use `layout.offsets` and `layout.inputSize`.
  - Add helpers:
    - `angleToCenteredBin(relAngle: number, bins: number): number`
    - `computeRadii(sizeNorm: number): { rNear: number; rFar: number }`
    - `fillFoodBins(...)`, `fillHazardBins(...)`, `fillWallBins(...)`, `fillHeadBins(...)`
  - Keep `buildSensors(world, snake, out?)` signature and typed-array reuse semantics.

## Hazard clearance geometry alignment

- Segment source: `_forEachNearbySegment` uses `world._collGrid.query` with segment indices.
- Include enemy head segments (segment index `1` covers head-to-second-point).
- Exclude self segments (`other === snake`).
- Geometry: `clear = sqrt(pointSegmentDist2) - (snake.radius + other.radius) * CFG.collision.hitScale`.
- Clamp to `[0, rNear]` and keep per-bin minima.
- Negative test: a head segment inside the threshold must reduce clearance; ensure the test fails if head segments are skipped.

```ts
const dist = Math.sqrt(pointSegmentDist2(hx, hy, p0.x, p0.y, p1.x, p1.y));
const thr = (snake.radius + other.radius) * CFG.collision.hitScale;
const clear = Math.max(0, dist - thr);
```

## Wall clearance raycast semantics

- Use `_distToWallAlongRay(headX, headY, theta, CFG.worldRadius)` from `src/sensors.ts`.
- Subtract `snake.radius` to convert center-to-wall distance to clearance.
- Clamp to `[0, rNear]` and normalize by `rNear` (never `rFar`).
- Edge case: if the head is outside bounds, treat clearance as 0 and log `sensors.v2.out_of_bounds`.

## Food potential and cap behavior

- Pellet selection uses `pelletGrid.map` when available; fallback sampling uses a step size to cap work.
- Determinism: with a fixed pellet grid and `maxPelletChecks`, the iteration order is stable (cell scan then array order).
- Numeric stability: when `accum = 0`, `frac = 0` yields `-1` after mapping.
- Log optional debug counter `sensors.v2.pellet_checks` when debug is enabled.

## Opponent head pressure semantics

- Include other snakes where `alive === true` and `id !== snake.id`.
- Multiple heads in the same bin use the minimum distance.
- No body segments in this channel; add a test that fails if bodies are sampled.

## Data model and data flow

- New `CFG.sense` fields (used only when `layoutVersion = 'v2'`):
  - `rNearBase`, `rNearScale`, `rNearMin`, `rNearMax`
  - `rFarBase`, `rFarScale`, `rFarMin`, `rFarMax`
  - `foodKBase`
- `CFG.brain.inSize` derived from layout helper when v2 is active.
- `buildArch` continues to use `CFG.brain.inSize` without additional changes.
- Graph spec validation in `src/main.ts` uses `CFG.brain.inSize`; ensure v2 path updates are visible before validation runs.

## Error handling strategy

- Clamp `bins`, `rNear`, and `rFar` to valid ranges before use.
- If `rFar <= rNear`, clamp `rFar` to at least `rNear + 1`.
- For non-finite inputs, default to safe values and log `sensors.v2.invalid_config`.

## Expected failure modes

- Hazard clearance too optimistic if `hitScale` or `snake.radius` is omitted (caught by clearance geometry tests).
- Wall clearance normalized by `rFar` instead of `rNear` (caught by wall normalization test).
- Centered bin mapping off-by-one at `-pi`/`pi` boundaries (caught by bin mapping edge tests).
- Head-pressure channel polluted by body segments (caught by head-only negative test).

## Performance considerations

- Use shared scratch arrays for each channel; do not allocate per call.
- Avoid redundant `sqrt` in hazard bins by only computing when a candidate improves the min.
- Maintain existing `maxPelletChecks` and `maxSegmentChecks` caps.

## Security and privacy

- Do not emit per-snake sensor arrays in logs.
- Debug output should be aggregate stats only (Stage 03+).

## Observability and debugability

- Add optional debug logs for v2 path entry:
  - `sensors.v2.enabled` with `{ bins, rNear, rFar }`.

## Rollout

- Rollout: v2 available via layoutVersion; default output unchanged.

## Tests and validation

- Modify tests:
  - `src/sensors.test.ts`: add new `describe('v2 layout')` tests that set layoutVersion to `v2`.
- New tests to add in `src/sensors.test.ts`:
  - Centered bin mapping edge cases (`-pi`, `pi`, tiny epsilon).
  - rNear/rFar bounds and monotonicity based on `sizeNorm`.
  - Hazard clearance geometry aligned with `hitScale`.
  - Wall clearance subtracts radius and normalizes by rNear.
  - Head pressure head-only behavior with multiple heads per bin.
  - Food potential saturation determinism under fixed pellets.
- Commands:
  - `npm test`
  - `npm run test:unit`

## Acceptance criteria mapping

- AC-SEN-002, AC-SEN-003, AC-SEN-004, AC-SEN-005, AC-SEN-006, AC-SEN-008 covered by new `src/sensors.test.ts` cases.
