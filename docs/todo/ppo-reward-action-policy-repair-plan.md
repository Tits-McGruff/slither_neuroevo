# PyRL / Slither PPO Reward and Action Policy Repair Plan

## Goal

Repair the PPO training stack so that external PPO agents train against an explicit Slither objective rather than the accidental survival-only objective produced by applying the legacy v3 reward parser to v4 Motion observations.

The repair must:

- establish a versioned, server-authoritative reward contract independent of sensor layout;
- preserve complete reward chronology across ordinary observations, death, generation replacement, and reconnect discontinuities;
- replace fixed steering variance with a trainable continuous-action policy;
- preserve synchronous PPO collection and exact frozen-policy evaluation;
- select evaluator policies using the repaired objective;
- separate deterministic implementation correctness from empirical evidence of useful learning.

The core ownership rule remains:

> Rust owns game semantics, authoritative event accounting, lifecycle boundaries, and reward-delta construction. Node routes bounded data. PyRL validates the contract and applies configured reward weights.

The existing four-actor collection barrier, coherent policy-version checks, KL protection, gradient clipping, sensor-contract validation, and exact frozen evaluator should remain. Preserve the intended `1e-4` learning rate and existing PPO hyperparameters unless a focused correctness test identifies a specific defect.

This repair does not promise that correct PPO plumbing will immediately produce strong gameplay. Implementation acceptance and learning qualification are defined separately below.

## 1. Scope and preserved configuration

Keep the initial repaired baseline configuration fixed:

| Setting | Initial repaired baseline |
|---|---:|
| Training actors | 4 |
| Frozen evaluator actors | 1 |
| Nominal horizon per actor | 256 transitions |
| Nominal full update | 1024 transitions |
| PPO epochs | 4 |
| Minibatch size | 256 |
| PPO clip | 0.2 |
| KL target | 0.025 |
| Learning rate | `1e-4` |
| Gamma | 0.99 |
| GAE λ | 0.95 |
| Entropy coefficient | Existing configured value |
| Network width and depth | Existing run configuration |

The 1024-transition batch is nominal. Death, truncation, and disconnect handling may produce partial or empty actor contributions. Every collection epoch must still receive one contribution from each configured training actor before updating.

Record the effective configuration at startup and in checkpoints. Verify the actual optimizer learning rate after startup and resume rather than assuming that a config default or checkpoint contains the intended value.

Do not introduce a second authoritative simulation, browser fallback, Node inference implementation, per-layer native bridge, or per-tick transport acknowledgement barrier.

## 2. Keep contract versions distinct

Keep Protocol 2, sensor layout/profile, binary observation envelope, server reward
contract, trainer reward configuration, policy parameterization, checkpoint schema,
published collection-policy version, and evaluator score definition distinct.

Changing trainer coefficients does not change the raw server contract. Changing a
raw field's meaning, units, or accounting rules requires a new reward version.
A new server using v3 sensors still supplies authoritative reward version 1.
Sensor compatibility must never select the historical reward parser implicitly.

## 3. Define reward contract version 1

Add an explicit reward-contract advertisement to welcome metadata. It must be separate from:

- Protocol 2 identity;
- sensor-layout identity;
- binary observation-envelope version;
- trainer reward-configuration version;
- policy parameterization version.

A new Slither server must provide the same reward contract for v3, v4 Basic, and v4 Motion sensors.

Reward version 1 contains raw authoritative quantities. It does not contain a preweighted PPO scalar.

### 3.1 Transition identity

Each reward-bearing observation or lifecycle closure must identify:

- the current run and world incarnation, using existing identity conventions where possible;
- the external assignment or lease incarnation;
- the physical snake/life associated with the observation;
- the current observation sequence;
- the previous observation sequence defining the interval, or `null` for an initial observation;
- the authoritative interval start and end boundaries;
- reward contract version 1.

Do not use visible snake ID alone as a complete transition identity.

Use bounded counters and the project's existing wire conventions for identities that cannot be represented safely as JavaScript numbers. Do not add a general provenance or transition-history framework.

### 3.2 Raw fields and units

Define the following fields:

| Field | Meaning | Units and constraints |
|---|---|---|
| `elapsedSeconds` | Simulated time between the two reward-accounting boundaries | Finite, nonnegative seconds |
| `aliveSecondsDelta` | Authoritatively credited live time during the interval | Finite, nonnegative seconds |
| `foodValueDelta` | Total value of pellets actually consumed during the interval | Finite, nonnegative food-value units |
| `foodCountDelta` | Number of pellets actually consumed | Nonnegative integer; diagnostic |
| `lengthDelta` | Signed change in finalized physical body length, measured before terminal body cleanup | Signed integer body segments |
| `retainedGrowthDelta` | Signed change in the retained-growth quantity defined below | Signed body segments |
| `pointsDelta` | Signed change in the snake's spendable point balance from ordinary game accounting | Finite signed points |
| `survivalPointsDelta` | Gross points awarded specifically by live-time accounting | Finite, nonnegative points |
| `foodPointsDelta` | Gross points awarded by food consumption | Finite, nonnegative points |
| `killPointsDelta` | Gross points awarded by credited kills | Finite, nonnegative points |
| `otherPointsDelta` | Other ordinary point adjustments not represented by the named categories | Finite signed points; normally zero |
| `boostPointsSpentDelta` | Actual points spent by authoritative boost accounting | Finite, nonnegative points |
| `killsDelta` | Credited kills awarded during the interval | Nonnegative integer |

The accounting identity is:

```text
pointsDelta
  = survivalPointsDelta
  + foodPointsDelta
  + killPointsDelta
  + otherPointsDelta
  - boostPointsSpentDelta
```

Test that identity within an explicit floating-point tolerance.

Initial assignment balances, imported state, reset, and assignment replacement are baseline establishment events. They are not ordinary earnings and must not appear as reward deltas.

`foodValueDelta` includes all food the game credits as consumed, including eligible boost-trail and corpse pellets. Do not silently change food-credit semantics for PPO. Any later decision to distinguish pellet origins is a separate objective revision.

### 3.3 Retained growth

Record the physical body length at the start of each assigned life as `startingLength`.

At each authoritative reward boundary, define:

```text
retainedGrowth = max(finalizedBodyLength - startingLength, 0)

retainedGrowthDelta
  = retainedGrowth_at_end
  - retainedGrowth_at_start
```

This is a signed delta.

Losing previously rewarded growth therefore produces a negative growth contribution. Returning to a previously reached length does not create additional net growth reward over a closed loss-and-regrowth cycle.

That telescoping claim applies to undiscounted growth accounting. It does not
prove that every discounted or complete food-economic cycle is unprofitable.

Use finalized body segments rather than requested target length. Fractional target-length changes receive growth credit when they result in finalized physical growth.

For terminal death, measure and freeze the final body length before corpse generation, body removal, or slot reuse. Death does not manufacture a negative growth delta by deleting the body. The separate death penalty represents the terminal event.

The same rule applies at generation truncation: preserve the final old-life growth measurement before replacement.

Include an unweighted boundary `state` snapshot in every envelope with
`startingLength`, `bodyLength`, and `retainedGrowth`. Starting and current length
are positive exact integers in body segments; retained growth is a nonnegative
exact integer equal to `max(bodyLength - startingLength, 0)`. Starting length is
constant for the physical life. The snapshot uses the same captured observation
or pre-cleanup closure boundary as the deltas. An initial snapshot may contain
pre-baseline growth while its deltas remain zero. PyRL must check continuous
snapshot differences against the signed length and retained-growth deltas, and
report absolute final length directly from the last authoritative snapshot.

### 3.4 Lifecycle fields

Every envelope must include a transition kind:

```text
initial
live
terminal
truncated
```

For a closure, include an explicit reason.

- Actual game death is `terminal`.
- Generation replacement is `truncated`.
- Manual generation completion is `truncated`.
- Reset, import, and world replacement end the old environment without manufacturing a death.
- Connection interruption is a transport discontinuity and does not itself prove death or an authoritative environment cutoff.

Define the supported reason enum in the shared protocol. Do not infer death merely because an assignment changed or a replacement reason is absent.

An initial observation establishes the baseline and carries zero deltas. PyRL must not create a synthetic action transition or pay a spawn reward for it.

## 4. Accumulation and delivery semantics

Rust accumulates authoritative deltas across all committed work between reward-accounting boundaries.

The reward attached to observation `n` describes the interval from its declared predecessor to observation `n`. It must not describe only the latest physics tick.

Preserve these invariants:

- Baseline probes and diagnostic sensor queries do not consume reward accounting.
- Failed or rolled-back simulation work does not advance counters.
- Ordinary observation accounting commits with admission of the complete ordered batch to the bounded Rust output stream.
- Ordinary delivery does not wait for a Python or per-tick Node receipt.
- Observation and reward data use the same identity and boundary.
- Reliable reward-bearing observations are not silently coalesced or dropped.
- Ownership-changing assignments retain the existing receipt-driven transaction semantics.

Document precisely how the shared pre-movement observation boundary relates to live-time accounting and completed physics work. Use the same boundary consistently for elapsed time, point credits, body state, and sensor data.

Physical observation time is `sourceCompletedStep * fixedDt` before movement and
`(sourceCompletedStep + 1) * fixedDt` after the step. Prefix live-time and point
credits are separate accounting facts; they must not move the physical clock
ahead by one tick. A final tail may contain elapsed physical time with no further
live-time credit. Test this explicitly at ordinary-observation-to-closure boundaries.

`elapsedSeconds` and `aliveSecondsDelta` are separate facts. An interval may
contain prefix live-time credits before physical time advances. Neither Rust nor
PyRL may require credited live time to be less than or equal to physical elapsed
time within every individual interval.

“Delivered” means committed to the existing bounded output path. It does not mean that Python has acknowledged processing the message.

Do not promise end-to-end exactly-once delivery across transport loss. Validate
intact sequence chains and explicitly discard unresolved experience after gaps.

## 5. Close the final old-life transition before reassignment

A reward payload on ordinary live observations is insufficient. The protocol must also close the interval after the last ordinary observation when a snake dies or the environment truncates its life.

Before a replacement assignment becomes visible to PyRL, deliver an ordered closure for the old life containing:

- the old-life transition identity;
- all remaining authoritative reward deltas;
- the exact terminal or truncation reason;
- the authoritative cutoff boundary;
- a final old-life observation when bootstrapping is supported and required.

PyRL must process that closure before accepting the next assignment.

### 5.1 Real death

For real death:

- include food, growth, points, and kill credits earned in the final interval;
- add the trainer's discrete death penalty exactly once;
- finish the pending transition as terminal;
- use zero bootstrap value;
- end the GAE segment before the next life.

Do not require a normal live sensor packet from a dead snake to deliver its final reward.

### 5.2 Generation truncation

For generation replacement:

- include accumulated ordinary reward through the cutoff;
- apply no death penalty;
- complete the pending transition;
- bootstrap using the final old-life observation evaluated by the matching collection policy;
- end the GAE segment before the replacement assignment.

Do not use the replacement snake's observation or the pending transition's starting-state value as a substitute for the cutoff-state value.

### 5.3 Other replacement boundaries

Use a reason-specific policy for reset, import, and world replacement.

Where a valid final old-environment observation can be captured, close and bootstrap the old segment explicitly. Where it cannot, retain only previously completed experience and discard the unresolved tail. Do not invent an observation, reward interval, or death.

### 5.4 Simultaneous events

Define and test event precedence:

- food consumption followed by death in the same step preserves the food credit;
- a kill awarded to a simultaneously dying snake remains credited to that snake;
- actual death on the generation-ending step remains terminal;
- an alive snake replaced at that boundary is truncated;
- one life receives at most one closure and one death penalty.

## 6. Preserve Protocol 2 and update binary transport

PyRL currently requests binary observations. The repair must cover both JSON and binary paths.

Introduce a negotiated binary observation-envelope revision that carries the complete reward envelope alongside the observation. Keep binary-envelope versioning separate from reward versioning.

Requirements:

- Existing Protocol 2 clients that do not negotiate the new envelope retain their supported observation format.
- New PyRL training requires reward version 1 and a transport capable of carrying it.
- JSON and binary decoders produce equivalent logical observations and reward data.
- Packet lengths, field types, counts, flags, and numeric values are validated.
- Unsupported envelope versions fail clearly.
- Lifecycle closures remain explicitly ordered relative to observations and assignments.
- Increased message sizes are included in existing queue and resource bounds.

Do not add reward fields only to JSON `SensorsMsg` while leaving the binary training path unchanged.

Prefer extending the existing encoder and decoder over introducing a separate independently ordered reward channel.

## 7. Define the initial trainer reward objective

Use a trainer-side weighted sum over authoritative fields.

Define:

```text
taskNetPointsDelta
  = pointsDelta - survivalPointsDelta
```

Equivalently:

```text
taskNetPointsDelta
  = foodPointsDelta
  + killPointsDelta
  + otherPointsDelta
  - boostPointsSpentDelta
```

This preserves boost expenditure while avoiding a second survival payment through the points term.

The initial reward configuration is:

| Component | Initial coefficient |
|---|---:|
| Survival | `0.001` per credited live second |
| Food | `0.10` per food-value unit consumed |
| Retained growth | `0.05` per signed retained-growth segment |
| Task net points | `0.01` per signed point |
| Credited kills | `1.00` per kill |
| Real death | `-0.50` once per terminal death |

The formula is:

```text
reward_survival = 0.001 * aliveSecondsDelta
reward_food     = 0.10  * foodValueDelta
reward_growth   = 0.05  * retainedGrowthDelta
reward_points   = 0.01  * taskNetPointsDelta
reward_kills    = 1.00  * killsDelta
reward_death    = -0.50 if transitionKind == terminal else 0

reward = reward_survival
       + reward_food
       + reward_growth
       + reward_points
       + reward_kills
       + reward_death
```

These are initial experimental weights. They are informed by the game's broad fitness priorities without copying generation-level fitness numbers into every PPO step.

Food, growth, points, and kills intentionally overlap. The combined payment for an event must be understood and recorded.

For example, in a fixture where one food-value unit awards 20 points and produces one retained-growth segment:

```text
food reward   = 0.10
growth reward = 0.05
points reward = 0.20
task total    = 0.35
```

Under this configuration:

- ten seconds of survival contributes `0.01`;
- an equal-duration snake that consumes that food receives an additional `0.35`;
- spending one point on boost contributes `-0.01` through points;
- losing one previously retained growth segment contributes `-0.05`;
- a closed growth-loss/regrowth cycle contributes zero net growth reward;
- survival points alone contribute zero task-points reward.

The complete reward of a boost-and-reconsume cycle may include food credit because the game itself credits consumed pellets. Do not claim that signed growth alone eliminates every possible economic loop. Measure such behavior during qualification and revise the objective separately if a concrete exploit is demonstrated.

All coefficients must be configurable, validated, logged, and persisted. Freeze them for the first repaired qualification run.

Validate finiteness of weighted components, their scalar sum, pending-decision
accumulations, and episode returns before admitting them.

Do not add reward clipping or normalization that silently changes these semantics. Any later scaling or clipping experiment must be explicit and versioned in trainer reward configuration.

## 8. Remove sensor-derived reward from the current PPO path

Replace calls to `default_reward_components()` in both supported PPO actor paths with a reward-contract parser.

The parser must:

- require reward version 1;
- validate required fields and numeric constraints;
- validate lifecycle values and interval identities;
- accept signed growth and economic deltas;
- reject impossible negative counts or nonfinite values;
- validate the point-accounting identity;
- reject duplicate, out-of-order, mismatched, or discontinuous intervals according to the documented policy.

Missing reward data must never become a zero-filled or survival-only fallback.

Reward compatibility errors must stop training. Introduce a fatal error type that bypasses the actor's ordinary reconnect loop, as sensor-contract incompatibility already does.

The historical v3 helper may remain only as an explicit legacy mode. It requires both a deliberately selected trainer configuration and an explicitly advertised compatible legacy server contract. Sensor version or the presence of familiar labels must not select it automatically.

A new server using v3 sensors must still provide authoritative reward version 1.

## 9. Align reward with pending PPO decisions

The actor must distinguish:

- server observation intervals;
- client decision transitions;
- authoritative control application;
- collection-policy versions.

Several observation intervals may contribute to one pending client decision because action pacing can prevent a new action on every observation.

Accumulate every valid reward interval exactly once. Finalize the pending decision only at the appropriate next decision or lifecycle boundary.

Initial observations establish state without reward attribution. Closing an old life must never transfer its remaining reward to the new life.

Keep episode accounting separate from PPO admission:

- episode metrics include valid authoritative events for the evaluated life;
- PPO diagnostics include the reward actually admitted to training;
- events received while an actor has already contributed to the barrier are not retroactively added to a submitted rollout.

The difference between received and admitted totals must be explainable through explicit collection boundaries or discarded incomplete experience.

Keep component accounting with the pending decision. Adding reward to a pending
decision does not constitute PPO admission: a disconnect may still discard it.
Count admitted components once after the completed rollout is queued successfully.

### 9.1 Bootstrap and GAE masks

Represent bootstrap eligibility separately from continuation into the next stored
transition:

```text
delta = reward + gamma * bootstrapMask * nextValue - value
```

| Boundary | Bootstrap | GAE continuation |
|---|---|---|
| Ordinary transition | Next-state value | Allowed |
| Ordinary rollout cutoff | Matching final-state value | Stop at rollout boundary |
| Real death | Zero | Stop |
| Generation truncation | Final old-life value | Stop |
| Unresolved disconnect tail | Discard tail | Stop |
| Replacement life | New segment | Never continue from old life |

A truncation may bootstrap without propagating advantages into a replacement life.

## 10. Define asynchronous action semantics and test them

The synchronous actor barrier synchronizes policy publication. It does not make the Slither server wait for actions.

Preserve the asynchronous server model and its bounded action routing.

For the supported training adapter, document that the policy's action is an issued steering/boost command. Transport delay, held controls, and command supersession are part of the environment interaction. Do not claim that every next observation was exclusively caused by the latest issued command.

Use existing action correlation where sufficient. Add bounded application metadata to subsequent observations or command results where needed to distinguish:

- the latest applied command;
- its application boundary;
- a command that was superseded without application;
- controls remaining held from an earlier policy epoch.

This metadata must not introduce per-tick acknowledgements that block physics.

Before collecting a new policy epoch, establish a clean accounting baseline and resolve or fence old-epoch commands. Do not admit an unexplained interval spanning old and new policy ownership.

Tests must cover:

- multiple observations between decisions;
- delayed command application;
- superseded commands;
- observations queued before a new command was issued;
- old controls held across an update;
- actor closure while waiting at the collection barrier.

Discard incomplete or unclassifiable experience explicitly and count it. If the adapter cannot establish its declared ordering, the first baseline must fail its collection-integrity check rather than silently claim exact action alignment.

After a reconnect gap, discard the unresolved pending decision, retain completed
experience, and establish a new accounting baseline. Do not assume unacknowledged
rewards will be replayed. A known old-life closure received after a gap may retire
the life without making the unresolved interval trainable. Incomplete evaluator
episodes cannot be promoted.

## 11. Keep discounting fixed and expose duration limits

Keep:

```text
gamma = 0.99
gae_lambda = 0.95
```

for the initial repair.

Time-based survival makes undiscounted survival accounting independent of observation cadence. It does not make fixed-per-transition discounting independent of decision duration.

The initial baseline continues using the existing per-decision GAE formulation. Record:

- simulated seconds per admitted decision;
- authoritative steps per decision;
- effective decisions per simulated second;
- long-duration decision intervals;
- discarded intervals caused by backlog or collection discontinuity.

Run the first learning comparison under a declared, stable pacing configuration. Record `simSpeed`, sensor cadence, action pacing, and the observed decision-duration distribution.

Do not describe gamma as operating at exactly 60 decisions per second unless measurements support that statement.

After the repair is accepted, compare gamma values such as `0.995`, `0.997`, or `0.999` under controlled conditions. A time-aware discount formulation is a separate experiment with an explicitly stated physical discount horizon.

## 12. Add trainable squashed-Gaussian steering

Add a state-independent trainable parameter owned by the policy:

```text
turn_log_std
```

The steering mean remains state dependent.

Use:

```text
effective_log_std = clamp(turn_log_std, -5.0, 1.0)
turn_std = exp(effective_log_std)
turn_latent ~ Normal(turn_mean, turn_std)
executed_turn = tanh(turn_latent)
```

Initialize:

```text
initial_turn_std = 0.50
initial_turn_log_std = log(0.50)
```

Allow the initial value to be configured within the supported numerical range. Freeze the selected initialization for the first repaired baseline.

The bounds are numerical guards. Keep the stored parameter within those bounds after optimizer steps so hard clamping cannot strand it outside the differentiable range. Validate loaded values and log how often the parameter reaches a bound. Do not reset learned variance during publication, evaluation, or resume.

Test the installed backend's clamp gradient at the bounds. If it vanishes, project
the stored parameter strictly inside the supported interval after optimization.

Retain:

- the stored pre-tanh latent action;
- stable tanh Jacobian correction;
- joint steering/boost log probability;
- the existing Bernoulli boost policy.

Sampling and likelihood reevaluation must use the same effective standard deviation and parameter bounds.

Include `turn_log_std` in:

- optimizer parameters;
- train and inference model state;
- published policy snapshots;
- frozen evaluator models;
- normal checkpoints;
- best-policy checkpoints;
- resume validation;
- model compatibility tests.

Do not silently load an old fixed-variance model into the new parameterization with an implicit default.

## 13. Make the entropy objective explicit

For the first repaired baseline, retain Gaussian steering entropy as an explicitly named regularization surrogate:

```text
entropy_turn_gaussian = entropy(Normal(turn_mean, turn_std))
```

Keep the existing entropy coefficient.

This choice minimizes simultaneous changes to the PPO loss while making variance trainable. It does not claim to maximize entropy of the executed bounded action.

Gaussian entropy increases with standard deviation and does not depend on the steering mean. High latent entropy can coexist with heavily saturated executed actions.

Do not automatically replace this term with a squashed-policy entropy estimate during the repair. That is a separate controlled experiment if diagnostics show the surrogate produces undesirable behavior.

Log at least:

- mean absolute pre-tanh steering mean;
- maximum absolute pre-tanh mean;
- deterministic-mean saturation fraction, using `abs(tanh(mean)) > 0.95`;
- learned effective steering standard deviation;
- mean absolute sampled executed turn;
- sampled executed-turn saturation fraction, using `abs(turn) > 0.95`;
- Gaussian steering entropy;
- boost entropy;
- effective `log_std` bound hits.

Optionally log sign disagreement between sampled actions and the deterministic mean.

Do not use one ambiguous `turn_sat` field for both deterministic means and sampled actions.

Increasing standard deviation is required to change the sampled distribution. It is not required to reduce saturation: it may increase switching between saturated left and right actions.

## 14. Preserve synchronous PPO collection

Each collection epoch must use one published policy version.

The learner updates only after every configured training actor contributes. A contribution may be:

- a full horizon;
- a partial rollout closed by a lifecycle boundary;
- an explicitly empty contribution after a discontinuity.

Preserve:

- one contribution per actor per epoch;
- coherent policy versions across the batch;
- matching policy values and log probabilities;
- generation-truncation bootstrapping;
- real-death terminals;
- gradient clipping;
- KL protection;
- exact frozen evaluator isolation.

An actor that has already submitted may continue controlling its snake under the existing behavior, but it must not append new data to the submitted rollout.

When a new epoch opens, establish the next collection baseline explicitly. Do not attach rewards accumulated during the waiting interval to the first new-policy action.

Log nominal and actual batch sizes, partial contributions, empty contributions, and discarded unresolved tails.

An all-empty epoch must release the barrier without an invalid optimizer update.

## 15. Preserve checkpoints and version compatibility

Keep existing checkpoint files available.

Start the repaired run in a new namespace, for example:

```text
checkpoints/ppo-authoritative-reward-v1-trainable-std/
```

Do not automatically resume from the old namespace.

If old files are moved into a legacy directory, stop task-owned writers first and verify that the move preserves the files. Leaving the old directory intact is also acceptable.

Checkpoint metadata must distinguish:

- sensor contract;
- server reward contract;
- trainer reward configuration and coefficients;
- policy parameterization;
- checkpoint schema;
- evaluation score definition.

Reject incompatible resume attempts clearly.

A normal recovery checkpoint must preserve the optimizer and relevant trainer state. An exact best-policy snapshot must preserve the exact evaluated policy, including `turn_log_std`, and remain clearly labelled if resuming it initializes a fresh optimizer.

Fresh repaired training starts from fresh weights and an empty evaluator leaderboard.

“Empty” means no incumbent result. Do not initialize the best score to zero, because valid evaluation scores may be negative.

Historical survival records remain historical diagnostics and do not determine repaired-policy promotion.

## 16. Evaluate the repaired objective immediately

Changing evaluator selection is part of the initial repair.

Replace lifetime-first selection with the repaired reward objective. Lifetime remains a reported metric.

For every frozen evaluation episode, record:

- candidate policy version and exact snapshot identity;
- simulated live time and authoritative steps;
- food value and pellet count;
- final pre-cleanup body length;
- retained growth;
- net points and point-accounting categories;
- credited kills;
- each weighted reward component;
- total undiscounted repaired reward;
- terminal or truncation reason;
- transport and collection integrity status.

For the first repaired evaluator:

- freeze one policy for the entire episode;
- sample from its frozen steering and boost distributions;
- use evaluator randomness separate from training randomness;
- use the same reward coefficients as training;
- apply a maximum evaluation duration of 240 simulated seconds;
- stop earlier on real death or authoritative environment truncation;
- treat the evaluator duration limit as truncation without a death penalty;
- select provisionally by total undiscounted repaired reward.

The duration cap must end at a valid accounting boundary. If a clean cutoff cannot
be obtained, mark the episode incomplete instead of fabricating final reward.

Use the first accepted observation at or after the nominal cap, include its
complete reward interval, and report actual duration and overshoot. Do not split
discrete food, kill, or death events proportionally. A trainer-local evaluation
cutoff must not be advertised as a server death. Start the next capped episode
with a fresh assignment identity and no reclaim token or legacy name match;
document the server's ordinary disconnect-grace effect on the old life.

Keep an evaluation score-definition/configuration version separate from trainer
reward weights. Changes to exposure cap, sampling, or seed protocol invalidate
the comparable evaluator leaderboard without changing model resume compatibility.

Exclude transport-corrupted or incompletely accounted episodes from promotion.

Record premature server truncations separately. Compare learning metrics under matched episode caps and environment settings; do not treat unequal exposure as a fair performance comparison.

A valid first result may be negative and must still be eligible to become the provisional incumbent.

Multi-episode promotion may follow after the first repair. When added:

- freeze one candidate across the complete evaluation group;
- begin with five episodes per candidate;
- compare mean repaired return under a declared scenario/seed set;
- report spread and component metrics;
- keep evaluation randomness separate from training randomness.

Never aggregate episodes from changing candidate policies and label the result as one frozen policy's score.

## 17. Add Slither contract tests

Add focused Rust and server tests covering:

1. Credited live time over a known interval.
2. Survival-point attribution and economic conservation.
3. Food value, food count, and food-point credits exactly once.
4. Finalized growth and signed retained-growth deltas.
5. Growth caps and fractional target-length accumulation.
6. Boost point spending and retained-growth loss.
7. A closed loss/regrowth cycle with zero net growth reward.
8. Kill credit exactly once for the correct life.
9. Food and death in the same step.
10. Kill credit to a simultaneously dying snake.
11. Real terminal closure with the complete final reward tail.
12. Generation truncation with no death event and a final old-life observation.
13. Death on the generation-ending step.
14. Multiple steps between observations.
15. Assignment replacement without identity mixing.
16. Initial balances and spawn state producing no reward.
17. Failed step preparation or output admission leaving markers unchanged.
18. Reconnect sequence handling without duplicate accounting.
19. JSON/binary equivalence.
20. Unsupported or malformed transport and reward versions.
21. Sensor independence across v3, v4 Basic, and v4 Motion.
22. Baseline probes and diagnostic queries leaving delivery accounting unchanged.

Use the same authoritative event fixture for sensor-independence tests.

Retain bounded queue behavior. Do not add unbounded per-snake event histories to implement reward deltas.

## 18. Add PyRL reward and lifecycle tests

Construct observations with v4 Motion metadata and no legacy reward labels.

Verify:

1. Survival scales with credited live seconds.
2. Each authoritative component reaches the intended pending decision.
3. Survival points are excluded from the task-points term.
4. Boost spending remains negative economic progress.
5. Signed retained growth is preserved.
6. Multiple observation intervals accumulate exactly once.
7. A terminal closure includes the final reward tail and one death penalty.
8. A generation closure includes the final reward tail and uses the final old-life value.
9. GAE does not propagate across reassignment.
10. Initial observations create no synthetic transition.
11. Unsupported reward versions stop training.
12. Missing reward data stops training.
13. Invalid numeric fields and accounting inconsistencies are rejected.
14. Duplicate, out-of-order, and mismatched envelopes follow the declared failure policy.
15. Disconnect gaps discard unresolved experience without inventing death.
16. Reclaim cannot attach an old interval to a new pending decision.
17. Received episode totals and admitted PPO totals are separately explainable.
18. Four-actor batches preserve coherent versions and correct admitted reward totals.
19. Partial and empty contributions release the barrier correctly.
20. Reward-contract errors bypass reconnect retry handling.

Delete or demote tests asserting that the legacy v3 reward vector is the universal default.

## 19. Add policy-distribution and checkpoint tests

Verify:

1. `turn_log_std` is a trainable parameter.
2. It appears in the optimizer.
3. PPO likelihood gradients can reach it.
4. Gaussian entropy gradients reach it within the unclamped range.
5. Changing standard deviation changes sampled dispersion.
6. Executed turns remain within `[-1, 1]`.
7. Stored latents reproduce log probabilities under an unchanged policy.
8. Initial unchanged-policy likelihood ratios are approximately one.
9. The tanh Jacobian remains finite for large positive and negative latents.
10. Effective standard deviation remains finite and within bounds.
11. Publication copies the learned value exactly.
12. Normal checkpoints round-trip it.
13. Best-policy snapshots retain the evaluated value exactly.
14. Resume does not reinitialize it.
15. Old fixed-variance checkpoints are rejected by the new parameterization unless an explicit future migration is selected.

Add a saturated-mean regression fixture around mean `+3`.

With small standard deviation, diagnostics must report near-total executed saturation. With larger standard deviation, the sampled distribution and sign-switching statistics must change as expected.

Do not assert that larger variance necessarily lowers saturation.

## 20. Add deterministic end-to-end correctness checks

Create a small seeded Slither scenario that exercises the real native engine, protocol transport, PyRL parser, synchronous collection, PPO update, and checkpoint path.

Arrange fixtures or controlled placements so the test reliably encounters:

- survival;
- food consumption;
- finalized growth;
- boost spending;
- collision/death;
- generation truncation;
- directional sensor information.

The smoke test is an integration correctness test. It must not depend on an untrained policy happening to discover food or improve gameplay within a few updates.

Assert:

- known authoritative events reach the correct transitions;
- expected task components become nonzero;
- point accounting is conserved;
- reward and observation identities remain aligned;
- PPO batches contain one policy version;
- initial unchanged-policy KL is close to zero;
- gradients, losses, and steering variance remain finite;
- at least one exercised training update changes trainable policy state;
- evaluator snapshots save and restore;
- death and truncation retain different bootstrap treatment.

Keep this test short enough for routine CI. Longer training and behavioral studies belong in explicit qualification runs.

## 21. Add diagnostics for contract and policy failures

At startup, log:

- sensor version, profile, and width;
- reward contract version;
- transport-envelope version;
- trainer reward configuration version and coefficients;
- policy parameterization version;
- initial or loaded steering standard deviation;
- effective optimizer learning rate;
- PPO collection mode;
- nominal batch configuration;
- action pacing and sensor cadence.

During training, report:

- received authoritative event totals;
- admitted PPO reward totals by component;
- component rates per simulated live second;
- decision-duration statistics;
- actual batch size and policy version;
- partial and empty actor contributions;
- discarded intervals and reasons;
- action-delay and supersession counters;
- steering diagnostics;
- KL, clipping, gradient, and critic diagnostics.

Add a diagnostic warning when task events remain absent across a substantial exposure window while survival remains nonzero.

Use an initial warning window of at least 10,000 received intervals and at least 60 aggregate simulated live seconds. Report both exposure measures.

Base this warning on raw event counters as well as weighted rewards so a configured zero coefficient does not look like a transport failure.

Do not automatically change coefficients, exploration, or PPO settings in response.

Also warn when steering variance repeatedly reaches a numerical bound or when sampled saturation remains high over a sustained window. These are investigation signals, not automatic proof of policy failure.

## 22. Documentation

Update both repositories.

Slither documentation must define:

- reward versioning;
- field units and signedness;
- food value versus pellet count;
- point-accounting conservation;
- retained-growth semantics;
- observation and reward boundaries;
- initial observations;
- final lifecycle closures;
- death/truncation precedence;
- delivery admission;
- reconnect gaps;
- JSON/binary negotiation;
- asynchronous action application.

PyRL documentation must state:

- current PPO reward is independent of sensor labels;
- Rust owns authoritative game events;
- coefficients are trainer-side weights;
- survival is time based;
- growth and task economics can be negative;
- trainable steering uses a squashed Gaussian;
- Gaussian entropy is an explicit surrogate;
- synchronous collection does not imply synchronous environment stepping;
- incomplete transport experience is discarded explicitly;
- evaluator selection uses the repaired objective;
- historical fixed-variance, broken-objective checkpoints are separate.

Remove claims that `default_reward_components()` is the general reward implementation.

Keep documentation proportional to the contract. Do not create source snapshots, reconstruction archives, manual test registries, or permanent experiment-management machinery.

## 23. Implementation sequence

Implement coherent changes in this order:

1. Finalize reward field definitions, lifecycle precedence, and the initial coefficient table.
2. Add Rust-owned reward counters and interval construction.
3. Add final old-life closures and cutoff observations.
4. Extend shared protocol types and negotiated binary transport.
5. Add focused Slither accounting, delivery, lifecycle, and transport tests.
6. Add PyRL reward parsing and fatal compatibility errors.
7. Replace legacy reward calls in supported PPO actor paths.
8. Repair pending-transition completion and truncation bootstrapping.
9. Add asynchronous action-ordering and reconnect-discontinuity tests.
10. Add trainable `turn_log_std`, numerical bounds, and explicit entropy naming.
11. Update publication, checkpoints, resume validation, and exact evaluator snapshots.
12. Add policy-distribution tests.
13. Replace lifetime-first evaluator promotion.
14. Add diagnostics and documentation.
15. Run the complete relevant routine suites in both repositories and cross-repository interoperability checks.
16. Preserve existing checkpoints and create the repaired namespace.
17. Run the deterministic end-to-end smoke test.
18. Start fresh repaired training.
19. Perform the learning qualification below before changing additional hyperparameters.

Investigate routine failures that exercise changed behavior. Keep unrelated infrastructure failures and expensive historical qualification separate from product correctness.

## 24. Implementation acceptance criteria

The implementation repair is complete when all of the following hold:

- Reward version 1 is advertised and validated.
- v3, v4 Basic, and v4 Motion receive equivalent reward for equivalent authoritative events.
- The actual binary training path carries complete reward data.
- Food, growth, economics, and kills reach PPO without legacy sensor labels.
- Missing or incompatible contracts stop training clearly.
- Point accounting satisfies its conservation identity.
- Retained-growth accounting does not pay repeated net reward for a closed loss/regrowth cycle.
- Spawn balances and replacement state create no reward.
- Final ordinary reward is preserved at death and generation truncation.
- Death receives one penalty and zero bootstrap.
- Generation truncation receives no death penalty and uses the final old-life observation.
- Reassignment, reconnect, and world replacement cannot mix identities or manufacture death.
- Incomplete or unclassifiable experience is explicitly discarded and counted.
- PPO batches preserve one coherent policy version.
- All configured actors contribute before an update.
- Trainable steering variance receives gradients and survives publication, evaluation, save, and resume.
- Latent-action likelihoods and tanh corrections pass numerical tests.
- Evaluator promotion uses repaired return and accepts an initial negative score.
- The deterministic integration smoke test passes.
- Full relevant routine PyRL checks pass.
- Full relevant routine Slither checks pass.
- Cross-repository interoperability checks pass.

Useful navigation, reduced circling, and improved food seeking are not deterministic implementation acceptance requirements. They belong to learning qualification.

## 25. Learning qualification protocol

Run qualification only after implementation acceptance passes.

### 25.1 Fixed first experiment

Use:

- fresh policy weights;
- the repaired checkpoint namespace;
- v4 Motion;
- the coefficient table in this plan;
- the preserved PPO configuration;
- the declared `0.50` initial steering standard deviation;
- one recorded environment configuration;
- a declared pacing and `simSpeed` configuration.

Identify tested code by clean Git commits in both repositories. Keep generated traces, models, and reports in ignored local output or external artifacts.

### 25.2 Budget and replication

Begin with three declared training seeds.

Use an initial budget of one million admitted PPO decision transitions per seed. Also report aggregate simulated live time and actual update counts.

Evaluate the initial policy and retained candidate snapshots on a common set of ten evaluation scenarios/seeds, using the 240-second cap and the same repaired objective.

These offline comparisons may run sequentially through the separate frozen evaluator. They do not require more training actors or a larger live evaluator pool.

A budget exhaustion without improvement is an inconclusive or negative learning result. It does not retroactively make correct event transport an implementation failure.

### 25.3 Reference policies

Compare against simple diagnostic controls:

- a persistent full-lock turning policy;
- a straight-ahead policy;
- the untrained repaired policy.

Declare steering and boost behavior for each reference before comparing results.

Use these only for evaluation. Do not train by imitation or add hand-coded food steering.

The comparisons should reveal whether the repaired score distinguishes useful task progress from survival with little task activity.

### 25.4 Reported outcomes

For each seed and candidate, report:

- mean and median repaired return;
- return spread;
- paired return differences against the untrained policy on common scenarios;
- food value per episode and per live second;
- retained growth;
- net task points;
- boost spending;
- kills;
- lifetime;
- death and truncation counts;
- steering variance and executed-action saturation;
- decision cadence;
- excluded or incomplete evaluations.

Include representative behavior recordings or concise observations where practical.

### 25.5 Learning qualification criteria

The first learning qualification is successful when:

- authoritative task rewards occur during ordinary training, beyond scripted correctness fixtures;
- at least two of three training seeds produce a candidate with improved mean repaired return over its untrained policy on the common evaluation set;
- the same successful candidates improve food consumption, retained growth, or both;
- return improvement is not explained solely by greater exposure time or survival reward;
- candidates outperform the persistent full-lock reference on repaired return and task-progress metrics under matched evaluation conditions;
- behavior inspection shows observation-dependent navigation and repeated task engagement rather than persistent full-lock circling;
- numerical and collection-integrity diagnostics remain healthy.

Do not require a fixed saturation threshold to pass. Interpret saturation with behavior, sign switching, task progress, and evaluation results.

Do not require kill learning in the first qualification. Kills may be too infrequent for this budget, while food and growth provide earlier evidence.

These criteria support a bounded learning claim for the tested configuration. They do not establish general competitive strength or eliminate all reward exploits.

## 26. If learning qualification fails

Keep the accepted implementation and record the observed failure accurately.

Investigate in this order:

1. Contract integrity and raw event availability.
2. Reward attribution and lifecycle tails.
3. Decision cadence, command delay, and discarded experience.
4. Component scales and whether boost/growth economics behave as intended.
5. Steering mean, variance, saturation, and entropy-surrogate behavior.
6. Critic fit, KL protection, and optimizer behavior.
7. Evaluation comparability and noise.
8. Longer-horizon credit assignment.

Only then introduce controlled experiments.

Change one justified factor at a time, retain an appropriate baseline, and identify the tested code and configuration. Do not make rejected experimental branches permanent CI obligations.

## 27. Explicitly deferred changes

Do not add the following during the initial repair unless a focused test exposes a separate concrete requirement:

- spin penalties;
- steering-magnitude penalties;
- forward-motion or distance rewards;
- map exploration rewards;
- novelty bonuses;
- punishment for repeated actions;
- continuous food-proximity rewards;
- hand-coded food steering;
- baseline imitation;
- network width or depth changes;
- recurrent PPO;
- gamma changes;
- GAE λ changes;
- time-aware discounting;
- PPO clip changes;
- more epochs;
- different horizon or minibatch size;
- different actor count;
- another learning-rate change;
- more aggressive KL limits;
- squashed-policy entropy replacing the declared Gaussian surrogate;
- pellet-origin-specific reward rules.

Potential-based navigation shaping remains a possible later experiment. If introduced, define the potential, discount treatment, terminal treatment, and objective version explicitly. Keep it disabled for the first repaired baseline.

The first repair establishes trustworthy game-event reward, complete transition accounting, a trainable steering distribution, and an evaluator that measures the declared objective. Learning qualification then determines what that implementation actually learns.
