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
