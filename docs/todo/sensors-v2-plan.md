# Sensor v2 Plan

## Goals
- Replace the legacy sensor layout with the agreed v2 observation spec.
- Align hazard clearance with the actual collision geometry.
- Centralize sensor layout metadata so bots/tests/server stay in sync.
- Expose all sensor-related config values in the UI, with reset-only sliders for input-shape changes.

## Decisions
- Drop `forwardClearDelta`.
- Include enemy head segments in hazard clearance (match collision checks).
- Keep head-pressure as a separate head-only channel.
- Make bin count adjustable via UI, but require reset when it changes.
- Do not support old saves/exports; clearing local storage is acceptable.

## Sensor spec v2

### Layout
- `bins = CFG.sense.bubbleBins` (default 16).
- `inputSize = 8 + 4 * bins`.
- Ordering:
  - 0..7: self scalars (8)
  - 8..(8 + bins - 1): food potential
  - next `bins`: lethal clearance
  - next `bins`: wall clearance
  - next `bins`: opponent head pressure

### Coordinate frame + binning
- All angles are relative to `snake.dir`.
- Centered bin mapping so forward is at bin center:
  - `u = (relAngle + pi) / (2 * pi)` in [0, 1)
  - `u = (u + 0.5 / bins) % 1`
  - `idx = floor(u * bins)` clamped to [0, bins - 1]

### Radii
- `s01 = clamp(snake.sizeNorm(), 0, 1)`
- `rNear = clamp(rNearBase + rNearScale * s01, rNearMin, rNearMax)`
- `rFar = clamp(rFarBase + rFarScale * s01, rFarMin, rFarMax)`
- Defaults (from the v2 spec):
  - `rNearBase = 420`, `rNearScale = 260`, `rNearMin = 350`, `rNearMax = 650`
  - `rFarBase = 900`, `rFarScale = 800`, `rFarMin = 900`, `rFarMax = 1700`

### Self scalars (indices 0..7)
- `sizeNorm`: `clamp(snake.sizeNorm() * 2 - 1, -1, 1)`
- `speedNorm`: `clamp((snake.speed / maxSpeed) * 2 - 1, -1, 1)` where `maxSpeed = max(CFG.snakeBaseSpeed, CFG.snakeBoostSpeed)`
- `boostBudgetNorm`: `margin = (snake.pointsScore - minBoostPts) / max(minBoostPts, 1e-6)` mapped to [-1, 1]
- `isBoosting`: `snake.boost > 0 ? 1 : -1`
- `sin(angleToCenterRel)` and `cos(angleToCenterRel)`
  - `angleToCenter = atan2(-snake.y, -snake.x)`
  - `angleToCenterRel = angNorm(angleToCenter - snake.dir)`
- Remaining scalar slots (if any) stay unused (none in v2).

### Channel A: food potential
- Iterate pellets within `rFar`.
- Accumulate `wDist = 1 - d / rFar` and `wVal = clamp(p.v / CFG.foodValue, 0, 6)`.
- Saturation: `KFood = foodKBase * (rFar / rFarBase)`, `frac = accum / (accum + KFood)`, map to [-1, 1].
- Use existing pellet grid and `maxPelletChecks`.

### Channel B: lethal clearance
- Initialize each bin clearance to `rNear`.
- Query collision grid around the head, include enemy head segments.
- For each segment:
  - `d = sqrt(pointSegmentDist2(...))`
  - `clear = d - (snake.radius + other.radius) * CFG.collision.hitScale`
  - Clamp to `[0, rNear]`, keep the minimum in the corresponding bin.
- Normalize `clear / rNear`, map to [-1, 1].

### Channel C: wall clearance
- For each bin:
  - `theta = snake.dir + binCenterAngle`
  - `t = distToWallAlongRay(head, theta, CFG.worldRadius)`
  - `clear = max(0, t - snake.radius)`
- Normalize `clear / rNear`, map to [-1, 1].

### Channel D: opponent head pressure
- For each other alive snake head within `rFar`:
  - Record min distance per bin.
- Convert to pressure: `prox = 1 - clamp(dist / rFar, 0, 1)` then map to [-1, 1].

## Action items
- [ ] Create a shared sensor layout helper (counts, offsets, order labels, input size) and use it in `src/sensors.ts`, `src/bots/baselineBots.ts`, `src/sensors.test.ts`, and `server/index.ts`.
- [ ] Replace `buildSensors` with the v2 spec implementation and remove any legacy sensor logic (including `bestPointsThisGen` from inputs).
- [ ] Align hazard clearance with collision math (`hitScale` + both radii) and include enemy head segments.
- [ ] Implement centered binning and rNear/rFar scaling; keep all outputs in [-1, 1].
- [ ] Update `CFG.brain.inSize` to derive from `bins` and the new layout; ensure graph spec validation uses the derived size.
- [ ] Expose all `CFG.sense` fields as UI sliders in `src/settings.ts` and allow them via `src/protocol/settings.ts`; mark `bubbleBins` as reset-only.
- [ ] Update baseline bot logic to use layout helper offsets for food/hazard/wall bins; ignore head-pressure unless explicitly added later.
- [ ] Update `server/index.ts` sensorSpec order to match the v2 layout helper.
- [ ] Update sensor and bot tests; add geometry-focused tests for hazard and wall clearance.
- [ ] Update `README.md` to reflect the new sensor slider names and meanings.

## Config changes
- Replace legacy sensor config fields with v2 fields where needed.
- Add v2 radius and food saturation constants to `CFG.sense` and expose them via UI sliders.
- Keep `maxPelletChecks` and `maxSegmentChecks` exposed for performance tuning.

## Validation
- Run `npm test` or relevant subsets to cover sensors/bots/server spec changes.
- Spot-check in dev server: verify bins populate, clearance aligns with deaths, and head-pressure tracks nearby heads.
