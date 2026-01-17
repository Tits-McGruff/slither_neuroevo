# Phase 4 plan: Server MT inference pool

## Revision notes

- 2026-01-15: Initial server MT pool plan created.
- 2026-01-15: Expanded structure to align with archive plan templates.

## Purpose and scope

Add a multithreaded inference pool to the Node server using worker_threads.
This parallelizes batched inference while keeping physics and world updates
single-threaded.

## Delta vs AGENTS.md

- Changes: add a server-side inference pool using worker_threads.
- Unchanged: world physics, serializer layout, and protocol shapes.
- Contract touch points: AGENTS.md “Runtime architecture and data flow” and
  “Simulation core”.

## Scope

- In: server worker pool, shared buffers, and deterministic dispatch.
- Out: browser MT (Phase 5) and SIMD kernel implementations (Phases 2/3).

## Non-goals

- No changes to physics or collision logic.
- No changes to frame serialization.

## Assumptions

- Node worker_threads are available in the runtime environment.
- SharedArrayBuffer is allowed in the server environment.

## Constraints and invariants

- World state is never accessed from worker threads.
- Shared buffers must be versioned by generation/spec.
- Worker pool size is configurable and bounded.
- Deterministic output ordering is preserved.

## Architecture narrative

The server owns a worker pool that receives slices of the batched input buffer
and writes outputs to a shared output buffer. Each tick:

1) The world builds the control batch and fills input buffers.
2) The server dispatches batch segments to workers.
3) Workers run SIMD (or JS) kernels in isolation.
4) The server waits for completion, then applies outputs in order.

Weights live in shared buffers, refreshed only when the population or graph
spec changes.

## Decisions locked for Phase 4

- SharedArrayBuffer is used for inputs and outputs.
- Worker threads are long-lived; they are not recreated per tick.
- If the pool is unavailable, fall back to single-threaded inference.

## Key decisions and invariants registry

### Decisions

- DEC-MT-SRV-001: Workers only read inputs and write outputs; no world access.
- DEC-MT-SRV-002: Pool lifecycle is owned by `SimServer`.
- DEC-MT-SRV-003: Shared buffers are recreated on spec changes.
- DEC-MT-SRV-004: Batch partitions are contiguous and computed deterministically.
- DEC-MT-SRV-005: Completion uses message acknowledgements (v1) before Atomics.

### Invariants

- INV-MT-SRV-001: Output ordering matches batch index order.
- INV-MT-SRV-002: Buffers are tagged with a spec key and generation.
- INV-MT-SRV-003: JS fallback is always available.
- INV-MT-SRV-004: Workers never access RNG, profiling counters, or world state.

## Module map and responsibilities

- `server/brainPool.ts`
  - Worker pool lifecycle and scheduling.
  - Shared buffer ownership and versioning.
- `server/worker/inferWorker.ts`
  - Runs batched kernels and signals completion.
- `server/simServer.ts`
  - Owns dispatch and waits for worker completion.
- `src/brains/wasmBridge.ts`
  - Used in workers to load SIMD kernels.

## Touch-point checklist (hard contracts)

- `server/simServer.ts`: pool lifecycle, dispatch, and fallback handling.
- `server/brainPool.ts`: buffer ownership, scheduling, and versioning.
- `server/worker/inferWorker.ts`: kernel execution and completion signaling.
- `src/brains/wasmBridge.ts`: SIMD loader inside workers.

## Buffer ownership and versioning

Each pool instance carries:

- `specKey`: graph spec hash.
- `weightsBuffer`: shared Float32Array.
- `inputBuffer`: shared Float32Array.
- `outputBuffer`: shared Float32Array.

On spec change or population rebuild:

- Invalidate existing buffers.
- Rebuild weights buffer.
- Reset worker state to the new specKey.

## Planned modules and functions

### `server/brainPool.ts`

```ts
type PoolStatus = 'disabled' | 'starting' | 'ready' | 'failed';

initPool(specKey: string, weights: Float32Array): Promise<void>;
dispatchBatch(batchStart: number, batchCount: number): Promise<void>;
shutdownPool(): Promise<void>;
```

### `server/worker/inferWorker.ts` message protocol

```text
init { specKey, inputStride, outputStride, weightsBuffer, inputBuffer, outputBuffer }
infer { batchStart, batchCount }
shutdown {}
```

### `server/simServer.ts`

```ts
ensurePool(): void;
dispatchInference(batch: ControlBatch): Promise<void>;
```

## Data model changes; migration; backward compatibility

- New shared buffers for inputs/outputs/weights in server process.
- No changes to saved populations or exports.
- Backward compatibility: MT pool can be disabled at runtime.

## State flow: worker pool lifecycle

State table

| State | Description | Invariants |
| --- | --- | --- |
| Disabled | MT disabled by config | JS path only |
| Starting | Workers spawning | no dispatch yet |
| Ready | Pool initialized | dispatch allowed |
| Failed | Init failed | JS fallback only |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| enablePool | Disabled | Starting | MT flag on | spawn workers | no dispatch |
| initOk | Starting | Ready | all workers init | start dispatch | ordering |
| initFail | Starting | Failed | any init fails | log once | JS fallback |
| disablePool | Ready | Disabled | flag off | shutdown workers | fallback |

## Detailed implementation checklist

- [ ] Add `server/brainPool.ts` with worker thread management.
- [ ] Define a worker message protocol (`init`, `infer`, `shutdown`).
- [ ] Add `server/worker/inferWorker.ts` with SIMD and JS paths.
- [ ] Integrate pool dispatch in `server/simServer.ts`.
- [ ] Add configuration flags for pool size and MT enable/disable.
- [ ] Add unit tests for scheduling, ordering, and buffer versioning.

## Error handling strategy

- If any worker fails to init, disable the pool and fall back to JS.
- If a worker crashes mid-run, log once and disable MT for the process.
- If specKey mismatches are detected, rebuild buffers and re-init pool.

## Performance considerations

- Keep workers warm; avoid per-tick startup.
- Dispatch contiguous batch ranges to reduce synchronization overhead.
- Use a bounded pool size (min 1, max CPU cores - 1).
- If message overhead dominates, migrate the completion barrier to Atomics
  wait/notify on a small shared `Int32Array` while keeping the same partition
  and ordering rules.

## Observability and debugability

- Debug-gated log: `mt.pool.ready { workers, specKey }`.
- Debug-gated log: `mt.pool.dispatch { batchCount }`.
- Always-on warn: `mt.pool.failed { reason }`.

## Determinism and ordering

- Batch indices remain stable.
- Outputs are applied in the original batch order.
- Workers never reorder outputs; they write to fixed offsets.

## Deterministic partitioning and barrier

- Partition rule: contiguous slices based on batchCount and workerCount.
- Completion rule: apply outputs only after all workers ack completion.
- Batch threshold: enable MT based on configured capacity, not live count.

Example partitioning (illustrative):

```ts
const chunk = Math.ceil(batchCount / workerCount);
const start = workerIndex * chunk;
const count = Math.max(0, Math.min(chunk, batchCount - start));
```

## Acceptance criteria

- Server MT produces identical outputs vs single-threaded baseline.
- Performance improves at high snake counts without regressions.
- Pool handles reset and graph changes without stale outputs.

## Acceptance criteria mapping

- AC-MT-SRV-001: MT output ordering matches JS baseline.
  - Tests: worker pool ordering tests + server integration parity.
- AC-MT-SRV-002: Pool handles spec changes without stale outputs.
  - Tests: worker pool specKey reset coverage.

## Tests and validation

- Unit:
  - Scheduling, ordering, and spec versioning tests for the pool.
  - Worker init/infer/shutdown protocol tests.
- Integration:
  - Fixed-seed action log parity for MT vs JS.
- Regression:
  - Existing server tests remain green (`npm run test:integration`).

## Risk register

- Risk: buffer mismatches on graph spec changes corrupt outputs.
  - Mitigation: specKey validation + pool rebuild.
- Risk: worker startup dominates tick time.
  - Mitigation: keep workers warm and reuse pool.
- Risk: IPC overhead erases gains on small batches.
  - Mitigation: disable MT below a batch size threshold.

## Rollout notes

- Default to MT off; enable via config flag.
- Add logging for pool initialization and fallback reasons.

## Compatibility matrix

- Server mode: MT optional; JS fallback always available.
- Worker mode: unchanged.
- UI/main thread: unchanged.
