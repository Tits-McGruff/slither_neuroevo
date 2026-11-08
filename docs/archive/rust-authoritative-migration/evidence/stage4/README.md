# Stage 4 heterogeneous-inference checkpoint

These artifacts measure the first Rust complete-population inference operation
against the two current TypeScript-controlled graph paths. They were generated
from clean commit `8b6dc57b1cd1cc026d7f6142919972c6fd5aeef4`; the evidence harness was added
by `85f52eefab0d75057a2c0ffa64a94b362a9951d9`, and `8b6dc57` corrected target
hostname discovery before any target-VM result was accepted. Both Rust builds
and both production addons embed native source SHA-256
`67d8c760b7c295d82cbb44cb656f8be8b1de1891ffd30c2fcc01cf2b3257ba3e`.

## Method

P0/P1 use the source-shaped default 83-input, 13,458-parameter graph. P2/P3
use the approved 147-input, 402,914-parameter large graph. P0/P2 contain 55
brains and P1/P3 contain 300. Every brain has distinct deterministic synthetic
weights and observations plus nonzero recurrent state. These are inference-only
fixtures, not actual evolved genomes or delivered sensor observations.

Each path ran in a separate process, sequentially:

- P0/P1: 20 warm-up passes and 120 measured complete-population passes;
- P2: 10 warm-up passes and 60 measured passes;
- P3: 5 warm-up passes and 20 measured passes.

The Rust timing covers the complete scalar heterogeneous-population operation
and staged recurrent commit, with no N-API calls. The TypeScript timings cover
the complete current per-brain graph loop. The current native path performs
three count-one calls per brain—165 calls for P0/P2 and 900 for P1/P3.

Every report retains the same raw one-step Float32 outputs and recurrent state.
The role-validating comparator checks every value directly at the existing
`1e-4` absolute backend tolerance. All comparisons passed; the largest observed
absolute difference was below `9.69e-8`. Rounded hashes are not used as a
tolerance test.

## Complete-population timing

All values are milliseconds.

### Windows development machine — Ryzen 7 5800X

| Scenario | Path | p50 | p95 | p99 | Maximum |
| --- | --- | ---: | ---: | ---: | ---: |
| P0 | Rust scalar | 0.50 | 0.72 | 0.75 | 0.76 |
| P0 | TypeScript/JavaScript | 0.79 | 0.91 | 1.01 | 1.17 |
| P0 | TypeScript/count-one native | 0.32 | 0.40 | 0.58 | 0.72 |
| P1 | Rust scalar | 2.81 | 3.12 | 4.85 | 4.87 |
| P1 | TypeScript/JavaScript | 4.67 | 6.04 | 6.66 | 6.76 |
| P1 | TypeScript/count-one native | 2.03 | 2.69 | 3.05 | 3.11 |
| P2 | Rust scalar | 14.10 | 14.66 | 16.72 | 19.03 |
| P2 | TypeScript/JavaScript | 18.98 | 20.15 | 23.19 | 24.13 |
| P2 | TypeScript/count-one native | 5.02 | 5.64 | 6.38 | 6.73 |
| P3 | Rust scalar | 77.06 | 79.30 | 81.50 | 82.05 |
| P3 | TypeScript/JavaScript | 134.44 | 145.82 | 145.92 | 145.95 |
| P3 | TypeScript/count-one native | 30.15 | 35.93 | 38.84 | 39.57 |

Windows is development evidence. Its reports correctly leave
`ownerTargetVmValidated` false.

### Oxygen target VM — Ryzen 7 2700, Debian, eight assigned logical CPUs, 16 GiB

| Scenario | Path | p50 | p95 | p99 | Maximum |
| --- | --- | ---: | ---: | ---: | ---: |
| P0 | Rust scalar | 0.93 | 1.07 | 1.15 | 1.16 |
| P0 | TypeScript/JavaScript | 1.43 | 1.82 | 2.19 | 2.34 |
| P0 | TypeScript/count-one native | 0.53 | 0.65 | 0.74 | 0.79 |
| P1 | Rust scalar | 5.00 | 5.69 | 6.27 | 6.71 |
| P1 | TypeScript/JavaScript | 7.78 | 8.27 | 8.61 | 8.66 |
| P1 | TypeScript/count-one native | 3.45 | 3.83 | 4.43 | 4.45 |
| P2 | Rust scalar | 25.57 | 26.45 | 27.86 | 28.16 |
| P2 | TypeScript/JavaScript | 32.96 | 35.94 | 36.42 | 36.52 |
| P2 | TypeScript/count-one native | 8.00 | 8.76 | 8.94 | 9.10 |
| P3 | Rust scalar | 138.17 | 142.04 | 142.19 | 142.22 |
| P3 | TypeScript/JavaScript | 173.54 | 184.15 | 196.93 | 200.13 |
| P3 | TypeScript/count-one native | 42.30 | 45.51 | 45.84 | 45.92 |

The target-VM identity guard passed for every Oxygen report. Observed process
high-water RSS for Rust scalar was 6.5 MiB (P0), 19.4 MiB (P1), 88.5 MiB (P2),
and 466.2 MiB (P3). The corresponding TypeScript processes retained roughly
107–111 MiB, 133–138 MiB, 272 MiB, and 1,032–1,034 MiB respectively.

## Interpretation and limits

The scalar Rust operation is correct, coarse, more memory-efficient, and faster
than the TypeScript/JavaScript graph. It is not yet the selected fast path. On
Oxygen, scalar P2 takes 26.45 ms p95, while the existing SIMD kernels complete
the same synthetic neural arithmetic in 8.76 ms p95 despite 165 language
crossings. Stage 4 therefore proceeds to scalar-parity-gated SIMD inside the
coarse Rust operation before sensing is built on top of this hot path.

This checkpoint does not close Stage 4 or prove the final P0/P1/P2 real-time
targets. It excludes actual fresh/evolved populations, real delivered sensors,
sensing and spatial queries, physics, frames, Node coordination, browser/LAN
control, the RL trainer, complete fixed steps, parallel calculation workers,
and sustained generation behavior. P3 remains a measured capacity case.
