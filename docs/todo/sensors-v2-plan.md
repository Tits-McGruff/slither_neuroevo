# Sensor v2 Plan

## Goals

- Replace the legacy sensor layout with the agreed v2 observation spec.
- Align hazard clearance with the actual collision geometry and wall checks.
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
- No additional scalars.

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

## Detailed implementation plan

### Step 0: Establish the shared layout contract

- Create a shared sensor layout helper that defines:
  - Scalar count (8).
  - Channel counts and offsets.
  - Total input size.
  - Label order for server sensorSpec.
- Use this helper everywhere to eliminate literals:
  - `src/sensors.ts` (allocation and offsets).
  - `src/bots/baselineBots.ts` (food/hazard/wall offsets).
  - `src/sensors.test.ts` and any bot tests.
  - `server/index.ts` sensorSpec order/count.

### Step 1: Replace the sensor builder with v2

- Update `buildSensors` to the v2 layout.
- Remove any dependence on `bestPointsThisGen` in inputs.
- Implement centered bin mapping.
- Implement rNear/rFar derived from `snake.sizeNorm()` and config constants.
- Keep all outputs within `[-1, 1]`.

### Step 2: Align clearance geometry with collisions

- Hazard clearance:
  - Include enemy head segments.
  - Use `CFG.collision.hitScale` and both radii.
  - Use `pointSegmentDist2` with `sqrt`.
- Wall clearance:
  - Use `_distToWallAlongRay` minus `snake.radius`.
  - Normalize by `rNear`.

### Step 3: Implement food potential and head pressure

- Food bins:
  - Use rFar.
  - Saturation with `foodKBase` scaled by rFar.
- Head pressure:
  - Head-only, within rFar.
  - No body segments in this channel.

### Step 4: Config and input size plumbing

- Update `CFG.sense` with v2 parameters:
  - `bubbleBins` (default 16).
  - `rNear` params and `rFar` params.
  - `foodKBase`.
  - `maxPelletChecks`, `maxSegmentChecks` remain.
- Update `CFG.brain.inSize` to derive from layout helper.
- Ensure graph spec validation uses derived input size.

### Step 5: UI slider exposure

- Add all sensor config values to `src/settings.ts`:
  - Expose bins and both radius parameter groups.
  - Expose food saturation constant.
  - Expose max check caps.
- Register allowed settings paths in `src/protocol/settings.ts`.
- Mark `bubbleBins` (and any input-size impacting settings) as reset-only.
- Update `README.md` sensor slider docs accordingly.

### Step 6: Baseline bot integration

- Replace `5 + 3 * bins` style offsets with the layout helper.
- Keep bot logic focused on food/hazard/wall channels.
- Do not use head-pressure unless explicitly added later.

### Step 7: Server sensorSpec update

- Generate the order list from the shared layout helper.
- Ensure `sensorCount` matches derived input size.
- Adjust any integration tests that assume the old layout.

### Step 8: Tests and validation

- Update `src/sensors.test.ts` for the new layout and scalars.
- Update baseline bot tests to use new offsets.
- Add a geometry test:
  - Place a segment at a known distance and validate lethal clearance.
  - Place snake near wall and validate wall clearance.
- Run `npm test` (or subsets) after changes.

## Acceptance criteria

- Lethal clearance bins match actual collision boundaries.
- Wall clearance bins match wall collision boundaries.
- Head pressure channel reflects nearby enemy heads and stays stable.
- All inputs remain within `[-1, 1]` with non-trivial variance.
- Bots and server sensorSpec remain aligned with the new layout.

## Config changes

- Remove legacy sensor config fields that are no longer used.
- Add v2 radius and food saturation constants to `CFG.sense`.
- Keep caps on pellet/segment checks for performance tuning.

## Validation

- Run `npm test` or relevant subsets to cover sensors/bots/server spec changes.
- Manual spot-check in dev server:
  - bins populate in all channels
  - collisions line up with lethal clearance
  - head pressure tracks nearby heads
