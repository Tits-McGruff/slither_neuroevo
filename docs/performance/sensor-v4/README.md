# Actual native sensor qualification

Build the bounded workload runner with:

```powershell
cargo build --manifest-path native/Cargo.toml --release --features engine-test-hooks --bin sensor-v4-qualification
```

Run one candidate into a new scratch directory and a new report outside it:

```powershell
native/target/release/sensor-v4-qualification --scenario P1 --profile motion --bins 16 --graph modest --respawn on --seed 42 --workers 4 --math-backend sse2 --minimum 1 --cap 256 --batch 65536 --warmup 5 --steps 300 --environment development --workspace scratch-sensor-p1 --output sensor-p1.json
```

The runner requires two GiB free on the scratch filesystem, preserving one GiB
beyond conservative checkpoint construction. It exclusively creates and removes
its scratch tree; reports use create-new semantics and remain after cleanup.
Version-3 reports optionally qualify the ordinary native teacher capture. Add
`--capture on` with v4 and v2; capture starts after warmup and selects all ten
baseline slots. Pending and queue budgets remain 16 MiB each. Explicit pressure
options are `--capture-drain-steps` (1..60,000, default 1) and
`--capture-queue-chunks` (1..65,536, default 1,024). Pressure flags require capture.
One drain transfers at most 64 chunks/one MiB; stopping seals and drains final
partial chunks. `--capture off` remains the default.

The capture report compares streaming per-slot witnesses from exact committed
observations/actions/outcomes against decoded complete chunks, including Float32
bits, IDs, steps/ordinals, strategy, life stage and strata. It validates descriptor
identity/digests, bounds and chronology. Reported counts reconcile observed rows
with drained plus dropped samples and assigned chunks with drained plus dropped
chunks. Zero samples or any drop yields no exact-sample pass. Per-teacher counts,
a due-count histogram and longest ten-due-teacher streak expose deaths/cadence;
they never substitute invented rows for a missing 600-sample/second workload.

Step service timing includes native encoding/queue publication. Observer hashing,
bounded drain/decoding and final partial-chunk publication have separate timing;
encoded bytes per native service second include final-stop time. The report
includes queue peaks, drops, rates per simulated second and process peak RSS.
This is native capture evidence, excluding Node spool/HTTP, training, shadows,
disk rotation and actual wall-debt scheduling. It does not complete the integrated
soak or sustained ten-living-teacher throughput gate. CI retains short normal and
one-chunk slow-drain reports on Windows/Linux.

Choose v3/basic/motion, P0 (55 evolved plus ten baselines) or P1 (300 plus ten),
modest/large graphs, respawn off/on, and scalar/sse2. Baselines default to frozen
v1. Select `--baseline v2-deterministic` or `--baseline v2-stochastic` with an
explicit `--baseline-cadence` (0.008 through 0.06 seconds) to measure v2. V2
requires v4; v1 rejects a cadence override. V4 requires explicit
minimum, cap and batch; v3 rejects ray limits. No values select production defaults.
Warmup plus measured steps is bounded at 60,000. The oxygen-debian environment
label identifies target runs; the report still records actual build and hardware.

Each measured service publishes exactly one authoritative complete step and packs
frame-v1. Analysis borrows the already-consumed observations and matching pre-control
world after publication. It never resamples, changes delivery markers or consumes
RNG. Observer analysis has a separate cost distribution. Tests compare observer
on/off and one/three-worker complete continuation witnesses, including world,
lives, RNG, allocators, weights and recurrent state.

Reports contain provenance, admission/reservation, exact contract/digest, due
counts, ray work and incomplete rates, complete-step percentiles, and fixed
32-bucket raw/normalized histograms with extrema and overflow. Normalized columns
are actual controller inputs. Clearance metres invert Float32 channel minima;
they are censored by range and conservative when incomplete. Pellet values are a
systematic maximum-64 subset per boundary. Linux reports total RAM; Windows and
Linux report the operating-system lifetime peak RSS, including startup/checkpoint
construction. Unavailable platform probes remain null. Version 2 adds fixed
`consumerWork` groups in neural/baseline/external order, v4-only incomplete
denominators, policy world-read/draw counts and a per-step shared batch-budget
witness. Legacy histogram/`observations` fields remain neural-only.

Version 4 adds fixed disjoint totals from the existing production phase timers,
for all measured steps and the same >16,667-microsecond slow-step subset used by
health. Index, sensing, inference, remaining control, physics and remaining
service partition integer service time; nested timers are bounded before adding.
These are aggregate costs, not phase percentiles. Collecting them adds no new
clock calls inside sensing or geometry and retains no per-step phase history.

These runs use seeded random populations, explicitly recorded baseline mode and
cadence, an immutable run-start checkpoint with benchmark acknowledgement, and controlled clock input.
They exclude Node/browser/network, SQLite, startup, checkpoint timing and wall-debt
behavior under load. Short random runs do not qualify trained-policy distributions,
external-controller workloads, tactical correctness, ablation or production
limits. Native p95 <= 16.667 ms and ordinary incomplete observations < 1% are
probe checks. No observations produces no quality pass. The fixture uses
a 604,800-second duration with duration-only completion, so bounded runs do not
enter generation persistence. It does not qualify generation-age distributions.

## Retained Windows exploration

The leaf-eight reports use Motion/B16, the modest 25,042-parameter graph,
unlimited respawn, seed 42, four SSE2 workers, ray minimum 1/cap 256/batch 65,536,
five warmup steps and 300 measured steps. Both sustain the full due population.

| Scenario | Observations | Incomplete observations | Complete-step p95 |
|---|---:|---:|---:|
| P0 | 16,500 | 0% | 4.8436 ms |
| P1 | 90,000 | 3.02% | 16.8993 ms |

P1 fails both probe checks on this development machine. This actual workload
supersedes the earlier synthetic zero-incomplete result for this configuration.
The JSON files retain all histograms and source/build identity. They do not freeze
normalization revisions or ray limits.

Four-segment leaves pass the exhaustive oracle and exhaustion tests. They
reduce rejected candidate work without changing quotas or formula revisions.
The following uncontended runs use the same seed, worker/backend, life and work
settings; all use the modest graph unless shown otherwise:

| Scenario/profile | Observations | Incomplete observations | Complete-step p95 |
|---|---:|---:|---:|
| P1 v3/B16 | 90,000 | Not applicable | 9.2040 ms |
| P1 Basic/B16 | 89,998 | 0.371% | 15.5287 ms |
| P1 Motion/B16 | 90,000 | 0.384% | 17.5945 ms |
| P0 Motion/B32 | 16,500 | 0% | 4.3390 ms |
| P1 Motion/B32 | 90,000 | 18.312% | 18.7079 ms |

Basic/B16 meets these two probe checks in this short development run; it
still requires target-platform, tactical, representative and all-consumer
qualification. Its two missing deliveries are visible in the report's due counts.
Motion/B16 passes only the quality gate. Motion/B32 fails both. The P1 large
Motion/B32 graph has 494,306 parameters per genome and rejects before population
allocation: its proposed checkpoint needs 593,324,203 decoded bytes against the
existing 536,870,912-byte ceiling. Its zero observations produce no quality or
timing verdict. No admission ceiling has been raised.

## Loaded Oxygen exploration

Oxygen used the same source digest, four SSE2 workers and run settings on
x86_64 Debian with a Ryzen 7 2700, eight available CPUs and approximately
16 GiB RAM. Another Node process was active during measurement; load average
was 4.43 and that process reported about 189% CPU. No unrelated process was
stopped. These are loaded-target observations, not clean 1x qualification.

| Scenario/profile | Incomplete observations | Complete-step p95 | Peak RSS |
|---|---:|---:|---:|
| P0 Motion/B16 | 0% | 36.818 ms | 18.4 MiB |
| P1 Motion/B16 | 0.560% | 143.150 ms | 44.2 MiB |
| P1 Motion/B32 | 19.258% | 146.064 ms | 63.0 MiB |
| P1 Basic/B16 | 0.390% | 124.428 ms | 34.2 MiB |
| P1 v3/B16 | Not applicable | 89.878 ms | 30.3 MiB |

The frozen-v3 reference also misses the cost gate under this load. B16 quality
remains below 1%; B32 requires traversal/profile work at unchanged budgets.
RSS includes process startup and checkpoint construction, whose timing is
excluded. Reports were copied back before the exact task-owned remote source,
runner, build cache and scratch were removed; the disk reserve was preserved.

## Baseline-v2 workload reports (version 2)

The version-2 Windows reports use the same modest graphs, four SSE2 workers,
seed 42, unlimited evolved respawn, five warmup and 300 measured steps. Unless labelled 60 Hz, v2
baselines use an explicit 0.06-second decision interval; v1 retains its normal
fixed-step cadence. Counts below combine neural and baseline v4 deliveries;
external consumers are absent. Background load is uncontrolled on this development
machine, so these measurements do not select production defaults.

| Scenario/profile/baseline | Neural / baseline deliveries | Incomplete v4 deliveries | Step p95 | Peak RSS |
|---|---:|---:|---:|---:|
| P0 Motion/B16 deterministic v2 | 16,500 / 617 | 0% | 5.983 ms | 18.9 MiB |
| P1 Basic/B16 deterministic v2 | 90,000 / 326 | 0.866% | 20.049 ms | 34.7 MiB |
| P1 Motion/B16 deterministic v2 | 90,000 / 395 | 0.374% | 18.145 ms | 46.1 MiB |
| P1 Motion/B16 stochastic v2 | 90,000 / 395 | 0.374% | 18.014 ms | 45.9 MiB |
| P1 Motion/B16 v1 | 90,000 / 1,886 | 0.384% | 18.426 ms | 45.9 MiB |
| P1 v3/B16 v1 | 90,000 / 1,579 | Not applicable | 12.516 ms | 31.3 MiB |
| P1 Motion/B16 deterministic v2 at 60 Hz | 90,000 / 1,549 | 0.779% | 22.447 ms | 47.0 MiB |
| P1 Motion/B16 stochastic v2 at 60 Hz | 90,000 / 1,549 | 0.779% | 20.393 ms | 46.0 MiB |

Every admitted v4 report preserves the shared batch budget. V2 action selection
performs zero world reads; the v1 Motion reference records 584,660 snake reads.
The v2 Motion baseline subgroup has five incomplete deliveries out of 395
(1.266%); its pooled rate must not obscure that teacher-specific quality issue.
Deterministic and stochastic v2 follow the same trajectory here with no strategy
draws: this short workload does not exercise or qualify stochastic exploration.
Cadence, baseline deaths and respawn waiting also limit teacher sample counts;
this is not sustained ten-teacher throughput evidence. At 60 Hz, the baseline
subgroup has four incomplete deliveries out of 1,549 (0.258%), still without
strategy draws. The change in cadence changes the simulated trajectory; these
are separate workloads, not an isolated timing comparison.

P1 large Motion/B32 with v2 rejects before allocation at 593,324,392 decoded
checkpoint bytes against the unchanged 536,870,912-byte ceiling. Its report has
no quality/timing verdict and a 6.5 MiB admission-stage peak RSS. Production
worker scratch remains 512 MiB.

The matching version-2 Oxygen reports use the same native source digest, worker
count, budgets and cadence choices on x86_64 Debian. Host load average changed
from 2.77/3.22/3.24 to 4.14/3.67/3.42 during the sequential matrix. Scratch free
space exceeded 6.9 GiB throughout. No unrelated process was stopped. These remain
loaded-target observations; even the fresh frozen-v3 reference fails the cost gate.

| Scenario/profile/baseline | Neural / baseline deliveries | Incomplete v4 deliveries | Step p95 | Peak RSS |
|---|---:|---:|---:|---:|
| P0 Motion/B16 deterministic v2 | 16,500 / 608 | 0% | 40.743 ms | 17.8 MiB |
| P1 Basic/B16 deterministic v2 | 90,000 / 373 | 1.069% | 135.631 ms | 34.7 MiB |
| P1 Motion/B16 deterministic v2 | 90,000 / 362 | 0.518% | 147.478 ms | 46.3 MiB |
| P1 Motion/B16 stochastic v2 | 90,000 / 362 | 0.518% | 140.174 ms | 46.1 MiB |
| P1 Motion/B16 v1 | 90,000 / 2,059 | 0.560% | 207.050 ms | 44.5 MiB |
| P1 v3/B16 v1 | 90,000 / 1,580 | Not applicable | 122.071 ms | 30.8 MiB |
| P1 Motion/B16 deterministic v2 at 60 Hz | 90,000 / 1,602 | 0.624% | 174.920 ms | 45.3 MiB |
| P1 Motion/B16 stochastic v2 at 60 Hz | 90,000 / 1,602 | 0.624% | 124.332 ms | 45.5 MiB |

Oxygen Basic/B16 fails the pooled quality gate as well as timing; its baseline
subgroup has eight incomplete deliveries out of 373 (2.145%). Motion's v2
subgroups have two of 362 (0.552%) at 0.06 seconds and 13 of 1,602 (0.811%) at
60 Hz. Shared budgets hold, policy world reads and v2 strategy draws are zero.
Platform/build-dependent trajectories remain distinct; equal seeds do not promise
cross-platform bit-identical continuation. The large graph rejects at the same
checkpoint ceiling, with a 5.2 MiB peak RSS. All nine JSON transfers were SHA-256
verified before deleting the exact task-owned remote source/build/scratch tree.

## Version-3 Windows native capture batches

The eight compressed `windows-p[01]-motion-b16-60hz-*-v3.json.gz` reports use
seed 42, the modest graph, unlimited evolved respawn, stochastic v2 at exact
60 Hz, five warmup and 1,200 measured steps (20 simulated seconds). Each workload
has capture-off/one-worker, normal/one-worker, normal/three-worker and
slow/three-worker runs. Normal capture uses the 16 MiB pending/queue allowances,
1,024 queue entries and one bounded drain per step. Slow capture admits one
queue entry and drains only after 1,200 steps. This forces count pressure; it
does not fill the 16 MiB byte allowance. Development-machine load is uncontrolled.

| Workload | Actual teacher rows | Rows/simulated second | Normal 3-worker p95 | Normal peak RSS | Slow dropped rows |
|---|---:|---:|---:|---:|---:|
| P0 | 8,444 | 422.20 | 5.708 ms | 19.29 MiB | 8,404 |
| P1 | 4,089 | 204.45 | 24.998 ms | 47.83 MiB | 4,084 |

Normal draining preserves every exact sample; the source and decoded per-slot
witnesses agree. Complete authority continuations, all consumer counters and
normalized histograms agree across capture off/on, one/three workers and slow
draining within each workload. Slow runs drop 141/72 complete chunks for P0/P1,
reconcile every sample/chunk and settle the final released watermark; their
exact-sample result remains null. All eight JSON streams were losslessly
compressed and roundtrip-verified before removing their disposable scratch.

Neither measured interval has a ten-due-teacher step: warmup deaths and later
lifecycle waiting reduce activity. These observations do not qualify sustained
600-sample/second production. P0 has no incomplete v4 delivery; P1 has 4.103%
pooled incompleteness and 133/4,089 (3.253%) in the teacher subgroup. P1 therefore
exceeds the native-step timing reference and fails the ordinary-quality gate in this workload.
These are longer gameplay cohorts, not an isolated comparison to earlier
five-second reports. No production defaults or 512 MiB worker/checkpoint scratch
limits change.

All 746 release library tests, 16 rebuilt-addon identity/runtime/capture/shadow
tests, default/all-target test-hook Clippy and rustfmt pass. CI retains short
normal/slow capture reports in addition to the native tactical batch. Full
16 MiB byte pressure, sustained ten-living-teacher capture, Node spool/trainer
capacity and the integrated six-hour soak remain separate required gates.

## Version-3 Oxygen native capture batches

The eight matching `oxygen-p[01]-motion-b16-60hz-*-v3.json.gz` reports use the
same source, arguments and capture pressure on x86_64 Debian. Complete authority
continuations, consumer counters, normalized histograms and source sample
witnesses agree across capture modes and one/three workers within each workload.
Normal draining loses no rows; the slow count-limited queues reconcile all drops.
Every transfer was SHA-256 verified before lossless compression and byte-for-byte
roundtrip verification. Cross-platform trajectories remain separate experiments.

| Workload | Actual teacher rows | Rows/simulated second | Normal 3-worker p95 | Normal peak RSS | Slow dropped rows |
|---|---:|---:|---:|---:|---:|
| P0 | 8,444 | 422.20 | 55.826 ms | 19.19 MiB | 8,404 |
| P1 | 3,575 | 178.75 | 150.829 ms | 46.05 MiB | 3,570 |

P0 has no incomplete delivery. P1 has 4.093% pooled incompleteness and 31/3,575
(0.867%) in the teacher subgroup; the pooled quality gate still fails. Neither
interval contains a ten-due-teacher step. These are loaded-target observations,
with no timing qualification: load average changes from 2.33/3.09/3.13 to
4.31/3.85/3.45. Each CLI case checks its disk reserve; completion free space is
6.9 GiB. Unrelated processes remain untouched. Existing Windows full-rate,
byte-pressure and integrated capacity limitations remain open.

## Existing-timer P1 attribution

The retained `*-p1-motion-b16-60hz-normal-w3-v4.json.gz` baselines add attribution
to the same 1,200-step workload. Each platform's complete continuation and
normalized histograms match its preceding version-3 cohort. They remain loaded
observations with recorded source/build identity, rather than a release cost pass.

| Platform | Mean native service | Sensing share | Inference share | Index share | Physics share |
|---|---:|---:|---:|---:|---:|
| Windows | 17.847 ms | 65.88% | 6.21% | 1.94% | 19.11% |
| Oxygen | 109.339 ms | 66.24% | 7.89% | 1.49% | 18.66% |

The slow-step subset shows the same dominant sensing cost. Remaining control
and service costs complete the partition. This directs optimization toward
sensing without changing graph dimensions, ray grants, quality thresholds or
scratch limits. Source and codec checks remain mandatory for any candidate.

## Reverted shared-bounds experiment

The two `bounds-abba-*-v1/` directories retain four version-4 workload reports
and one comparison manifest per platform. Baseline commit `74dd8dc` and
candidate commit `10c5383` differ only in sharing validated inflated bounds
across a ray packet. Commit `c32f6cb` reverts the candidate. Source/build and
comparison-script fingerprints are retained in the reports/manifests; no
executable is retained.

Each platform runs baseline/candidate/candidate/baseline with the same P1
Motion/B16 modest graph, seed 42, three workers, SSE2, stochastic-v2 exact 60 Hz,
capture, five warmup and 1,200 measured steps. Every case has the same complete
authority continuation, consumer counters, normalized histograms and exact
source/decoded teacher-sample witnesses within its platform. Compressed report
bytes roundtrip to the SHA-256 fingerprints in `comparison.json`.

| Platform | Baseline mean / candidate | Baseline sensing / candidate | Baseline p95 / candidate |
|---|---:|---:|---:|
| Windows | 21.138 / 20.868 ms | 14.407 / 14.090 ms | 24.256 / 23.976 ms |
| Oxygen | 99.113 / 100.919 ms | 64.038 / 64.651 ms | 153.586 / 158.935 ms |

Values are medians of the two runs for each role. Windows mean service improves
1.28%; Oxygen worsens 1.82%, with no demonstrated cross-platform benefit. Median
whole-process CPU time is 59.758 / 59.398 seconds on Windows and 227.054 /
227.196 seconds on Oxygen. These shared-host descriptive observations carry no
confidence or release-pass claim. Process CPU/wall covers CLI startup, observer
and drain work; native step timings exclude the external observer. Windows uses
an eight-core Ryzen 7 5800X; Oxygen uses its existing loaded Ryzen 7 2700 target.
The experiment does not address the unchanged approximately 4.1% pooled
incomplete rate. Production defaults and admission limits remain unchanged.

Build the two immutable commits separately using
`cargo build --manifest-path native/Cargo.toml --release --features engine-test-hooks --bin sensor-v4-qualification`,
preserving each executable before building the other. With those executable
paths, reproduce the comparison from the repository root:

```text
python scripts/qualification/compare-sensor-workloads.py --baseline BASELINE_EXE --candidate CANDIDATE_EXE --baseline-source 3ddf0559e779bf30b74da3fe86f0756791f86dd2635184af286244271bfdb4c2 --directory NEW_REPORT_DIRECTORY
```

Use `--environment oxygen-debian` on Oxygen. The directory must not already
exist. The utility bounds runtime/report size, fingerprints binaries before and
after each run, rejects changing/incompatible workloads and keeps its checks
active under Python optimization. It does not automate statistical qualification.
The exact task-owned comparison executables, raw transfers and remote build tree
were removed after permanent report verification.

## Shared head facts

Candidate commit `95b1d64` retains one cold-admitted array of validated head
velocities, bound to the same immutable pre-movement world. Alive-head validation
and Motion's separate cosine/sine products run once per boundary. All observers
still use the original formulas, physical-array indices and ray grants. The
buffer costs 16 bytes per admitted world slot, within the existing staging
check; its actual capacity is included in diagnostics. Both 512 MiB scratch
ceilings remain unchanged. Frozen v3 allocates no head buffer.

The five `heads-abba-*/` directories retain 20 losslessly compressed version-4
reports, comparison manifests and the two Linux affinity records. Baseline is
`74dd8dc` (also the native source in `41d6e28`); candidate is `95b1d64`.
Every case uses the same Motion/B16 modest graph, seed 42, three workers, SSE2,
stochastic v2 at exact 60 Hz, normal capture, five warmup and 1,200 measured steps.
Each row below is the median of two runs per role in baseline/candidate/candidate/
baseline order. Milliseconds describe complete native service or sensing; CPU
seconds cover the entire CLI process, including startup and observer work.

| Host/workload | Mean service, baseline / candidate | Sensing, baseline / candidate | Native p95, baseline / candidate | Process CPU seconds, baseline / candidate |
|---|---:|---:|---:|---:|
| Windows P0 | 4.608 / 4.602 ms | 1.932 / 1.933 ms | 5.527 / 5.482 ms | 10.625 / 10.617 |
| Windows P1 | 18.786 / 17.288 ms | 12.461 / 11.304 ms | 22.998 / 18.828 ms | 54.305 / 50.258 |
| Oxygen P1, unrestricted | 75.783 / 78.527 ms | 51.442 / 52.719 ms | 124.459 / 133.304 ms | 180.049 / 183.573 |
| Oxygen P0, fixed affinity | 15.405 / 14.465 ms | 7.354 / 6.819 ms | 28.310 / 25.765 ms | 28.218 / 26.694 |
| Oxygen P1, fixed affinity | 65.929 / 61.792 ms | 44.358 / 41.455 ms | 110.066 / 104.095 ms | 159.519 / 149.668 |

Windows P1 mean service and process CPU decrease 7.98%/7.45%; Windows P0 is
effectively unchanged. The unrestricted Oxygen P1 set is noisy and shows no
benefit. One follow-up fixes only the benchmark process and its children to
logical CPUs 1,4,6,7, selected once from the eight initially allowed CPUs using
one-second utilization deltas. Fixed-affinity P1 service/CPU decrease 6.28%/6.18%.
P0 still shows host drift. These are descriptive shared-host observations,
without a statistical-confidence or production-cost qualification claim.

All source/drained teacher witnesses, complete continuations, consumer counters
and normalized histograms match within each platform/workload. Oxygen P1 also
matches its unrestricted set after affinity changes, including raw histograms.
Every transferred file was fingerprint-verified before compression, and every
compressed stream roundtrips to its raw report fingerprint. Exact equality is
evidence for these finite same-target cohorts, not a cross-build/platform replay
promise.

P1 remains over the 16.667 ms p95 reference on both platforms and retains
4.103%/4.093% pooled incomplete observations on Windows/Oxygen. The sparse teacher
cohorts still do not qualify sustained 600-row/second capture. No sensor/teacher
default, normalization revision or ray admission is frozen by this optimization.

Reproduce with the preceding comparison utility and the two source commits,
adding `--scenario P0` for P0. For the Linux affinity cases, set the parent
process's `os.sched_setaffinity(0, {1, 4, 6, 7})` before invoking the comparator;
native children inherit that set. The retained `affinity.json` records the
selection and load context. Disposable comparison copies, source trees and raw
transfers were removed after permanent evidence verification.


## Native byte pressure with frequent teacher replacement

The `capture-byte-pressure-windows-v1/` and `capture-byte-pressure-oxygen-v1/`
cohorts use the same current native source, fingerprinted in each
`qualification.json`, with P0, Motion B32 (504 inputs), the modest graph, seed 42,
SSE2, v2 stochastic teachers at exact 60 Hz, no warmup and 1,200 completed steps.
The qualification-only teacher respawn option explicitly selects the existing
0.5-second minimum. Its default remains 20 seconds; production settings and both
512 MiB scratch ceilings are unchanged. Report version 4 gains additive delay,
actual payload allowance and disjoint byte/count drop fields. Count pressure
takes precedence if both limits reject the same chunk; external transport fields
retain their existing meanings.

Each four-case cohort compares capture off, fast drain with one/three workers,
and a three-worker slow drain. The driver rejects changed complete authority
continuations, raw/normalized histograms or consumer work. Fast runs independently
reconcile every exact source/drained row. Slow runs verify descriptor/codec bytes,
whole-chunk loss, visible released sequences and observed = drained + dropped rows;
loss receives no exact-sample pass.

| Target | Actual rows / simulated second | Longest ten-teacher interval | Slow queue peak bytes / payload allowance | Byte-limited chunks / rows dropped | Count-limited drops |
|---|---:|---:|---:|---:|---:|
| Windows | 589.5 | 272 steps (4.53 simulated seconds) | 12,388,442 / 12,451,840 | 89 / 5,428 | 0 |
| Oxygen/Debian | 592.5 | 557 steps (9.28 simulated seconds) | 12,429,262 / 12,451,840 | 90 / 5,490 | 0 |

Both pressure cases peak at 103 queued chunks against the 8,192-chunk limit.
The payload allowance charges mailbox/metadata, descriptor-ring and encoding
reserves inside the unchanged 16 MiB queue ceiling; pending capture has its own
16 MiB ceiling. Final draining releases all 202/203 assigned sequences on
Windows/Oxygen. Slow-run peak RSS is 37,347,328 / 37,097,472 bytes. The normal
three-worker native p95 is 5.489 / 18.654 ms; these shared-host observations do not
qualify real-time scheduling. Each platform is compared internally: neither
row counts nor trajectories are promised identical across platforms.

The intervals with ten living/due teachers reach 600 rows per simulated second;
the complete measured cohorts include ordinary death gaps and remain below that
rate. This qualifies native byte-pressure behavior and isolation, without claiming
sustained full-stack 600-row throughput, 4 GiB spool rotation, trainer/VRAM
admission, wall-debt scheduling or the six-hour integrated soak. Those gates remain
open. All eight compressed reports roundtrip to the fingerprints in their cohort
manifests; the transferred Oxygen files are independently verified before retention.

After building the qualification binary, reproduce with an exclusive output path:

```text
python scripts/qualification/qualify-capture-pressure.py --executable NATIVE_QUALIFICATION_EXE --directory NEW_REPORT_DIRECTORY --workers 3
```

Use `--environment oxygen-debian` on that target. The bounded driver uses only
the Python standard library and keeps checks active under `python -O`. Node 24 CI
runs the same cohort on Windows and Ubuntu and retains the complete JSON reports.
Disposable source/build trees and raw transfers are removed after verified retention.


## Shared centered-sector classification

V4 now prepares one bounded stack array of angular boundaries per dense observation
and reuses it for food, body, head and predicted-contact sectors. Interior vectors
use the same geometric classification already used by frozen v3 food bins. Sparse
queries retain the original formula. Near a boundary, at zero/subnormal/nonfinite
norms, or for large restored headings, v4 retains its original sine/atan2 angle
normalization; v3 retains its own remainder normalization. Squared body distances
can underflow for nonzero vectors, so they explicitly fall back as well. Continuous
bearings and all other channel formulas remain unchanged. No sensor revision,
ray-work unit, query limit, heap buffer or scratch ceiling changes.

Three new native regressions compare the old formulas across both profiles and
all B8..32 layouts, dense/boundary/tiny/huge/signed-zero vectors, sparse and dense
raw food accumulation, body/head minima and IDs, and projected urgency sectors.
The existing v3 classifier and five source-identity fixtures remain unchanged in
meaning and bytes. Source comments and the regression preserve the underflow case.
All 754 release library tests and 183 rebuilt-addon required-native checks pass,
with six existing skips in that overlay; rustfmt and both Clippy configurations
pass.

The four `sectors-abba-*/` directories retain sixteen compressed version-4 native
reports and comparison manifests. Baseline is commit `601acaf` (native source
`9e913dd9`); the candidate source begins `b9dec41f`. Full source/build identities,
binary/report fingerprints and sample witnesses are in the retained metadata.
Every case uses Motion B16, the modest graph, seed 42, three workers, SSE2,
stochastic v2 at exact 60 Hz, capture enabled, five warmup and 1,200 measured steps.
Each table value is the median of two runs in baseline/candidate/candidate/baseline
order. CPU seconds cover the complete CLI process; native times exclude the
external observer and startup/checkpoint publication.

| Target/workload | Mean native service, baseline / candidate | Sensing, baseline / candidate | Native p95, baseline / candidate | Process CPU seconds, baseline / candidate |
|---|---:|---:|---:|---:|
| Windows P0 | 4.369 / 3.760 ms | 1.817 / 1.341 ms | 5.345 / 4.664 ms | 10.172 / 8.094 |
| Windows P1 | 19.677 / 16.243 ms | 13.037 / 9.764 ms | 22.481 / 18.920 ms | 56.742 / 44.625 |
| Oxygen P0 | 31.348 / 25.405 ms | 14.240 / 9.986 ms | 64.183 / 51.203 ms | 50.287 / 39.240 |
| Oxygen P1 | 105.160 / 91.427 ms | 67.834 / 53.260 ms | 194.379 / 167.700 ms | 230.213 / 190.938 |

Windows mean/CPU decrease 13.9%/20.4% for P0 and 17.5%/21.4% for P1. Oxygen
mean/CPU decrease 19.0%/22.0% for P0 and 13.1%/17.1% for P1. Oxygen is a loaded
shared host: one one-second utilization sample selected logical CPUs 0,3,4,5 from
the eight initially allowed CPUs. Only the comparison process and its children
use that affinity; both workloads retain the same set. Each Linux cohort includes
that selection record. Load drift is visible within the four P1 runs, so these are
descriptive comparisons without statistical confidence or a release-cost pass.

Every case preserves complete continuation, consumer work, raw/normalized
histograms and exact source/drained teacher row witnesses within its platform and
workload. All eleven Oxygen transfers were independently fingerprint-verified,
and each compressed raw report roundtrips to its manifest fingerprint. These
finite same-target equalities do not promise cross-build/platform long-horizon
replay. The P1 p95 reference remains exceeded on both platforms; pooled ray
incompleteness remains 4.103% / 4.093% on Windows/Oxygen. No v4/v2 production
default, ray admission, normalization freeze or full-stack throughput is selected.
The unchanged capture owner reuses the preceding byte-pressure qualification;
this change adds no new transport ownership or quotas.

Reproduce with the existing comparison utility and these two source revisions,
using `--scenario P0` for P0 and `--environment oxygen-debian` on that target.
For the Linux measurements, set the parent process's affinity to `{0,3,4,5}`
before launching the utility; children inherit it. The reusable source/build
copies and raw transfers are removed after permanent evidence verification.

## Bounded spatial splits for body rays

The v4 body-ray hierarchy uses an eight-bin perimeter heuristic on both axes to
keep nearby body segments together. Each split retains at least
`max(2, floor(N/4))` records on each side; degenerate centroids use the previous
median rule. This bounds depth and preserves the existing node-capacity bound.
Fixed stack arrays add no heap buffer or capacity setting. Ray bounds, capsule
math, individual cell/candidate/intersection charges, fair grants and conservative
incomplete markers are unchanged. Both native scratch ceilings remain 512 MiB.
V3 does not construct this hierarchy.

A regression uses two separated clusters of long segments with interleaved
physical IDs. Three cell visits prove the intervening gap clear without candidate
or capsule work, including reversed world storage. Another regression checks
unique records, conservative node bounds, balanced depth and the existing capacity
bound across 771 degenerate, scattered and skewed layouts. Existing complete
capsule oracles, every-budget exhaustion checks and warm-capacity tests also pass.

The portable driver checks Basic/Motion B8/B16/B32 in P1, Basic/Motion B16 in P0,
three smaller-budget probes and exact one/three-worker continuation for P1 Motion
B16. Every case uses the modest graph, seed 42, unlimited evolved respawn, ten
stochastic v2 teachers at exact 60 Hz, ordinary 20-second teacher replacement,
capture with fast drain, SSE2, five warmup and 1,200 measured steps. Admission,
actual evolved deliveries and due envelopes, finite normalized samples,
charged-work partitions, shared/per-observation bounds and exact committed capture
must reconcile before reporting any fraction. Failed quality gates remain data.
Checks remain active under optimized Python.

```powershell
python scripts/qualification/qualify-ray-hierarchy.py --executable NATIVE_QUALIFICATION_EXE --source-sha256 SOURCE_SHA256 --directory NEW_REPORT_DIRECTORY --workers 3
```

Use `--environment oxygen-debian` on Debian. The twelve probes are not a complete
cap matrix. Better clearance can change later actions, deaths, pellets and teacher
availability, so reports from the two hierarchy revisions describe different
resulting gameplay. They are not an identical-workload speed comparison, a trained
policy or tactical/ablation result, a 1x wall-debt test, external-controller
qualification or production-default selection. Same-target worker equality is
checked separately; cross-build/platform exact long-horizon replay is not promised.

The Windows checkpoint passes 701 default-feature release library tests and 183
required-native checks across 17 files, with six existing skips. Rustfmt and
all-target/test-hook Clippy pass. Production and isolated test-hook addons are
rebuilt and independently source-identified. This source begins `bc056c98`.

`ray-hierarchy-windows-v1/` and `ray-hierarchy-oxygen-v1/` each retain twelve
native reports (gzip), exact raw-byte fingerprints and a qualification manifest. Completed reports can be supplied
with `--reuse-directory`; they must pass the complete source/workload/target and
accounting checks and are copied byte-for-byte into a new exclusive directory.
The manifest identifies reused cases. Actual delivery counts remain explicit:
the Windows 32,768-unit probe records 359,999 neural deliveries and a 299..300
due range. Debian Motion B8 likewise records 359,999 actual deliveries.

| Workload, three workers | Incomplete observations, Windows / Debian | Native p95, Windows / Debian |
|---|---:|---:|
| P1 Basic B8 | 0.000275% / 0.000000% | 16.438 / 148.583 ms |
| P1 Basic B16 | 0.124800% / 0.064553% | 17.529 / 149.655 ms |
| P1 Basic B32 | 16.343710% / 13.383582% | 19.589 / 167.340 ms |
| P1 Motion B8 | 0.000000% / 0.000000% | 17.300 / 150.070 ms |
| P1 Motion B16 | 0.201630% / 0.048156% | 18.961 / 188.301 ms |
| P1 Motion B32 | 14.476863% / 11.688614% | 21.335 / 210.371 ms |
| P0 Basic B16 | 0.000000% / 0.000000% | 4.474 / 39.317 ms |
| P0 Motion B16 | 0.000000% / 0.000000% | 4.735 / 66.351 ms |
| P1 Motion B16, cap 128, batch 65,536 | 34.691113% / 36.940588% | 18.688 / 146.382 ms |
| P1 Motion B16, cap 256, batch 32,768 | 59.148624% / 60.061437% | 18.789 / 163.496 ms |
| P1 Motion B16, cap 256, batch 16,384 | 90.557802% / 91.407433% | 17.893 / 148.231 ms |

P1 Motion B16 falls from the previous hierarchy's 4.103% / 4.093% pooled
incompleteness to 0.202% / 0.048% on Windows / Debian at the same 256/65,536
admission. Both B16 profiles and B8 pass this seeded
quality probe, while both B32 profiles fail. The 128-unit cap and smaller batch
probes fail as well. These results do not qualify every admitted consumer or
select a production default. The native 60 Hz p95 reference remains exceeded for
P1 Motion; even a native p95 pass would not establish supported 1x wall-debt
scheduling. The one-worker repeat preserves full continuation, consumer work,
raw/normalized histograms and exact source/drained teacher rows for P1 Motion
B16. Timing comparisons across hierarchy revisions remain descriptive because
ray completion changes gameplay. Trained-policy distributions, tactical quality,
ablations, loaded checkpoint/resource admission and integrated soak gates remain
open. Frozen v3 and ordinary v1 production defaults are unchanged.

Debian ran on a loaded shared host. A single one-second utilization sample chose
CPUs 0,1,2,5 from eight allowed CPUs; only the owned benchmark parent and children
used that affinity, including the worker-count repeat. Its selection record is
retained. Load drift prevents a statistical timing claim, and its native p95 fails
the 60 Hz requirement. Every transferred file and decompressed native report was
fingerprint-verified before permanent retention. The twelve probes reconcile
actual delivered observations, shared work and exact committed capture, including
finite samples and per-class fractions; no unavailable external-controller class
is counted as a quality pass.


The Windows P1 Motion B16 CPU sample profile in
`sensing-cpu-windows-v1.json` attributes 31.2% of sensing CPU time to pellet
collection (including sorting), 15.9% to angular classification, 15.6% to ray
traversal and 9.2% to body-candidate collection. Inclusive categories overlap.
Samply captured the qualification process and its three calculation workers at
1,000 Hz; matching-PDB DbgHelp symbols and positive `threadCPUDelta` values
exclude synthetic off-CPU weights. The compact report records the actual
workload/cadence, source/binary/profile fingerprints, CPU accounting and source
lines. Startup/observer work is present in whole-process samples, while sensing
rows require the v4 sampling function in their stack. These are descriptive
attributions, separate from release throughput/debt qualification.

`half-plane-search-windows-v1/` retains an exact rejected patch and four adjacent
ABBA reports for a prepartitioned angular lookup. Raw/normalized samples, full
continuation, per-consumer work and committed teacher hashes agree exactly,
using the comparison driver's added explicit raw-histogram check. With 1,200
measured P1 steps per run, median native mean/p95 rise from 14.748/17.784 ms to
15.518/17.935 ms; median child CPU rises from 39.922 to 42.766 seconds.
Run-to-run variation is substantial, and no useful whole-workload gain is
established. The patch is reverted; production and qualification source remain
`bc056c98`. The profile prioritizes pellet collection/sorting for further work;
none of the outstanding performance, quality or default-selection gates closes.


Spatial candidate distances now cache the reversible `f64::total_cmp` order key
once, replacing repeated sign folding inside sort/heap comparisons. Restoring
the key recovers every original distance bit for numeric ring-stop checks.
Body/pellet candidates remain 32/16 bytes with unchanged capacity admission.
A regression compares the cached order with the standard library across signed
zeros, subnormals, infinities, both NaN signs/payloads and 16,384 generated bit
patterns, and checks exact roundtrip and candidate sizes.

`spatial-order-keys-windows-v1/` retains sixteen adjacent ABBA reports for native
sources `bc056c98` and `d1cf40a0`. Each uses Motion B16, modest graph, seed 42,
unlimited evolved respawn, ten stochastic v2 teachers at exact 60 Hz, capture,
cap 256/batch 65,536, five warmup and 1,200 measured steps. Full continuation,
raw/normalized samples, per-consumer work and committed teacher hashes agree
within each population across both builds; P1 also agrees across 1/3/5 workers.
Five workers match this host's CPU-normalized production default.

| Case | Mean step, before / after | Native p95, before / after | Child CPU, before / after |
|---|---:|---:|---:|
| P0, 3 workers | 4.062 / 4.020 ms | 5.270 / 5.172 ms | 8.375 / 8.172 s |
| P1, 1 worker | 41.187 / 40.562 ms | 56.882 / 55.375 ms | 45.258 / 44.633 s |
| P1, 3 workers | 18.035 / 18.395 ms | 22.645 / 24.125 ms | 46.641 / 44.602 s |
| P1, 5 workers | 13.432 / 13.300 ms | 16.164 / 16.179 ms | 46.570 / 46.070 s |

These shared-host medians show modest CPU reductions and mixed wall timings;
the three-worker P1 wall-time regression remains explicit. This is descriptive
evidence with two runs per role, without a statistical confidence claim. Native
p95 relative to one fixed-step interval is a diagnostic; the plan's P0/P1 1x
gate requires actual wall-clock throughput/debt qualification. Node/browser,
loaded spool/replay/training, representative policies, tactical/ablation and
integrated-soak gates remain open. No production defaults or quotas change.


The real-clock host `scripts/qualification/sensor-wall.ts` uses the same native
P0/P1 modest graph/settings builder as the controlled-step benchmark. It starts
the autonomous Rust coordinator only after a real run-start checkpoint is
committed to an isolated SQLite store. Ten stochastic v2 teachers use exact
60-Hz cadence, Motion B16, explicit ray cap 256/batch 65,536, a 16-MiB pending
buffer, a 16-MiB native queue and the default 4-GiB spool. Teacher death gaps
remain part of the workload. Latest packed frames copy into one reused buffer
at at most 30 Hz. This test constructor exists only in the source-identified
release hooks addon; ordinary startup remains v3/v1.

```powershell
node node_modules/tsx/dist/cli.mjs scripts/prepare-panic-test-addon.ts
node node_modules/tsx/dist/cli.mjs scripts/qualification/sensor-wall.ts --scenario P0 --seconds 60 --warmup-seconds 10 --report wall-P0.json
node node_modules/tsx/dist/cli.mjs scripts/qualification/sensor-wall.ts --scenario P1 --seconds 60 --warmup-seconds 10 --report wall-P1.json
```

Build the source-identified production addon first; the spool worker uses it.
`--workers` defaults to the normal server's five-worker choice normalized to
available CPUs. The bounded window is 60..600 seconds after 5..60 seconds of
warmup. Reports retain monotonic clock brackets, exact counters, native timing
endpoint histograms, dataset watermarks/drop counters, frame work, queue diagnostics and
source/build/environment identities. The existing real-time criterion is at
least 98% simulated/wall progress with no newly dropped wall debt or observed
overload; the separate existing P0/P1 step criterion is p99 bucket ceiling at
most 16.667 ms. Their conjunction is explicit. Measured failures are saved as
false results; authority faults, invalid evidence or incomplete diagnostic
settlement reject the run. The report is written only after orderly shutdown
and removal of its temporary checkpoint/spool tree.

Ubuntu and Windows Node-24 CI jobs run both scenarios and upload `sensor-wall`
artifacts. Performance results are diagnostic on shared runners; operational
failures still fail the job. This scope excludes browser/WebSocket transport,
trainer/shadows, full-capacity rotation, representative trained policies and the
integrated soak, so it cannot by itself select production defaults.


`real-clock-windows-v1/` retains the two first real-clock windows for native
source `60845bd3`, five workers on the Ryzen 7 5800X, after ten seconds of
warmup. Each stores recomputable counter/clock evidence and exact endpoint
histograms; the saved summaries and host source fingerprints were independently
rechecked. Both runs shut down cleanly and removed their isolated data trees.

| Scenario | Fixed steps / wall seconds | Minimum sim/wall ratio | p95 / p99 bucket ceiling | New dropped wall debt | Measured gates |
|---|---:|---:|---:|---:|---|
| P0, 55 evolved | 3,601 / 60.019 | 0.99996 | 8 / 8 ms | 0 | Pass |
| P1, 300 evolved | 3,255 / 60.069 | 0.90312 | 24 / 32 ms | 5.571334 s | Fail; overload observed |

P0/P1 copy 1,094/1,061 frames during the measured window and retain
32,558/14,589 actual teacher rows across capture startup, warmup, measurement
and final seal, with nonzero rows from all ten selected slots. Capture has no
native or spool drops and final watermarks settle. These row totals are not
exact window throughput or a claim of ten continuously alive teachers. Process
peak RSS is 157.6/174.5 MiB and measured process CPU is 41.532/166.048 seconds.
Whole-machine CPU counters show about 42.0%/55.6% busy time; this is a shared
owner machine, not an isolated performance lab. P0 passes this bounded Windows
workload window. P1 fails both existing measured gates in this configuration;
short controlled-step timings cannot substitute for that result. Other
platforms, longer runs and the excluded consumers/capacity gates remain open.


`real-clock-windows-v1/P1-w5-phases.json` repeats P1 with the existing coarse
native timing counters sampled beside the acceptance window. Progress is 0.89484
sim/wall with p95/p99 ceilings 24/32 ms and 6.075667 seconds of new dropped debt.
The 2,237 attributed slow steps average about 19.659 ms. Their accumulated wall
cost assigns 43.68% to sensing, 29.21% to the world step, 7.02% to inference,
3.88% to control index work and 4.45% to other service work; the remaining
11.76% is other control work. Control/neural counters are parents of those
subcategories and must not be added again. Independently loaded atomic counters
are coarse diagnostics beside the consistent histogram window, not an exact
transaction snapshot. This repeat confirms an unresolved P1 deficit on the
shared host; it does not establish a performance change or select defaults.


`physics-costs-windows-v1/P1-w5.json` extends the controlled-step native run to
600 warmup and 3,600 measured steps at the same Motion B16, modest graph,
seed-42, five-worker and explicit ray settings. Version 5 adds ten existing
hook-only physics timers, collected after complete-step publication without new
hot-path probes; the qualification drivers also accept retained version 4.
All-step mean/p95 are 18.014/23.420 ms. Sensing accounts for 45.56% of measured
native service time and world steps for 27.45%. Inside world time, collision
indexing is 23.65%, head/head work 20.62%, pellet indexing 18.00%, result
application 10.07%, food 7.17%, movement 7.04% and head/body work 6.15%.
Collision indexing/head work are nested in the reported collision total, not
additional additive phases. Complete-step continuation and exact teacher-byte
checks pass with diagnostic observation enabled/disabled and across workers.

This longer trajectory delivers 1,080,000 neural and 14,344 baseline observations.
Pooled ray incompleteness is **4.405%**, so the less-than-1% quality gate fails at
unchanged cap 256/batch 65,536. The earlier short seeded B16 probe's pass does
not establish longer-run quality. Both timing and ray quality remain unresolved;
no workload, ray quota or production-default gate closes from this evidence.

`real-clock-ci-ubuntu-v1/` retains byte-verified compressed windows from the
[61beaa4 CI run](https://github.com/Tits-McGruff/slither_neuroevo/actions/runs/39083120438).
Its full Ubuntu/Windows Node-24/26 and Rust-check matrix passes. On the Ubuntu
runner's four available CPUs, P0 passes (1.00009 sim/wall, p99 ceiling 8 ms),
while P1 fails (0.79439, 32 ms, 12.335 seconds new dropped debt). Capture has
zero drops in both. These measured performance failures are explicit report
results and do not turn the operational CI checks red.

`real-clock-oxygen-v1/` retains the same source's five-worker windows on the
shared Debian VM, with run-start SQLite/checkpoints and diagnostic objects on
the physical data filesystem. Both fail: P0 reaches 0.79165 sim/wall with a
96-ms p99 ceiling and 12.512666 seconds new dropped debt; P1 reaches 0.21063,
192 ms and 47.433334 seconds. Captures have zero drops and final watermarks
settle. These are shared-environment measurements; their stalls do not identify
a new product regression or isolate a cause. They remain negative qualification
evidence. Compressed raw bytes, source/environment metadata and digest provenance
are retained; the clearly named remote checkout/build cache remains available
for the next target-platform comparison, with no live qualification process.


`ray-split-bins16-windows-v1/` retains an unselected sixteen-bin split patch and
four adjacent long P1 runs (600 warmup/3,600 measured steps, five workers,
unchanged leaf size, balance bound and ray quotas). Each build repeats its own
continuation, consumer work, raw/normalized histograms and teacher hashes
exactly. Pooled incompleteness falls from 4.405% to 3.714% in this seeded
workload, but still fails the 1% gate. Median native mean/p95 are
16.717/20.399 ms before and 16.480/20.121 ms after; child CPU is essentially
unchanged at 192.531/192.313 seconds. Changed truncation changes subsequent
gameplay, so these timings do not isolate a kernel speed gain. The ordinary
eight-bin index remains selected while other approaches are tested. Raw reports
are retained losslessly as gzip, with original/compressed digests and the exact
comparison script; no production defaults or resource ceilings change.

`packet-bounds-windows-v1/` retains an unselected prepared-AABB/common-origin
packet rejection experiment. Four adjacent five-worker P1 runs use 600 warmup
and 3,600 measured steps. Continuation, charged work, raw/normalized histograms
and teacher hashes agree exactly across both builds. Median mean/p95 changes
from 17.257/21.162 ms to 17.196/21.035 ms; child CPU medians change from
200.789 to 199.242 seconds. The roughly 0.35% whole-step mean difference on this
shared host does not justify the added geometry code, so it is reverted. The
lossless reports, exact patch and comparison script remain reproducible. The
comparison driver now accepts an explicit bounded warmup and verifies the
reported warmup/measurement counts; its existing default remains five steps.

`pellet-cell-rank-windows-v1/` retains an unselected change replacing indirect
cell-sort tie comparisons with canonical pellet ranks. All four 600-warmup/3,600-
measured-step P1 runs preserve exact continuation, work, histograms and teacher
hashes. The nested pellet-index time falls, but median whole-step mean/p95 rises
from 18.213/23.020 ms to 19.283/25.400 ms and child CPU from 207.563 to 215.234
seconds. Sensing also slows although its code is unchanged. This shared-host
comparison does not isolate the cause; the measured whole-workload regression
is enough to leave this change unselected. Reports, patch and script are retained.

Both comparison and ray-quality cohort drivers now accept bounded `--warmup`
(default five steps), require warmup plus measurement at most 60,000 steps, and
validate both counts in each report. Cohort reuse rejects a mismatched warmup.
For later-trajectory quality probes, use `--warmup 600 --steps 3600`; these remain
seeded random-population probes rather than trained-policy qualification.

`pellet-counting-windows-v1/` retains the selected warmed counting path. It
reuses existing dense-lookup capacity for cell counts and placement cursors;
grids outside the existing capacity/density bounds keep the comparison sort. Canonical
pellet iteration preserves exact ID/source-index ties and x-before-y cell order.
No additional storage is reserved. Tests compare retained versus freshly sorted
indexes through dense/sparse/expanded/shrinking/empty grids and full-width ID ties.

Eight adjacent ABBA runs use 600 warmup/3,600 measured steps. Every continuation,
consumer-work record, raw/normalized histogram and captured-row witness agrees
across builds. All comparisons retain the same native memory admission. Nested pellet-
index time falls about 64% in both workloads. P0 (three workers) median mean/p95
improves from 4.331/5.226 ms to 3.917/4.823 ms; child CPU falls 29.750 to 28.344
seconds. P1 (five workers) remains mixed: mean/p95 rises 16.082/19.533 ms to
16.353/19.720 ms and child CPU rises 185.695 to 196.922 seconds, despite the lower
index cost. These shared-host descriptive measurements support the P0 benefit,
not a P1 speedup or release pass. The long P1 trajectory's 4.405% incompleteness
is unchanged. Compressed reports, exact measured patch/base and comparison
script remain reproducible; real-clock, representative-policy and soak gates
remain open.


## Large Motion/B32 workload and current admission

The bounded `scripts/qualification/qualify-large-workload.py` host first verifies
the current source's early rejection of 300 large Motion/B32 snakes, then runs
55 large snakes with one and three workers. Use an exclusive report directory:

```powershell
python -B scripts/qualification/qualify-large-workload.py --executable native/target/release/sensor-v4-qualification.exe --directory NEW_DIRECTORY --source-sha256 NATIVE_SHA256 --environment development --workers 3 --warmup 600 --steps 3600
```

The native constructor, fixed-step loop, sensors, v2 stochastic teachers and
capture remain authoritative. The graph has 494,306 parameters and 504 inputs.
The host checks actual due envelopes, finite histograms, one shared ray budget,
exact committed teacher bytes and exact within-target worker continuation.
Source mismatch fails during the small rejected case before the gameplay cases.
Reports and exact Python sources are retained in `large-motion32-windows-v1/`
and `large-motion32-debian-v1/`.

| Target | Workers | Native p95 ms | Pooled incomplete observations | Native peak RSS MiB |
|---|---:|---:|---:|---:|
| Windows | 1 | 17.502 | 0.002235% | 119.96 |
| Windows | 3 | 9.140 | 0.002235% | 120.25 |
| Debian | 1 | 98.377 | 0.002250% | 119.20 |
| Debian | 3 | 96.156 | 0.002250% | 120.31 |

Each measured case completes 3,600 steps after 600 warmup steps, delivers 198,000
neural observations and loses no captured chunks. Continuations, consumer work,
raw/normalized histograms and teacher witnesses agree exactly between worker
counts on each target. Cross-platform bit equality is not claimed. Debian's
shared-host timings miss the service reference; these remain negative timing
evidence, not a completed performance qualification.

The existing qualifier construction ceiling is 4 GiB, independently of its
512-MiB checkpoint envelope and the unchanged native scratch ceilings. The
55-snake authoritative reservation estimate is about 1.53 GiB; observed RSS in
this short cohort is lower and does not price a saturated long-run world. The
300-snake case rejects at 565.84 MiB of decoded checkpoint data, before sensing,
complete steps or capture. Its process peaks are about 6.33/5.24 MiB on
Windows/Debian. No checkpoint envelope is increased by this result. Large
population admission, crowded/saturated geometry, trained policies, real-clock
large-graph scheduling and integrated soak remain separate gates.

The Windows directory also retains a current-source P1 modest-graph baseline.
At five workers and cap 256/batch 65,536, its 3,600 measured steps have 19.169 ms
p95 and 4.405% pooled incompleteness. Mean native service is 16.007 ms: sensing
7.409, world step 4.560, other control 1.848, inference 1.053, other 0.716 and
index 0.421 ms. These phase components partition the recorded service total;
physics detail is nested. This identifies the next workload investigation
without asserting that an older build's timings describe the current source.


## Larger ray allowance, unchanged timing and memory gates

`sensor-wall.ts` accepts explicit `--ray-cap` and `--ray-batch` within the native
qualification constructor's existing bounds; defaults remain 256/65,536. The
candidate below uses 512/131,072, Motion/B16, 300 modest brains, ordinary v2
teachers and unlimited evolved respawn. Both controlled runs use 600 warmup and
3,600 measured steps on the same Windows target/source.

| Workers | Controlled native p95 ms | Pooled ray incompleteness | Real-clock minimum sim/wall ratio | Wall p95/p99 bucket ceilings ms | Combined wall gate |
|---|---:|---:|---:|---:|---|
| 7 | 16.454 | 0% | 0.999858 | 24 / 24 | Fails timing |
| 8 | 16.046 | 0% | 1.000031 | 16 / 24 | Fails timing |

The two controlled runs preserve exact continuation, consumer work, histograms
and teacher witnesses. Both real-clock windows complete over 3,600 steps, copy
actual packed frames, settle durable teacher storage and have no capture drops,
wall debt or observed overload. Sustained 1x is established for these windows;
the existing p99-at-most-16.667-ms criterion still fails. Neither the criterion
nor its conservative histogram interpretation changes.

A ten-worker probe rejects during native loop handoff: neural staging needs
604,613,230 bytes against the unchanged 536,870,912-byte ceiling. Its checked
receipt is separate from completed-world reports. Each extra worker currently
reserves a conservative 64 MiB of sensor scratch; increasing worker count cannot
bypass that admission. No staging or checkpoint ceiling is raised. Exact native
and wall reports, measured TS sources, recomputed window assertions and the
negative admission receipt are in `ray-budget-windows-v1/`. These measurements
do not select production defaults or qualify other platforms, trained policies,
full-stack contention or integrated soak.


## Conservative head-pair bound

The collision pass caches each alive snake's maximum movement/final radius and
uses it to reject separated swept head envelopes before loading full pair state.
Zero cache entries identify dead source snakes. Invalid, overflowing or
underflowing scaled radii keep the original exact checks and their errors. The
cache remains six doubles per snake and uses the existing reusable allocation.
The exact collision solver, contact tolerance and ordering remain unchanged.

A Windows P1 ABBA comparison uses eight workers, Motion/B16, modest graphs,
cap 512/batch 131,072 and 600 warmup plus 3,600 measured steps per run. Median
native mean falls from 13.001 to 12.372 ms and p95 from 15.698 to 14.900 ms.
The nested head/head phase falls from about 1.105 to 0.485 ms per step. Child CPU
medians are 197.47/198.85 seconds; these results do not establish CPU savings.
Continuation, sensor work, raw/normalized histograms and teacher bytes match
exactly between all four runs on this target; pooled incompleteness is zero.

The same candidate's 60-second Windows background-clock P1 window completes
3,603 steps at a minimum 1.000135 simulated/wall ratio, with zero dropped wall
time, overload or capture drops and p50/p95/p99 upper buckets of 16 ms. It
passes the unchanged measured throughput and step-timing criteria. It includes
durable checkpoint startup, ordinary stochastic teachers, diagnostic spool and
packed-frame copying. This is one seeded desktop window, excluding browser and
WebSocket delivery, training/device contention, shadows and integrated soak.
Production defaults remain unchanged. Exact reports, measured source bytes and
automated recomputation are retained in `head-pair-bound-windows-v1/`.


The matching Debian ABBA comparison preserves the same within-target equality
and zero pooled incompleteness. Native mean/p95 medians improve from
38.940/69.827 to 36.799/64.760 ms; child CPU medians fall from 504.60 to
497.77 seconds. The nested head/head phase is reduced, but this shared-host
workload still misses the service gate. Reports are retained unchanged in
`head-pair-bound-debian-v1/`; this is comparative evidence, without a Linux
real-time or cross-platform exact-replay claim.


CI's existing Node 24 P1 real-clock window now records the explicit
512/131,072 ray candidate on both runner targets, retaining automatic admitted
worker selection and diagnostic performance outcomes. P0 retains the existing
ray defaults. This adds no matrix job or additional clock window and does not
change application defaults.


A separate three-worker Debian probe completes the same 4,200-step workload
with exactly matching eight-worker continuation, consumer work, histograms and
teacher witnesses. Its measured mean/p95 is 57.976/111.770 ms, still failing
timing; reducing workers does not establish a service pass on this host. This
single probe is retained in the Debian directory, outside the four-run ABBA
comparison, with automated equality and negative-outcome assertions.


After the full JavaScript suite exits, the same Windows candidate/configuration
also passes a separate 60-second P0 background-clock window: 3,603 steps,
minimum sim/wall 1.000001, p95/p99 upper buckets 4/6 ms, zero dropped wall time,
overload or capture drops. Both P0 and P1 clock reports are retained and
recomputed from health reads. The same exclusions and production gates apply.


## Committed tactical corpus for ablation training

`baseline-v2-qualification` accepts optional `--corpus-directory NEW_DIRECTORY`.
It exports Motion/B16 deterministic and stochastic teacher observations/actions
from all 14 native physical fixtures, preserving each scenario/seed/mode as a
separate sequence. Samples are staged at delivery and exported after successful
complete-step physics; held/dead rows are absent. Capture leaves every tactical
trajectory/RNG witness unchanged. The existing tactical report stays compatible.

The exclusive directory contains little-endian `observations.f32` (N by 264),
`actions.f32` (N by turn/boost-request), and `steps.u32` (N delivered step IDs).
The final atomic `corpus.json` binds exact object hashes/sizes, canonical sensor
bytes/digest, cadence, native source/target and contiguous sequence ownership.
Worst-case binary output plus a 1-MiB metadata reserve must fit an independent
256-MiB offline ceiling before paths are created. Files are synced before final
publication; interrupted directories without a valid manifest are incomplete.
No runtime scratch, checkpoint or spool admission changes.

PyRL's `load_tactical_corpus` verifies this format and exposes read-only arrays;
`split_by_seed` keeps whole native trajectories out of the training split. A
small exact native fixture covers interoperability and malformed input rejection.
CI retains fresh corpora beside its existing tactical reports. This supplies
reproducible tactical training data; trained population distributions, channel
ablations, held-out agreement and student gameplay evaluation remain open.
