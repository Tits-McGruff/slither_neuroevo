# Phase 3 plan: WASM SIMD for GRU, LSTM, and RRU

## Revision notes

- 2026-01-15: Initial recurrent SIMD plan created.
- 2026-01-15: Expanded structure to align with archive plan templates.

## Purpose and scope

Extend SIMD acceleration to recurrent units (GRU/LSTM/RRU). These layers are
expensive at higher hidden sizes and dominate inference cost for deep graphs.

## Delta vs AGENTS.md

- Changes: add SIMD-accelerated recurrent kernels and wasm bridge integration.
- Unchanged: parameter layouts, sensor inputs, serialization, and physics.
- Contract touch points: AGENTS.md “Neural controllers and evolution” and
  “Project-specific conventions and gotchas”.

## Scope

- In: SIMD kernels for GRU/LSTM/RRU, wasm bridge integration, and parity tests.
- Out: Dense/MLP kernels (Phase 2), MT pools (Phases 4/5).

## Non-goals

- No changes to gate math or bias initialization.
- No changes to graph compilation or spec validation.

## Assumptions

- Hidden state buffers remain JS-owned and reused per snake.
- Batch buffers from Phase 1 supply contiguous inputs.
- Hidden state is indexed by stable population slot, not batch index.

## Constraints and invariants

- Preserve exact parameter layouts from `src/brains/ops.ts`.
- Preserve state update order and gating math.
- Maintain deterministic output ordering.
- JS fallback remains available and selectable.

## Architecture narrative

Recurrent units require computing multiple gates per timestep. Each gate uses
`W * x + U * h + b` plus a nonlinearity. SIMD kernels accelerate the matrix-
vector operations while the JS runtime maintains the hidden-state buffers and
applies the final update formulas.

The WASM kernel interface updates hidden state in-place to avoid extra copies.

## Decisions locked for Phase 3

- Hidden state buffers remain owned by JS, passed by pointer to WASM.
- Gate ordering and bias offsets match the JS implementations exactly.
- Recurrent kernels accept batched inputs to align with Phase 1 buffers.

## Key decisions and invariants registry

### Decisions

- DEC-SIMD-REC-001: Hidden state is updated in-place by kernels with no copies.
- DEC-SIMD-REC-002: Kernel output order matches JS per-gate ordering.
- DEC-SIMD-REC-003: Per-unit SIMD enablement can be toggled independently.

### Invariants

- INV-SIMD-REC-001: Gate ordering and bias offsets match `src/brains/ops.ts`.
- INV-SIMD-REC-002: Hidden state buffers are not shared across snakes.
- INV-SIMD-REC-003: JS fallback remains available.
- INV-SIMD-REC-004: Hidden state indexing uses stable population slots.

## Kernel API contract

```text
gru_step(
  weights_ptr, input_ptr, h_ptr, z_ptr, r_ptr,
  in_size, hidden_size, batch_count, input_stride
)

lstm_step(
  weights_ptr, input_ptr, h_ptr, c_ptr,
  in_size, hidden_size, batch_count, input_stride
)

rru_step(
  weights_ptr, input_ptr, h_ptr,
  in_size, hidden_size, batch_count, input_stride
)
```

The bridge validates sizes and falls back to JS on mismatch.

## Touch-point checklist (hard contracts)

- `src/brains/ops.ts`: recurrent layout and gate ordering.
- `src/brains/graph/runtime.ts`: per-node SIMD selection.
- `src/brains/wasmBridge.ts`: load + validate recurrent kernels.
- `scripts/build-wasm.ts`: build and package recurrent wasm assets.

## Module map and responsibilities

- `src/brains/wasmBridge.ts`
  - Adds recurrent kernel loaders and wrappers.
- `src/brains/ops.ts`
  - Adds optional SIMD execution path per unit.
- `src/brains/graph/runtime.ts`
  - Uses SIMD kernels for GRU/LSTM/RRU nodes when enabled.

## Planned modules and functions

### `src/brains/wasmBridge.ts`

```ts
getGruKernel(): GruKernel | null;
getLstmKernel(): LstmKernel | null;
getRruKernel(): RruKernel | null;
```

### `src/brains/ops.ts`

```ts
class GRU {
  stepSimd?(x: Float32Array, batchIndex?: number): Float32Array;
}
```

## Data model changes; migration; backward compatibility

- New wasm assets for GRU/LSTM/RRU under `src/brains/wasm/`.
- No changes to graph specs or saved genomes.
- Backward compatibility: JS path remains the reference.

## Recurrent state ownership and indexing

- Hidden state buffers are keyed by stable population slot or snake index.
- Batch indices map into this stable storage; they are not the storage key.
- Population slots retain identity for the duration of a generation; respawns
  replace in place rather than reindexing active slots.

Example mapping (illustrative):

```ts
const slot = populationIndex;
const hOffset = slot * hiddenSize;
const xOffset = batchIndex * inputStride;
```

## Execution path selection flow

State table

| State | Description | Invariants |
| --- | --- | --- |
| SimdDisabled | SIMD off or unsupported | JS path only |
| SimdReady | SIMD kernels loaded and validated | SIMD path available |
| SimdFailed | Kernel load/validation failed | JS path only |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| loadKernels | SimdDisabled | SimdReady | validation ok | enable SIMD | layout match |
| loadFail | SimdDisabled | SimdFailed | load error | log once | JS fallback |
| disableSimd | SimdReady | SimdDisabled | flag off | JS path | deterministic |

## Detailed implementation checklist

- [ ] Implement GRU kernel with SIMD for W and U multiplies.
- [ ] Implement LSTM kernel with SIMD for W and U multiplies.
- [ ] Implement RRU kernel with SIMD for W and U multiplies.
- [ ] Add wasmBridge wrappers with strict size validation.
- [ ] Add unit parity tests for GRU/LSTM/RRU (single + batched).
- [ ] Validate reset behavior and determinism across repeated runs.

## Output parity and tolerances

- Define tolerance thresholds per unit.
- Use fixed weights/inputs and compare outputs per timestep.
- Track max drift across 100+ steps for recurrent state stability.
- Record first divergence tick for controller outputs when recurrent nodes are
  present in the graph.

Example drift log format (illustrative):

```text
diverge.t=214 slot=19 node=LSTM maxAbs=3.4e-4 maxRel=2.1e-3
```

## Data layout details

GRU layout (unchanged):

```text
Wz, Wr, Wh, Uz, Ur, Uh, bz, br, bh
```

LSTM layout (unchanged):

```text
Wi, Wf, Wo, Wg, Ui, Uf, Uo, Ug, bi, bf, bo, bg
```

RRU layout (unchanged):

```text
Wc, Wr, Uc, Ur, bc, br
```

## Error handling strategy

- If kernel validation fails, disable SIMD for that unit and fall back to JS.
- If buffer sizes mismatch hiddenSize/inSize, skip SIMD for that call and log
  a debug warning.
- If hidden state buffers are unexpectedly shared, log an error and reset the
  affected snake’s brain to avoid contamination.

## Performance considerations

- Avoid per-step allocations in the kernel interface.
- Prefer contiguous memory for hidden states and gate scratch.
- Reuse scratch buffers already present in the JS implementations.

## Observability and debugability

- Debug-gated log: `simd.gru.enabled { available }`.
- Debug-gated log: `simd.lstm.enabled { available }`.
- Debug-gated log: `simd.rru.enabled { available }`.
- Always-on warn: `simd.recurrent.load.failed { reason }`.

## Acceptance criteria

- GRU/LSTM/RRU outputs within tolerance vs JS for fixed inputs.
- No regression in stack regression tests or graph integration tests.
- Profiling shows reduced inference time at higher hidden sizes.

## Acceptance criteria mapping

- AC-SIMD-REC-001: GRU/LSTM/RRU parity within tolerance.
  - Tests: extend existing recurrent parity suites.
- AC-SIMD-REC-002: Stack regression tests still pass.
  - Tests: existing stack regression coverage.

## Tests and validation

- Unit:
  - GRU/LSTM/RRU parity (single + multi-step) in existing brain tests.
  - Graph forward with recurrent SIMD in existing integration coverage.
- Regression:
  - Stacked brain determinism regression suite.
- Manual:
  - Enable SIMD for recurrent units and compare profiler output vs JS.

## Risk register

- Risk: gate ordering mistakes silently change behavior.
  - Mitigation: parity tests with fixed inputs and explicit gate offsets.
- Risk: SIMD approximations drift over long sequences.
  - Mitigation: multi-step parity tests with drift thresholds.
- Risk: hidden state aliasing across snakes.
  - Mitigation: enforce per-snake buffers and add debug asserts.

## Rollout notes

- Keep SIMD disabled for recurrent units until parity tests are stable.
- Provide a per-unit disable flag for debugging.

## Compatibility matrix

- Server mode: SIMD optional; JS fallback always available.
- Worker mode: SIMD optional; JS fallback always available.
- UI/main thread: unchanged.
