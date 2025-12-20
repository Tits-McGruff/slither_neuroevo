# Phase 5 plan: Brain registry + multiple types

## Purpose and scope

This phase introduces a pluggable brain registry so the simulation can swap
between MLP, GRU, LSTM, and RRU without changing the core sim logic. The goal
is to preserve the current behavior while enabling new brain types and making
future graph-based layouts possible.

## Architecture narrative

Today the `mlp.ts` module bundles architecture decisions, parameter layouts,
and forward execution. This phase extracts the concept of a "brain" into a
separate interface and registry. World and Snake only depend on the Brain
interface; they do not know or care which architecture is in use. Each brain
implementation is responsible for its own parameter layout and forward math.

## Decisions locked for Phase 5

The brain interface is minimal and allocation-free. The genome stores
`brainType` and `weights`, existing MLP/GRU forward logic is preserved, and new
LSTM/RRU implementations use typed arrays with preallocated buffers.

## Module layout

```text
src/brains/
  types.ts
  registry.ts
  mlpBrain.ts
  gruBrain.ts
  lstmBrain.ts
  rruBrain.ts
```

## Brain interface and JSON format

```ts
export interface Brain {
  forward(input: Float32Array): Float32Array;
  reset(): void;
  toJSON(): BrainJSON;
  fromJSON(json: BrainJSON): Brain;
  paramLength(): number;
}

export interface BrainJSON {
  type: string;
  weights: number[];
  spec: BrainSpec;
}

export interface BrainSpec {
  type: string;
  inputSize: number;
  outputSize: number;
  hiddenSizes?: number[];
  gruHidden?: number;
  lstmHidden?: number;
  rruHidden?: number;
}
```

## Parameter layouts

### MLP layout

For each layer `in -> out`, the layout is `out * in` weights followed by `out`
biases. Layout is contiguous across layers, and activation is `tanh`.

### GRU layout

Matches the current GRU layout:

- gates: z, r, h~
- each gate has W (H x I), U (H x H), b (H)
- layout: Wz, Wr, Wh, Uz, Ur, Uh, bz, br, bh

### LSTM layout

Standard LSTM:

- gates: i, f, o, g
- each gate has W (H x I), U (H x H), b (H)
- layout: Wi, Wf, Wo, Wg, Ui, Uf, Uo, Ug, bi, bf, bo, bg
- update equations: `c=f*c+i*g`, `h=o*tanh(c)`

### RRU layout

Residual recurrent unit:

- candidate: `h~ = tanh(Wx x + Wh h + b)`
- gate: `r = sigmoid(Wr x + Ur h + br)`
- update: `h=(1-r)*h+r*h~`

## Brain implementations

### `mlpBrain.ts`

Wraps the existing MLP implementation. It uses the same weight layout and
returns the same output buffer each call.

### `gruBrain.ts`

Wraps the existing MLP feature extractor + GRU + DenseHead pipeline. It keeps
hidden state across steps and resets it on `reset()`.

### `lstmBrain.ts`

Adds a new LSTM-based controller. It stores `h` and `c` buffers and implements
standard LSTM update math. It reuses the MLP feature extractor and a DenseHead
for output.

### `rruBrain.ts`

Adds a residual recurrent controller. It keeps a single hidden state and uses
simple gating to update it each tick. Output is produced by a DenseHead.

## Registry behavior

The registry maps `type` to factory. A factory receives a `BrainSpec` and
optional weights. The registry throws if the type is unknown. The default type
is `mlp` if a genome does not specify one.

## Genome changes

Genome JSON is extended to include `brainType`. Older genomes that lack
`brainType` default to `mlp` when loaded.

## Settings changes

Add a brain type selector to the UI. This maps to `CFG.brain.type`. Existing
settings for GRU remain in place; they are ignored if the selected brain is
not GRU/LSTM/RRU.

## Detailed design notes

Wrapping the existing MLP and GRU must preserve the exact math and parameter
layout already used in evolution. The registry should call into the existing
forward functions rather than rewriting them. This ensures that saved genomes
remain compatible and that results are reproducible across the migration.

The new LSTM and RRU implementations should follow the same pattern as the GRU:
preallocate buffers for hidden state and gate intermediates, and avoid per-tick
allocations. Forward passes should return stable buffers to avoid extra GC
pressure when many snakes are updated each tick.

Genome JSON changes must be backward compatible. When a genome lacks a
`brainType` field, the loader defaults to `mlp` so older snapshots still load
without manual migration. The UI should expose the new brain type selector but
should not break the existing GRU settings; those settings simply have no
effect when a different brain type is chosen.

## Tests

Tests verify that the registry builds the correct brain type for each spec,
that LSTM and RRU forward outputs are deterministic for fixed weights, and
that Genome JSON includes `brainType` while still loading older genomes with a
default type.

## Footguns

Avoid copying weights on every tick. Keep all recurrent buffers preallocated,
and ensure `forward()` returns a stable buffer on each call.

## Acceptance criteria

MLP and GRU behave identically to the current implementation, LSTM and RRU can
be selected and run, and all tests pass.

## Execution checklist

- [ ] Add `src/brains/` module structure
- [ ] Implement Brain interface
- [ ] Wrap MLP and GRU
- [ ] Implement LSTM and RRU
- [ ] Update Genome JSON format
- [ ] Update settings UI
- [ ] Add tests

## Function-by-function pseudocode

### Pseudocode: `registry.ts`

```text
function register(type, factory):
  registry[type] = factory

function build(spec, weights?):
  if spec.type not in registry: throw
  return registry[spec.type](spec, weights)
```

### Pseudocode: `mlpBrain.ts`

```text
function forward(input):
  return mlp.forward(input)

function reset():
  no-op
```

### Pseudocode: `gruBrain.ts`

```text
function forward(input):
  features = mlp.forward(input)
  h = gru.step(features)
  return head.forward(h)

function reset():
  gru.reset()
```

### Pseudocode: `lstmBrain.ts`

```text
function forward(input):
  features = mlp.forward(input)
  lstm.step(features) -> updates h and c
  return head.forward(h)

function reset():
  h.fill(0)
  c.fill(0)
```

### Pseudocode: `rruBrain.ts`

```text
function forward(input):
  features = mlp.forward(input)
  h = rru.step(features)
  return head.forward(h)

function reset():
  h.fill(0)
```

## Error handling and edge cases

If a genome references an unknown `brainType`, the loader defaults to `mlp` and
logs a warning rather than throwing. If a weight array length does not match
`paramLength`, the constructor should either throw with a clear message or
regenerate weights; in this phase we choose to throw to avoid silent corruption.

## Sample payloads and example WS session transcript

Example genome JSON with brain type:

```json
{ "archKey": "mlp:16x8", "brainType": "gru", "weights": [0.1, -0.2], "fitness": 0 }
```

Example WS session transcript (no new messages in this phase):

```text
client -> server: {"type":"hello","clientType":"ui","version":1}
server -> client: {"type":"welcome",...}
server -> client: <binary frame>
```

## Test matrix

| Test name | Setup / input | Expected result | Failure cases to verify |
| --- | --- | --- | --- |
| registry_unknown_type | spec.type = "unknown" | build throws with message | Returns null or wrong brain |
| genome_default_type | JSON lacks brainType | Defaults to mlp | Crash or undefined brain |
| weight_length_mismatch | Provide too-short weights | Constructor throws | Silent truncation |
| lstm_determinism | Fixed weights, fixed input | Same output every run | Non-deterministic output |
