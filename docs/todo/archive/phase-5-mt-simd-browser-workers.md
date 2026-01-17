# Phase 5 plan: Browser MT inference pool

## Revision notes

- 2026-01-15: Initial browser MT plan created.
- 2026-01-15: Expanded structure to align with archive plan templates.

## Purpose and scope

Add a browser-side inference pool in worker mode using Web Workers and
SharedArrayBuffer. This parallels the server MT approach but is gated on
cross-origin isolation and provides a SIMD-only fallback.

## Delta vs AGENTS.md

- Changes: add browser worker pool in worker mode with SharedArrayBuffer.
- Unchanged: render pipeline, serializer layout, and protocol shapes.
- Contract touch points: AGENTS.md “Runtime architecture and data flow”.

## Scope

- In: browser worker pool, shared buffers, COOP/COEP headers, and capability
  detection.
- Out: server MT details (Phase 4) and SIMD kernel implementation (Phases 2/3).

## Non-goals

- No changes to main thread rendering.
- No changes to protocol message shapes.

## Assumptions

- Browser supports Web Workers and (optionally) SharedArrayBuffer.
- Worker mode remains the execution environment for local simulation.

## Constraints and invariants

- Requires cross-origin isolation for SharedArrayBuffer.
- Must work in both Vite dev and server modes.
- UI thread remains render-only; inference stays in worker mode.
- SIMD-only fallback must be available without isolation.
- Worker continues to post a transferable frame buffer to the UI thread.

## Architecture narrative

The worker-mode simulation creates a pool of inference workers. The primary
worker builds the control batch and writes inputs to a shared buffer. Workers
process disjoint ranges and write outputs back. The primary worker then applies
outputs to snakes and continues physics.

When `crossOriginIsolated` is false, the system disables MT and runs SIMD-only
or JS-only inference in the main worker.

## Decisions locked for Phase 5

- The worker pool is opt-in and requires capability checks.
- SharedArrayBuffer is used only when isolation is confirmed.
- JS-only fallback remains the safety net.

## Key decisions and invariants registry

### Decisions

- DEC-MT-BR-001: Pool only starts when `crossOriginIsolated` is true.
- DEC-MT-BR-002: Pool lifecycle is owned by the worker runtime, not the UI.
- DEC-MT-BR-003: MT enablement is opt-in via config flag.

### Invariants

- INV-MT-BR-001: Worker pool never mutates world state directly.
- INV-MT-BR-002: Output ordering matches batch index order.
- INV-MT-BR-003: JS fallback remains available.
- INV-MT-BR-004: Worker-to-UI frame messages remain transferable (SAB is
  internal to inference only).

## Module map and responsibilities

- `src/worker.ts`
  - Orchestrates batch build and dispatch.
  - Applies outputs and runs physics.
- `src/workerPool.ts`
  - Manages worker lifecycles and synchronization.
- `src/worker/inferWorker.ts`
  - Runs SIMD kernels on assigned batch ranges.
- `vite.config.ts`
  - Adds COOP/COEP headers in dev.
- `server/index.ts`
  - Adds COOP/COEP headers in server mode.

## Touch-point checklist (hard contracts)

- `src/worker.ts`: pool dispatch and fallback handling.
- `src/workerPool.ts`: worker lifecycle and synchronization.
- `src/worker/inferWorker.ts`: SIMD kernel dispatch for assigned ranges.
- `vite.config.ts` and `server/index.ts`: COOP/COEP headers.

## Planned modules and functions

### `src/workerPool.ts`

```ts
type PoolStatus = 'disabled' | 'starting' | 'ready' | 'failed';

initPool(specKey: string): Promise<void>;
dispatchBatch(batchStart: number, batchCount: number): Promise<void>;
shutdownPool(): Promise<void>;
```

### `src/worker/inferWorker.ts` message protocol

```text
init { specKey, inputStride, outputStride, buffers }
infer { batchStart, batchCount }
shutdown {}
```

## Data model changes; migration; backward compatibility

- New SAB-backed buffers when MT is enabled.
- No changes to saved exports or graph specs.
- Backward compatibility: SIMD-only or JS-only fallback when MT is disabled.

## State flow: capability detection and pool lifecycle

State table

| State | Description | Invariants |
| --- | --- | --- |
| Unsupported | SAB not available | MT disabled |
| Disabled | Capability OK, MT off | JS/SIMD path only |
| Starting | Pool spawning | no dispatch |
| Ready | Pool initialized | dispatch allowed |
| Failed | Pool init failed | JS fallback |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| detect | Unsupported | Disabled | crossOriginIsolated true | enable option | fallback ok |
| enablePool | Disabled | Starting | MT flag on | spawn workers | no dispatch |
| initOk | Starting | Ready | all workers init | allow dispatch | ordering |
| initFail | Starting | Failed | init error | log once | JS fallback |
| disablePool | Ready | Disabled | flag off | shutdown | fallback |

## Detailed implementation checklist

- [ ] Add `src/workerPool.ts` with worker lifecycle management.
- [ ] Define worker message protocol (`init`, `infer`, `shutdown`).
- [ ] Implement `src/worker/inferWorker.ts` for batched inference.
- [ ] Integrate pool dispatch in `src/worker.ts`.
- [ ] Add COOP/COEP headers in dev and server modes.
- [ ] Add capability detection and fallback logic.
- [ ] Add basic unit tests for capability checks and fallback paths.
- [ ] Verify inference workers spawn correctly from inside `src/worker.ts`
      in dev, build, and preview.
- [ ] Keep worker message protocol shapes unchanged (no new public messages).
- [ ] Keep frame transfer semantics unchanged (transferable buffer to UI).

## Cross-origin isolation handling

- Enable headers:
  - `Cross-Origin-Opener-Policy: same-origin`
  - `Cross-Origin-Embedder-Policy: require-corp`
- Detect with `crossOriginIsolated` and `typeof SharedArrayBuffer !== 'undefined'`.
- If false, disable MT and log a single warning.

Example capability gate (illustrative):

```ts
const canUseMt = crossOriginIsolated && typeof SharedArrayBuffer !== 'undefined';
```

## COOP/COEP operational notes

- All subresources must be same-origin or explicitly CORP/CORS compatible.
- Verify wasm, worker scripts, and fonts are served with compatible headers.
- Avoid third-party CDNs by default when MT is enabled.
- Verify behavior in dev, build, and preview modes.

## Error handling strategy

- If worker init fails, disable MT and fall back to SIMD/JS.
- If a worker crashes mid-run, log once and disable MT for the session.
- If buffer sizes mismatch, skip MT for that tick and fall back.

## Performance considerations

- Keep workers warm; avoid per-tick startup.
- Use contiguous batch ranges to reduce coordination overhead.
- Disable MT below a batch size threshold.

## Observability and debugability

- Debug-gated log: `mt.browser.pool.ready { workers }`.
- Debug-gated log: `mt.browser.pool.dispatch { batchCount }`.
- Always-on warn: `mt.browser.pool.failed { reason }`.

## Acceptance criteria

- Worker mode runs with MT when isolation is enabled.
- Worker mode falls back to SIMD-only when isolation is missing.
- Behavior matches baseline outputs for fixed seeds and inputs.

## Acceptance criteria mapping

- AC-MT-BR-001: Pool runs only under crossOriginIsolated.
  - Tests: capability gating unit coverage.
- AC-MT-BR-002: Output ordering matches JS baseline.
  - Tests: add parity test in existing worker coverage.

## Tests and validation

- Unit:
  - Capability detection and fallback logic.
  - MT output ordering vs JS baseline.
- Manual:
  - Run Vite dev with headers and verify worker pool is active.
  - Run without headers and verify fallback path is used.

## Risk register

- Risk: COOP/COEP breaks embedding or cross-origin assets.
  - Mitigation: keep MT opt-in and document requirements.
- Risk: worker pool overhead offsets gains for small batches.
  - Mitigation: disable MT below threshold.
- Risk: debugging complexity increases with multiple workers.
  - Mitigation: add clear logs and JS fallback switch.

## Rollout notes

- Keep MT disabled by default in browser builds.
- Provide a user-facing note in README about isolation requirements.

## Compatibility matrix

- Server mode: unchanged.
- Worker mode: MT optional; SIMD/JS fallback available.
- UI/main thread: unchanged.
