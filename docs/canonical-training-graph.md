# Canonical training graph tool

The offline Rust tool exports a training manifest, initializes or validates one
flat canonical Float32 weight vector, and evaluates a contiguous sequence with
the deployed whole-graph executor. It owns no world or controller and does not
advance any live gameplay RNG. This is Stage F's Rust contract. The corresponding
[PyRL canonical graph and optimizer roundtrip](https://github.com/Tits-McGruff/PyRL-trainer/commit/6e3bdd840389594db1387ca0d23eed76e5968dda)
passes CPU/RTX4080 parity with both native math backends. Bounded sequence training,
teacher transport and normal deployed-candidate acceptance remain in progress.

Build and run from the repository root:

```powershell
cargo build --manifest-path native/Cargo.toml --release --features engine-test-hooks --bin canonical-training-graph
native/target/release/canonical-training-graph.exe request.json new-result-directory
```

On Linux the executable has no `.exe` suffix. The destination directory must not
exist. Inputs are read and validated before the tool claims the destination;
existing results are never overwritten. Relative input paths resolve beside the
request file. A request is bounded to 1 MiB and rejects unknown fields.

Example request for frozen v3, eight bins, one affine head:

```json
{
  "schemaVersion": 1,
  "graphSpec": {
    "type": "graph",
    "nodes": [
      {"id": "input", "type": "Input", "outputSize": 51},
      {"id": "head", "type": "Dense", "inputSize": 51, "outputSize": 2}
    ],
    "edges": [{"from": "input", "to": "head"}],
    "outputs": [{"nodeId": "head"}],
    "outputSize": 2
  },
  "sensorProfile": "v3",
  "bins": 8,
  "controlIntervalSeconds": 0.03,
  "lifeStateRule": "reset",
  "mathBackend": "rust-scalar-v1",
  "initializationSeed": 42,
  "observationsPath": "observations.f32"
}
```

`sensorProfile` accepts `v3`, `basic` or `motion`; bins are 8..32. V4 contracts
are still labelled candidates, with their exact native canonical bytes and
digest. Cadence is 0.008..0.06 seconds; life state is `reset` or `preserve`.
The other math backend is `rust-sse2-v1`. These settings describe deployment;
the offline sequence advances once per supplied delivery row.

Select exactly one `initializationSeed` (u32, isolated labelled Rust stream) or
`weightsPath` (existing canonical weights). Optional `initializationBiases`
contains all three keys `gruUpdateBias`, `lstmForgetBias`, `rruGateBias`; ordinary
native bounds apply. Optional `initialStatePath` supplies exact packed recurrent
state. Omission initializes zero state. All numeric inputs are raw little-endian
Float32; weights and state must have exact canonical lengths. Observations are
1..256 complete rows. Gaps, generation changes and required life resets start a
new sequence with the appropriate initial state.

The destination contains:

- `manifest.json`: compiler-derived graph/spec/key/digest, equations/revision/digest,
  topological order, resolved ports, parameter views, hidden/cell state, outputs,
  sensor canonical JSON/digest, initialization and deployment rules. Maximum 1 MiB.
- `weights.f32`: exact admitted or Rust-initialized canonical parameter bytes.
- `outputs.f32`: step-major two-output rows, turn then boost.
- `states.f32`: step-major recurrent continuation after every delivery; empty for
  graphs without recurrence.
- `receipt.json`: counts, numeric-file SHA-256 digests, graph/sensor/equation identity,
  exact math backend, source digest, target, compiler and optional initialization seed.

Parameter offsets, lengths, shapes and strides are in Float32 elements. A block's
`length` is its logical element count. Dense/MLP `layerN.W` and `layerN.b` have
interleaved storage: each affine row contains all input weights followed by its
bias. Their bounding storage spans overlap, but their addressed elements are
disjoint. Recurrent W/U/b blocks are contiguous in the declared gate order.
LSTM state stores hidden then cell. A trainer must use these views over one flat
parameter tensor, preserve ordered Split/Concat/output semantics, and execute
the recorded custom equations. The manifest is descriptive metadata; Rust must
independently admit deployed graph and weights.
