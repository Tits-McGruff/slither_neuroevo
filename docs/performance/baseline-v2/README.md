# Native baseline-v2 tactical batches

Build and run from the repository root:

```powershell
cargo run --manifest-path native/Cargo.toml --release --features engine-test-hooks --bin baseline-v2-qualification -- --seed-count 16 --steps 180 --workers 3 --cadence "0.016666666666666666" --output tactical.json
```

The report must be new. Bounds are 1..64 seeds, 1..600 complete steps per trial,
and one through the detected CPU count for workers. Cadence is explicit within
0.008..0.06 seconds; frozen v1 remains fixed-step. CI runs four seeds on Windows
and Ubuntu and retains each job's actual report.

Fourteen fixtures cover wandering, approaching/receding heads, crossing,
head-on approach, corridor, crowd opening, wall escape, food behind a body,
smaller-target interception, larger threats, boosted rear escape, predicted
crossing and saturated geometry. Seeded offsets and mirrored sides vary initial
geometry. Tests verify paired initial worlds agree and no head starts in a
foreign capsule. Opponents use actual native Dense inference with constant zero
turn; the rear pursuer additionally requests boost. Each fixture compares frozen
v1 with Basic/Motion in deterministic/stochastic modes.

Every step uses the existing native prefix, shared indexed control, internal
control commit and all three movement/collision/food/effect substeps. The offline
harness copies a result only after that complete preparation succeeds and stops
at the subject's first death. This is a tactical experiment, not a scheduler,
Node/browser, persistence, respawn, evolution or throughput qualification.

Outcomes include source-alive prefix time, actual food/kill credits, boost spend,
collision cause/contact flags, delivered/incomplete observations, policy world
reads and behavior draws. Smaller-target interception requires the sole target
to die and the subject to receive the actual native kill award. First-action
turn difference and boost agreement use the same initial geometry as v1; later
trajectories can diverge. These are teacher comparisons, not student agreement.
A single-subject test hook aggregates accepted physics deltas only; its result
is unavailable after a failed preparation. Production builds contain neither
the extra fields nor the work.

Reports bind exact source, build, target, math backend and the three canonical
sensor digests. Teacher dataset identity also binds the native source hash,
keeping this policy revision distinct from earlier captures. Trajectory witnesses include every completed world, RNG,
allocator, brain and baseline continuation. Their Debug value encoding is only a
within-build comparison, not a persistence format or cross-platform replay claim.
Neither short synthetic batches nor first-action checks establish universal
policy superiority or select production defaults. Wall/food-trap weaknesses and
representative held-out/student agreement remain evaluation gates.

The crossing regression checks 180 actual steps on both mirrored sides in both
Motion modes. Urgency names relative head contact, which can occupy a different
sector from the current straight route. An imminent front contact therefore
vetoes continuing into the front half of candidate headings and biases safe
escape away from lateral incoming pressure. Rear pressure still permits a safe
forward boost escape. Basic has no urgency channels and preserves its limited
information behavior; frozen v1 is unchanged.

## Retained Windows batch

The Windows release reports use seeds 1..16, B16, 180 steps (three simulated
seconds), explicit 60 Hz v2 cadence and one/three native workers. All 1,120
trial records and complete trajectory witnesses match between worker counts.
Every v4 shared budget holds, v2 policy world reads are zero and deterministic
v2 consumes no behavior draws. Stochastic open-world trials exercise actual
behavior RNG. Survival counts below are evidence for these fixed synthetic
situations, not universal policy rankings.

| Scenario | V1 | Basic deterministic | Basic stochastic | Motion deterministic | Motion stochastic |
|---|---:|---:|---:|---:|---:|
| open-wander | 16/16 | 16/16 | 16/16 | 16/16 | 16/16 |
| approaching | 0/16 | 0/16 | 0/16 | 16/16 | 16/16 |
| receding | 5/16 | 16/16 | 16/16 | 16/16 | 16/16 |
| crossing | 0/16 | 0/16 | 0/16 | 16/16 | 16/16 |
| head-on | 0/16 | 0/16 | 0/16 | 0/16 | 0/16 |
| corridor | 16/16 | 16/16 | 16/16 | 16/16 | 16/16 |
| crowd-opening | 16/16 | 16/16 | 16/16 | 9/16 | 9/16 |
| wall-escape | 0/16 | 0/16 | 7/16 | 0/16 | 7/16 |
| food-body-trap | 0/16 | 0/16 | 0/16 | 0/16 | 0/16 |
| smaller-intercept | 16/16 | 16/16 | 16/16 | 16/16 | 16/16 |
| larger-threat | 16/16 | 16/16 | 16/16 | 16/16 | 16/16 |
| boost-escape | 16/16 | 16/16 | 16/16 | 16/16 | 16/16 |
| predicted-crossing | 0/16 | 0/16 | 0/16 | 16/16 | 14/16 |
| saturation | 14/16 | 5/16 | 6/16 | 5/16 | 6/16 |

Motion deterministic survives every crossing and predicted-crossing trial;
Motion stochastic survives all crossing and 14/16 predicted-crossing trials.
The close head-on, crowd-opening, wall escape, food-trap and remaining stochastic
predicted-crossing outcomes expose teacher limitations. This matrix is not an
acceptance pass for baseline v2. Interception, food, kill, boost, contact and
first-action comparison details remain in each JSON record. No product defaults
or admission ceilings change.

## Retained Oxygen batch

The x86_64 Debian release reports use the same source digest, seeds, B16, exact
60 Hz cadence, 180-step bound and one/three-worker choices. All 1,120 trial
records and witnesses match between worker counts on Oxygen. Host load average
changed from 2.90/3.36/3.23 to 2.55/3.03/3.12; scratch free space remained above
6.9 GiB. No unrelated processes were stopped. This is functional tactical
qualification under load, with no timing verdict.

Oxygen produced the same survival counts shown in the Windows table above.
Full trajectory witnesses are compared only within each platform.

Shared v4 budgets hold, v2 action selection performs zero world reads and
deterministic v2 consumes no strategy draws. Platform-dependent trajectories
remain separate experiments; matching seeds do not imply cross-platform exact
replay. Both report transfers were SHA-256 verified before task-workspace cleanup.
