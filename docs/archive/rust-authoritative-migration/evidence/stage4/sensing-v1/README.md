# Stage 4 corrected sensing and spatial-index checkpoint

These artifacts measure the corrected Rust sensor-v3 path and the complete
body/pellet indexes that feed it. The optimized reports were generated from
clean commit `07b0ae7c8967c484db18d385a3f8f6ee0a4df97a`; both machines embed
native source SHA-256
`f740df61ad4ca46703afcf23e898ec8e4ba63b614c805bc60620cf9a81b05856`.
The retained Oxygen before-results came from clean implementation commit
`5a4d09384fef7d1d4e29ed7bf5913db197776f51`.

## Method

Every report runs the real corrected Rust evaluator over one immutable indexed
world view. The source-shaped deterministic fixtures are:

- P0: 55 evolved snakes, 10 baseline bots, five-point bodies, 3,500 pellets,
  and the 16-bin/83-input sensor layout;
- P1: 300 evolved snakes plus 10 baseline bots with the P0 body, pellet, and
  sensor shape;
- P2: the P0 world load with the supported 32-bin/147-input sensor layout;
- dense-body: 300 evolved snakes plus 10 baseline bots, 700 body points per
  snake, and 12,000 pellets; and
- dense-pellet: 55 evolved snakes plus 10 baseline bots, five-point bodies,
  and the configured maximum 25,000 pellets.

Each optimized process performs 20 warm-up passes and 120 measured passes. The
retained pre-optimization P0/P1/P2 reports use the same counts; dense-body uses
5/30 and dense-pellet uses 10/60 because those earlier passes are materially
longer. Index timing rebuilds complete body and pellet indexes. Sensing timing
samples every live snake, including all scalar fields, food/body/wall/head
bins, nearest values, work-cap diagnostics, and delivery markers. The
standalone allocator wrapper counts real allocation operations. The proof pass
records query work and hashes every Float32 observation outside the timed
samples.

The optimized index adds a bounded direct cell lookup only when the coordinate
envelope is sufficiently small and dense; sparse or overflowing envelopes use
the complete sorted-span fallback. Candidate retention delays heap construction
until a sensor cap is actually filled. Pellet records use stable-ID rank instead
of copying an ID into every candidate, and irrelevant far-away head checks avoid
square roots. Complete collision-oriented queries remain uncapped.

## Optimized sensing timing

All values are milliseconds for one complete population sensing pass.

### Windows development machine — Ryzen 7 5800X

| Scenario | p50 | p95 | p99 | Maximum | Index p95 |
| --- | ---: | ---: | ---: | ---: | ---: |
| P0 | 2.27 | 2.52 | 2.57 | 2.65 | 0.29 |
| P1 | 11.49 | 11.94 | 12.15 | 12.60 | 0.40 |
| P2 | 2.29 | 2.37 | 2.60 | 2.69 | 0.23 |
| dense-body | 54.91 | 56.99 | 59.32 | 75.76 | 22.87 |
| dense-pellet | 4.79 | 5.15 | 5.47 | 5.59 | 1.90 |

Windows is development evidence. Its reports correctly leave target-VM
validation false, and this runner does not currently retain Windows RSS.

### Oxygen target VM — Ryzen 7 2700, Debian, eight assigned logical CPUs, 16 GiB

| Scenario | p50 | p95 | p99 | Maximum | Index p95 | Peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| P0 | 3.92 | 4.14 | 5.05 | 5.79 | 0.36 | 4.1 MiB |
| P1 | 19.60 | 23.75 | 30.33 | 31.10 | 0.65 | 4.2 MiB |
| P2 | 4.19 | 4.87 | 4.93 | 4.94 | 0.33 | 4.0 MiB |
| dense-body | 88.92 | 103.47 | 107.34 | 109.86 | 39.14 | 27.7 MiB |
| dense-pellet | 8.28 | 9.43 | 10.08 | 10.23 | 2.66 | 6.6 MiB |

Every Oxygen report passed the target identity guard and measured approximately
one fully occupied CPU core. Warmed sensing made zero allocator operations in
all 600 measured passes. Index construction made seven allocations for P0, P2,
and dense-pellet and eight for P1 and dense-body; the extra bounded lookup
storage is included in the reported index byte estimates.

## Retained before/after comparison on Oxygen

The proof observation digest and every query-work count remain identical
between the two clean commits.

| Scenario | Before p95 | Optimized p95 | Change |
| --- | ---: | ---: | ---: |
| P0 | 7.43 | 4.14 | 44.3% faster |
| P1 | 36.87 | 23.75 | 35.6% faster |
| P2 | 7.03 | 4.87 | 30.7% faster |
| dense-body | 113.18 | 103.47 | 8.6% faster |
| dense-pellet | 12.19 | 9.43 | 22.7% faster |

The optimized P1 result still exceeds the 16.67-ms interim single-step budget.
That miss remains open. Two additional output-preserving experiments—linear
dot-product bin search and polar-boundary bin search—were slower and were not
retained. The final mandatory P1 target still requires complete-step profiling
and the approved bounded Rust calculation-worker work; this evidence does not
claim that parallel result in advance.

## Correctness and limits

For each scenario, Windows and Oxygen have identical native source identity,
fixture dimensions, query-work counts, capacity/cap behavior, and Float32
observation digest. The fixture world digest itself differs across the two
platforms because fixture placement uses platform `f64` trigonometric
functions before values enter the sensor path. That is recorded rather than
hidden; the resulting Float32 observations are nevertheless identical.

Dense-body deliberately reaches both sensor-only caps for all 310 snakes. It
reports 310 pellet-cap hits and 310 body-cap hits, conservatively saturates body
hazards, and performs no detailed segment calculations after saturation. The
collision-oriented body index still contains all 243,617 required cell entries;
sensor caps do not truncate collision truth.

This checkpoint is sensing/index evidence, not a complete game-step or cutover
result. It does not include graph inference in the same timed pass, movement,
food ownership, collision commit, frames, Node coordination, a browser, the RL
trainer, generation transitions, or sustained operation. The worlds are
deterministic source-shaped fixtures rather than owner saves or evolved runs.
