# Full matrix-i16 evolutionary qualification

Full `rust-i16-matrix-fma128-v1` completed this sustained evolutionary study
without a numerical or evolutionary failure. The results support offering it
as an explicit opt-in on the tested hardware and configuration. **Keep
`rust-sse2-v1` as the default.** Five pairs from one mature cohort do not
establish general noninferiority, and the lower population medians and lifetimes
deserve follow-up before any default-selection decision.

## Scope and reproducibility

Measured native commit: `b5fb7f4cf2e904a33dfae966efbded9abf5e42ae`, based on
`a98e1aa3671e8b819acef588ccc4e23357f187dc`. Later experiment commits change
analysis, tests and compact receipts; native files remain identical to the
measured commit. Branch: `experiment/matrix-i16-evolution-20261029`.
The [predeclared protocol](full-i16-evolution-protocol-20261029.md) contains the
reproduction command; the runner builds and validates before timing begins.

Every run starts from the same generation-21 canonical F32 population, transferred
to fresh lives: 85 genomes, Input264 / Dense396 + LSTM396 / Concat792 /
MLP264,134,2; 1,397,096 parameters per genome. Motion/B16 includes body/wall rays
and supplies 264 inputs. Seven workers run on the eight-vCPU Ryzen 7 2700 Linux
VM. Rust 1.92 GNU x86_64 uses `RUSTFLAGS='-C target-cpu=native'` for qualification.
The existing runtime SSE4.1/FMA checks and 128-bit kernel remain unchanged.

Starting checkpoint SHA256:
`f2230b4147180cdb32366a283388eac40a1bd3cacf30ca313a5470685c6be935`.
Graph layout SHA256:
`87d6630df32fd9fea900835db0daee478f799507d3fedb242ad63680640fbfd0`.
Native source SHA256:
`3ab5d5172813504bbcfc55d3dd6b10ccfcc1f7ca558be3a54f6880e3ddf3ddac`.
Linux build contract:
`sha256:033119a7009787468f6cc885add333b42396edc60fc10e1a0b4d347927e587f3`.

Five paired seeds each completed 20 generations per backend, in alternating
within-pair order: 200 generation records, 5,760,200 fixed steps and 26.67
simulated hours. Every evaluated generation reached the recorded 480-second
limit (28,801 steps with existing time accounting); all ordinary boundary
checkpoints completed. Generation numbering restarts at one, and the retained
successor checkpoint is generation 21 after 20 genetic transitions. This is
population transfer, not exact continuation of the old world.
Measured processes took **27.60 wall hours** in total, including cold diagnostics:
15.52 SSE2 and 12.08 i16. No production configuration, default, mutation,
crossover, canonical weight storage, quantizer, bias policy or SIMD width changed.

## Evolutionary outcomes

Primary outcome is mean population fitness averaged over each run's final five
generations. Each row below is one independent seed pair; generations and
genomes are not independent replicates.

| Seed | SSE2 | Full i16 | i16 minus SSE2 |
|---|---:|---:|---:|
| 104729 | 2315.18 | 2741.81 | +426.63 |
| 130363 | 2240.18 | 2265.73 | +25.54 |
| 155921 | 2224.12 | 2069.00 | -155.12 |
| 196613 | 2074.63 | 2131.63 | +57.00 |
| 262147 | 2137.92 | 2117.70 | -20.23 |

The paired mean difference is **+66.77**, about +3.0% of the SSE2 mean;
exploratory paired-bootstrap 95% interval **[-76.56, +263.33]**, exact two-sided
sign-flip p=0.625. Averaging all 20 generations gives **+0.37**, interval
[-122.59, +118.14], p=1.0. These wide, small-sample summaries demonstrate
uncertainty; they do not prove equivalence or noninferiority.

| Other final-five-generation outcome | SSE2 seed mean | i16 seed mean | Paired mean difference |
|---|---:|---:|---:|
| Best fitness | 10563.12 | 11019.83 | +456.71 |
| Population median fitness | 1436.74 | 1323.10 | -113.64 |
| Food | 26.476 | 26.907 | +0.431 |
| Observed lifetime, seconds | 222.72 | 208.14 | -14.58 |
| Net points | 1.2807 | 1.2696 | -0.0111 |
| Alive/censored fraction | 20.94% | 19.11% | -1.84 percentage points |

Median fitness was lower for i16 in four pairs. Its exploratory difference
interval is [-272.73, +130.22]. Lifetime differences range from -43.77 to +12.52
seconds, with interval [-31.61, +2.04]. These secondary signals should not be
hidden behind the primary mean result. Lifetimes include survivors censored at
the common generation boundary; they are not uncensored survival estimates.
All per-generation best/mean/median scores, food, lives, turnover and reproduction
details remain in the CSV/JSONL artifacts.

Both backends improved: mean fitness across seeds rose from 1229.63 to 2541.84
for SSE2 and 1166.32 to 2448.73 for i16, comparing first and last generations.
Every population boundary retained 85 distinct canonical genomes. Reproduction
kept 10 exact elites and created 75 offspring; selected-parent counts ranged
33–46 and effective counts 20.99–32.23. Recorded configuration stayed unchanged.
Both populations converged under selection: sampled coordinate variance fell
from 0.01194 to 0.00119 (SSE2) / 0.00139 (i16). This 4096-coordinate genetic
proxy and unique hashes do not establish preservation of functional diversity;
there is no additional i16-specific collapse in these measurements.

## Quantization over time

The audit covers **204,000 matrix records**, including 68,000 affine matrix
records. Actual pre-clamp clipping and forbidden -32768 counts are **zero**.
Ordinary rounded endpoint occupancy was 1020–1027 values per population for i16
and 1020–1022 for counterfactual quantization of SSE2 populations; this is not
clipping or evidence of saturation.

Initial population relative weight RMSE is 3.3890e-5. Final i16 populations range
3.5371e-5–3.7274e-5 (+4.4%–10.0% relative growth); SSE2-evolved populations,
quantized only diagnostically, range 3.5357e-5–3.6027e-5 (+4.3%–6.3%). Errors
increase modestly as canonical weights evolve, with no runaway trend. The
largest observed i16-population weight magnitude is 1.68224, scale 5.13395e-5,
and matrix max/P99 ratio 4.044; SSE2 equivalents are 1.64270, 5.01327e-5 and
3.761. Per-matrix scales, RMS, MAE/RMSE/max/relative errors and scale setters are
retained for every boundary. Rare ordinary-weight maxima set the expected
matrix scales without driving a loss of usable precision in this study.

Only two of 34,000 i16 affine matrix records had a bias strictly setting scale:
seed155921, generations 4/5, Dense `mlp-a`, different slots/genome IDs. The bias
maximum 1.04706 versus ordinary-weight maximum 0.95460 inflated scale by 9.7%;
matrix relative RMSE remained about 3.51e-5. These are observations in adjacent
populations, not established parentage. No such cases persisted after generation
five. SSE2 seed262147 also produced one transient MLP bias-set scale, inflated
by 1.1%. None of these slots were in the fixed sampled-controller set. These
small, rare cases do not justify a bias-policy or quantizer change.

There are 3,351 common-state, one-step controller comparisons: mean sample MAE
1.0523e-4, maximum error 0.00222285; sampled one-step recurrent-state mean MAE
1.5068e-5 and maximum error 0.00088501. Samples use a recorded observation and
common **post-publication** recurrent state; they are bounded counterfactual
forwards, not independent-feedback equivalence or actual pre-control errors.
All sampled inputs/states/outputs and matrix diagnostics are finite. Missing
later strata reflect delivery/death availability. Prior independent-feedback
divergence results remain preserved and are not invalidated by this study.

## Sustained performance

| Seed | SSE2 engine seconds | i16 engine seconds | Simulation throughput ratio | Inference cost/delivery ratio |
|---|---:|---:|---:|---:|
| 104729 | 12273.22 | 9840.82 | 1.247x | 1.509x |
| 130363 | 10479.16 | 8384.45 | 1.250x | 1.475x |
| 155921 | 10797.50 | 8224.69 | 1.313x | 1.508x |
| 196613 | 9302.27 | 7928.92 | 1.173x | 1.305x |
| 262147 | 12075.56 | 8154.75 | 1.481x | 1.755x |

Median simulation-throughput gain is **1.250x**; pooled engine time gives
**1.291x**, or 22.6% less engine wall time. Median inference-phase cost per
nonterminal neural delivery improves **1.508x**. Engine time includes ordinary
checkpoint/handoff, excluding cold diagnostics. Different descendants and
lifetimes change observation workloads. Aggregate VM steal ranges 6.63%–11.06%
for SSE2 and 5.87%–8.47% for i16; all per-run deliveries, phase costs, steal and
process RSS are retained. These are measured workload gains, not a controlled
claim about pure kernel speed. No linear steal-time correction is applied.

Canonical population weights remain 453.007 MiB, with a derived i16 cache of
226.508 MiB (one scale per major matrix). Both coexist: about 679.515 MiB of
weight storage. No transformed F32 inference population was added. Compiled
assembly inspection confirmed XMM signed conversion and packed FMA without
YMM conversion/FMA in the quantized dot path.

## Validation, retained artifacts and decision

Windows and Linux passed rustfmt, cargo check, strict all-target/all-feature
Clippy, default Rust tests and release `engine-test-hooks` tests. Library test
counts are 745 default on each, 836 release-hook on Windows and 837 on Linux,
plus applicable binary/doc tests. Focused observer noninterference, diagnostic,
inference/cache/checkpoint/replay coverage passed. The paired-analysis unit
suite passes all three tests. No unrelated failures were observed. The final
data audit verifies all ten populations/settings, graph/sensors/backend/build
identities, 20 sequential generations per run, canonical final checkpoint
hashes, and unchanged native source since the measured commit.

Artifacts live under ignored `data/benchmarks/matrix-i16-evolution-20261029/`
locally and in the configured Linux artifact location. `study/` contains the
ten final checkpoints, initial audits, generation JSONL, native reports, process
receipts/logs and validation logs. Its full 94-entry SHA256 manifest is
`1839a128f733ae21dd82b0889b917baa662f6f38250b7838e5da21aae99bb0f2`,
verified on both hosts. `analysis-complete/` contains CSVs, plots, full paired
statistics and the completion audit. The committed
[artifact manifest](full-i16-evolution-artifacts-20261029.sha256) and
[validation receipt](full-i16-evolution-validation-20261029.json) identify the
retained final results. Raw payloads and plots are not committed to Git.
All 343 previous qualification entries were reverified unchanged on both hosts;
all 22 original unfinished-worktree file hashes also remain unchanged.

Answers to the qualification questions:

1. **Normal sustained evolution:** yes, within these five 20-generation runs.
2. **Systematic degradation:** none established for mean fitness; lower medians
   and lifetimes remain uncertain secondary concerns, not evidence of equivalence.
3. **Precision deterioration:** modest error growth on both populations, no runaway.
4. **Scale dominance:** ordinary maxima are expected; rare transient bias dominance
   is small and does not support redesign.
5. **Numerical/evolutionary pathology:** no observed clipping, nonfinite sampled
   state, failed generation or additional genetic collapse; functional diversity
   and rare failures remain outside the strength of this sample.
6. **Performance:** consistent measured benefit, median 1.25x whole engine and
   1.51x inference cost per delivery, subject to workload/VM variability.
7. **Normal production-selectable opt-in:** supported for this tested platform,
   graph/profile and resource envelope, retaining explicit backend identity.
8. **Future default:** further qualification is warranted. The smallest useful
   next study keeps this backend unchanged and adds independent mature cohorts
   with longer paired horizons, a predeclared acceptable loss, and primary
   population-median/lifetime/censoring comparisons alongside mean fitness.
   Exercise the actual default production build and supported Windows hardware
   before a broader promotion. Do not change scales or bias handling first.
