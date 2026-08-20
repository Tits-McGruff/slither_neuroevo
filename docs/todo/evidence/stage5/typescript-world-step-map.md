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

The TypeScript wall path advances scalar `x/y` and then returns from
`Snake::die()` before copying that position into `points[0]`. Rust preserves
the final scalar position, death, body length, lack of corpse pellets, and lack
of follow/grow/radius work, but normalizes the dead body's first coordinate to
that position. This is an internal representation correction required by the
admitted Rust invariant `body[0] == position`. A source-alive snake remains a
collision obstacle for the immutable snapshot in which its wall death is
staged. After that result commits it is neither rendered nor collision-active
in later substeps.

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

Rust movement retains tail-derived boost requests without drawing RNG. The
post-collision effect phase realizes those requests first, in stable snake-ID
then tail-to-head order. Each request consumes exactly two uniform draws from
the owning snake's gameplay stream for X and Y jitter. Evolved and resurrected
snakes use the world gameplay stream, external snakes use the isolated
external-controller stream, and each baseline slot uses its own durable
stream. Evolution RNG is never used for movement or death effects.

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
after all claims are known. Winning claims and surviving pellets are applied in
stable pellet-ID order so container order cannot alter floating-point update
order. Newly requested boost drops are realized later in the complete
transaction and are not eligible for consumption in the substep that creates
them, matching the approved phase order rather than TypeScript's immediate
pellet-array mutation.

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

The executable current-source artifact
`typescript-death-fixtures.json`, regenerated by
`scripts/stage5/generate-death-fixtures.ts`, retains the exact length-12
large-then-small pellet sequence and uniform RNG continuation. Rust orders all
boost requests before all corpse drops, then orders corpse owners by stable
snake ID, with large pellets before small pellets for each owner. Pellet IDs
come from one checked contiguous allocator reservation after the complete
result count fits the admitted world capacity. Frame-v1 pellet kinds remain
large corpse `1`, small corpse `2`, and boost `3`; owner identity and the
snake's exact frame-v1 ID remain attached for rendering. Wall deaths produce no
corpse pellets or RNG draws but still emit the baseline-death event when the
victim is a baseline bot. The staged RNG bundle, allocator continuation,
pellets, and events remain non-authoritative until the later complete physics
commit revalidates its source key and publishes every phase together.

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
owners are ordered by earliest continuous swept contact; an exact computed-time
tie uses stable body-owner ID and then stable segment offset. A simultaneous
head-head pair cannot award itself a kill merely because each head also lies on
the other snake's first body segment. These rules never use storage or worker
order. All deaths are committed in stable ID order, and every body remains an
obstacle for the whole snapshot.

The Rust broad phase covers every cell touched by swept segment bounds expanded
by collision radius and either stores all entries or rejects the substep before
commit. The narrow phase must detect moving head/body and head/head crossings;
final-position-only overlap is insufficient. The relative moving-segment search
uses a convex-hull lower bound and conservatively treats an unresolved spatial
gap no larger than `1e-9` world units as contact; the diagnostic count makes
those tolerance decisions visible rather than permitting a tunnelling miss. A
contact search that reaches its checked interval or depth limit rejects the
complete substep; it never manufactures a collision to satisfy a work budget.
Segments newly created by post-food growth become collision-active at the final
substep boundary; they are not projected backward through time before they
existed. Boost-removed tail segments are absent because boost burn precedes
movement. Segments removed by ordinary post-food shrink retain their
post-movement geometry for the continuous sweep and disappear at the final
boundary, so a moving segment cannot evade collision merely because it is
trimmed after movement. The continuous interval likewise uses the pre-food
radii; post-food growth or shrink changes collision radius only at the final
boundary. Broad-phase bounds cover the larger of the pre-food and final radii,
so neither interval can be omitted.

## Multi-substep physics transaction

Current `World::_advanceFixedStepPhysics()` mutates the TypeScript world after
each collision substep. The Rust migration cannot publish those intermediate
boundaries: a capacity, arithmetic, identity, controller-replacement, or later
substep failure must leave the last authoritative fixed step unchanged.

`engine::physics` therefore begins from one immutable physical boundary and
keeps a reusable non-authoritative working world for the declared substep
count. Each accepted substep contains the complete movement, food, collision,
effect, RNG, and allocator continuation. Deaths are applied in stable snake-ID
order; unambiguous head-to-body credit increments the selected body owner's
kill count and adds `reward.pointsPerKill`, including when that owner also dies
in the same immutable snapshot. Simultaneous head-to-head deaths award neither
participant. Later substeps consume the prior accepted working boundary, so a
snake cannot die or receive the same award again merely because the fixed step
was subdivided.

The working transaction is bound to the process-local world epoch, generation,
source completed-step, population epoch, configuration revision/hash,
operation epoch, substep ordinal, and the complete projected movement, food,
collision, death, index-capacity, delta, and kill-credit configuration. The
transaction itself supplies every phase with its current working world, RNG,
and allocator continuation; a caller cannot submit an independently prepared
effect result and label it as current. It exposes a complete result only after
every declared substep has joined. Errors may change retained scratch capacity
but do not change the source world or expose a successful result. Live
controller leases are deliberately not copied into the physical scratch;
until the later full-step coordinator stages death/reassignment as one
operation, a collision involving a controlled snake is rejected with an
explicit replacement-required result. The later coordinator must project the
key and phase configuration from the same admitted authority, revalidate that
key, combine controller/recurrent and before/after-step state, and perform the
single authoritative swap. This working transaction alone is not that
publication boundary.

## Spawn correction

Current `Snake` construction consumes three uniform draws: polar angle,
area-uniform radius `sqrt(draw) * worldRadius * 0.60`, and heading. It then lays
`snakeStartLen` points straight behind the head at `snakeSpacing`, without
checking the wall or any existing head/body. `World::_spawnAll()` supplies the
world stream in population-array order; baseline construction supplies each
slot's independent stream; resurrection currently consumes the world stream;
and the defective external join consumes both evolution RNG for a genome and
world RNG for geometry. The retained `SPAWN-001` and `RNG-001` correction
fixtures prevent blind admission and external-join contamination from becoming
parity targets.

`engine::spawn` version 1 preserves the three-draw random-candidate formula but
visits a batch in stable domain/slot order. It constructs every complete
head-to-tail body before acceptance. Every body point must clear the circular
wall by its radius plus configured wall clearance, and every polyline segment
must clear every live source body and earlier staged body by both radii plus
configured body clearance. Live source obstacles are themselves canonicalized
by stable snake ID before work-budget accounting, so source snake-array order
cannot change a pass/fail or overload result. Reversing request or source
storage produces the same stable-key placements and RNG continuation.

Each request has a checked finite random-attempt count. Exhaustion switches to
a finite deterministic low-discrepancy position scan with a finite set of
headings and no additional RNG draws. Their checked sum must fit the declared
per-request work budget and the implementation's admission ceiling. The whole
batch also has explicit candidate and geometry-comparison ceilings; exceeding
either rejects staging rather than hanging the engine, accepting an overlap, or
reducing the population. These defaults are provisional until Stage 5 P0–P3
spawn measurements establish a practical supported-work budget.

If neither search finds a placement, the whole prepared result remains
unavailable, the source world and serialized RNG remain unchanged, and the
error names the stable request and both search counts. Placement scratch
retains stable order, complete bodies, one candidate body, and capacity
diagnostics across batches. The later generation/controller coordinator must
invoke separate preparations for independently owned world, baseline, and
external-controller streams, join all required placements and ID/brain state,
preflight the combined admitted body-point ceiling, and publish them only as one
complete boundary.

The executable current-source artifact
`typescript-spawn-fixtures.json`, regenerated by
`scripts/stage5/generate-spawn-fixtures.ts`, retains seed `0x5afe`, the exact
angle/radius/heading draw order, the five TypeScript-constructed body points,
and the RNG continuation after three draws. Rust compares the literal captured
coordinates with an explicit cross-language tolerance and the continuation
state exactly; it does not derive the expected body through its own helper.

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
- `scripts/stage5/generate-death-fixtures.ts` and
  `docs/todo/evidence/stage5/typescript-death-fixtures.json` retain the current
  death/drop formula, RNG continuation, and pellet ordering as executable
  non-performance evidence.
- `scripts/stage5/generate-spawn-fixtures.ts` and
  `docs/todo/evidence/stage5/typescript-spawn-fixtures.json` retain the current
  constructor draw/body order and exact RNG continuation as executable
  non-performance evidence.
- controller, scheduler, world-ordering, snake, spatial-hash, determinism, and
  sensor tests remain the selected TypeScript oracle during migration.
- `docs/todo/evidence/stage2/behavior-source-map.md` indexes the wider current
  behavior corpus and distinguishes preserve from correct.
