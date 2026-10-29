# Mature matrix-i16 recurrent-feedback investigation

Status: the full three-way comparison, matrix-group ablations, internal LSTM
traces and finite-prefix causal controls are complete.
SSE2 remains the production default; this study does not promote any backend.

## Source and retained inputs

The requested baseline is `642cfbc96f492882e6a8bd60ee40e0622abbae23`.
The feature branch's rebased commit `b2c72b019316464586b178baf366c50f1e55455e`
has an identical complete source tree. Investigation commits extend that branch.
The full comparison ran at `7279ad6723f28405bc54ef7c997a9d5a4932166e`, with
native source SHA256 `acab13970d5b0693fd4a4a88633b3a246fbed478472b14730c77967137c34cc3`.
It used Rust 1.92.0, x86_64 Linux GNU, release/test-hooks,
`RUSTFLAGS='-C target-cpu=native'`, and the original build contract
`sha256:033119a7009787468f6cc885add333b42396edc60fc10e1a0b4d347927e587f3`.
The later trace/prefix binary was built at
`3a7bfba8f10b68ed090c248faaa4d381a3c31396`, native source SHA256
`0c5e925ed5d469cf2ff544e674a3ea9b8ac33096feea17e4c4286a64079565ce`.
Its changes add cold prefix/label support and lint fixes; production arithmetic
is unchanged. Later runner/analysis/documentation commits retain that native
tree. Full versus traced errors/outputs match exactly for all 10,000 deliveries,
all seven comparisons and all three sensitive slots.

The graph is the owner's exact Input264 / Dense396 + LSTM396 / Concat792 /
MLP264,134,2 graph, layout SHA256
`87d6630df32fd9fea900835db0daee478f799507d3fedb242ad63680640fbfd0`.
The generation-21 cohort has 85 genomes, 1,397,096 parameters/genome, and 792
recurrent floats/genome. Its checkpoint file SHA256 is
`f2230b4147180cdb32366a283388eac40a1bd3cacf30ca313a5470685c6be935`;
the retained logical root is
`a99cedbfa8fb3d4f7ef028472cb460713ee78f4ed813f1f65122cc0c7feaa160`.

Each genome starts from its original checkpoint state and receives the same
original 32 actual Motion/B16/ray observations, repeated cyclically for 10,000
deliveries. Each observation has 264 inputs. Every trajectory retains its own
Float32 recurrent feedback. The comparison uses seven workers on the original
Ryzen 7 2700 Oxygen VM. There is no new world simulation or altered observation
stream in this experiment.

## Full-cohort numerical results

Errors below compare independent trajectories with SSE2. MAE/RMSE include all
1,700,000 controller components. Maximum errors are actual component maxima,
not maxima of window averages. Ablation names are experimental modes, not new
persisted backend identities. Unquantized ablation matrices use existing SSE2.

| Candidate | Controller MAE | Controller RMSE | Controller max | Genomes max > .01 | Genomes max > .1 |
| --- | ---: | ---: | ---: | ---: | ---: |
| F32 FMA128 | 1.59585e-6 | 1.73289e-5 | .00239709 | 0 | 0 |
| All matrix-i16 | .000546567 | .00574741 | .890036 | 14 | 6 |
| i16 Dense/MLP only | .000115647 | .000252639 | .00333805 | 0 | 0 |
| i16 LSTM only | .000492458 | .00574312 | .891057 | 14 | 6 |
| i16 LSTM input matrices only | .000380865 | .00601206 | 1.357323 | 15 | 3 |
| i16 LSTM recurrent matrices only | .000380285 | .00588803 | 1.123133 | 9 | 3 |

All-i16 compared directly with F32-FMA128 has controller MAE .000547138,
RMSE .00575766, and maximum .889241. Changing the reference to FMA does not
remove the large i16 excursions.

Affine-only quantization has exactly zero hidden/cell error across all 85
genomes and 10,000 deliveries. LSTM-only quantization reproduces the all-i16
recurrent error series because this graph's LSTM receives the original input
directly. Input and recurrent quantization can each cause separation; their
effects are not additive or monotone.

| Slot | F32-FMA128 max | All-i16 max | Affine-only max | LSTM input-only max | LSTM recurrent-only max |
| --- | ---: | ---: | ---: | ---: | ---: |
| 24 | .00239709 | .678973 | .00158712 | .0519248 | .729341 |
| 30 | .000528812 | .512600 | .00108905 | .327981 | .246500 |
| 35 | .00184216 | .890036 | .00142771 | 1.357323 | 1.123133 |

The original completed qualification is reproduced: controller MAE and maximum
match exactly; recurrent maximum matches exactly; combined recurrent MAE differs
by approximately 1e-19 because this analysis sums hidden and cell errors
separately. The original qualification artifacts are not rewritten.

## Onset and internal attribution

For investigation purposes, persistent onset means the first 128-delivery
window containing at least 96 deliveries above a stated RMS threshold. This is
not a backend admission tolerance. The full traces retain first crossings,
window boundaries, maxima and fractions above several thresholds, including
cases without persistent controller separation.

For all-i16 hidden RMS > .001, first crossings are deliveries 601 / 40 / 238
for slots 24 / 30 / 35; first qualifying persistent windows begin at
1804 / 1381 / 228. A window can begin before its first crossing. None of these
three F32-FMA128 trajectories crosses hidden RMS .001. Controller RMS > .01
has intermittent crossings in the i16 trajectories but does not meet the
96-of-128 persistence criterion in these slots.

The test-hooks tracer captures input and recurrent dot terms separately, gate
preactivations/outputs, previous and next cell/hidden states, and controller
outputs. It also evaluates SSE2 on each candidate's own prior state. This
decomposes preactivation error into fresh arithmetic error and propagated state
error. It retains both cells with large cell-state differences and cells with
large hidden-output differences; a large saturated cell-state difference alone
does not establish the cause of a controller excursion.

The 1,945 main internal captures contain every cell and gate in all seven modes,
plus SSE2 counterfactuals on each mode's own prior state. Input-dot terms in the
SSE2 counterfactuals match the reference exactly; recurrent biases match across
all modes. There are no changed sensors, bias conventions or nonlinear functions.

At the large controller peaks, fresh preactivation error is much smaller than
the error propagated through the different previous recurrent states:

| Slot / peak delivery | Max fresh preactivation error | Max propagated preactivation error | Largest hidden-error cell | Reference / candidate next cell | Reference / candidate next hidden |
| --- | ---: | ---: | ---: | --- | --- |
| 24 / 9437 | .000307798 | .142070 | 201 | .206506 / -.137903 | .197118 / -.132653 |
| 30 / 8083 | .000368357 | .102307 | 136 | -.535132 / -.188685 | -.437079 / -.166077 |
| 35 / 6764 | .000395060 | .332729 | 303 | -.447249 / .224393 | -.412541 / .217091 |

This is gradual accumulation/carried-state drift with intermittent nonlinear
trajectory separation, rather than a single large erroneous dot product. Slots
24 and 30 show a growing, periodically modulated error envelope. Slot 35 has
stronger intermittent bursts and recovery. A 32-observation forcing cycle does
not imply a fixed 32-delivery recurrent orbit; the reference states continue
moving between observation cycles.

For slot 24 cell 201 at delivery 9437, the cell difference is -.344409: the
carried-memory contribution is -.322403 and changed candidate drive contributes
-.0220035. Its forget gate is .983866 / .983698, retaining most of the prior
difference. Slot 30 cell 136 similarly has carried-memory contribution .307212
and changed candidate drive .0394192. Slot 35 cell 303 has .518553 from carried
memory and .149443 from changed candidate drive. F32 update-rounding residuals
in these decompositions are approximately 1e-8, rather than the source of the
large differences.

The controlling cells pass through the unsaturated part of `tanh(cell)`. Their
hidden outputs can separate or change sign, and the downstream MLP amplifies
that difference into a large turn difference. Candidate-gate differences are
larger than the other gate differences near these excursions. This does not
establish a particular candidate-gate matrix as the unique causal parameter:
all gates and recurrent features remain coupled. The nonlinear functions are
smooth; there is no hard gate discontinuity. Some larger absolute cell-state
differences are in saturated cells: for example, a slot 24 cell is -73.09 versus
-74.69, which contributes little hidden-output difference. Ranking cells only
by cell-state error would misidentify that controller mechanism.

## Finite-prefix causal controls

These controls use i16 for one delivery or one 32-observation cycle, then execute
only canonical F32/SSE2 for the rest of the trajectory. After the switch the
candidate/reference have identical weights and arithmetic; only state differs.
Unchanged comparator/ablation trajectories and every pre-switch delivery match
the original full run exactly.

| Slot | One-delivery prefix: controller max | 32-delivery prefix: controller max | 32-prefix peak delivery | Continuous-i16 max |
| --- | ---: | ---: | ---: | ---: |
| 24 | .000436604 | .00892282 | 222 | .678973 |
| 30 | .000712901 | .0171747 | 72 | .512600 |
| 35 | .00547245 | .981848 | 8683 | .890036 |

Slots 24 and 30 retain much smaller errors after a finite prefix than under
continuous quantization. Their last-1000-delivery controller MAEs after the
32-prefix are .0000416238 and .000426460, versus .000169887 and .000488076 in
their first 1000 deliveries. Continuing to use perturbed matrices is important
for the growing error envelope in those trajectories.

Slot 35 is different. At delivery 32 its state perturbation has hidden RMS
`3.40045e-5` and cell RMS `.000423888`. Deliveries 33 through 10,000 then use
only canonical F32/SSE2, yet controller error later reaches `.981848` at delivery
8683. Its last-1000 controller MAE is .00596738 versus .00128543 in the first
1000. A further 226 internal captures around the switch, first state crossing
and late peak verify that fresh preactivation error is exactly zero in every
captured post-switch gate record. At the peak, propagated preactivation error
is .270570. Cell 303's previous cells are .0266662 / -.522244, next cells
-.188260 / -.666546 and next hidden values -.180038 / -.562696. The resulting
turn outputs are -.603953 / .377895.

Thus, large trajectory separation is not exclusive to ongoing i16 execution:
slot 35's canonical SSE2 dynamics can amplify a sufficiently directed small
finite state perturbation. However, the ordinary F32-FMA128 perturbation does
not produce comparable large excursions in this 10,000-delivery experiment.
Sensitivity depends on genome, perturbation size/direction and horizon. These
observations are not a formal proof of chaos or a claim about every possible
arithmetic perturbation.

## Smallest next implementation worth testing

Keep the entire LSTM in F32/SSE2 and quantize only Dense/MLP in a separate opt-in
mixed candidate, then benchmark its actual speed and qualify its world behavior.
That numerical ablation has zero recurrent divergence here and controller max
.00333805. Merely leaving input or recurrent LSTM matrices unquantized is not
enough: either complementary quantized group can independently produce large
slot 35 excursions. The affine matrices comprise only about 25% of this graph's
parameters, so the original all-i16 throughput gain cannot be assumed for the
mixed candidate. No row/block scaling, bias change or production mixed backend
is introduced by this investigation.

SSE2 remains the default. Further diverse observation streams and behavioral
qualification are still needed for any candidate; this repeated-bank study
isolates arithmetic/state sensitivity rather than establishing live-game parity.

## Validation

Windows validation passed: rustfmt, `cargo check`, ordinary `cargo test` (741
library tests plus the doctest), release/test-hooks library tests (830), and
Clippy across all targets/features with warnings denied. Oxygen validation
passed: rustfmt, `cargo check`, release/test-hooks library tests (831), the
investigation executable's unit test, and Clippy across all targets/features
with warnings denied. The test-hooks LSTM tracer matches the ordinary SSE2,
F32-FMA128 and matrix-i16 endpoints in both the small-shape unit test and the
mature-cohort captures. All new Python tools pass syntax compilation.

All six experiment/validation phases completed successfully; that status means
the commands and structural/numerical consistency checks completed, not that
matrix-i16 passed a default-promotion gate. No remaining test failure was found.

## Artifacts and reproduction

Generated results remain outside Git under the preserved qualification root:
`data/benchmarks/matrix-i16-evolved-20261027/exact-396-motion-b16-v3/feedback-investigation-20261028/`.
It contains per-genome compressed per-delivery CSVs, aggregate error-over-time
CSV, per-genome 100-delivery window CSV, complete native/analysis JSON and PNG/PDF
plots. The detailed internal JSONL traces are gzip compressed. The existing
checkpoint and original reports remain in the parent directory.
Plots include `sensitive-genome-feedback`, `all-genome-ablation`,
`finite-prefix-feedback` and `lstm-error-decomposition`, each as PNG and PDF.

The full comparison's native `full/report.json` SHA256 is
`4b4653c0ab2b2fae7e0985f4b6d65cffb52b51ceb0ffc91bf71a75c3ba7b9b6a`.
The analysis summary SHA256 is
`f02bb7185d7992588c032b76605e555cc8ffe0c93930df505fda919f62f570f6`.
Large traces/checkpoints are retained as local/target-host artifacts, not Git
payloads or source snapshots.
The companion [SHA256 manifest](mature-matrix-i16-feedback-20261028.sha256)
identifies 265 retained new result files. Mutable progress/cleanup records are
excluded from those scientific artifact checksums.

The ordinary test-hooks executable can be run against the retained inputs with
a new output directory:

```sh
RUSTFLAGS='-C target-cpu=native' cargo run --manifest-path native/Cargo.toml \
  --release --features engine-test-hooks --bin recurrent-feedback-investigation -- \
  --checkpoint "$MATURE_CHECKPOINT" --observation-bank "$ORIGINAL_OBSERVATION_REPORT" \
  --workers 7 --deliveries 10000 --output "$NEW_OUTPUT_DIRECTORY"
```

The task runner `scripts/qualification/run-recurrent-feedback.py` handles the
full, trace and finite-prefix phases. Analysis and plotting live beside it.
Output directories refuse replacement. The production kernels, quantizer,
bias handling, canonical genomes, checkpoint format and default selection are
unchanged.
