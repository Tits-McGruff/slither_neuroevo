# MT+SIMD Plan

## Revision notes

- 2026-01-15: Initial MT+SIMD plan split into detailed multi-phase files.
- 2026-01-15: Added explicit invariants, validation matrix, and rollout gates.
- 2026-01-15: Expanded structure to align with archive plan templates and AGENTS.md anchors.

## Summary

Deliver a high-performance MT+SIMD brain/sensor pipeline while preserving the
simulation’s determinism, data contracts, and UI behavior. The plan stages the
work into batch-friendly refactors, SIMD kernels, MT worker pools, and
validation/rollout. Each phase is designed to be merge-safe and measurable,
with the JS path preserved as a correctness reference.

## Delta vs AGENTS.md

- Changes: batched control evaluation, SIMD kernels, MT inference pools, and
  new configuration flags for runtime selection of JS/SIMD/MT paths.
- Unchanged: world physics, serializer buffer layout, worker/server protocol
  shapes, and sensor layout contracts.
- Contract touch points: AGENTS.md “Simulation core” (controlDt and sensor
  sizing), “Binary frame format and rendering pipeline” (buffer invariants),
  and “Runtime architecture and data flow” (worker/server symmetry).

## Stage files

- `phase-1-mt-simd-batching.md` — refactor control flow into a batched pipeline.
- `phase-2-mt-simd-mlp.md` — SIMD Dense/MLP kernels in WASM + integration.
- `phase-3-mt-simd-recurrent.md` — SIMD GRU/LSTM/RRU kernels + integration.
- `phase-4-mt-simd-server-workers.md` — Node worker_threads inference pool.
- `phase-5-mt-simd-browser-workers.md` — Browser worker pool + COOP/COEP.
- `phase-6-mt-simd-validation.md` — parity, perf, and rollout gates.

## Scope

- In: batched control pipeline; SIMD kernels for Dense/MLP/GRU/LSTM/RRU; MT
  inference pools (server + browser worker mode); shared buffers; COOP/COEP
  headers; perf and parity validation; feature flags and fallbacks.
- Out: physics multithreading, GPU compute, gameplay rule changes, new
  serialization formats, or any changes to evolution semantics.

## Non-goals

- No changes to the binary frame format or render pipeline.
- No automated migration for existing saves beyond current behavior.
- No re-tuning of fitness or reward weights.
- No new network protocol beyond configuration flags and capabilities.

## Assumptions

- The world loop remains single-threaded; only inference is parallelized.
- `CFG.brain.controlDt` remains the gate for inference scheduling.
- Sensor layout is canonical; `CFG.brain.inSize` must match and is validated.
- The JS brain path remains available as a fallback and reference.
- SIMD and MT are treated as optional accelerators; they do not become
  correctness requirements.
- Worker mode and server mode must remain behaviorally consistent.

## Compatibility intent

- SIMD/MT are opt-in or capability-gated until parity is proven.
- Browser MT requires cross-origin isolation; SIMD-only fallback remains.
- Server MT can be enabled without changing client or protocol behavior.
- Old exports/imports remain valid; no data schema migration is required.

## Constraints and invariants

- INV-MT-001: Sensor vector length always equals `CFG.brain.inSize`.
- INV-MT-002: Brain outputs are applied in the baseline iteration order per
  tick (stable population slot order, no additional sorting).
- INV-MT-003: No per-tick allocations in hot paths; reuse typed buffers.
- INV-MT-004: JS path remains functionally identical to legacy behavior.
- INV-MT-005: SIMD/MT paths must be within defined tolerance vs JS outputs.
- INV-MT-006: Shared buffers never mix generations or mismatched graph specs.
- INV-MT-007: World physics and collisions remain single-threaded.
- INV-MT-008: External control behavior (player/bot) is unchanged.
- INV-MT-009: Buffer contracts for serialization/rendering remain unchanged.
- INV-MT-010: Parameter layouts in `src/brains/ops.ts` are unchanged.
- INV-MT-011: Graph compilation keys remain stable for existing specs.
- INV-MT-012: Recurrent state buffers are indexed by stable population slot
  (not transient batch index) and never shared across snakes.
- INV-MT-013: Worker mode continues to post a transferable frame buffer to the
  UI thread; SharedArrayBuffer is internal to inference only.
- INV-MT-014: External controller sensor publication cadence and sampling
  point remain identical to baseline; action application stays tick-aligned.
- INV-MT-015: Sensor layout and ordering are unchanged, including bubbleBins
  clamping and histogram ordering as described in AGENTS.md.

## Tolerance policy (contract)

- Phase 1 refactor must be bitwise identical:
  - Sensor vectors for each population slot at the same tick.
  - Controller outputs and applied actions for the same tick.
  - Frame buffer digest over a fixed window for fixed-seed runs.
- SIMD phases allow bounded drift:
  - Node output parity: maxAbs + maxRel thresholds per node output buffer,
    with tighter thresholds at controller outputs than intermediate nodes.
  - Action parity: record first divergence tick for (turn, boost) at controller
    outputs over a fixed window.

Example parity check (illustrative):

```ts
const maxAbs = Math.max(...diffs.map(Math.abs));
const maxRel = Math.max(...diffs.map((d, i) => Math.abs(d) / Math.max(1e-6, Math.abs(js[i]))));
if (maxAbs > ABS_TOL || maxRel > REL_TOL) throw new Error('parity fail');
```

## Touch-point checklist (hard contracts)

- Batched control pipeline used by both worker mode and server mode.
- Sensor layout contract shared by sensors, bots, and server sensorSpec.
- Brain parameter layout preserved for all kernels.
- Output order is stable and deterministic across JS/SIMD/MT.
- COOP/COEP headers applied for browser MT and documented for users.
- Profiling remains enabled behind flags for ongoing validation.
- Worker frame transfer semantics to main thread remain unchanged.
- Core modules updated together:
  - `src/world.ts`, `src/snake.ts`, `src/sensors.ts`
  - `src/brains/ops.ts`, `src/brains/graph/runtime.ts`, `src/brains/types.ts`
  - `server/simServer.ts`, `server/index.ts`, `src/worker.ts`
  - `vite.config.ts`, `src/protocol/messages.ts`

## Decisions

- DEC-MT-001: Maintain a JS fallback for every SIMD kernel.
- DEC-MT-002: Use batched buffers as the primary data interchange.
- DEC-MT-003: SharedArrayBuffer is the transport for MT outputs.
- DEC-MT-004: SIMD kernels are integrated node-by-node in graph runtime.
- DEC-MT-005: MT worker pools never touch world state; they only read inputs
  and write outputs.
- DEC-MT-006: Browser MT is opt-in and gated on crossOriginIsolated.
- DEC-MT-007: Server MT can be enabled independently of browser MT.
- DEC-MT-008: SIMD/MT selection is controlled via config flags and runtime
  capability checks (exact flag names finalized in Phase 1).

## Key decisions and invariants registry

- Decisions: DEC-MT-001 through DEC-MT-008 define fixed MT+SIMD choices.
- Invariants: INV-MT-001 through INV-MT-015 define non-negotiable contracts.
- If a decision changes, add a new DEC-MT-### and mark the old one superseded.
- Phase-scoped invariants (e.g., INV-BATCH-###) apply only within their phase
  and do not override program-level INV-MT-### contracts.

## Delta architecture overview

- Introduce a batched control pipeline at the world layer.
- Add SIMD kernels for Dense/MLP and recurrent units, preserving layouts.
- Add MT inference pools for server and browser worker mode.
- Keep the JS implementation as a reference and fallback path.

## Planned modules and functions (summary)

- `src/world.ts`
  - `buildControlBatch(...)` (new helper for per-tick batches)
  - `applyControlBatch(...)` (applies outputs back to snakes)
- `src/brains/types.ts`
  - Optional `forwardBatch(inputs, outputs, stride)` interface
- `src/brains/graph/runtime.ts`
  - Batched execution path for graph nodes
- `src/brains/wasmBridge.ts`
  - `loadSimdKernels()`, `DenseKernel.forwardBatch(...)`, `MlpKernel.forwardBatch(...)`
  - `GruKernel.stepBatch(...)`, `LstmKernel.stepBatch(...)`, `RruKernel.stepBatch(...)`
- `server/brainPool.ts`
  - Worker pool management for Node MT
- `src/workerPool.ts`
  - Worker pool management for browser worker mode

## Data flow and timing

```text
World.update
  -> build control batch (snake indices + input buffer)
  -> sensors fill input buffer
  -> brain batch inference (JS/SIMD/MT)
  -> apply outputs back to snakes
  -> physics + collisions
```

Sensors are computed for a sensors batch (all snakes that need sensors for any
consumer). Inference runs only on the inference subset; external control
snakes bypass inference but still use the shared sensor batch snapshot for
publishing.

## Data model changes overview

- New per-tick batch buffers (inputs, outputs, index map).
- Optional per-node kernel adapters (SIMD vs JS).
- Configuration flags for SIMD and MT (server + worker).
- COOP/COEP headers for browser MT.
- Optional shared buffer handles for worker pools (SAB-backed).

## Spec key contract

- specKey must include:
  - graphSpec canonical serialization
  - `CFG.brain.inSize` and `CFG.brain.outSize`
  - layout/ops version tag (bumped only when parameter layout changes)
- Canonical graphSpec serialization must be deterministic and stable across
  runs (object key ordering must not change the key).

Example spec key composition (illustrative):

```text
specKey = hash(graphKey + '|' + inSize + 'x' + outSize + '|ops:v1')
```

## Alternatives considered

- GPU compute via WebGPU: rejected for now due to complexity and portability.
- Physics multithreading: rejected due to heavy shared-state coupling.
- Removing JS fallback: rejected to preserve correctness reference.

## Error handling strategy

- If SIMD load fails, fall back to JS and log once.
- If MT pool fails to start, fall back to single-threaded inference.
- If graph spec changes mid-run, rebuild shared buffers and invalidate pools.
- If buffer size mismatches are detected, disable SIMD/MT for that tick and
  fall back to JS to preserve correctness.

## Performance considerations

- Memory bandwidth dominates at high snake counts; prefer batch sizes that
  reduce overhead and improve SIMD efficiency.
- Avoid per-snake Array.from in hot paths; gate debug buffers.
- Keep controlDt gating unchanged to avoid destabilizing memory behavior.

## Security and privacy

- No new data collection; profiling logs remain local.
- SharedArrayBuffer use is limited to worker-local contexts.

## Observability and debugability

- Keep profiler output to track brain vs sensor time by tick.
- Add optional per-kernel timing to validate SIMD and MT effectiveness.
- Provide a config flag to force JS-only for debugging.
- Log a one-time summary of the chosen execution path (JS/SIMD/MT) at startup.

## Debug playbook

- Enable profiling and confirm brain/sensor ratios match baseline before
  enabling SIMD or MT.
- Toggle SIMD on/off and verify parity tests remain green.
- Enable server MT with a fixed seed and compare action logs for determinism.

## Dependencies and sequencing

- Phase 1 is required for any SIMD or MT work.
- Phase 2 must land before Phase 3 to lock kernel API patterns.
- Phase 4 (server MT) can start after Phase 2, but is safer after Phase 3.
- Phase 5 (browser MT) depends on COOP/COEP and worker pool abstractions.
- Phase 6 gates default enablement of SIMD and MT.

## Rollout

- Default path: JS-only (baseline).
- Enable SIMD in dev and CI once parity tests pass.
- Enable server MT behind a config flag after perf targets are met.
- Keep browser MT opt-in until cross-origin isolation is validated.
- Rollback: disable SIMD/MT flags and fall back to JS without data migration.

## Acceptance criteria and test mapping

- AC-MT-001: Batched pipeline yields identical actions vs JS per-snake path.
  - Tests: new batch parity unit tests.
  - Commands: `npm test`, `npm run test:unit`.
- AC-MT-002: SIMD kernels match JS outputs within tolerance.
  - Tests: kernel parity tests for Dense/MLP/GRU/LSTM/RRU.
  - Commands: `npm test`, `npm run test:unit`.
- AC-MT-003: Server MT produces deterministic output ordering.
  - Tests: MT scheduling unit tests; server integration tests.
  - Commands: `npm run test:integration`.
- AC-MT-004: Browser MT falls back to SIMD-only when isolation is absent.
  - Tests: worker capability tests; manual smoke check.
  - Commands: `npm run dev` + manual.
- AC-MT-005: Performance targets met at 150/300 snakes.
  - Tests: perf runs with profiler output captured.

## Compatibility matrix

- Server mode: MT optional; SIMD available; protocol unchanged.
- Worker mode: SIMD available; MT gated by crossOriginIsolated.
- UI/main thread: unchanged; render pipeline remains frame-buffer driven.

## Risk register

- Risk: SIMD numeric drift alters evolution outcomes.
  - Mitigation: parity tests with tight tolerances; JS fallback.
- Risk: MT scheduling changes action ordering.
  - Mitigation: fixed batch indices and deterministic output application.
- Risk: COOP/COEP headers break hosting setups.
  - Mitigation: keep browser MT opt-in; document requirements.
