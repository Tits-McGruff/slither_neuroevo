# Stage D mixed-consumer selection measurements (2026-10-25)

Motion/B16 meets the owner-selected P0 sustained-real-time and query-quality
criteria on Windows and Oxygen: no accumulating/discarded wall debt and zero
observed query/ray saturation in each active consumer class. Oxygen p99 is
reported as a 24 ms upper histogram bound, above the nominal 16.667 ms reference;
the owner explicitly removed that reference as a separate hard promotion gate.
These windows do not establish learned-policy maturity or integrated soak/capacity qualification.
P1 is an overload measurement, not a hard real-time promotion gate, as specified
by the owner in the revised plan.

## Scope and source

The existing `scripts/qualification/sensor-wall.ts` driver runs the real Rust
clock, checkpoint persistence, ten stochastic v2 teachers with diagnostic spool,
frame copying, one player controller and one bot/trainer controller. External
controllers exercise ordinary leases and accepted local delivery with fixed
opposite turns and boost off; this excludes WebSocket latency and policy quality.
P0 has 55 evolved snakes; P1 has 300. Unlimited evolved respawn and a long
generation maintain load rather than allowing the evolved population to die out.

Each window measures 60 seconds after five seconds of warmup, seed 42, cadence
1/60 s. P0 ray minimum/cap/batch are 1/256/65536. The P1 stress candidate uses
1/512/131072; it is not evidence for the smaller P0 ray tuple at P1.

Windows P0 selection and all Oxygen windows identify clean tracked source at
`d4c4910e457fdd64fbb29e306e0f893c67275417`; the earlier Windows P1 comparisons
use `c655f704ea563de5a7f6b4451eade2b3b3c92b65` (same measurement implementation).
The five-worker Oxygen run visibly records `?? node_modules`: installed
dependencies were temporarily accessed through a RAM symlink, with Node's
standard preserve-symlinks flags. No tracked source changed. The clean one- and
eight-worker comparisons use the ordinary dependency directory. All temporary
RAM paths were removed after measurement.

## Results

Ratios are conservatively bracketed simulated seconds / elapsed wall seconds.
Percentiles are histogram upper bounds, not exact durations. RSS is process
peak including startup. Every listed window has zero incomplete rays and zero
query saturation in each of neural, baseline and external classes, except the
explicitly rejected 900-check food-detail candidate described below. No native
teacher samples/chunks or spool chunks were dropped.

| Host / workload | Workers | Bins | Pellet / segment checks | Sim/wall | p99 upper ms | Discarded wall s | Peak RSS MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| Windows P0 | 5 | 16 | 3000 / 2200 | 0.99990 | 6 | 0 | 146.8 |
| Oxygen P0 | 5 | 16 | 3000 / 2200 | 1.00029 | 24 | 0 | 240.8 |
| Oxygen P0 | 8 | 16 | 3000 / 2200 | 1.00005 | 24 | 0 | 208.9 |
| Oxygen P0 | 1 | 16 | 3000 / 2200 | 0.99995 | 24 | 0 | 216.0 |
| Windows P1 | 8 | 16 | 3000 / 4000 | 0.94428 | 24 | 3.095 | 183.3 |
| Windows P1 comparison | 8 | 8 | 3000 / 4000 | 0.98459 | 16 | 0.689 | 173.7 |
| Oxygen P1 | 5 | 16 | 3000 / 2200 | 0.47683 | 64 | 31.437 | 228.9 |

The existing 900 pellet / 2200 segment candidate passed Windows P0 timing but
saturated baseline queries in 450/30011 observations (1.499%), exceeding the
1% query criterion. Neural saturation was 1137/198330 (0.573%); external was
zero. Increasing only pellet checks to 3000 removed observed saturation. This
supports further evaluation of 3000/2200 for v4, without changing recorded v3
settings or raising segment checks without evidence.

P1 continued advancing complete steps with bounded queues, retained capture and
conservative sensor validity. Oxygen achieved about 28.6 simulation steps/s.
Its final inbound queue was drained and had no admission rejections or priority
overflows. This is controlled overload in this window, not a claim about long
soaks, full storage capacity or checkpoint-generation overlaps.

## Resource coverage and limits

Each window has four contemporaneous aggregate observations at 30-second
intervals from `system-resource-observer.py`, projected by
`export-resource-counters.py`. Windows coverage includes already-running WSL
and overall GPU counters; no process inventory or distribution names are
published. Oxygen has eight available CPUs; cgroup counters report no CPU
throttling. GPU counters are unavailable there. Neither measurement proves the
physical host or hypervisor was otherwise idle.

Existing coarse Oxygen P1 slow-step counters attribute 27.8 cumulative phase seconds
to neural sensing, 5.0 to inference, 12.6 to world work and 2.4 to other work.
Nested counters overlap and cover only steps slower than 16.667 ms. They support
investigating sensing/world work before more SIMD; they do not isolate a
serialization bottleneck or justify a sensor/serialization rewrite here.

## Reproduction and retained output

Use the exact source above and a matching release `engine-test-hooks` addon,
Node 24, and the existing driver. Windows used Node 24.19.0; Oxygen used
24.12.0. Both builds used Rust 1.92.0. For the five-worker P0 candidate:

```sh
node node_modules/tsx/dist/cli.mjs scripts/qualification/sensor-wall.ts \
  --scenario P0 --profile motion --bins 16 --workers 5 \
  --seconds 60 --warmup-seconds 5 --ray-cap 256 --ray-batch 65536 \
  --pellet-checks 3000 --segment-checks 2200 --external-controllers 2 \
  --report OUTPUT.json
```

Select the matching addon with `SLITHER_ENGINE_PANIC_TEST_ADDON`. For P1 change
scenario/cap/batch to P1/512/131072; preserve the explicitly reported worker and
segment values when reproducing an individual row. Observe/export aggregate
resources alongside the run. The normal disk-admission reserve remains enabled.

Full local JSON reports and public aggregate sidecars remain in ignored
`data/benchmarks/stage-d-mixed-20261025/`. They are run output, not golden
fixtures or a new permanent archive. The manual qualification workflow retains
routine outputs through CI artifacts. This compact summary is the only new
checked-in performance record; no source snapshots or experiment bundles are
created.


## B16 tactical profile check

A bounded quality comparison at clean source
`342dba41028a57237a9570865bf585e7fdc25e06` ran the existing native tactical driver:
14 scenarios × four seeds × five policy/profile variants, at most 180 fixed
steps (three simulated seconds) per trial. Windows MSVC, Rust 1.92.0, release
`engine-test-hooks`, SSE2, one calculation worker, B16 and exact v2 cadence 1/60 s.
No corpus was generated. These are synthetic trajectories, not timing results.

```sh
cargo run --manifest-path native/Cargo.toml --release --features engine-test-hooks --bin baseline-v2-qualification -- --seed-count 4 --steps 180 --workers 1 --bins 16 --cadence '0.016666666666666666' --output OUTPUT.json
```

Survivors out of four seeds are shown below. Deterministic and stochastic v2
had the same survival counts in this batch; their actions and boost spending
were not identical. Frozen v1 uses its original v3 observation/control path.

| Scenario | Frozen v1 | Basic v2, either mode | Motion v2, either mode |
|---|---:|---:|---:|
| Open wander | 4 | 4 | 4 |
| Approaching | 0 | 0 | 4 |
| Receding | 0 | 4 | 4 |
| Crossing | 0 | 0 | 4 |
| Near head-on | 0 | 0 | 0 |
| Corridor | 4 | 4 | 4 |
| Crowd opening | 4 | 4 | 3 |
| Wall escape | 0 | 4 | 4 |
| Food/body trap | 0 | 4 | 4 |
| Smaller intercept | 4 | 4 | 4 |
| Larger threat | 4 | 4 | 4 |
| Boost escape | 4 | 4 | 4 |
| Predicted crossing | 0 | 0 | 4 |
| Deliberately saturated rays | 4 | 1 | 1 |

Across the 13 ordinary scenarios, each v2 mode survived 47/52 Motion trials
or 36/52 Basic trials, versus 24/52 for frozen v1. Motion's extra information
helped the approaching/crossing cases, with a tradeoff in crowd-opening seed 4
(body death at step 40). The near head-on fixture starts opponents about 45
units apart; every policy died by step five. Avoidability was not established.
No policy actually intercepted the smaller target within the three-second
horizon, so survival there does not establish pursuit success.

Every v2 trial reported zero privileged policy world reads, and every shared
ray budget held. Deterministic v2 consumed zero behavior draws; stochastic
Basic/Motion consumed 138/140 respectively. Ordinary cases had no incomplete
v2 observations. The stress case deliberately reduces ray limits to
minimum/cap/batch 1/1/256; each v2 variant recorded 453 incomplete observations
and three deaths across its four stress trials. The ordinary fixture limits
are 1/256/4096 for at most 16 snakes; this is not a loaded P0 budget comparison.

Together with the mixed-consumer P0 windows, this supports the selected
Motion/B16 fresh default while exposing specific limitations. Four seeded synthetic geometries, constant-control
opponents and short horizons do not establish population-policy maturity,
student quality, universal superiority or integrated soak/capacity qualification.
The scenarios and seeds remain reproducible from maintained driver source;
no failed trajectory/source bundle is added to Git.

Full output remains local at ignored
`data/benchmarks/stage-d-quality-20261025/tactical-b16-exact-cadence.json`, SHA-256
`d7e4d0d715f0afdd452056188ca9d1871ace3505c6d6c659abd73d6d8cea5983`.
