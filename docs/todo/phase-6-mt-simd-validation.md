# Phase 6 plan: Validation and rollout

## Revision notes

- 2026-01-15: Initial validation/rollout plan created.
- 2026-01-15: Expanded structure to align with archive plan templates.

## Purpose and scope

Validate MT+SIMD correctness, measure performance gains, and define rollout
gates. This phase ensures behavior parity, guards against regressions, and
documents operational guidance.

## Delta vs AGENTS.md

- Changes: add validation gates, perf targets, and rollout guidance.
- Unchanged: buffer layout and protocol shapes.
- Contract touch points: AGENTS.md “Tests and verification”.

## Scope

- In: parity tests, perf harness, rollout gates, and documentation updates.
- Out: new runtime features or gameplay changes.

## Non-goals

- No changes to brain math or sensor layout.
- No new network protocol fields.

## Assumptions

- JS remains the reference path for parity.
- SIMD and MT can be disabled independently for debugging.
- The binary frame format remains unchanged.

## Constraints and invariants

- INV-VAL-001: JS path is the ground truth for correctness.
- INV-VAL-002: SIMD/MT can be disabled without code changes.
- INV-VAL-003: No serializer/layout changes in this phase.

## Architecture narrative

Validation focuses on two axes:

1) Correctness: outputs match JS within tolerance, determinism preserved.
2) Performance: measured improvements at 100–300 snakes.

SIMD and MT are treated as accelerators, not required code paths. Any failure
falls back to JS with clear logging.

## Decisions locked for Phase 6

- SIMD defaults on after parity tests pass.
- MT defaults on only for server builds and only after perf targets are met.
- Browser MT stays opt-in due to COOP/COEP requirements.

## Key decisions and invariants registry

### Decisions

- DEC-VAL-001: Default enablement follows parity + perf gates.
- DEC-VAL-002: Browser MT remains opt-in.

### Invariants

- INV-VAL-001 through INV-VAL-003 remain enforced.

## Validation matrix

- Unit parity tests:
  - Dense/MLP kernels (single + batch).
  - GRU/LSTM/RRU kernels (multi-step).
- Integration tests:
  - Graph brain forward pass parity.
  - Server run with fixed seed and action logs.
- Performance tests:
  - Fixed workloads at 100, 150, 300 snakes.
  - Compare JS vs SIMD vs MT (server + worker).
- Manual checks:
  - Worker mode rendering and controls.
  - Server mode spectate/play behavior.
  - Hall of Fame spawning and God Mode interactions.

## Touch-point checklist (hard contracts)

- `src/brains/ops.ts` + `src/brains/graph/runtime.ts`: parity tests cover all kernels.
- `server/simServer.ts` + `src/worker.ts`: execution path toggles and logs.
- `docs/AGENTS.md` + `README.md`: documentation updates for MT/SIMD.

## Planned modules and functions

- New perf harness script under `scripts/` to compare JS/SIMD/MT.
- Add deterministic action log capture helper for server integration tests.
- Add a deterministic digest helper that hashes per-tick action outputs.

## Data model changes; migration; backward compatibility

- Add config flags for SIMD/MT enablement if not already present.
- No persistence or export changes required.

## Detailed implementation checklist

- [ ] Add parity tests for all kernels with fixed inputs.
- [ ] Add deterministic action log capture in server mode.
- [ ] Add perf harness to compare JS/SIMD/MT with profiler output.
- [ ] Emit perf results as JSON lines with metadata (commit, flags, counts).
- [ ] Add config flags for SIMD/MT enable/disable.
- [ ] Add documentation for COOP/COEP and troubleshooting.
- [ ] Add first-divergence reporting (tick, snake, node) to parity failures.

## Acceptance criteria

- Outputs match JS within tolerance across all kernels.
- Deterministic ordering preserved in server MT.
- Measured speedup meets targets (define baseline and target thresholds).
- No regressions in existing test suites.

## Performance targets

- Define baseline ms/tick at 150 and 300 snakes (JS-only batched pipeline).
- Target: >= 1.5x speedup with SIMD, >= 2.0x with server MT at 300 snakes.

## Correctness vs default-enable gates

- Correctness gate: parity within tolerance must pass before SIMD/MT can be
  enabled at all.
- Default gate: perf targets + stability must pass before defaults flip.

Example divergence report (illustrative):

```text
diverge tick=421 snake=37 node=gru maxAbs=2.3e-4 maxRel=8.1e-3
```

## Artifacts and output formats

Action digest (illustrative):

```text
digest mode=simd ticks=500 hash=9b3a7d6f6b2c2c9f
```

Perf JSON line (illustrative):

```json
{"mode":"mt","snakes":300,"ticks":2000,"warmup":200,"msPerTick":14.2,"p95":15.7,"commit":"abc123","node":"v22.4.0","flags":{"simd":true,"mt":true}}
```

## Acceptance criteria mapping

- AC-VAL-001: Kernel parity within tolerance.
  - Tests: extend existing brain parity suites.
- AC-VAL-002: Deterministic ordering under MT.
  - Tests: existing server integration parity coverage.

## Error handling strategy

- SIMD load failure falls back to JS and logs once.
- MT pool failure falls back to single-threaded inference.
- Graph spec mismatches force pool rebuild and output buffer reset.

## Observability and debugability

- Retain profiling logs for brain/sensors/physics breakdowns.
- Add one-time logs for SIMD and MT activation states.
- Provide a debug flag that logs the chosen execution path per tick.

## Documentation updates

- Update AGENTS.md with MT+SIMD architecture and invariants.
- Add README notes for COOP/COEP requirements and performance flags.
- Add a short troubleshooting section for disabled MT.

## Risk register

- Risk: floating-point drift alters evolution outcomes.
  - Mitigation: parity thresholds and long-run drift checks.
- Risk: MT scheduling bugs change control ordering.
  - Mitigation: fixed batch ordering and deterministic dispatch tests.
- Risk: COOP/COEP headers affect hosting configuration.
  - Mitigation: keep browser MT opt-in and document requirements.

## Rollout plan

1) Land SIMD with JS fallback and parity tests.
2) Enable SIMD by default after CI and local validation.
3) Enable server MT behind a config flag; collect perf metrics.
4) Enable server MT by default once perf targets are met.
5) Keep browser MT opt-in and document requirements.

## Compatibility matrix

- Server mode: SIMD on by default after parity; MT gated by perf targets.
- Worker mode: SIMD on by default after parity; MT opt-in only.
- UI/main thread: unchanged.
