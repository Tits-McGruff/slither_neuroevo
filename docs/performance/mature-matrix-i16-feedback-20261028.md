# Mature matrix-i16 recurrent-feedback investigation

Status: the full three-way comparison and matrix-group ablations are complete.
Internal LSTM traces and finite-prefix causal controls are still being collected.
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

Finite-prefix controls use i16 for one delivery or one 32-observation cycle,
then execute only canonical F32/SSE2 for the rest of the trajectory. These test
whether a small state perturbation alone persists or whether continuing to use
changed matrices is necessary for the observed growth. Their results and the
internal mechanism will be added after collection.

## Artifacts and reproduction

Generated results remain outside Git under the preserved qualification root:
`data/benchmarks/matrix-i16-evolved-20261027/exact-396-motion-b16-v3/feedback-investigation-20261028/`.
It contains per-genome compressed per-delivery CSVs, aggregate error-over-time
CSV, per-genome 100-delivery window CSV, complete native/analysis JSON and PNG/PDF
plots. The detailed internal JSONL traces are gzip compressed. The existing
checkpoint and original reports remain in the parent directory.

The full comparison's native `full/report.json` SHA256 is
`4b4653c0ab2b2fae7e0985f4b6d65cffb52b51ceb0ffc91bf71a75c3ba7b9b6a`.
The analysis summary SHA256 is
`f02bb7185d7992588c032b76605e555cc8ffe0c93930df505fda919f62f570f6`.
Large traces/checkpoints are retained as local/target-host artifacts, not Git
payloads or source snapshots.

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
