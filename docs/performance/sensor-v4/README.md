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
modest/large graphs, respawn off/on, and scalar/sse2. V4 requires explicit
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
systematic maximum-64 subset per boundary. Linux reports RAM and process peak RSS;
unavailable platform probes remain null.

These runs use seeded random populations and frozen baseline v1, an immutable
run-start checkpoint with benchmark acknowledgement, and controlled clock input.
They exclude Node/browser/network, SQLite, startup, checkpoint timing and wall-debt
behavior under load. Short random runs do not qualify trained-policy distributions,
baseline v2/all-consumer workloads, tactical correctness, ablation or production
limits. Native p95 <= 16.667 ms and ordinary incomplete observations < 1% are
necessary gates. No observations produces no quality pass.

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
