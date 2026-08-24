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

The existing coarse `engine::control` batch now accepts a strictly canonical
set of due brain handles whose recurrent input is replaced by one retained
exact-zero block for that evaluation. This is the neural half of the explicit
takeover boundary: every other brain keeps its own recurrent state and packed
weights, source state remains unchanged until the existing complete commit,
and optional focused-activation capture uses the same zero-state choice as the
control evaluation. Duplicate, out-of-order, stale, or non-due reset handles
make the whole batch unavailable. The later control coordinator must derive a
handle only from a validated lease proposal whose one neural takeover begins
on that boundary; it must never expose this reset list as an unkeyed caller
choice or apply one reset more than once.

`engine::control_phase` version 1 now stages that complete pre-movement source
selection from the joined fixed-step prefix. It rebuilds one retained complete
body/pellet index pair, visits live snakes by stable internal ID, samples every
baseline, connected external client and due neural graph from that same
immutable indexed world, and runs all due differently weighted brains in one
coarse heterogeneous batch. A disconnected lease in its exclusive grace
period produces held or neutral external input and no neural work. Grace expiry
adds exactly that snake's validated brain handle to the zero-state takeover
set. External observation payloads remain packed Float32 values with their
delivery markers uncommitted until the matching Node connection accepts them;
baseline and neural delivery/state are likewise only staged at this point.

The persistent cadence accumulator represents simulated time accrued since the
last neural evaluation. Control-phase version 1 reserves the maximum admitted
`brain.controlDt` value as a finite pending-first-action sentinel; ordinary
post-action remainders are strictly smaller. A newly created neural snake uses
that value, encoding TypeScript's missing-first-action state without another
checkpoint field and keeping its first action due even if the live interval is
increased between creation and evaluation. A committed evaluation stores the
ordinary modulo remainder. External control keeps that accumulator neutral,
and the explicit post-grace takeover is force-due independently. The joined
result retains its complete source/config/key references but exposes no
authority-writing operation yet; the later fixed-step coordinator must
revalidate and publish controller transitions, chosen controls, internal
delivery markers and recurrent continuations as one complete transaction.

`engine::control_phase::ControlCommitWorkspace` now performs that internal
control publication into one reusable non-authoritative working boundary. It
copies the joined prefix, source brains and generation sensor continuation;
preflights every control, lease proposal, baseline slot/RNG result, packed
external event range and neural delivery/recurrent result; and only then
applies the infallible internal commit. Baseline and due-neural observation
markers advance because those observations were consumed inside Rust.
External observation markers remain unchanged and travel with the retained
packed event until matching Node acceptance is implemented. Initial grace
expiry clears the expired external action before applying the zero-state
neural result, while later `NeuralTakeover` boundaries do not neutralize an
already-held neural action. Immutable non-population weights become reusable
within one world/population epoch when each retained record still has the same
handle, owner and shape; recurrent blocks are refreshed and published on every
applicable boundary. The working result is a
physics input, not authority. `engine::world_step` now consumes that boundary
through complete non-authoritative physics and post-physics continuation
staging; current-state key/config projection, generation decisions and the
single final swap remain the later authority coordinator's responsibility.

## Per-step scalar accounting

`Snake::prepareForStep(dt)` ensures a head point exists, increases age by `dt`,
and adds `dt * reward.pointsPerSecondAlive` to points. The Rust transaction does
this once per fixed step, not once per collision substep. `points_delta_norm`
remains tied to an actually delivered observation boundary rather than movement
or frame publication. A newly respawned baseline receives the same one-step age
and survival-point increment before the shared observation boundary; existing
dead snakes receive neither. Rust authority admission already requires every
live snake to own a nonempty body, so the TypeScript empty-body repair is an
admission invariant rather than a normal hot-path insertion.

`engine::accounting` version 1 stages the next generation elapsed time plus all
live-snake age and survival-point changes in stable-ID order. The proposal is
bound to the complete world/generation/step/population/config/operation key,
immutable source world, source elapsed time, fixed delta, projected reward
setting and snake ceiling. It checks every derived scalar before making a
result available, retains canonical-order/update storage, and can change only
a coordinator-owned pre-step working copy whose snake records, world shape and
controller leases still match after a complete no-write preflight. The later
fixed-step coordinator remains responsible for enforcing this first-phase
ordering, applying it once, using the same formula for same-boundary baseline
newborns, and performing the one authoritative swap only after every later
phase succeeds.

## Ambient pellet accumulation and generation

`World::_spawnAmbientForFixedStep()` treats `_pelletSpawnAcc` as fractional
pellet credit despite its stale “seconds” comment. Each fixed step adds
`pelletSpawnPerSecond * dt`, realizes at most the current target deficit, and
subtracts only the realized integer count. Credit therefore continues growing
while corpse or boost pellets keep the world at or above the ambient target;
the target is a refill floor, not a hard pellet-storage ceiling. The initial
generation fill resets credit to zero and generates the complete target after
snakes are spawned. A legacy population-import path currently reverses that
initial ordering; that path is compatibility evidence rather than the Rust
transaction order.

`World::_spawnAmbientPellet()` reads the already-advanced generation time and
the world RNG. Every rejection candidate consumes angle, area-uniform radius,
then acceptance draws in that order. It tries at most eight candidates. An
early acceptance consumes exactly three draws per attempted candidate; after
eight rejections it chooses the greatest-probability candidate without another
draw, and strict `>` retains the earliest exact tie. The resulting ambient
pellet has configured food value, kind `ambient`, color ID zero, and no owner.
The current JavaScript path subtracts credit before generation and appends to
the array before spatial insertion, so an exception can expose partial state;
that failure behavior is corrected rather than preserved.

`engine::ambient` version 1 stages the accumulator, exact Rust pellet IDs,
world-RNG continuation, allocator continuation, and every generated pellet as
one unavailable-until-complete result. It retains generated storage across
steps, reuses serialized world-RNG storage, checks the admitted pellet ceiling,
and leaves the source boundary unchanged on RNG, ID, capacity, allocation, or
finite-math failure. The proposal is bound to the complete world/generation/
step/population/config/operation key and retains its source world, world RNG,
allocator continuation, credit, advanced generation time, delta, projected
config, and capacity for exact revalidation before later acceptance. The
executable current-source artifact `typescript-ambient-fixtures.json`,
regenerated by `scripts/stage5/generate-ambient-fixtures.ts`, records a
six-attempt early acceptance, the 24-draw strict-first fallback tie, and credit
accrual while full followed by bounded refill. Rust compares the literal
position with an explicit cross-language tolerance and the uniform RNG
continuation exactly.

## Baseline lifecycle and respawn timing

The current `BaselineBotManager.update()` interleaves per-slot respawn and
controller sampling. An alive low-index baseline can sample before a later
slot respawns, while a later alive baseline sees earlier respawns. A newborn is
aged/scored immediately, receives neutral input, and is not itself sampled
until the next fixed step. External observations are currently published
before any same-step respawn, while neural observations see every successful
respawn. The approved correction replaces all of those mixed boundaries:
advance every due timer, stage every due collision-safe respawn, then rebuild
spatial views and sample all controller classes from one complete boundary.

A notified death starts the configured timer without an immediate subtraction;
the first `dt` subtraction occurs on the next fixed-step update. A merely
missing/dead slot whose death callback was bypassed first initializes the full
delay and starts subtracting one boundary later. Duplicate notifications do
not restart a running timer. Lowering the live delay caps remaining timers;
raising it does not extend them. Successful respawn resets the slot's behavior
and wander state but continues its independent baseline RNG, consumes the
ordinary three spawn draws, receives a new baseline-domain ID, and keeps its
stable slot. The current array replacement assumes population-first and
baseline-by-slot storage; Rust must use stable identities instead.

`engine::baseline` version 1 adds generation-scoped stable-slot state and
stages the timer portion of step 9 without touching RNG, IDs, bodies, or the
authoritative world. Physics notifications enter only through the complete
keyed physics result; a raw same-ID event cannot be relabelled after Reset,
New Run, or import. A notified death begins the full configured delay at the
physics commit boundary; the next fixed step performs the first subtraction.
A dead slot whose notification was missed begins the full delay without that
subtraction. Active timers are capped before subtraction when a lower live
delay is admitted and are not extended by a higher delay. Exact expiry emits a
stable due-slot list and neutralizes the dead slot's action. The proposal is
bound to the complete step key, immutable world, immutable lifecycle state,
fixed delta, slot count, and delay; its three vectors retain capacity. The
snake record owns the single canonical baseline-strategy enum; lifecycle state
does not retain a second copy.

Lifecycle initialization requires one live snake record for every configured
slot, so Rust cannot silently start with a reduced baseline population. The
future coordinator may call it only after the complete initial spawn has
separately proved collision-safe placement. The timer module deliberately does
not perform replacement itself: a timer-only result containing any due slot is
ineligible for commit until collision-safe placement and replacement are
resolved. The joined fixed-step prefix described below now performs that reset
and replacement on its private working boundary while retaining the stable
slot and continuing its separately owned RNG stream. The lifecycle module and
prefix still preserve both possible reviewed scheduler outcomes for placement
failure rather than choosing one implicitly.

Draft 4 does not explicitly settle the rare mid-generation outcome when no
collision-safe baseline placement exists. Whether that rejects the whole step
or retains the dead slot for a later retry changes visible timing and must be
reviewed before the full coordinator finalizes it. No current non-atomic
partial-ID/RNG/container behavior is a preservation target.

## Joined fixed-step prefix

`engine::fixed_step` version 2 now joins once-per-step accounting, ambient
generation, baseline-timer staging, and every successful collision-safe due
baseline respawn from one immutable admitted source. It first derives the
advanced generation time, supplies that value to ambient generation, applies
accounting, generated pellets and timer updates to one reusable
non-authoritative working boundary, and then resolves due slots in stable slot
order before any controller can sample the world. Each replacement continues
its independent per-slot RNG, consumes fresh baseline-domain and exact frame-v1
IDs, retains its stable slot, resets lifecycle strategy/wander/action state,
and receives the same fixed-step newborn age and survival accounting as the
TypeScript order. An earlier replacement is a complete collision obstacle for
later slots. Old packed bodies are compacted, and pellet references to replaced
snake IDs clear only the internal owner while preserving the browser-visible
color ID.

Candidate and geometry ceilings are aggregate limits for the complete due-slot
set, not fresh allowances per slot. Each placement receives only the remaining
work budget. Failure therefore rejects the prefix before publication without
silently multiplying the configured ceiling by the number of baseline slots.
The result retains the complete step key, exact source
world/RNG/allocator/lifecycle references, elapsed time, ambient credit and
projected configuration for later revalidation. Serialized RNG and controller
text storage is retained across warm copies, including logical `Some`/`None`
transitions of Gaussian spare state.

This is the first composed pre-control boundary, not the complete fixed-step
coordinator and not an authority swap. A successful due respawn is included in
the boundary made available to shared controller sensing. An impossible or
work-budget-exhausted placement makes no prefix available and leaves source
world, RNG, allocators and lifecycle unchanged; the later scheduler's
owner-visible retry-versus-fault rule remains unresolved. Physics, generation
decisions, frame packing and publication remain outside this slice.

## Authoritative fixed-step continuation ownership

`StateCandidate::fixed_step` now owns the ambient-pellet accumulator, durable
baseline lifecycle, and sensor-generation best beside the authoritative world,
RNG, allocators, brains, and scheduler state. A running state cannot be admitted
with a missing baseline slot, mismatched slot/snake identity, incoherent
alive/dead respawn timer, or a baseline strategy/timer/wander combination that
the controller would reject at its next boundary. A live baseline's lifecycle
action must also equal the turn/boost held by its snake record, and the retained
generation best cannot trail any currently alive evolved snake's points. The
controller and state admission share the durable strategy-state validator.

Checkpoint-v3 bytes do not change for this addition. Ordinary exact saves are
already restricted to the pre-spawn generation boundary, where ambient credit
and sensor-generation best are exactly zero and baseline lifecycle slots have
not yet been initialized. Both final decode and the pre-allocation shell derive
that one reset continuation, while boundary admission rejects any nonzero or
initialized live value. Retained live baseline-slot capacity is included in the
authoritative memory ceiling.

This closes a split-ownership prerequisite only. The joined prefix, control and
post-control world-step phases still expose non-authoritative proposals. The
later authority coordinator must publish their complete continuation,
observation, control, recurrent, physics and generation results in one
validated swap.

## Baseline strategy and action evaluation

The current baseline controller chooses its life-stage policy from body-point
count: fewer than 25 points is small, 25 through 79 is medium, and 80 or more
is large. It first advances the durable roam/seek/avoid/boost state. Avoid and
boost timers subtract the fixed delta and expire to roam at zero. The worst of
body-hazard, wall and other-head clearance enters avoid below `-0.15`, consuming
one uniform draw for a duration in `[0.35, 0.70)`. Otherwise food above `0.1`
selects seek and safe snakes above 110 percent of the configured boost minimum
consume a chance draw; a result below `0.02` consumes a second draw for a boost
duration in `[0.2, 0.4)`. Roam consumes two draws only when its wander timer
expires.

Every policy scores the Float32 values from the four sensor-v3 bin channels in
Float64 arithmetic. Clearance is the minimum of hazard, wall and head. Unsafe
bins below `-0.4` receive the 1,000-point veto, sharp low-clearance turns receive
the current feasibility penalty, score ties within `1e-6` prefer the smaller
absolute angle, and an all-veto result uses the best-clearance bin. Small bots
weight clearance most heavily, clamp food to `0.4`, and boost only while
escaping. Medium bots may bias toward the nearest head within 25 radii, use the
current cutoff or encircle formula, and permit an attack boost only into a path
whose hazard/wall average exceeds `-0.1`. Large bots bias toward the centroid of
heads within 30 radii and do not attack-boost.

The medium hunter override changes only the action calculation to seek; it does
not rewrite the durable state chosen earlier in the boundary. The current
TypeScript nearest-target equal-distance result depends on snake-array order,
and large-centroid addition follows that same container order. Rust corrects
both by visiting other snakes in stable internal-ID order. This changes only
ambiguous ties and floating-point accumulation order and prevents storage order
from changing controller behavior.

`engine::baseline_control` version 1 implements the pure formula from one
already-built corrected observation. It validates exact sensor-v3 offsets and
length, finite Float32 inputs, the live baseline slot/snake/RNG mapping, bounded
world size, relevant body heads and scalars, then stages the next lifecycle
timer/wander/action, canonical world-owned strategy, per-slot RNG continuation,
selected bin and draw diagnostics. It neither samples sensors nor exposes an
authority apply method. The full coordinator must evaluate it from the shared
pre-movement observation boundary, retain and commit that observation's
delivery marker exactly once, join every slot result to the complete fixed-step
key, and then publish the combined authority. Dead and due-respawn slots remain
the lifecycle/placement coordinator's responsibility.

The executable current-source artifact
`typescript-baseline-control-fixtures.json`, regenerated by
`scripts/stage5/generate-baseline-control-fixtures.ts`, records small roam and
avoid behavior, medium cutoff and relative-heading encircle behavior, the
two-draw random boost, large crowd bias, action values, timers and exact uniform
continuations. It is formula/RNG evidence, not performance evidence.

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
but do not change the source world or expose a successful result. Controller
leases are copied with checked reusable outer and text storage and are carried
unchanged through every physical substep. Every snake still named by a lease,
including a record of a completed neural takeover, remains fail-closed on death
until the later authority coordinator can atomically remove or replace the
associated controller lifecycle state. A collision involving such a snake is
therefore rejected with an explicit replacement-required result rather than
producing a world that contains a lease targeting a dead snake. The later
coordinator must project the key and full phase configuration from the same
admitted authority, revalidate both against the exact retained
`WorldStepConfig`, combine controller/recurrent and before/after-step state,
and perform the single authoritative swap. This working transaction alone is
not that publication boundary.

## Complete post-control world-step staging

`engine::world_step` version 1 is the first complete non-authoritative join from
an already committed control boundary through all configured collision
substeps. It accepts the unforgeable `PreparedControlCommit`, requires exact
prefix and control settings from that boundary, checks the physics/lifecycle
capacity and timing relationships, enforces a hard substep-count ceiling, and
drives `PhysicsStepWorkspace` itself. The prepared result retains that complete
configuration for later authority revalidation. No caller can inject a
separately staged substep result.

After complete physics, the workspace consumes the keyed baseline-death proof,
starts the full configured respawn delay in the same working step, neutralizes
that baseline slot, and advances the generation-best sensor continuation from
the post-physics world. It retains the post-control brain state, generation
elapsed time, ambient accumulator, controller leases, RNG/allocator
continuations, and packed external observations whose delivery boundary still
awaits matching Node acceptance. Checked lease/RNG copies retain their outer,
per-baseline and string storage across warmed attempts.

Focused integration coverage drives one real prefix, shared corrected sensing,
baseline/external/neural/takeover selection, complete heterogeneous inference,
internal control commit and all three default physics substeps. It also proves
same-step baseline death timing; stable warmed diagnostics; rejection of stale
or inconsistent phase configuration; and fail-closed controlled death without
source-world, RNG, allocator, lifecycle or brain writes.

This result is deliberately still not authority when used by itself. The
nonterminal coordinator described below now projects its configuration from the
admitted state, drives this workspace and performs the one final swap for an
internally controlled step that does not end the generation. External delivery,
controller death/replacement, terminal generation transition, and the reviewed
impossible-respawn outcome remain explicit fail-closed boundaries rather than
TypeScript fallbacks.

## Atomic nonterminal running-step publication primitive

`AuthoritativeState` now owns a unique nonzero process-local world epoch assigned
when a separately validated candidate becomes authoritative, a monotonically
increasing operation epoch and the original admitted memory ceiling. A running
step can begin only from `AuthorityPhase::Running`; beginning a newer attempt
invalidates every older proposal. The resulting key binds world, generation,
source completed-step, population, configuration revision/hash and operation
identity. Publication accepts only the mutable buffers that one nonterminal
step can replace: world, gameplay RNG/allocator continuations, brain runtime
records, baseline lifecycle, ambient credit, generation-best sensor state and
scheduler scalars. Population weights, the graph, normalized configuration and
run/build identity remain owned by the existing authority and cannot enter the
replacement.

Before any swap, publication requires an exact current key, exactly one
admitted fixed-delta increase in generation time, non-regressing generation
best, unchanged brain handles/owners/non-population weight bits, unchanged
evolution and external-controller RNG streams, unchanged non-gameplay allocator
domains and non-regressing gameplay allocators. It then swaps all large mutable
buffers once, recomputes the complete admitted memory estimate and reruns every
mutable state validator while exclusive authority access is held. A validation
or memory-ceiling failure restores the prior scalars and swaps every old buffer
back; the unwind path performs the same restoration before resuming the panic.
Focused tests cover successful all-field publication and reusable old-buffer
return, every stale key component, superseded attempts, pre-swap immutable and
monotonic-contract rejection, post-swap malformed-state rollback, post-swap
memory-ceiling rollback, successful retry and generation-boundary rejection.

This primitive is intentionally lower-level than the authoritative scheduler
and is not a production fixed-step entrance. It does not independently prove
that caller-supplied buffers were calculated with the admitted gameplay
formulas. The nonterminal coordinator below is now the coarse entrance that
privately supplies those buffers for its supported internal-control case.
Reset, New Run and import must still advance world identity when their
replacement paths are implemented. No scheduler, frame, Node, browser, RL,
performance or production-cutover gate is claimed here.

## Strict running-step configuration projection

Current-source inspection at parent commit `006e7bf` maps the authoritative
running-step values from `src/config.ts::CFG_DEFAULT`,
`src/protocol/settings.ts::SETTINGS_PATHS`, the ranges and scalar kinds in
`src/protocol/settingDefinitions.ts`, the sensor formulas in `src/sensors.ts`,
and `src/world.ts::World._advanceFixedStepPhysics`. The TypeScript physics path
clamps `collision.substepMaxDt` to `[0.001, fixedDt]`, calculates
`ceil(fixedDt / maxSubstep)`, caps that count at 64, then divides the one fixed
delta evenly across the selected collision-only substeps.

`engine::step_config` projection version 1 now derives one complete
`WorldStepConfig`, `SensorConfig`, and nonterminal generation guard from the
path-sorted `NormalizedEngineConfig` owned by `AuthoritativeState`. It requires exact
integer, floating, and boolean kinds; applies the current owner-facing setting
ranges; checks the duplicated world radius, population count, baseline count
and simulation-speed projections against their typed authoritative fields; and
rejects missing, unsupported, non-finite, out-of-range or inconsistent values.
The projection includes behavior-changing `CFG` fields not present in the
current browser slider snapshot, specifically `pelletGrid.cellSize`,
`snakeTurnPenalty`, and every `death.*` value. A future construction boundary
must normalize the complete experiment configuration rather than treating the
browser slider list as a complete Rust state contract. The current Rust corpse
color path supports `death.useSnakeColor = true`; a false value rejects clearly
instead of being ignored.

User values populate survival accounting, ambient food, baseline timing and
spawn geometry, neural cadence, controller wall-time rules, shared sensor
configuration and indexes, movement/boost/body formulas, food scoring/growth,
collision settings, death drops, kill credit, derived physics subdivisions,
ordinary generation duration, and the early-end time/alive threshold.
The admitted top-level body, pellet, snake and brain ceilings are copied into
the applicable phase contracts. Every nested phase now exposes crate-private
shape validation, so projection rejects an invalid accounting, ambient,
baseline, control/index, movement, food, collision, effect, physics or joined
world-step configuration before hot work begins.

Spatial, collision-search and collision-safe-spawn work ceilings remain an
explicit `RunningStepWorkLimits` policy rather than gameplay settings. The
current values are labelled provisional. Exhausting them must reject the step;
they do not authorize omitted sensors, segments, candidates, collisions or a
reduced population. P0-P3 complete-step measurements must justify later limit
changes.

Focused tests prove the complete current default projection, TypeScript's
derived three-substep default, changed live values and a changed two-substep
case, strict scalar kinds, missing and out-of-range rejection, unsupported
corpse-color rejection, top-level projection disagreement, and invalid work
limits. The all-feature release library suite passed 334 tests after the join.
The Node/Rust normalized-config builder and live revision replacement remain
open. The nonterminal coordinator below now constructs the matching sensor and
graph pipeline and consumes this projection; no normal server path or
performance result is claimed.

## Private nonterminal authoritative coordinator

Current-source inspection at parent commit
`41f48541e43a6be37bd21a62a5fde92224f76989` rechecked
`World._finishFixedStep`: the current reference ends a generation after the
completed physical step when simulated generation time reaches
`generationSeconds`, or when elapsed time reaches
`observer.earlyEndMinSeconds` and the alive evolved population count is at or
below `observer.earlyEndAliveThreshold`. Baseline, external and resurrected
snakes do not count toward that rule.

`engine::running_step` version 1 is the first coarse Rust entrance that can
publish a complete ordinary nonterminal fixed step. Construction reads the
admitted normalized configuration and compiled graph from one
`AuthoritativeState`, builds the corrected sensor-v3 evaluator and complete
heterogeneous graph pipeline once, and retains every prefix, control, physics,
spatial and lifecycle workspace. One call obtains a fresh process-local step
key, stages accounting/ambient/baseline respawn, samples every selected control
from the same immutable boundary, commits internal control/recurrent state,
executes every configured physics substep, updates post-step baseline and sensor
continuations, checks the reference generation-ending rules, and supplies all
mutable buffers to the reversible authority publication exactly once.

The coordinator accepts only wall-clock controller time and the scheduler's
remaining nonnegative finite debt for each attempt. It rejects regressing wall
time. The unique world incarnation, configuration revision/hash and graph-layout
identity are rechecked before work, while the step key and publication boundary
bind the live world, generation, population, completed step and operation
epoch. This prevents a warmed neural cache or complete staged proposal from one
separately admitted world being relabelled for another world with otherwise
matching config, graph, handles and epochs. Failed staging may change reusable
scratch and advances the attempt epoch, but it cannot replace the authoritative
`StateCandidate`.

This entrance is deliberately restricted to a definitely nonterminal step for
which no external observation awaits a Node send result. An external delivery
request stops before the internal control commit and physics. A terminal
post-physics result stops before publication so Stage 6 can later consume that
exact result while evolving/replacing the generation atomically. Death of a
lease-owned snake, an impossible due-baseline spawn, or any capacity/math/index
failure propagates as a complete rejected attempt. There is no TypeScript
fallback and no partial frame, stats or checkpoint result.

Focused authority tests prove one real default three-substep publication with
world, gameplay RNG, recurrent state, elapsed time and scheduler debt advancing
while immutable run/population identity stays fixed; pending external delivery
and a later regressing clock leave authority unchanged; both duration and
early-alive generation guards withhold publication; and invalid scheduler debt
does not begin an attempt. A warmed coordinator is also rejected by a second
authority that deliberately has the same persisted identity, config, graph and
brain handles but different non-population weight bytes. This is still an
internal Rust vertical slice. Node
delivery acceptance, controller death/replacement, the actual generation
transition/evolution, scheduler pumping, frames, browser/LAN, Protocol 2 RL,
complete-step allocation/performance evidence, Oxygen validation, persistence
integration and production cutover remain open.

Windows validation of this slice passed 339 all-feature release library tests
plus both enabled benchmark-binary tests, 327 no-default-feature release library
tests, strict release all-target Clippy, rustdoc, rustfmt and diff checks. One
existing independent read-only reviewer inspected the stable authority diff
without starting a duplicate build. It found the cross-authority warmed-weight
cache defect described above; the process-unique world incarnation and focused
two-authority regression corrected it, and its final static recheck found no
remaining blocker, P1 or P2. The reviewer changed no files and no review turn
was blocked or wasted.

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
- `scripts/stage5/generate-baseline-control-fixtures.ts` and
  `docs/todo/evidence/stage5/typescript-baseline-control-fixtures.json` retain
  baseline life-stage formulas, transitions, actions and uniform continuation
  as executable non-performance evidence.
- controller, scheduler, world-ordering, snake, spatial-hash, determinism, and
  sensor tests remain the selected TypeScript oracle during migration.
- `docs/todo/evidence/stage2/behavior-source-map.md` indexes the wider current
  behavior corpus and distinguishes preserve from correct.
