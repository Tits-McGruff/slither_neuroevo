# Phase 2 plan: WASM SIMD for Dense and MLP

## Revision notes

- 2026-01-15: Initial SIMD MLP/Dense plan created.
- 2026-01-15: Expanded structure to align with archive plan templates.

## Purpose and scope

Introduce SIMD-accelerated Dense and MLP kernels in WASM and integrate them
with the graph runtime and batched control pipeline. This phase targets the
highest-volume feed-forward math while preserving layouts and outputs.

## Delta vs AGENTS.md

- Changes: add SIMD-accelerated Dense/MLP kernels and a wasm bridge.
- Unchanged: parameter layouts, graph compilation, and serialization.
- Contract touch points: AGENTS.md “Neural controllers and evolution” (layout
  stability) and “Project-specific conventions and gotchas” (hot-path buffers).

## Scope

- In: SIMD kernels for Dense/MLP, wasm loader bridge, graph runtime integration,
  and parity tests.
- Out: recurrent kernels, MT pools, and any changes to graph specs or layouts.

## Non-goals

- No changes to the JS reference implementation.
- No new sensor inputs or control outputs.
- No changes to render/serialization paths.

## Assumptions

- `Float32Array` remains the canonical weight and activation format.
- The batch pipeline from Phase 1 provides contiguous inputs/outputs.
- wasm assets can be bundled by Vite and loaded by Node for server mode.

## Constraints and invariants

- Weight layouts must match `src/brains/ops.ts` exactly.
- Activations use the same tanh/sigmoid semantics as JS.
- Outputs must be within a defined tolerance vs JS.
- JS fallback must remain available and selectable.

## Architecture narrative

Dense/MLP layers are dominated by matrix-vector multiplies and bias adds. WASM
SIMD provides acceleration for these loops, especially when running batched
inputs. This phase adds a WASM bridge that exposes SIMD kernels for:

- Dense head: `out = tanh(Wx + b)`
- MLP: series of Dense layers with tanh activations

The batched control pipeline from Phase 1 supplies contiguous buffers, which
map cleanly to WASM memory.

## Decisions locked for Phase 2

- SIMD kernels operate on Float32 data only.
- JS remains the reference path; SIMD is opt-in via capability detection.
- SIMD kernels do not own weights; they receive pointers into shared buffers.
- SIMD MLP is initially composed from per-layer Dense kernels (no fusion).

## Toolchain and math parity

- Toolchain: Rust + `wasm32-unknown-unknown`, built with SIMD enabled.
- Math parity: implement tanh/sigmoid in WASM by matching the exact function
  logic used in `src/brains/ops.ts` (port the same algorithm), or accept
  bounded drift under the defined tolerances.
- If parity fails, fall back to JS and mark SIMD unavailable.

## Key decisions and invariants registry

### Decisions

- DEC-SIMD-MLP-001: Dense/MLP kernels are integrated via a wasm bridge that
  validates sizes before dispatch.
- DEC-SIMD-MLP-002: SIMD kernels never allocate; all buffers are owned by JS.
- DEC-SIMD-MLP-003: Kernel output ordering matches JS implementation exactly.

### Invariants

- INV-SIMD-MLP-001: Weight layout and bias ordering match `src/brains/ops.ts`.
- INV-SIMD-MLP-002: Output buffer length equals `outSize * batchCount`.
- INV-SIMD-MLP-003: JS fallback is always available.

## Kernel API contract

Each kernel exposes a deterministic C/wasm ABI:

```text
dense_forward(
  weights_ptr, input_ptr, output_ptr,
  in_size, out_size, batch_count, input_stride, output_stride
)

mlp_forward(
  weights_ptr, input_ptr, output_ptr,
  layer_sizes_ptr, layer_count,
  batch_count, input_stride, output_stride
)
```

The bridge validates sizes and falls back to JS on mismatch.

Example bridge wrapper (illustrative):

```ts
export function runDenseSimd(
  kernel: DenseKernel,
  inputs: Float32Array,
  outputs: Float32Array,
  inSize: number,
  outSize: number,
  count: number,
  inputStride: number,
  outputStride: number
): void {
  kernel.forward(
    kernel.weightsPtr,
    inputs,
    outputs,
    inSize,
    outSize,
    count,
    inputStride,
    outputStride
  );
}
```

## Loading strategy (Node + browser)

Browser (module URL):

```ts
const url = new URL('./brains/wasm/mlp.wasm', import.meta.url);
const bytes = await fetch(url).then(res => res.arrayBuffer());
const mod = await WebAssembly.instantiate(bytes, imports);
```

Node (filesystem):

```ts
const url = new URL('../brains/wasm/mlp.wasm', import.meta.url);
const bytes = await fs.promises.readFile(url);
const mod = await WebAssembly.instantiate(bytes, imports);
```

## Touch-point checklist (hard contracts)

- `src/brains/ops.ts`: parameter layout reference; must stay unchanged.
- `src/brains/graph/runtime.ts`: node execution path selection (JS vs SIMD).
- `src/brains/wasmBridge.ts`: wasm loading, buffer validation, and dispatch.
- `scripts/build-wasm.ts`: build process and output placement.
- `vite.config.ts`: ensure wasm assets are bundled for browser usage.

## Module map and responsibilities

- `src/brains/wasmBridge.ts`
  - Loads SIMD modules in Node and browser contexts.
  - Exposes typed wrappers with safe bounds checks.
- `src/brains/graph/runtime.ts`
  - Uses SIMD kernels when available and enabled.
  - Falls back to JS per node type.
- `scripts/build-wasm.ts`
  - Builds wasm modules (SIMD enabled) and places them in `src/brains/wasm/`.

## Planned modules and functions

### `src/brains/wasmBridge.ts`

```ts
type SimdKernelStatus = 'unavailable' | 'loading' | 'ready' | 'failed';

loadSimdKernels(): Promise<void>;
isSimdAvailable(): boolean;
getDenseKernel(): DenseKernel | null;
getMlpKernel(): MlpKernel | null;
```

### `src/brains/graph/runtime.ts`

```ts
runDenseBatch(inputs: Float32Array, outputs: Float32Array, count: number): void;
runMlpBatch(inputs: Float32Array, outputs: Float32Array, count: number): void;
```

## Data model changes; migration; backward compatibility

- New wasm assets under `src/brains/wasm/`.
- No changes to existing graph specs, weights, or exports.
- Backward compatibility: JS path remains the default when wasm is absent.

## Build and packaging strategy

- Build wasm with SIMD enabled and commit artifacts under `src/brains/wasm/`.
- Ensure Vite bundles wasm assets for browser builds.
- Ensure Node can load wasm assets for server mode (path-resolved).

## Execution path selection flow

State table

| State | Description | Invariants |
| --- | --- | --- |
| SimdUnavailable | wasm or SIMD not supported | JS path only |
| SimdReady | wasm loaded and kernels validated | SIMD path available |
| SimdFailed | load or validation failed | JS path only |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| loadKernels | SimdUnavailable | SimdReady | wasm loads + validates | enable SIMD | layout match |
| loadFail | SimdUnavailable | SimdFailed | load/validation error | log once | JS fallback |
| disableSimd | SimdReady | SimdUnavailable | config flag off | JS path | no drift |

## Detailed implementation checklist

- [ ] Define the kernel ABI and document parameter layouts.
- [ ] Implement Dense kernel with SIMD loops and tail handling.
- [ ] Implement MLP kernel using Dense kernel per layer or fused loops.
- [ ] Add wasmBridge loader with feature detection and fallback logic.
- [ ] Add node-level integration in graph runtime for Dense/MLP.
- [ ] Add unit tests comparing JS vs SIMD outputs for fixed weights/inputs.
- [ ] Add perf smoke test to confirm SIMD path is active and faster.

## Output parity and tolerances

- Define acceptable absolute and relative tolerances for outputs, with tighter
  thresholds for controller outputs than for intermediate node buffers.
- Use deterministic inputs/weights in tests.
- Record max error per layer and fail if thresholds are exceeded.

## Data layout details

Dense weight layout (unchanged):

```text
for each output neuron o:
  W[o * in + i] for i in 0..in-1
  bias[o]
```

MLP weight layout (unchanged):

```text
W(layer 0) + b(layer 0) + W(layer 1) + b(layer 1) + ...
```

## Error handling strategy

- If wasm kernels fail validation, mark SIMD unavailable and fall back to JS.
- If buffer sizes mismatch expected strides, skip SIMD for that node and log
  a debug warning.
- If wasm assets are missing, log once and continue in JS-only mode.

## Performance considerations

- Use batch loops to amortize kernel launch overhead.
- Keep memory contiguous and aligned when possible.
- Avoid per-call allocations in wasmBridge.

## Observability and debugability

- Debug-gated log: `simd.dense.enabled { available }`.
- Debug-gated log: `simd.mlp.enabled { available }`.
- Always-on warn: `simd.load.failed { reason }`.

## Acceptance criteria

- Dense/MLP outputs within tolerance vs JS path.
- SIMD path can be enabled/disabled without changing outputs.
- Profiling shows measurable improvement at 100+ snakes.

## Acceptance criteria mapping

- AC-SIMD-MLP-001: Dense/MLP parity within tolerance.
  - Tests: extend existing brain parity suites.
- AC-SIMD-MLP-002: SIMD enable/disable preserves outputs.
  - Tests: add a toggle-based parity check in the graph suite.

## Tests and validation

- Unit:
  - Dense parity (single + batch) in existing brain tests.
  - MLP parity across layer sizes in existing graph tests.
- Regression:
  - Graph forward integration with SIMD enabled.
- Manual:
  - Run server and worker mode; confirm SIMD path logs appear when enabled.
  - Compare profiler output with SIMD on/off for performance deltas.

## Risk register

- Risk: incorrect weight offsets cause silent behavior drift.
  - Mitigation: parity tests with fixed weights and explicit offsets.
- Risk: SIMD tail handling reads past buffer ends.
  - Mitigation: bounds checks and dedicated tail loop in kernels.
- Risk: non-matching tanh/sigmoid implementations.
  - Mitigation: use explicit implementations and tolerance tests.

## Rollout notes

- Keep SIMD disabled by default until parity tests pass in CI.
- Add an environment flag to force JS-only for debugging.

## Compatibility matrix

- Server mode: SIMD optional; JS fallback always available.
- Worker mode: SIMD optional; JS fallback always available.
- UI/main thread: unchanged.
