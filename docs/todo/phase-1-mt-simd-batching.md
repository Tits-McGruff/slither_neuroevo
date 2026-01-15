# Phase 1 plan: MT+SIMD baseline and batched control pipeline

## Revision notes

- 2026-01-15: Initial batch refactor plan created.
- 2026-01-15: Expanded structure to match archive plan templates.

## Purpose and scope

Build a batch-friendly control pipeline without changing simulation behavior.
This phase reorganizes the control path so sensors and brain inference operate
on contiguous buffers, enabling SIMD and MT in later phases.

## Delta vs AGENTS.md

- Changes: control evaluation is refactored into a batched pipeline.
- Unchanged: physics loop, serializer buffer layout, worker/server protocol
  shapes, and sensor layout contract.
- Contract touch points: AGENTS.md “Simulation core” (controlDt + sensors) and
  “Runtime architecture and data flow” (worker/server symmetry).

## Scope

- In: batched control flow, shared input/output buffers, and JS-only batch
  inference plumbing.
- Out: SIMD kernels, MT worker pools, physics changes, serialization changes,
  and network protocol changes.

## Non-goals

- No change to the brain math or activation functions.
- No change to sensor layout or sizing.
- No change to `World.update` physics semantics.

## Assumptions

- Sensor layout is canonical; `CFG.brain.inSize` must match the layout.
- `CFG.brain.controlDt` continues to gate control updates.
- External controller logic remains the source of truth for player/bot inputs.

## Constraints and invariants

- Preserve the exact controlDt gating semantics in `Snake.update`.
- Preserve external control behavior (player/bot commands).
- Preserve external controller sensor publication cadence and sampling point.
- Avoid per-tick allocations; reuse typed arrays and per-snake scratch.
- No changes to render buffer contracts or network protocols.
- Maintain JS path as the correctness baseline for later SIMD/MT phases.

## Architecture narrative

Currently, each snake computes sensors and runs brain.forward inside its own
update call. That makes inference hard to batch, and it duplicates sensor work
for external controllers. This phase splits the control path into:

1) Build a stable list of snakes requiring control updates.
2) Compute sensors for that list into a packed input buffer.
3) Run batched inference (JS path for now) into an output buffer.
4) Apply output actions to the corresponding snakes.

The world loop remains in control of ordering. The physics loop is unchanged.
The per-snake update only handles movement and local effects after control is
resolved.

## ControlDt placement (explicit)

- Batch inference runs at the same point where `Snake.update` currently
  evaluates controlDt and accumulates `_ctrlAcc`.
- If control gating today happens inside the physics substep loop, the batch
  pipeline must also run inside that loop to preserve timing.

Example placement (illustrative):

```ts
for (let s = 0; s < steps; s++) {
  const batch = buildControlBatch(stepDt);
  computeBatchSensors(batch);
  runBatchInference(batch);
  applyControlBatch(batch);
  stepPhysics(stepDt);
}
```

## Decisions locked for Phase 1

- Batch buffers are `Float32Array` with stable stride per snake.
- Control batch ordering follows baseline iteration order (stable population
  slot order, no additional sorting).
- Sensor vectors remain the authoritative input; no normalization changes.
- JS brain path remains the only execution path in this phase.

## Key decisions and invariants registry

### Decisions

- DEC-BATCH-001: Batch buffer ordering follows the stable snake order used by
  the world (population slot order, no reordering by size or controller type).
- DEC-BATCH-002: Control batch contains only snakes that need new actions; all
  others reuse their last action.
- DEC-BATCH-003: External control paths bypass batch inference, not sensors.
- DEC-BATCH-004: Batch buffers are reused across ticks and resized only when
  population size or `CFG.brain.inSize` changes.

### Invariants

- INV-BATCH-001: Sensor output length equals `CFG.brain.inSize`.
- INV-BATCH-002: Output stride equals `CFG.brain.outSize`.
- INV-BATCH-003: ControlDt gating remains per snake and unchanged.
- INV-BATCH-004: Physics update ordering is unchanged.
- INV-BATCH-005: `bestPointsThisGen` is initialized before any sensor pass.
- INV-BATCH-006: Sensor layout ordering is unchanged; `CFG.brain.inSize`
  matches the active sensor layout helper.

## Data flow and timing

```text
World.update
  -> build sensors batch (indices)
  -> compute sensors into input buffer
  -> build inference subset (indices)
  -> run JS batch inference (inference subset)
  -> apply outputs to snakes (inference subset)
  -> physics, collisions, particles
```

## Batch buffer contract

- `inputStride = CFG.brain.inSize`
- `outputStride = CFG.brain.outSize` (turn + boost)
- `batchIndices`: list of snake indices in the world population
- `inputs`: `Float32Array(batchCount * inputStride)`
- `outputs`: `Float32Array(batchCount * outputStride)`

Index mapping:

```text
inputs[batchIndex * inputStride + i] = sensor[i]
outputs[batchIndex * outputStride + 0] = turn
outputs[batchIndex * outputStride + 1] = boost
```

## Control gating semantics

- `Snake._ctrlAcc` and `_hasAct` remain authoritative.
- If a snake does not require a control update in this tick, it is excluded
  from the batch and keeps its previous action.
- External control bypasses inference but still participates in the sensor
  batch for controller publishing.

Example split (illustrative):

```ts
const sensorsBatch = buildSensorsBatch(dt);
computeBatchSensors(sensorsBatch);
const inferenceBatch = filterInferenceSubset(sensorsBatch);
runBatchInference(inferenceBatch);
applyControlBatch(inferenceBatch);
```

## Touch-point checklist (hard contracts)

- `src/world.ts`: batch lifecycle, buffer reuse, and output application.
- `src/snake.ts`: split control application from brain inference.
- `src/sensors.ts`: batch-friendly sensor writes into provided buffers.
- `src/brains/types.ts`: optional batch API surface for later phases.
- `src/worker.ts` and `server/simServer.ts`: reuse batch sensors for
  controller publishing to avoid recompute.
- `src/world.ts`: ensure `bestPointsThisGen` stays finite before batching.

## Module map and responsibilities

- `src/world.ts`
  - Owns per-tick batch building and dispatch.
  - Reuses buffers; avoids per-tick allocations.
  - Uses batch sensors for controller publishing to avoid recompute.
- `src/snake.ts`
  - Exposes a new method to apply action outputs (turn/boost) without running
    brain inference internally.
  - Keeps movement, boost, and physics logic as-is.
- `src/sensors.ts`
  - Exposes a batch-friendly entry point or a helper that writes into a target
    buffer without reallocating.
- `src/brains/types.ts`
  - Adds an optional `forwardBatch` interface for future phases.

## Planned modules and functions

### `src/world.ts`

```ts
type ControlBatch = {
  indices: Uint32Array;
  count: number;
  inputStride: number;
  outputStride: number;
  inputs: Float32Array;
  outputs: Float32Array;
};

buildControlBatch(dt: number): ControlBatch;
computeBatchSensors(batch: ControlBatch): void;
runBatchInference(batch: ControlBatch): void;
applyControlBatch(batch: ControlBatch): void;
```

### `src/snake.ts`

```ts
applyBrainOutput(turn: number, boost: number): void;
needsControlUpdate(dt: number): boolean;
```

### `src/sensors.ts`

```ts
buildSensors(world: WorldLike, snake: SnakeLike, out: Float32Array): Float32Array;
```

### `src/brains/types.ts`

```ts
interface Brain {
  forward(input: Float32Array): Float32Array;
  forwardBatch?(inputs: Float32Array, outputs: Float32Array, count: number): void;
}
```

## Data model changes; migration; backward compatibility

- New runtime-only batch buffers in `World` (inputs/outputs/index map).
- No persistence changes; no import/export changes.
- Backward compatibility: unchanged behavior when batch path is disabled.

## State flow: control batch lifecycle

State table

| State | Description | Invariants |
| --- | --- | --- |
| Idle | No batch built this tick | buffers retain last capacity |
| BatchBuilt | Indices and counts computed | ordering stable |
| SensorsFilled | Input buffer filled | inputStride matches CFG.brain.inSize |
| Inferred | Outputs computed | outputStride matches CFG.brain.outSize |
| Applied | Outputs copied to snakes | actions applied once |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| startTick | Idle | BatchBuilt | none | allocate/resize buffers | stable ordering |
| computeSensors | BatchBuilt | SensorsFilled | count > 0 | fill inputs | input stride |
| runInference | SensorsFilled | Inferred | count > 0 | fill outputs | output stride |
| applyOutputs | Inferred | Applied | count > 0 | update snakes | single apply |
| endTick | Applied | Idle | none | reset counts | buffers reused |

## Error handling strategy

- If batch buffer sizes do not match expected strides, skip batch inference
  for that tick and fall back to per-snake JS inference.
- If a snake’s sensor buffer size mismatches `CFG.brain.inSize`, log a warning
  and recompute into a correctly sized buffer.
- If batch indices are empty, skip inference and preserve prior actions.
- If `bestPointsThisGen` is non-finite before sensors, reinitialize to 0 and
  log a warning.

## Performance considerations

- Reuse `Float32Array` buffers across ticks; resize only when capacity grows.
- Avoid `Array.from` in hot paths; keep debug copies behind a flag.
- Avoid per-snake temporary arrays inside the batch loop.

## Observability and debugability

- Debug-gated log: `control.batch.summary { count, inputStride, outputStride }`.
- Debug-gated log: `control.batch.fallback { reason }`.

## Detailed implementation checklist

- [ ] Add a `ControlBatch` helper struct to `World` (indices, buffers, sizes).
- [ ] Add `World.buildControlBatch()` to gather snakes requiring control updates.
- [ ] Add `World.computeBatchSensors()` to fill the input buffer using
      `Snake.computeSensors` with a provided target buffer.
- [ ] Add a sensors batch and a separate inference subset (reuse the same
      buffers; inference subset is a filtered view of the sensors batch).
- [ ] Add `World.runBatchInference()` using the JS path as a baseline.
- [ ] Add `Snake.applyBrainOutput(turn, boost)` to cleanly apply actions.
- [ ] Update `_publishControllerSensors` to reuse batch sensor outputs when
      possible.
- [ ] Gate `lastSensors` and `lastOutputs` updates behind a debug flag or
      reuse batch buffers to avoid per-tick `Array.from`.
- [ ] Add a JS-only fallback path that can be toggled for parity testing.
- [ ] Add a batch size guard to skip inference when no snakes need updates.
- [ ] Assert `bestPointsThisGen` is finite before `computeBatchSensors`.
- [ ] Validate `CFG.brain.inSize` against the active sensor layout helper and
      log a warning if mismatched.

## Behavioral invariants

- Movement, boosting, and growth are identical per tick.
- Brain outputs are applied in the same tick as before.
- ControlDt gating remains unchanged and per-snake.
- External controller sensor publication cadence remains unchanged.

## Acceptance criteria

- Control outputs match the legacy path for a fixed seed and graph spec.
- Sensor vectors are bitwise identical to baseline for the same tick and
  population slot.
- Frame buffer digest is bitwise identical over a fixed window (fixed seed).
- Profiling shows equal or lower per-tick allocations and no regressions.

## Tests and validation

- Unit:
  - Compare per-snake JS inference vs batched outputs for a fixed seed.
  - Verify `applyBrainOutput` updates control values without running inference.
- Integration:
  - Fixed-seed window with batch enabled produces identical action ordering.
  - Frame buffer digest parity over a fixed tick window.
- Manual:
  - Worker mode: run with profiling enabled and confirm batch counts match
    expected snakes per tick.
  - Server mode: run `SLITHER_PROFILE=1 npm run server` and confirm brain
    calls per tick align with batch size.

## Risk register

- Risk: duplicate sensor computation for controller publishing.
  - Mitigation: reuse batch sensors for `_publishControllerSensors`.
- Risk: accidental allocations in debug/viz paths.
  - Mitigation: gate debug copies; reuse buffers.
- Risk: incorrect batch index mapping.
  - Mitigation: add unit parity test and log mismatches with snake id.

## Rollout notes

- Keep legacy path available behind a flag for quick comparison.
- If discrepancies appear, log the first mismatch with snake id and inputs.

## Compatibility matrix

- Server mode: unchanged behavior; batch path runs inside server world loop.
- Worker mode: unchanged behavior; batch path runs inside worker world loop.
- UI/main thread: unchanged; rendering still consumes serialized buffers.
