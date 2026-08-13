# Stage 5 TypeScript world-step map

Evidence class: current-source trace plus the retained Stage 1 and Stage 2
fixtures named below. This is a behavior map for the approved forward port. It
does not make the TypeScript implementation a golden master for defects already
accepted for correction.

## Authoritative ordering currently implemented

`src/world.ts::World.step()` currently performs one fixed step in this order:

1. advance generation time, particle time, alive-snake age, and survival score;
2. create due ambient pellets;
3. publish external-controller observations;
4. update and respawn baseline bots;
5. collect all controls from the resulting pre-movement state;
6. apply the staged controls;
7. run movement, food, body, and collision substeps;
8. update observer/statistics and possibly end the generation; and
9. publish the supplied tick identity.

The Rust transaction preserves one stable pre-movement observation boundary but
corrects step 3 versus step 4: due baseline respawns occur before every due
controller samples the shared environment. Controls remain held through every
collision substep. `simSpeed` remains scheduler demand and never changes the
fixed delta or collision substep delta.

## Controller source selection

Current selection in `World::_collectFixedStepControls()` is baseline strategy,
active external lease, neutral `external-only`, and then a due neural graph.
All neural observations are sampled before any snake moves, and serial and
pooled results are staged before `_applyFixedStepControls()`.

The following current details are evidence, not the final rule:

- `Snake::_syncControlSource()` resets recurrent state when changing between
  external and neural input.
- `Snake::needsControlUpdate()` accumulates the fixed delta and uses
  `brain.controlDt`, with the first neural action immediately due.
- external turn is clamped to `[-1, 1]` and boost to `[0, 1]`;
- neural turn and boost outputs are clamped to `[-1, 1]`, with boost enabled by
  the later `> 0.35` threshold; and
- the current registry neutralizes a stale or disconnected lease, but removes
  the lease at grace expiry and therefore permits an implicit brain return.

The approved correction is one Rust-owned wall-time lease: hold the newest
accepted action for configurable 500 ms, then neutral steering and boost-off;
reserve exclusive ownership for configurable 30 seconds after disconnect with
no neural evaluation; then commit one explicit neural takeover and invalidate
the old controller. Browser actions are latest-value updates independent of
sensor delivery. Protocol 2 RL actions remain observation-driven.

## Per-step scalar accounting

`Snake::prepareForStep(dt)` ensures a head point exists, increases age by `dt`,
and adds `dt * reward.pointsPerSecondAlive` to points. The Rust transaction does
this once per fixed step, not once per collision substep. `points_delta_norm`
remains tied to an actually delivered observation boundary rather than movement
or frame publication.

## Steering, speed, boost, and movement formulas

The following current formulas are preservation targets, subject to Float64
Rust/TypeScript comparison tolerances:

- size fraction is
  `clamp((length - snakeStartLen) / max(1, snakeMaxLen - snakeStartLen), 0, 1)`;
- turn rate is `snakeTurnRate / (1 + snakeTurnPenalty * sizeFraction)`;
- radius is
  `clamp(snakeRadius + snakeThicknessScale * log1p(max(0, length - snakeStartLen) / max(1e-6, snakeThicknessLogDiv)), snakeRadius, snakeRadiusMax)`;
- non-boost speed target is
  `snakeBaseSpeed * (1 - snakeSizeSpeedPenalty * sizeFraction)`;
- effective boost addition is the base/boost speed ratio reduced by
  `snakeBoostSizePenalty * sizeFraction`;
- speed approaches its target with
  `lerp(speed, target, 1 - exp(-dt * 6.5))`;
- direction advances by `turn * turnRate * dt` and is normalized to one turn;
- position advances by the new direction and interpolated speed; and
- touching or crossing `worldRadius` by `head distance + radius` causes a wall
  death without corpse pellets.

Boost is requested when the held boost value exceeds `0.35`. It is eligible
only above `snakeMinLen + 1` body points and at or above
`boost.minPointsToBoost`. Its point cost is
`boost.pointsCostPerSecond * (1 + boost.pointsCostSizeFactor * sizeFraction) * dt`,
bounded by current points. Target length loses
`spent * boost.lenLossPerPoint`. Removed tail points request boost pellets of
`max(0.02, foodValue * boost.pelletValueFactor)` with configured jitter.

The current code consumes boost and emits trail pellets independently in every
collision substep. The Rust result must preserve that fixed-delta/substep
meaning and deterministic RNG sequence while staging all writes so a later
collision failure cannot leave partial authority.

## Body following and length

The current body is head-to-tail. After moving the head, every later point is
pulled to exactly `snakeSpacing` behind the already-updated predecessor. Growth
extends the tail along its last segment until `floor(targetLen)` points exist;
shrink removes tail points until the clamped target is reached. Radius is then
recomputed from the resulting point count.

Rust uses checked pooled body storage and reusable next-body scratch. A body
range is never partly rewritten in authority: capacity and every source range
are checked first, proposals are assembled in stable snake-ID order, and the
new packed body buffer is swapped only during the complete commit.

## Food and pellet behavior

The current eating radius is `snake.radius + 6`. An eaten pellet adds its value
to `foodEaten`, adds `value * reward.pointsPerFood` to points, and adds
`growPerFood * value` to target length within the configured min/max.

Current `Snake::advance()` mutates the pellet array while snakes are visited in
array order. That makes a pellet reachable by multiple heads an implicit
first-visited winner. This result is not preserved. Rust builds claims from one
immutable pellet/head snapshot, chooses the nearest eligible head, uses stable
snake ID for an exact squared-distance tie, and applies winning claims only
after all claims are known.

Ambient pellet debt accumulates at `pelletSpawnPerSecond * fixedDt`; only whole
pellets up to the configured target are emitted before observations. The
current fractal rejection sampler in `World::_spawnAmbientPellet()` and its
world-RNG draw order remain behavior to map when the ambient generator is
implemented. Large candidate arrays created per snake/substep are not copied.

## Death and drops

`Snake::die()` is idempotent. A non-wall death converts a length-dependent
fraction of body mass into large and small pellets, capped by
`death.maxPellets`; values and configured jitter currently consume the snake's
injected gameplay RNG. Baseline death also informs the baseline manager.

Normal Rust death keeps the scoring, mass fraction, pellet count/value, RNG,
and baseline notification meanings. Particle bursts are presentation events,
not authoritative particle state. Death proposals and pellet requirements are
preflighted before stable-ID commit. God Mode uses this same committed death
path.

## Collision behavior: evidence and corrections

Current `World::_resolveCollisionsGrid()` is not a preservation target. It:

- indexes only one midpoint for each body segment;
- always searches a hard-coded 3-by-3 cell neighborhood;
- visits snakes, grid cells, and segment linked lists in mutable container
  order;
- stops on the first detected body; and
- kills and awards immediately, so a newly dead body disappears from later
  tests in the same pass.

`src/stage2.killCredit.characterization.test.ts` proves these current examples:

- an unambiguous live body owner receives one kill and
  `reward.pointsPerKill`;
- exact head overlap leaves the later array entry alive and credited;
- an already-dead body is ignored; and
- an exact tie between two bodies credits grid insertion order.

The approved Rust rule preserves only the unambiguous body-owner credit. One
immutable swept snapshot produces every outcome. Simultaneous head-to-head
contact kills both with no kill credit. A head-to-body victim credits the body
owner even when that owner also dies in the same snapshot. Multiple valid body
owners use a stable deterministic selection rule defined and tested by the
collision implementation, never storage or worker order. All deaths are
committed in stable ID order, and every body remains an obstacle for the whole
snapshot.

The Rust broad phase covers every cell touched by swept segment bounds expanded
by collision radius and either stores all entries or rejects the substep before
commit. The narrow phase must detect moving head/body and head/head crossings;
final-position-only overlap is insufficient.

## Spawn correction

Current construction selects random head position and direction and lays the
body straight behind it, without checking the complete new body against
existing heads or bodies. The retained `SPAWN-001` fixture classifies that as a
defect. Rust tests complete initial bodies, wall clearance, and existing
geometry with bounded random attempts followed by a deterministic spatial
fallback. An impossible request fails clearly and never silently reduces the
population.

## Scheduler correction

`src/sim/SimCore.ts::update()` correctly derives whole fixed steps from wall
time times requested `simSpeed`, passes exactly `fixedDt` to each world step,
and records dropped debt at the pump cap. The current Node pump can still spend
too long inside one catch-up slice. Rust checks commands and newest accepted
actions before every overdue step, exposes overload, and gives Node/socket work
a service opportunity between overdue steps when interactive control exists.

## Named evidence retained for the port

- `src/stage2.killCredit.characterization.test.ts` records current kill-credit
  and order-bias examples.
- `src/stage1.correctionFixtures.test.ts` records body-sensing, collision
  capacity/order, and spawn corrections.
- `server/stage1.browserControl.integration.test.ts` records sensor-independent
  browser action transmission.
- controller, scheduler, world-ordering, snake, spatial-hash, determinism, and
  sensor tests remain the selected TypeScript oracle during migration.
- `docs/todo/evidence/stage2/behavior-source-map.md` indexes the wider current
  behavior corpus and distinguishes preserve from correct.

