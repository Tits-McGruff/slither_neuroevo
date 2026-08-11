# Stage 4 coarse-SIMD inference checkpoint

These artifacts measure the first runtime-selected SIMD implementation inside
the coarse Rust heterogeneous-population operation. They were generated from
clean commit `69997a0ad7b605f47d8f83860378ace02dd4c307`. Both Rust evidence
runners and both production addons embed native source SHA-256
`6fbe621157cfd737d0bd941ea95331a7f2eddafb1b36412e9dd843ed08c728d4`.

## Method

The fixtures and process isolation match the preceding scalar checkpoint:

- P0/P1 use the source-shaped default 83-input, 13,458-parameter graph;
- P2/P3 use the 147-input, 402,914-parameter large graph;
- P0/P2 contain 55 differently weighted brains and P1/P3 contain 300;
- every brain has a distinct deterministic observation, weights, and nonzero
  recurrent state;
- P0/P1 use 20 warm-up and 120 measured passes, P2 uses 10 and 60, and P3
  uses 5 and 20; and
- Rust SSE2, Rust scalar, current TypeScript/JavaScript, and current
  TypeScript/count-one-native each run in a separate process, sequentially.

The version-2 comparator fails closed unless the four reports have the same
fixture, pass counts, normalized host facts, source identities, and supported
target. Rust scalar, Rust SSE2, and the production addon must share one native
source SHA-256. The two current-runtime reports must share one clean Git
commit. The comparator checks raw Float32 output and recurrent values directly
at the existing `1e-4` absolute tolerance, including a direct scalar-to-SSE2
and scalar-to-count-one-native comparison.

All comparisons passed. The largest absolute difference was
`8.195638656616e-8` on Windows and `9.685754776001e-8` on Oxygen. No report
used a rounded hash as a tolerance test.

## Complete-population timing

All values are milliseconds.

### Windows development machine — Ryzen 7 5800X

| Scenario | Path | p50 | p95 | p99 | Maximum |
| --- | --- | ---: | ---: | ---: | ---: |
| P0 | Rust SSE2 | 0.21 | 0.23 | 0.26 | 0.26 |
| P0 | Rust scalar | 0.38 | 0.40 | 0.47 | 0.49 |
| P0 | TypeScript/JavaScript | 0.78 | 0.90 | 0.97 | 1.27 |
| P0 | TypeScript/count-one native | 0.32 | 0.36 | 0.38 | 0.42 |
| P1 | Rust SSE2 | 1.18 | 1.30 | 1.57 | 1.79 |
| P1 | Rust scalar | 2.14 | 2.33 | 2.39 | 2.45 |
| P1 | TypeScript/JavaScript | 4.36 | 4.89 | 6.14 | 6.55 |
| P1 | TypeScript/count-one native | 2.01 | 2.38 | 2.48 | 2.49 |
| P2 | Rust SSE2 | 5.21 | 6.25 | 7.88 | 8.53 |
| P2 | Rust scalar | 13.10 | 13.51 | 13.64 | 13.65 |
| P2 | TypeScript/JavaScript | 18.70 | 19.09 | 19.64 | 19.78 |
| P2 | TypeScript/count-one native | 5.29 | 7.61 | 10.73 | 12.90 |
| P3 | Rust SSE2 | 26.91 | 27.53 | 27.85 | 27.92 |
| P3 | Rust scalar | 72.17 | 73.79 | 74.04 | 74.11 |
| P3 | TypeScript/JavaScript | 119.87 | 135.76 | 142.97 | 144.77 |
| P3 | TypeScript/count-one native | 28.58 | 35.69 | 37.96 | 38.53 |

Windows is development evidence. Its reports correctly leave
`ownerTargetVmValidated` false.

### Oxygen target VM — Ryzen 7 2700, Debian, eight assigned logical CPUs, 16 GiB

| Scenario | Path | p50 | p95 | p99 | Maximum |
| --- | --- | ---: | ---: | ---: | ---: |
| P0 | Rust SSE2 | 0.36 | 0.44 | 0.47 | 0.51 |
| P0 | Rust scalar | 0.73 | 0.78 | 0.79 | 0.79 |
| P0 | TypeScript/JavaScript | 1.39 | 1.68 | 1.89 | 1.92 |
| P0 | TypeScript/count-one native | 0.54 | 0.74 | 1.19 | 1.32 |
| P1 | Rust SSE2 | 2.19 | 2.45 | 2.65 | 2.67 |
| P1 | Rust scalar | 4.07 | 4.18 | 4.48 | 4.63 |
| P1 | TypeScript/JavaScript | 7.87 | 8.57 | 9.55 | 9.79 |
| P1 | TypeScript/count-one native | 3.53 | 4.01 | 6.36 | 7.90 |
| P2 | Rust SSE2 | 7.79 | 8.45 | 9.14 | 9.89 |
| P2 | Rust scalar | 22.16 | 23.10 | 23.47 | 23.78 |
| P2 | TypeScript/JavaScript | 31.76 | 34.02 | 39.43 | 40.47 |
| P2 | TypeScript/count-one native | 9.32 | 10.96 | 11.38 | 11.60 |
| P3 | Rust SSE2 | 41.53 | 45.07 | 46.08 | 46.33 |
| P3 | Rust scalar | 115.15 | 116.59 | 116.90 | 116.97 |
| P3 | TypeScript/JavaScript | 179.73 | 187.48 | 189.89 | 190.49 |
| P3 | TypeScript/count-one native | 42.58 | 58.03 | 88.04 | 95.54 |

Every Oxygen report passed the target-VM identity guard. The Rust SSE2 process
high-water RSS was 6.4 MiB (P0), 19.5 MiB (P1), 88.5 MiB (P2), and 466.2 MiB
(P3). Sampled TypeScript/JavaScript peak RSS was 111.2, 131.2, 273.3, and
1,032.2 MiB respectively. The count-one-native TypeScript process was 105.8,
138.6, 268.0, and 1,033.8 MiB.

## Interpretation and limits

The selected coarse Rust SSE2 path is faster than scalar Rust and both current
TypeScript-controlled paths in every retained scenario on both machines. On
Oxygen, P2 improved from 23.10 ms scalar p95 to 8.45 ms SSE2 p95, so the
inference-only path no longer misses the 16.67-ms interim budget that blocked
building sensing on top of it.

This does not close Stage 4 or prove the final P0/P1/P2 real-time targets. The
fixture is deterministic and source-shaped, not an actual fresh/evolved
population using delivered sensor observations. It excludes sensing and
spatial queries, physics, frames, Node coordination, browser/LAN control, the
RL trainer, complete fixed steps, bounded calculation workers, generation
transitions, and sustained operation. P3 remains a measured capacity case.
