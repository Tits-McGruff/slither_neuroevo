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
necessary gates. No observations produces no quality pass. The fixture uses
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

P1 fails both necessary gates on this development machine. This actual workload
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

Basic/B16 meets these two necessary gates in this short development run; it
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
fails both necessary 1x timing and ordinary-quality gates in this workload.
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

P1 remains over the necessary 16.667 ms p95 bound on both platforms and retains
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
