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
keeping this policy revision distinct from earlier captures. Trajectory witnesses
include every completed world, RNG,
allocator, brain and baseline continuation. Their Debug value encoding is only a
within-build comparison, not a persistence format or cross-platform replay claim.
Neither short synthetic batches nor first-action checks establish universal
policy superiority or select production defaults. Representative held-out/student
agreement and the remaining tactical limitations stay open.

The crossing regression checks 180 actual steps on both mirrored sides in both
Motion modes. Urgency names relative head contact, which can occupy a different
sector from the current straight route. An imminent front contact therefore
vetoes continuing into the front half of candidate headings and biases safe
escape away from lateral incoming pressure. Rear pressure still permits a safe
forward boost escape. Basic has no urgency channels and preserves its limited
information behavior; frozen v1 is unchanged.

## Retained version-1 Windows batch

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

## Retained version-1 Oxygen batch

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

## Avoidance correction and version-2 reports

The decision traces exposed premature steering relaxation while the current
forward path still approached a wall/body. Avoidance now uses stronger bounded
proportional steering, keeps an escape side while a safe route remains there,
and suppresses exploration until the manoeuvre finishes. If that side becomes
fully blocked, the controller can select the other side. Existing safe commitment
and score-improvement rules remain. Odd bin counts check both sectors bracketing
straight ahead. These changes use declared observations and existing policy state;
they add no world scans, probes, RNG owners or serialized state fields.

All 256 wall/food-trap regression trajectories pass across 16 seeds, both v2
profiles/modes and both 60 Hz/0.06-second cadences. All 740 native release tests
pass. The version-2 reports retain at most 32 actual delivered decisions for each
failed v2 trial: pre-movement position/heading, issued action, heading commitment
and encoded forward clearances. Held steps do not fabricate samples, and surviving
or frozen-v1 trials omit the tail. This is diagnostic evidence, not policy input.

Each Windows/Oxygen release cadence has 1,120 trials at one and three workers:
8,960 trials across the two platforms and cadences. Complete trial records agree
between worker counts within each platform; every frozen-v1 record also matches
the preceding platform cohort. Both platforms produce the survival counts below. Shared v4 budgets hold and v2 policy world reads are zero.

| Scenario | Policy | Prior 60 Hz | Corrected 60 Hz | Corrected 0.06 s |
|---|---|---:|---:|---:|
| Wall escape | Basic/Motion deterministic | 0/16 | 16/16 | 16/16 |
| Wall escape | Basic/Motion stochastic | 7/16 | 16/16 | 16/16 |
| Food-body trap | All four v2 variants | 0/16 | 16/16 | 16/16 |
| Predicted crossing | Motion deterministic | 16/16 | 16/16 | 16/16 |
| Predicted crossing | Motion stochastic | 14/16 | 16/16 | 16/16 |
| Crowd opening | Both Motion modes | 9/16 | 12/16 | 14/16 |

Close head-on, crowded and intentionally one-unit ray-cap cases still expose
limits. These reports predate Basic's geometric boost escape correction below;
representative student agreement remains open. They do not freeze the teacher or
production sensor defaults.

Version-2 report files are retained as `.json.gz` to keep complete decision tails
compact. Decompression reproduces the original CLI JSON byte stream; the older
version-1 files remain unchanged. CI uploads its ordinary JSON report. For example:

```powershell
node -e "const fs=require('node:fs'),z=require('node:zlib');const r=JSON.parse(z.gunzipSync(fs.readFileSync(process.argv[1])));console.log(r.version,r.trials.length)" docs/performance/baseline-v2/windows-b16-60hz-w1-v2.json.gz
```


Oxygen's load average changed from 3.83/3.46/3.15 to 3.45/3.61/3.33 during this
matrix, and scratch free space stayed above 6.8 GiB. Reports retain target/source
provenance and make no timing claim. All four JSON transfers were SHA-256 verified
before lossless compression and removal of the exact task-owned remote workspace.

## Economic escape correction

Basic v2 can request boost away from a close body/head using only its declared
clearance channels. The chosen heading must point at least 90 degrees away from
the threatening sector; wall pressure alone supplies no boost reason. It still
has no target size, velocity or TTC data. Both profiles now require half a second
of boost-speed runway along the current forward path and the chosen ray, because
turning toward a clear target does not immediately clear the current route.
Existing eligibility, point-budget and incomplete-geometry guards remain.

All 744 native release library tests pass. Pure policy cases cover B8..32, both
modes, each declared body/head channel, the transmitted half-second wallet
boundary, lateral escape direction, wall-only pressure, saturation and both
forward neighbours in odd layouts. Actual native regressions cover 128 boosted
rear-pursuer trajectories across 16 seeds, four v2 variants and two cadences;
every subject spends positive boost points and survives 180 complete steps.
The 256 wall/food-trap trajectories and Motion crossing escapes also still pass.

Four Windows `*-boost-escape-v2.json.gz` reports retain 4,480 complete trials:
1,120 per cadence/worker pair at 60 Hz and 0.06 seconds, one/three workers.
Complete trial records and continuation witnesses agree across worker counts;
every frozen-v1 record matches its preceding cohort. Shared budgets hold,
v2 policy world reads remain zero and deterministic strategy draws remain zero.
Survival has no regression in any scenario/profile/mode cohort. Motion crowd
opening improves from 14/16 to 16/16 at 0.06 seconds in both modes; it stays 12/16
at 60 Hz. All other cohort survival counts are unchanged.

Basic rear escape spends positive points in all 16 seeds per mode/cadence,
compared with zero previously, while preserving 16/16 survival. Basic also
survived this fixture before the correction: these results establish the actual
boost path, not a survival benefit. The full JSON streams are losslessly
compressed and roundtrip-verified. Existing Windows/Oxygen reports are untouched;
earlier Oxygen results remain scoped to their recorded source, and CI supplies
fresh Linux confirmation for this ordinary policy correction. This is not a
timing, throughput, ablation or representative student-quality gate. Production sensor/baseline defaults and
512 MiB worker/checkpoint scratch limits remain unchanged.

Rebuilt production and isolated test-hook addons pass 16 actual-addon tests for
source identity, retired-export boundaries, native startup/runtime and committed
teacher capture with four shadows across one/three workers. Default Clippy,
all-target test-hook Clippy and rustfmt pass. The preceding full JavaScript,
type/lint/client-build evidence is reused because no TypeScript, wire or UI
code changes in this slice; CI provides the full cross-platform matrix.
