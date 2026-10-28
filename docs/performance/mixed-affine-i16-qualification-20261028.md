# Mixed affine-i16 / SSE2-recurrent qualification

Status: implementation, measurements, analysis and validation complete. The backend remains opt-in.
SSE2 remains the default. Production configuration is unchanged.

## Implementation and source

The distinct opt-in backend is `rust-i16-affine-fma128-sse2-v1`, selected by
`InferenceMathBackend::I16AffineFma128`. Qualification CLIs accept
`--math-backend i16-affine-fma128-sse2`; the evolved inference benchmark accepts
`--candidate i16-affine-fma128-sse2`.

Dense/MLP use the existing symmetric matrix-i16 quantizer, interleaved affine
bias handling and 128-bit FMA kernel. Entire recurrent nodes, including all
LSTM input/recurrent matrices and biases, use canonical F32 and existing SSE2
arithmetic. GRU/RRU also remain F32/SSE2 in other valid graphs. Runtime admission
requires SSE4.1 and FMA, including OS vector-state support through Rust's FMA
feature detector. AVX2 and 256-bit vectors are not required by this backend.

The compact cache belongs to the existing graph execution plan and immutable
brain bindings. Compiled offsets pack only Dense/MLP matrices. Publication of a
new population epoch or changed brain ownership rebuilds the corresponding
caches before worker dispatch; repeated evaluations reuse them. No transformed
F32 copy is retained. Canonical weights, genetic operators and checkpoint weight
encoding are unchanged. Backend identity is persisted distinctly and validated;
checkpoint/replay tests cover both opt-in variants. Existing scalar/SSE2/full-i16
identities and arithmetic remain available.

The measured source commit is
`1679938909744bb63c11259c361cb6bc3fa0bf53`, native source SHA256
`4ea6c1308f933bf613b962d61e1fa00eb6dd69513ac9910d38b8a833b4f9ae9f`.
The later native revision `c2315c5048e5d3a5ae138127e46bbcc62262a9ca` adds only a
full-layout diagnostic cache rejection guard and its regression test. Its native
source SHA256 is
`9e54ebc21df34d0b9e65a07fb74b16627791d78e4acca00c4ea55b065d3bb494`.
Production inference arithmetic is unchanged; measured and validation identities
remain distinct. No cross-build exact-replay claim or identity exception is made.
Builds use Rust 1.92.0, x86_64 Linux GNU, release/test-hooks and
`RUSTFLAGS='-C target-cpu=native'`. Build/source checks are unchanged.

## Cohort and setup

The original generation-21 cohort contains 85 genomes with 1,397,096 parameters
and 792 recurrent floats each. Checkpoint SHA256:
`f2230b4147180cdb32366a283388eac40a1bd3cacf30ca313a5470685c6be935`.
The graph is Input264 / Dense396 + LSTM396 / Concat792 / MLP264,134,2,
layout SHA256
`87d6630df32fd9fea900835db0daee478f799507d3fedb242ad63680640fbfd0`.
All measurements use seven calculation workers on the Ryzen 7 2700 Oxygen VM.
The controlled worker comparison also runs with one worker.

Inputs are the original 32 actual observations per genome from the mature SSE2
run, repeated identically for 10,000 deliveries. Motion/B16 includes body and
wall rays, with exactly 264 inputs. Each trajectory starts at its original
checkpoint state and retains its own recurrent feedback.

## Isolated inference and storage

Each backend receives 50 warmup population passes and 500 measured passes in
each of eight repetitions. Rotation and reversal balance all four backends
across timing positions. Cache conversion/swaps and numerical diagnostics are
outside timed inference. No other task-owned measured workload runs concurrently.

| Backend | Median ms/population pass | Passes/s | Speedup vs SSE2 | Trial min–max ms |
| --- | ---: | ---: | ---: | ---: |
| SSE2 F32 | 29.7126 | 33.6557 | 1.0000 | 28.4663–31.3413 |
| F32 FMA128 | 30.6998 | 32.5735 | .9678 | 29.0061–32.6820 |
| Full matrix-i16 FMA128 | 17.5287 | 57.0493 | 1.6951 | 17.0438–21.3231 |
| Mixed affine-i16 / SSE2 recurrence | 26.4261 | 37.8414 | 1.1244 | 25.4592–29.2701 |

Mixed reduced median inference time by 11.06% and increased throughput by
12.44%; it won seven of eight paired repetitions. It retains about 27% of
full-i16's inference-time reduction in this comparison. VM variation matters:
aggregate steal across the combined isolated/numerical process was 19.06%.
That aggregate is not a timing-only steal measurement. Trial ranges and paired
samples are retained; this is not a claim of a stable 12.44% gain on every pass.

Conversion for all 85 mixed caches took 181.365 ms. Cache numeric allocations
are 59,513,600 bytes (56.7566 MiB), with four scales per genome and no recurrent
placeholders. Canonical weights remain 475,012,640 bytes (453.0074 MiB).
Canonical plus mixed numeric weights therefore total 509.764 MiB. Full-i16
numeric cache storage remains 226.508 MiB. Allocation/binding metadata and engine
workspaces are additional. Cold histogram/trace memory is diagnostic overhead,
not inference-cache storage.

## Controlled recurrent-feedback results

| Comparison with SSE2 | Controller MAE | Controller RMSE | Controller maximum | Recurrent MAE / maximum |
| --- | ---: | ---: | ---: | ---: |
| Same-state paired inference | .0001156470721 | .0002526392804 | .003338046372 | 0 / 0 |
| Independent recurrent feedback | .0001156470721 | .0002526392804 | .003338046372 | 0 / 0 |

Both trajectories assert exact Float32 bits for all 673,200,000 recurrent
components each, rather than accepting zero rounded summary error. All 85
genomes pass, including the previously sensitive slots. Every genome's
controller MAE/RMSE/maximum matches the prior affine-only ablation.

| Slot | Controller MAE | Controller maximum | Hidden/cell error |
| --- | ---: | ---: | ---: |
| 24 | .0001424311502 | .001587122679 | Exactly zero |
| 30 | .00009415484152 | .001089051366 | Exactly zero |
| 35 | .00009289460736 | .001427710056 | Exactly zero |

No genome's controller maximum exceeds .01. Controlled runs with one/seven
workers have identical continuation hashes. In this graph the LSTM reads the
original input directly. Quantizing a Dense node upstream of recurrence in a
different graph can change recurrent inputs despite unchanged SSE2 arithmetic;
a focused regression test covers that distinction. Closed-loop worlds can also
produce different future observations after controller differences.

## Closed-loop whole engine

Each fresh current-source run uses 300 warmup steps and 60,000 measured
60-Hz steps (1,000 measured simulation seconds), real-clock scheduling, recorded
simSpeed 12/control interval .017 s/generation length 480 s, and 25 baseline-v2
stochastic bots at 1/60 s. Motion/B16/body/wall rays and all existing bounded
ray/detail budgets match the previous qualification. The offline ceiling is
8 GiB; production admission is unchanged. Backends run sequentially from the
same mature population and seed 42, through two ordinary generation handoffs.

| Backend | Wall seconds | Sim/wall ratio | Mean step ms | P50 step ms | P99 step ms | Mean inference ms | CPU steal |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| SSE2 | 1225.189 | 0.8162 | 20.160 | 17.619 | 43.341 | 13.168 | 9.35% |
| Mixed | 1079.184 | 0.9266 | 17.731 | 16.161 | 35.987 | 10.997 | 8.85% |
| Full matrix-i16 | 784.885 | 1.2741 | 12.846 | 11.826 | 25.790 | 6.709 | 5.50% |

| Backend | Peak RSS MiB | Neural deliveries | Baseline deliveries | Incomplete observations | Body/pellet capped observations |
| --- | ---: | ---: | ---: | ---: | ---: |
| SSE2 | 937.85 | 1,865,556 | 1,459,243 | 0 | 0 / 0 |
| Mixed | 991.80 | 1,933,713 | 1,457,926 | 0 | 0 / 0 |
| Full matrix-i16 | 1164.26 | 1,861,262 | 1,460,650 | 0 | 0 / 0 |

Mixed achieves 1.1353x simulation throughput versus SSE2 in this paired run,
with 11.92% lower wall time and 3.65% more neural deliveries. Its mean inference
phase time is 16.49% lower. Work-normalized inference wall time is retained in
the detailed analysis; it includes dispatch/batch effects and is not single-brain
latency. Closed-loop batch sizes, activation values, evolved successor genomes
and physical trajectories differ, so the isolated 1.1244x result remains the
controlled estimate of backend speedup. Both existing SSE2 and full-i16 runs
match their historical gameplay hashes and complete observation counters.

The requested simSpeed 12 saturates this setup. Reported dropped wall seconds
are 1,132.375 (SSE2), 986.215 (mixed), and 692.473 (full-i16); all measured
physics steps commit completely. Mixed does not exceed 1x simulation/wall time
in this run. P99 is reported rather than treated as a separate per-step gate.

| Backend | Completed population lives | Mean completed lifetime s | Mean completed food | Mean completed net points |
| --- | ---: | ---: | ---: | ---: |
| SSE2 | 167 | 113.973 | 9.1060 | 1.4443 |
| Mixed | 181 | 122.200 | 9.6856 | 1.4928 |
| Full matrix-i16 | 173 | 108.560 | 8.4078 | 1.3500 |

Behavior summaries include the five-second warmup. Completed lifetimes exclude
surviving censored ages, which remain separately available in source snapshots.
One seed/cohort run per backend cannot establish behavioral equivalence or a
quality ranking. Small affine output differences change later observations
and selection; controlled recurrent parity does not imply equal closed-loop
hidden/cell values after worlds separate.

Plots `performance.png`/`.svg` and `feedback-errors.png`/`.svg` are retained with
the generated artifacts, alongside aggregate and all-genome per-delivery traces.
Their numerical completeness checks and per-genome ablation comparison pass.

## Validation and decision

Windows: standard `cargo test` passes 745 library tests plus binary/doc tests.
Final release/test-hooks passes 836 library tests plus binary/doc tests. Formatting,
`cargo check`, strict all-target/all-feature Clippy and nine source-identity
unit tests pass. Three source-identity symlink tests skip for unavailable Windows
privileges. A debug hook-suite attempt hit seven existing release-profile gates;
all seven passed in the required release run.

Linux measurement-source validation passes 836 release hook library tests plus
all binary/doc tests, fmt/check and strict Clippy. Final native revision
validation passes 837 release hook and 745 default library tests plus all
binary/doc tests, fmt/check and strict Clippy. Generated Linux and Windows
source identities match the recorded final native SHA256. Automatic CI does not
run on this experiment branch; the explicit receipts are retained.

All 280 prior qualification artifacts verify against their existing manifests
on both hosts. All 22 recorded original unfinished source files are unchanged.
The original local and Oxygen owner checkouts remain at their prior commits.

This experiment keeps both opt-in i16 backends and SSE2 as the production
default. It does not change quantization domains, rounding or bias handling.
The measured tradeoff is a modest inference/engine gain with exact controlled
SSE2 recurrence and a 56.76 MiB cache. It retains about 27% of full-i16
inference-time savings. Default promotion remains a separate decision after
owner review of these results and the single-run behavioral limitations.

Reproduce on the configured Linux qualification host using the checked-in
`scripts/qualification/run-mixed-i16-qualification.py` with explicit clean
checkout and preserved-artifact paths. It refuses to replace an existing result
directory. Analyze the completed directory with
`scripts/qualification/analyze-mixed-i16.py` (NumPy/Matplotlib). Large reports,
per-genome delivery CSVs, plots and logs stay under the existing artifact root's
`mixed-affine-20261028` sibling directory; compact summaries and manifests are
retained in Git. Earlier qualification artifacts are preserved unchanged.
