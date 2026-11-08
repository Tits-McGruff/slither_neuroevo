# Stage 4 combined neural-control checkpoint

These artifacts measure the first complete Rust control boundary that joins
the corrected sensor-v3 and spatial-index path to the complete heterogeneous
population graph executor. They were generated from clean commit
`9c38b487d07ce3ec88794a5cd793c2cf7624cc04`. Both machines embed native source
SHA-256 `1ffbbeabeb6d133402b34e99f21615b26c216f184fe43bb2ba453e0d1282a772`.

## Method

Each measured boundary performs this sequence in one Rust process:

1. build complete immutable body and pellet indexes from one stable world;
2. construct corrected delivered sensor-v3 observations for every due evolved
   snake;
3. evaluate the complete differently weighted, stateful graph population;
4. destroy the borrowed index view; and
5. atomically commit every observation-delivery marker and recurrent block.

There are no Node, TypeScript, worker-thread, N-API, per-snake, per-node, or
per-layer crossings. Activation capture is disabled. Every process performs 20
warm-up boundaries, 120 individually measured stateful boundaries, and one
extra untimed proof boundary. The proof boundary begins with one deliberately
nonzero delivered-points delta per due snake, hashes all Float32 observations
and outputs, records sensor work, and verifies that all delivery markers commit.
Recurrent state commits after every boundary and becomes the next boundary's
input.

The deterministic source-shaped fixtures are:

- P0: 55 due evolved brains, 10 baseline snakes, five-point bodies, 3,500
  pellets, and the default 16-bin/83-input/13,458-parameter graph;
- P1: 300 due evolved brains plus 10 baselines with the P0 body, pellet, sensor,
  and graph shape;
- P2: 55 due evolved brains, 10 baselines, five-point bodies, 3,500 pellets,
  and the supported 32-bin/147-input/402,914-parameter large graph; and
- P3: 300 due evolved brains plus 10 baselines with the P2 sensor and graph
  shape.

All weight blocks and initial recurrent blocks are independently generated and
the runner proves that every block is unique. Baseline snakes are present in
sensor truth but are not neural candidates in this checkpoint.

## Complete-boundary timing

All values are milliseconds. The phase p95 columns are diagnostic; the complete
distribution is measured directly and includes phase bookkeeping and index
destruction rather than being reconstructed by adding percentiles.

### Windows development machine — Ryzen 7 5800X

| Scenario | p50 | p95 | p99 | Maximum | Index p95 | Sensing + inference p95 | Commit p95 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| P0 | 2.395 | 2.678 | 2.896 | 2.917 | 0.263 | 2.432 | 0.002 |
| P1 | 12.934 | 13.417 | 15.397 | 15.912 | 0.292 | 13.143 | 0.009 |
| P2 | 7.326 | 7.700 | 8.292 | 8.451 | 0.302 | 7.396 | 0.007 |
| P3 | 39.187 | 40.513 | 41.510 | 44.224 | 0.399 | 40.067 | 0.031 |

Windows is development evidence. Its reports correctly leave target-VM
validation false, and this runner does not currently retain Windows RSS.

### Oxygen target VM — Ryzen 7 2700, Debian, eight assigned logical CPUs, 16 GiB

| Scenario | p50 | p95 | p99 | Maximum | Index p95 | Sensing + inference p95 | Commit p95 | Peak RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| P0 | 4.061 | 4.649 | 5.059 | 5.103 | 0.351 | 4.273 | 0.002 | 7.4 MiB |
| P1 | 23.821 | 28.708 | 29.640 | 31.325 | 0.641 | 28.056 | 0.025 | 20.4 MiB |
| P2 | 11.825 | 13.825 | 15.033 | 15.567 | 0.413 | 13.462 | 0.011 | 89.2 MiB |
| P3 | 65.805 | 71.972 | 76.176 | 127.884 | 0.643 | 71.417 | 0.074 | 466.8 MiB |

Every Oxygen report passed the Linux, Debian, hostname, CPU, assigned-thread,
and memory guard. The measured process occupied approximately one CPU core,
as expected for this deliberately single-worker checkpoint.

## Allocation and correctness results

After warm-up, sensing plus inference and recurrent/delivery commit made zero
allocator operations in all 960 measured machine/scenario boundaries. Complete
index construction made seven allocation operations per P0/P2 boundary and
eight per P1/P3 boundary. All pipeline and sensor-query capacities remained
unchanged after warm-up.

The evidence harness initially exposed 440 eager error-string allocations per
P0 evaluation. Commit `9c38b48` changes successful checked graph-range access
to construct those strings only on error and adds a process-allocator
regression. The retained reports were produced only after that correction.

Across Windows and Oxygen, every scenario has identical source, graph, weights,
initial recurrent state, index counts, query-work counts, retained capacities,
and Float32 observation digest. The synthetic world digest differs because the
fixture uses platform `f64` trigonometric placement before sensing; the final
Float32 observations are nevertheless identical. Output and long-running
recurrent hashes are platform-specific, consistent with the plan's explicit
numeric-tolerance contract rather than a cross-platform bit-identity promise.
The earlier retained scalar/SIMD evidence provides the direct tolerance checks.

## Interpretation and limits

P0 and P2 clear the 16.67-ms interim Stage 4 budget on Oxygen in this
single-worker combined operation. P1 does not: its 28.708-ms p95 remains an
open mandatory performance miss. The approved next correction is complete-step
profiling and bounded stable-handle Rust calculation workers. Sensors, collision
truth, population size, and physics must not be weakened to make the number
pass. P3 remains a measured capacity case.

This checkpoint satisfies the Stage 4 functional boundary: Rust produces real
corrected observations and complete heterogeneous controller outputs without
TypeScript or N-API work in the hot loop. It is not a complete game step or a
production-cutover result. It excludes controller-output selection, movement,
food, collision resolution, spawning, frames, Node coordination, the browser,
the RL trainer, generation transitions, sustained operation, and parallel
calculation workers. The worlds and genomes are deterministic source-shaped
fixtures rather than owner saves or evolved runs.
