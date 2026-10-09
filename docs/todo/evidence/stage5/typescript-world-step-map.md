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
packed event until the nonterminal coordinator receives the matching local
Node result described below. Initial grace
expiry clears the expired external action before applying the zero-state
neural result, while later `NeuralTakeover` boundaries do not neutralize an
already-held neural action. Immutable non-population weights become reusable
within one world/population epoch when each retained record still has the same
handle, owner and shape; recurrent blocks are refreshed and published on every
applicable boundary. The working result is a
physics input, not authority. `engine::world_step` now consumes that boundary
through complete non-authoritative physics and post-physics continuation
staging; the private authority coordinator below owns current-state key/config
projection, the nonterminal generation guard, external delivery resolution and
the single final swap.

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
unchanged through every physical substep. A snake named by a lease may die in
this intermediate transaction; its dead record and lease remain together so
the complete coordinator can resolve them without hiding the collision. The
physics transaction cannot publish that intermediate boundary by itself. The
joined world step requires the graph, wall time, external-controller RNG,
identity allocators and placement contract before it can replace or remove the
lease target. A low-level caller that omits that context receives
`ExternalReplacementContextRequired`. The complete coordinator now supplies
that context and performs the single authoritative swap described below.

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
awaits a matching local Node accepted/failed result. Checked lease/RNG copies retain their outer,
per-baseline and string storage across warmed attempts.

Focused integration coverage drives one real prefix, shared corrected sensing,
baseline/external/neural/takeover selection, complete heterogeneous inference,
internal control commit and all three default physics substeps. It also proves
same-step baseline death timing; stable warmed diagnostics; rejection of stale
or inconsistent phase configuration; and explicit low-level rejection when a
controlled death is staged without the graph and external-replacement context.

This result is deliberately still not authority when used by itself. The
nonterminal coordinator described below now projects its configuration from the
admitted state, drives this workspace, resolves matching local Node-send results
for packed player/RL observations, and performs the one final swap for a step
that does not end the generation. It now also resolves controller death and
replacement before exposing the reliable event batch. Terminal generation
transition and the reviewed impossible-respawn outcome remain explicit
fail-closed boundaries rather than TypeScript fallbacks.

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
evolution RNG, unchanged non-gameplay allocator domains and non-regressing
gameplay allocators. The controlled-death path is the one narrow exception: an
opaque workspace proof binds the exact provisional world, external-controller
RNG, allocator and brain payload plus the exact replacement/removal counts. It
then swaps all large mutable buffers once, recomputes the complete admitted
memory estimate and reruns every
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

This entrance remains restricted to a definitely nonterminal step, but it now
supports the retained external player/RL observation batch. Rust completes the
internal control commit, every physics substep, post-step lifecycle work and the
generation guard before an event becomes visible. It then prevalidates the
canonical event ranges, exact snake/lease/connection identities, score markers,
both accepted-send and failed-send controller outcomes, complete mutable state,
memory ceiling, publication buffers and process-local preflight identity. A
terminal post-physics result therefore still exposes no observation and stops
before publication so Stage 6 can later evolve/replace the generation
atomically. Death of a lease-owned snake during physics, an impossible due-
baseline spawn, or any capacity/math/index failure remains a complete rejected
attempt. There is no TypeScript fallback and no partial frame, stats or
checkpoint result.

The borrowed bridge batch carries one monotonic event sequence plus the full
step, connection and assignment epochs for every packed observation. Node's
local result is first-result-wins. Unknown, stale, replaced and duplicate
results of either polarity are ignored. Partial resolution keeps the complete
working step private and leaves authority unchanged. Once every event resolves,
an accepted result advances only that event's prevalidated
`delivered_observation_points` marker. A failed result leaves that marker
unchanged, so its delta remains accumulated, and applies the exact prevalidated
disconnect at the retained fixed-step wall boundary. The existing configurable
500-ms input hold and 30-second exclusive grace therefore take effect without a
player/brain mixture or immediate retry on the failed socket. Accepted and
failed results may coexist in one batch.

Every recoverable validation/allocation operation occurs before the batch is
exposed. After the first local result changes retained status, the private path
contains only prevalidated marker/disconnect writes and the preflighted
authority swap; the final publication method returns no `Result`. Published
diagnostics report zero pending deliveries. Event-envelope, status,
disconnect-proposal and packed-observation capacities remain retained across
warmed steps.

Focused authority tests prove one real default three-substep publication with
world, gameplay RNG, recurrent state, elapsed time and scheduler debt advancing
while immutable run/population identity stays fixed; stale external results and
a later regressing clock leave authority unchanged; both duration and
early-alive generation guards withhold publication; and invalid scheduler debt
does not begin an attempt. External coverage includes all-accepted, all-failed
and mixed player/RL batches, partial multi-call acceptance, a later negative
duplicate that cannot override prior acceptance, exact marker behavior,
disconnect deadlines, stale connection/lease/operation rejection, pre-exposure
physics/generation/memory failure and 24-step retained capacities. A warmed
coordinator is also rejected by a second authority that deliberately has the
same persisted identity, config, graph and brain handles but different non-
population weight bytes.

This is still an internal Rust vertical slice. The actual Node/N-API drain and
`ws.send` call are not connected, so the tests supply the same small accepted/
failed result that the thin bridge must later return; they do not claim socket,
browser, LAN or trainer integration. Controlled-death replacement is now joined
internally as described below. Serial evolution and the terminal managed-file
checkpoint handoff are also joined in the later generation section, while the
SQLite metadata acknowledgement, successor-world spawn and authority swap
remain open. Node/N-API scheduler-pump wiring, browser/LAN, Protocol 2 RL,
persistence integration and production cutover also remain open. Complete-step
allocation/performance evidence exists only for ordinary nonterminal fixtures;
the rare replacement and terminal paths have not been measured as production
server paths.

Windows validation of the extended slice passed 346 all-feature/all-target
release library tests plus both enabled benchmark-binary tests, 334 no-default-
feature release library tests, strict release all-target Clippy, rustdoc,
rustfmt and diff checks. One existing independent read-only reviewer inspected
the external-delivery authority transition without starting a duplicate build.
It found three P1 defects—negative duplicates overriding acceptance, failed
sends leaving a live lease, and fallible validation after Node acceptance—plus
one P2 stale pending diagnostic. First-result-wins status, prevalidated
disconnect alternatives, the infallible post-result path and post-commit
diagnostics corrected them. Its final recheck found no remaining blocker, P1 or
P2. The reviewer changed no files and no review turn was blocked or wasted.

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

## Random genome initialization

`src/mlp.ts::Genome.random()` walks the compiled graph's parameter ranges in
compiled-node order. Input, Split and Concat consume no draws. MLP parameters
each use `(rng() * 2 - 1) * 0.6`; Dense parameters use the corresponding `0.45`
scale and the currently inert `[-5, 5]` clamp. GRU, LSTM and RRU constructors
then consume all input-weight draws, all recurrent-weight draws and finally
their gate-bias draws in packed runtime order. Input weights use scale `0.35`,
recurrent weights use `0.18`, ordinary biases use `0.10`, and the configured
GRU update, LSTM forget and RRU reset bias is added only to its named gate.
Assignment into `Float32Array` rounds each JavaScript Float64 result to one
packed Float32 immediately.

`engine::genome` version 1 implements that formula against the canonical Rust
compiled graph. It first validates every contiguous node range and all three
reset-only bias settings, fallibly reserves the complete packed output, restores
an immutable source RNG copy, consumes exactly one uniform draw per parameter,
and returns the complete vector with its exact continuation. Uniform draws do
not consume or alter a cached Gaussian spare. Source graph container order is
irrelevant after canonical compilation. A malformed graph range, invalid
setting, invalid RNG state or reservation failure returns no initialized
genome and cannot advance the borrowed authoritative stream.

The executable current-source generator
`scripts/stage5/generate-genome-init-fixture.ts` runs the real
`Genome.random()` path at TypeScript source revision
`258ac69e80df411fa724ad16f1b2cb19e1ae210c`. The retained native input
`native/fixtures/genome-init-reference.json` covers every node type,
non-default recurrent biases, all 129 exact Float32 bit patterns and the exact
xorshift continuation. The fixture is part of the native source-content hash,
and regeneration is compared as parsed JSON rather than trusting copied
numbers. This compatibility fixture deliberately uses a chain whose current
TypeScript locale order and canonical raw-UTF-8 Rust order coincide; legacy
locale-dependent graph layouts still require their separately documented
weight remapping.

The cross-language result deliberately does not claim identical serialized
Gaussian algorithm metadata. The fixture makes no Gaussian draw: it proves the
uniform xorshift continuation and unchanged spare value. TypeScript labels its
legacy V8 transform `box-muller-polar`, while newly admitted Rust state uses the
versioned Rust standard-library transform label; conversion between those
identities remains the existing explicit migration seam.

The strict running-step projection now requires the three bias values with the
same reset-only ranges as the current UI. Stream ownership is not inferred by
the initializer: the joined external-controller replacement transaction now
supplies and publishes only the isolated external-controller stream, while the
later generation/evolution path must supply the evolution stream. Evolution is
not claimed by this standalone primitive.

## Controlled external-death replacement

Current TypeScript calls
`ControllerRegistry::reassignDeadSnakes(() => World::spawnExternalSnake().id)`
after `SimCore::update`. `World::spawnExternalSnake()` creates the genome from
the evolution stream, creates geometry from the world stream, and may reuse a
dead external snake ID. The registry then creates a token and attempts the
assignment send before changing its own indexes. A failed send removes the
lease, but the already-created world snake and consumed world/evolution draws
remain. That current-source order explains the old behavior; its RNG
contamination, identity reuse and non-atomic world/lease/send split are
correction targets, not compatibility requirements.

`engine::external_replacement` version 1 operates on the complete post-physics
working boundary. It visits dead lease targets in stable lease-ID order. A dead
lease with no live connection is removed without consuming a token, RNG draw or
new identity. Each connected player or Protocol 2 RL owner instead receives a
fresh internal external-snake ID, exact frame-v1 ID, brain handle and lease ID;
a complete collision-safe body; current TypeScript-compatible random graph
weights drawn only from the isolated external-controller RNG; zero recurrent
state; and a 24-byte operating-system-entropy token encoded as 32 unpadded
base64url characters. Existing live bodies and earlier replacements are
placement obstacles. Aggregate candidate and geometry ceilings reject the
whole attempt rather than reducing the configured population or publishing a
partial batch.

The dead snake's pre-movement observation is removed. Reliable replacement
assignments are emitted before surviving ordinary sensor observations, with
the exact new position and direction. Authority remains unchanged until every
event has a first matching local result. Acceptance installs the new token and
keeps exclusive external ownership. Failure preserves the old known token,
disconnects at the retained wall boundary, applies the selected 500-ms input
hold and 30-second exclusive grace, and cannot mix the player with a brain.
Stale or duplicate results are ignored. Once all results resolve, the complete
world, external RNG, allocators, brain records, lease state, controller markers,
physics result and scheduler continuation publish together.

Before any event is exposed, an opaque replacement-workspace proof binds the
exact provisional world, RNG, allocator and brain payload and the exact
replacement/removal counts. The final post-result swap additionally requires a
sealed replacement wrapper that only `engine::running_step` can construct;
another engine module cannot combine a retained preflight token with arbitrary
mutable buffers. Authority rechecks the key, buffer/scalar identity, mutation
counts and unchanged admitted memory estimate before the swap, restoring the
old authority on a memory mismatch. Focused tests cover accepted assignment,
failed assignment and exact grace, disconnected-death removal, stale/duplicate
results, entropy/work-limit failure, isolated RNG and allocator continuations,
zero recurrent state, opaque-proof substitution rejection and complete
authority immutability while delivery remains pending.

This is still an internal Rust contract. The N-API/Node socket call, browser
assignment handling, separate trainer assignment handling and real LAN failure
path remain Stage 6A integration work; no socket-delivery or production-latency
claim is made by the Rust-only result tests.

## Scheduler correction

`src/sim/SimCore.ts::update()` correctly derives whole fixed steps from wall
time times requested `simSpeed`, passes exactly `fixedDt` to each world step,
and records dropped debt at the pump cap. The current Node pump can still spend
too long inside one catch-up slice.

`engine::scheduler` version 1 is the first Rust-owned scheduling contract. It
binds itself to one process-local authoritative world incarnation and copies
the admitted config revision/hash, fixed delta, requested `simSpeed`, completed
step and fractional debt from that authority. Wall-clock initialization is a
single-use startup operation: a later forward or backward rebase rejects
without changing debt or diagnostics. Each observed elapsed wall interval adds
`elapsed * simSpeed` requested simulation time; `simSpeed` never changes the
fixed delta supplied to the world.

The provisional default retains enough debt for one complete next fixed step
plus 250 ms of real-wall catch-up backlog. Older excess demand is discarded
with latest and lifetime values reported in both simulated and wall seconds;
the overload indicator stays raised while the retained due backlog drains. A
low-rate configuration whose one fixed step takes longer than that horizon can
still mature one complete step. The horizon is operational policy, not a
physics shortcut, and remains provisional until the approved P0-P3 VM
measurements.

The thin bridge must explicitly call one service boundary after it drains
commands and newest accepted actions. That boundary authorizes at most one
step ticket, even when many steps are overdue; another step requires another
drain/service opportunity. A ticket awaiting player/RL observation delivery
blocks another service or step. A rejected physics or terminal attempt retains
all debt and requires another command-service boundary. A successful step
spends exactly one fixed delta only after the authority has published the
ticket's exact remaining accumulator and a publication matching the current
world, generation, source step, population, config revision/hash, operation
epoch and admitted memory result. Foreign, stale or fabricated publication
fields cannot retire a ticket.

Focused Rust tests cover startup initialization, forward/backward rebase
rejection, 12x debt without delta enlargement, a 1 Hz/0.1x maturation case,
catch-up dropping, regressing time, one service per overdue step, external-
delivery blocking, terminal rejection/debt retention, all seven forged key
components and a forged memory result. Current Windows validation passed 354
all-feature/all-target release library tests plus both enabled benchmark-
binary tests, 342 no-default-feature release library tests, strict release all-
target Clippy, rustdoc, rustfmt and diff checks.

One existing independent reviewer performed the required read-only scheduler/
authority review. It found two P1 defects: reusable clock rebasing and commit
validation that did not bind the complete publication identity. One-shot clock
initialization, full authority-publication validation and the focused
regressions corrected both; the final recheck found no blocker, P1 or P2. The
reviewer changed no files, ran no duplicate build and no review turn was
blocked or wasted.

This does not yet claim that the real Node/N-API pump invokes the boundary,
drains every queue, or yields socket work between tickets. It also does not yet
provide recent-window achieved speed, fixed-step percentiles, browser input
latency or control-message latency. The isolated complete-step Oxygen
checkpoint below does not exercise this scheduler/Node boundary, so those
integration gates remain open.

## Complete nonterminal step performance checkpoint

`engine::step_fixture` version 2 constructs admitted P0-P3 running authorities
with the approved default/large graphs, differently weighted and nonzero-
recurrent population brains, ten baseline bots, five-point bodies and 3,500
pellets. One `RunningStepCoordinator` then advances the real corrected sensing,
controller selection, heterogeneous graph, movement, food, continuous
collision, effects, accounting and reversible authority publication. Coarse
test-hook timers cover non-overlapping phases; a process allocator records
allocation operations around each complete step.

The coordinator now resolves the exact versioned neural math backend in
`RunIdentity` and builds the graph plan with it. It no longer silently runtime-
detects SSE2 while a run claims scalar continuation. Unknown or unavailable
backends fail construction. The evidence runner requires explicit `scalar` or
`sse2`, verifies the coordinator selection and records the exact stable label.
Initial and final recurrent-state proofs use the same logical digest over
brain identities, epochs, lengths and Float32 bytes.

Publication originally rescanned every immutable population weight on every
fixed step through full population validation. P3 profiling attributed about
58.5 ms of one development-machine step to that publication scan. A running
step cannot replace population records or genome weights, and its pre-swap
contract already checks every brain handle, owner, shape and non-population
weight. The hot publication validator now checks every mutable recurrent block
for exact length and finiteness, then retains full post-swap world, RNG,
allocator, continuation and memory admission. Focused regressions reject a
non-finite recurrent block and forbidden immutable weight changes with complete
rollback. The same development P3 publication phase fell to about 0.28 ms; this
is a profiling observation, not retained owner-VM evidence.

The retained owner-VM artifacts and hashes are under `step-v1/`. Exact working
source SHA-256
`b59cf03ff5a67db7fee1908290282621c5e465bd6fd41ac0bdce256ad20d7f97`
was built in an isolated `/tmp` checkout on Debian host `oxygen`; the live app
checkout and saves were untouched. Each valid run used three stateful warm-up
steps and 30 measured steps with every configured evolved snake and baseline
still alive. Longer P1 attempts were rejected after legitimate collisions
reduced the active workload and are not retained.

Oxygen single-worker SSE2 p95 was 9.42/47.73/19.16/89.14 ms for P0/P1/P2/P3,
or 2.07/0.45/0.95/0.21 simulated seconds per wall second. Scalar p95 was
11.22/43.89/30.31/155.71 ms. P0 clears this isolated real-time checkpoint; P1
and P2 do not, and P3 remains capacity-only. Control selection dominates every
miss. The complete path still performs about 920 allocation operations per P0/
P2 step and 1,132 per P1/P3 step. Allocation removal and the approved bounded
calculation-worker path remain mandatory; the result does not authorize weaker
sensors, collision, physics or workloads.

The eight reports are exact source-hash evidence, not yet clean-commit
evidence. Local validation passed 357 all-feature/all-target release library
tests, all enabled benchmark binaries and strict release all-target Clippy. One
independent read-only reviewer found two P1 evidence defects in the first
unretained harness: incomparable initial/final recurrent hashes and false
scalar provenance while execution runtime-selected SSE2. The common digest and
run-identity-bound explicit backend corrected both; final review found no
remaining blocker/P1/P2. The reviewer changed no files and no reviewer turn was
blocked or wasted. One remote build attempt was wasted because the transfer
overlay omitted `inference.rs`; the complete overlay then built and produced
the retained reports.

The follow-up artifacts under `step-v2/` retain the reusable-staging pass.
Test-hook-only counters now attribute non-overlapping allocation operations to
the coarse authority/prefix/control/world/generation/publication phases and to
the nested physics phases. Reusable serialized-RNG copying retains absent
Gaussian-spare text storage, effect/physics staging no longer clones complete
RNG bundles, baseline control reuses its result state, and authoritative
publication reuses validation scratch instead of allocating temporary identity
lists. Production builds do not include the counters.

Independent review found one P1 reuse defect before the follow-up evidence was
retained: after a reset/import-style baseline-count reduction, Gaussian-spare
scratch kept its larger logical length and generated effects rejected the
otherwise valid smaller bundle. The active-prefix correction retains spare
storage without treating it as active state. A three-to-one-baseline regression
executes real death-pellet generation, compares pellets, RNG, allocator and
baseline-death continuation with a fresh workspace, then compares a second
exact continuation. The reviewer also required the allocation evidence to stop
short of classifying uncorrelated counts as capacity growth or a fixed floor;
the runner and retained README now state that limit explicitly. Its final
read-only recheck found no blocker/P1/P2 and changed no files.

Exact working source SHA-256
`4d83339805fe5f5737f7131b0e68faee3fd0a15731bc98cb1c5fa5462f14de50`
was rebuilt in the disposable Oxygen checkout after that correction. The eight
three-warm-up/30-measured-step reports share that source, explicit scalar/SSE2
backend and validated owner-VM identity. The live `/opt` checkout and saves
were untouched. SSE2 mean/p95/p99 was 7.17/7.94/8.67 ms for P0,
32.93/35.51/36.41 ms for P1, 15.12/17.49/17.86 ms for P2 and
84.39/100.12/100.71 ms for P3. Short-run achieved simulated/wall ratios were
2.32/0.51/1.10/0.20. P0 clears this isolated checkpoint; P2 SSE2 clears only
the short average-rate checkpoint, not its later ten-minute/server/generation
gate; P1 still fails and P3 remains capacity-only. Control selection remains
the dominant limit.

Mean allocation-operation counts fell from the first checkpoint's roughly
920-1,132 to 8.37-17.40. Publication recorded zero operations in every measured
pass. These are raw per-phase operation counts. They are not correlated with
per-step buffer growth, so neither intermediate equal capacities nor a zero
sample is presented as proof of a general allocation-free steady state. A
future fixed-versus-growth claim requires correlated samples or a controlled
warmed/no-growth fixture.

Both checkpoints exclude the real scheduler pump, Node/N-API, browser/LAN,
Protocol 2 RL, frames, generation transition/evolution, persistence and a
sustained round. Thirty samples identify current subsystem shares but are not a
final tail-latency or cutover result.

## Serial generation evolution preparation

Current-source behavior was mapped from `World._endGeneration()` and
`tournamentPick()` in `src/world.ts`, `Snake.computeFitness()` in
`src/snake.ts`, and `crossover()`, recurrent-block crossover, and `mutate()` in
`src/mlp.ts`. The TypeScript generation boundary currently performs these
operations in order:

1. map each evolved snake to its durable population slot;
2. calculate the generation maximum points value, logarithmic normalized
   points, the `1e-6` top-points set, and configured fitness terms;
3. sort by descending fitness, relying on stable source order for exact ties;
4. calculate the eight-field generation summary and select the generation's
   Hall-of-Fame genome and snake metadata;
5. clone at least one elite, then fill the remaining dense population with
   size-five tournament selection, configured crossover, graph-node-specific
   mutation, `[-5, 5]` clamping, and zero child fitness; and
6. publish the new population, advance the generation, clear transient state,
   emit the exact generation boundary, and only then draw the new world.

`engine::evolution` version 1 implements only the serial preparation in steps
1-5. It accepts borrowed world, packed population, compiled graph and evolution
RNG state; validates the complete input and output bounds before exposing a
result; and returns one prepared population, source fitness vector, stable
source-slot order, eight-field summary, Hall-of-Fame candidate, next
best-ever value and exact RNG continuation. It never writes the source world,
population or RNG. Every evolved member retains distinct packed Float32
weights, and recurrent crossover selects one parent per hidden unit across all
of that unit's input rows, recurrent rows and gate biases for GRU, LSTM and RRU
layouts. Mutation walks compiled graph parameter order and uses the configured
recurrent or ordinary rate and standard deviation for each node.

The implementation makes the current stable-sort dependency explicit by using
descending fitness followed by durable source slot. Tournament ties retain the
first sampled candidate, matching TypeScript. The current empty-parameter RMS
diagnostic produces `NaN` and therefore places each empty genome in a separate
diagnostic species; this oddity is preserved as observed behavior rather than
silently normalized. TypeScript/Rust uncached Gaussian values may differ by one
Float64 ULP under the already documented RNG compatibility rule. Accordingly,
the retained fixture requires exact uniform state and cached-spare presence,
while comparing final Float32 weights within `1e-6`; it does not claim
cross-runtime bit identity for Gaussian mutation.

The current TypeScript code truncates `fitnessHistory` to 100 entries and adds
Hall-of-Fame data through its existing global helper. Those are reference
behaviors to correct, not destination contracts: the approved Rust path must
preserve every compact eight-field generation summary, keep Hall of Fame
run-scoped, and apply the selected best-50-unique-plus-pinned policy only after
existing data migration is verified. The prepared result deliberately does
not allocate new lineage identities, bind new brain handles, spawn the next
world, mutate authoritative state, publish history/Hall-of-Fame metadata, or
construct a checkpoint. The later generation-boundary transaction must do all
of those atomically in the approved order and must bind the evolution settings
to the authority's admitted configuration.

The fixture generator executes current TypeScript source identified as Git
revision `7925faf7aef33bd3de3e1b6d3c021c4320a8dd68`. It retains a four-member
83-to-MLP-to-GRU-to-Dense population with distinct weights and source stats,
the complete next packed population, evolution RNG continuation, summary,
Hall-of-Fame candidate and best-ever value. Regeneration and Rust comparison
both pass. Reversing the world snake container cannot alter the result, equal
fitness uses stable source-slot order, malformed late input produces no source
write, and focused GRU/LSTM/RRU tests cover every recurrent unit region.

The required independent selection-pressure review was requested only after
the concrete implementation existed, but the reviewer could not begin because
its separate usage limit was exhausted. No replacement reviewer was spawned
while that blocker remained unchanged. This slice therefore remains awaiting
independent review and is not a Stage 6 evolution or generation-transition exit
claim despite its direct validation passing.

## Direct display-frame v1 preparation

Current-source behavior was mapped from `src/serializer.ts`,
`src/protocol/frame.ts`, `src/render.ts`, and the server-mode camera and render
calls in `src/main.ts`. `engine::frame_v1` version 1 packs that existing
Float32 wire layout directly into caller-owned reusable bytes. It writes the
four authoritative header values, every alive snake in current world-array
order, every body point, and every pellet with the current type/color mapping.
It does not clone a `World`, construct JavaScript objects, or cross Node while
walking entities.

The last three frame-v1 header fields remain presentation-only. The packer
echoes an explicit caller-supplied view descriptor or documented neutral
defaults. The current browser always calls `renderWorldStruct` with its own
locally smoothed camera and zoom overrides, confirming that those header
values do not make camera state authoritative in Rust. A later thin routing
integration may echo a per-connection descriptor as the approved plan allows.

The packer performs complete checked sizing and value validation before
changing its output. It rejects non-finite or out-of-Float32 values, malformed
body ranges, unknown pellet kinds, admitted-frame-ceiling violations, and IDs
outside frame v1's exact Float32 integer range. A failed attempt leaves the
previous output bytes untouched; a successful warmed call reuses caller
capacity. Returned small metadata includes the exact packed byte length so the
Node welcome path can later stop serializing a complete world merely to obtain
that value.

`scripts/stage6/generate-frame-v1-fixture.ts` executes the current TypeScript
serializer over a mixed alive/dead world and retains every output Float32 bit.
Rust reproduces the artifact exactly, including view fields and all four pellet
kinds. Focused tests also cover neutral view defaults, warmed reuse, exact-ID
rejection, malformed bodies, late validation failure, and unchanged output on
failure.

This is frame preparation only. It is not yet exposed through the coarse N-API
bridge, routed to WebSockets, double-buffered, consumed in a real browser/LAN
session, or used by `refreshWelcomeState()`. Frame-v1 capacity and exact-ID
limits remain explicit, and optional frame v2 remains post-cutover unless
production measurement promotes the smallest necessary protocol change.

## Durable next-generation boundary preparation

`engine::generation` version 1 joins the source-matched evolution result to a
complete pre-spawn `StateCandidate` while the old running authority remains
unchanged. It requires the exact post-step world, RNG and allocator
continuations; verifies one fixed delta of elapsed generation time; rejects
evolution-RNG contamination and allocator regression; and projects every
selection-pressure setting from the admitted normalized configuration. The
configuration contract now also includes
`baselineBots.seed` and `baselineBots.randomizeSeedPerGen`, because current
`BaselineBotManager.resetForGeneration()` derives new per-slot streams at each
generation boundary.

The prepared population keeps dense slots, moves each new packed Float32
weight buffer once, advances the population epoch, allocates fresh brain and
genome identities, records elite/child lineage, and zeroes every recurrent
state. It advances generation and completed-step identity, carries the exact
post-step world/external RNG and allocator continuations, installs evolution's
RNG continuation, reconstructs baseline streams with the current
`deriveBotSeed` formula, and leaves world snakes, bodies, pellets, controller
leases and scheduling debt empty at the approved durable pre-spawn boundary.
Its separate small handoff contains the completed generation's eight-field
summary and run-scoped Hall-of-Fame candidate. Preparation verifies that the
selected genome is preserved bit-exactly as the slot-zero elite in the staged
next population, and metadata references that slot rather than retaining a
second weights copy.

Focused tests prove that preparation cannot mutate the old source or supplied
post-step continuations, produces a fully admissible exact boundary, matches
retained TypeScript baseline-seed vectors, and rejects stale continuations,
elapsed-time mismatch, ID exhaustion and allocator regression. One test sends
the prepared candidate through the production checkpoint-v3 managed-file
writer and restore reader and compares the decoded boundary bit-exactly. The
test uses a unique operating-system temporary directory and does not touch the
owner database, managed checkpoint directory or save files.

Current-source tracing of `World._spawnAll()`, `Snake` construction,
`BaselineBotManager.resetForGeneration()` and `World._initPellets()` establishes
the post-checkpoint draw order. Evolved snakes are visited in dense population
order and each consumes angle, area-uniform radius and heading from the world
RNG. Baseline slots follow in stable slot order using only their independently
derived streams. The complete initial ambient target is then generated at
generation time zero from the continued world RNG. No external-controller
snake is part of `_spawnAll()`; the server's later
`ControllerRegistry.reassignDeadSnakes()` creates fresh snakes only for live
connected controllers. Disconnected reservations are not automatically
assigned a new snake.

`engine::generation_start` version 1 now stages the complete evolved/baseline
world and time-zero ambient fill from an admitted pre-spawn boundary. Gameplay
values are re-projected from that boundary's normalized configuration; a
caller supplies only explicit non-gameplay work ceilings. Evolved placement is
one stable world-RNG batch, each baseline advances only its own slot stream,
and the ambient fill continues the post-evolved world stream. It reserves
separate monotonic internal, baseline and exact frame-v1 identities, binds
every evolved snake to its dense population brain, installs the pending-first-
neural-action sentinel, initializes every baseline lifecycle slot, leaves
controller leases empty and resets generation sensor state. Every complete
body is checked against the wall and all prior bodies. A valid configuration
whose requested bodies cannot fit returns an explicit placement failure; it
never reduces the population or exposes partially advanced RNG/allocators.
Focused tests cover baseline-stream isolation, full running-state admission,
ID exhaustion, stale source/config rejection, impossible geometry, unchanged
source state and warmed retained storage.

The private running-step coordinator now supplies the opaque postphysics world,
RNG and allocator continuations to this builder. It evaluates the generation
guard before spending old-generation external-controller RNG or identities,
admits the complete current-plus-successor memory charge, invalidates the old
step-publication scratch, and retains one source-keyed pending generation
transition while the current authority remains unchanged. Starting another
fixed step is rejected during that handoff. A production checkpoint-v3 writer
seam can write the immutable managed file from the admitted successor without
changing authority or SQLite, and an exact-key escape hatch can discard a
reviewed failed handoff only when the caller also rejects the held scheduler
ticket. Tests prove that a terminal controlled death reaches this path even
when old-generation replacement-ID allocation would fail, and that scheduler
debt and the due ticket are neither spent nor released implicitly.

The coordinator now retains the first immutable checkpoint descriptor and
accepts a persistence success only when the worker echoes that complete exact
descriptor, including operation, transition, run, generation, completed-step,
logical-root and managed-path identity. A wrong or premature acknowledgement
changes nothing. The exact acknowledgement is recorded before next-world
construction; if construction then fails, the durable boundary remains the
current database meaning and the deterministic construction can be retried
without rerunning evolution or acknowledging SQLite again. A successful
construction is retained and reborrowed without repeating RNG draws. Once the
commit is acknowledged, the transition cannot use the old uncommitted-discard
escape hatch. The focused integration writes only to a unique operating-system
temporary directory and simulates the small commit acknowledgement; no Node
persistence worker or SQLite schema is connected yet.

The following terminal work remains open:

- connect the selected persistence worker so its real `synchronous=FULL`
  metadata/current-pointer, compact-history and run-scoped Hall-of-Fame
  transaction produces the exact acknowledgement already enforced by Rust;
- apply the reviewed controller reassignment/invalidation semantics without
  sending a replacement assignment for a snake immediately cleared by the
  generation boundary;
- publish the successor authority and explicitly complete the scheduler ticket,
  rebasing the wall clock so persistence wait is not charged as simulation
  debt; and
- connect and exercise the handoff through the coarse Node bridge, persistence
  worker, browser/LAN and Protocol 2 trainer.

The independent selection-pressure/authority review gate is still open because
the requested reviewer turn could not start after its separate usage limit was
exhausted. No Stage 6 exit or production-authority claim is made.

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
- `scripts/stage5/generate-genome-init-fixture.ts` and the source-identified
  `native/fixtures/genome-init-reference.json` retain every current random
  brain-initialization formula, exact Float32 bits and uniform continuation as
  executable non-performance evidence.
- `scripts/stage6/generate-evolution-fixture.ts` and the source-identified
  `native/fixtures/evolution-reference.json` retain current fitness, stable
  selection, crossover, mutation, compact summary, Hall-of-Fame candidate and
  evolution-RNG continuation as executable non-performance evidence.
- `scripts/stage6/generate-frame-v1-fixture.ts` and the source-identified
  `native/fixtures/frame-v1-reference.json` retain the current complete
  Float32 display-frame v1 bits, explicit presentation descriptor, alive/dead
  filtering, body order and pellet encodings as executable non-performance
  evidence.
- `scripts/stage5/generate-baseline-control-fixtures.ts` and
  `docs/todo/evidence/stage5/typescript-baseline-control-fixtures.json` retain
  baseline life-stage formulas, transitions, actions and uniform continuation
  as executable non-performance evidence.
- controller, scheduler, world-ordering, snake, spatial-hash, determinism, and
  sensor tests remain the selected TypeScript oracle during migration.
- `docs/todo/evidence/stage2/behavior-source-map.md` indexes the wider current
  behavior corpus and distinguishes preserve from correct.
