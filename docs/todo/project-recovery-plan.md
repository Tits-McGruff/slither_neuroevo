# Slither Neuroevolution project recovery plan

## Document control

- Status: authoritative, owner-approved implementation plan; Phases 0 and 1
  are complete, and Phase 2 has not started.
- Created: 2026-07-21.
- Branch: `exclusive-server-mode-refactor`.
- Audit baseline commit: `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`.
- Current implementation HEAD: `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`.
- Last fully verified HEAD: `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`.
- Baseline worktree: clean before the two planning-document changes.
- Current expected worktree changes are recorded under "Live execution status".
- Scope owner: the repository owner.
- Execution owner: the Codex session currently implementing the checked phase.
- Supersedes: `docs/todo/native_refactor_plan.md` and every implementation
  proposal under `docs/todo/archive/`.

This plan exists so implementation can continue safely after a fresh context,
an interruption, or a change of agent. It is deliberately more explicit than
a normal task list. An implementing agent must update the live status,
revision history, checkboxes, verification results, and handoff notes as work
progresses.

## Revision history

- 2026-07-21: Initial plan created from a local audit of baseline commit
  `cb276cc`.
- 2026-07-21: Locked the owner's two project-level decisions:
  Rust is the required neural-kernel accelerator, not a second full simulation;
  normal resets retain the seed and startup resumes the latest compatible
  checkpoint, while an explicit New Run action creates a new seed.
- 2026-07-21: Reconciled an independent plan review against the local source.
  Corrected the fixed-step speed model, phase numbering, deterministic-fixture
  dependency, checkpoint boundary, MT fault contract, protocol ordering,
  persistence format validation, early test hardening, and handoff metadata.
  Declined additions that would create unsupported framework or security scope.
- 2026-07-21: Began Phase 0 after verifying the repository root, branch,
  baseline HEAD, and the two expected planning-document worktree changes.
- 2026-07-21: Completed Phase 0 with an authoritative-state digest helper,
  truthful inference-mode diagnostics, and passing time, MT, persistence, and
  ordering characterizations. No Phase 1 repair was started.
- 2026-07-21: Began Phase 1 after re-reading the authoritative plan and
  repository instructions, verifying the Phase 0 handoff worktree, and
  confirming the implementation HEAD remained `cb276cc`.
- 2026-07-21: Implemented and focused-verified the Phase 1 canonical World
  step, durable population slots, fixed-step scheduler, honest scheduler
  diagnostics, and awaited server lifecycle/fault boundary. The focused gate
  passes; full regression verification is in progress.
- 2026-07-21: Completed Phase 1 after the required focused contracts, strict
  TypeScript, repository-wide ESLint, all 45 JavaScript test files, the Vite
  production build, stale-reference scan, and diff-hygiene check passed. Phase
  2 was not started.

## How to resume this work

Before editing source code in a new context:

1. Read this file completely.
2. Read the repository-root `AGENTS.md` for workflow rules, but consult the
   "Known stale documentation" section below before trusting its architecture
   descriptions.
3. Run `git status --short --branch` and preserve unrelated user changes.
4. Confirm the checked-out commit and compare it with the audit baseline,
   current implementation HEAD, and last fully verified HEAD recorded above.
   A later implementation HEAD is expected only when the handoff record
   explains it.
5. Read "Live execution status" and the most recent handoff journal entry.
6. Inspect the code touched by the next unchecked item. Do not assume the code
   still matches this plan if the branch has advanced.
7. Run the smallest relevant pre-change test to confirm the local baseline.
8. Implement only the current phase unless a prerequisite from an earlier
   phase is demonstrably missing.
9. Update this document before ending the session, even if the phase is only
   partially complete. Record current HEAD, last fully verified HEAD, worktree
   summary, active checklist item, last acceptance gate, and command results.

Do not commit or push merely to advance these fields; that still requires owner
authorization. If verified work is uncommitted, leave the HEAD fields truthful
and tie verification to an explicit worktree file list and command result in
the handoff journal.

Never mark a checkbox complete merely because code was written. A checkbox is
complete only when its stated verification has passed or the plan explicitly
records why verification is deferred.

## Authority and source-of-truth order

Use this order when sources disagree:

1. Direct owner instructions and decisions recorded in this plan.
2. Current, verified behavior of the checked-out source code.
3. This plan's target contracts and acceptance criteria.
4. Current automated tests, after checking that they genuinely exercise the
   claimed path.
5. Current README, AGENTS, and API documentation.
6. Archived plans, which are historical records only.

The archive was consulted only for document structure. No archived technical
assumption, architecture, fallback rule, performance target, or implementation
decision has been copied into this plan.

## Known stale documentation

The following statements in current documentation are contradicted by the
baseline code and must not be used as implementation truth:

- `AGENTS.md` says the active MT pool uses Atomics-based synchronization. The
  active pool uses per-message promises.
- `AGENTS.md` says live settings are posted to the server. They currently only
  mutate the browser's private `CFG`.
- `AGENTS.md` and `README.md` claim chunked persistence through
  `src/persistence/chunked.ts`. That file does not exist.
- `README.md` presents the Rust engine and MT execution as working runtime
  features. The active server path does not load the kernel bridge.
- `AGENTS.md` describes a graceful single-threaded JS/SIMD fallback, while the
  actual runtime has multiple conflicting fallback paths and an unfinished
  full-world native adapter.
- `docs/todo/native_refactor_plan.md` proposes a complete Rust simulation. The
  owner explicitly rejected that direction on 2026-07-21.
- `AGENTS.md` references `markdown-rules/rules.md`, but that file is absent at
  the baseline commit.

Documentation is corrected only after runtime behavior and tests are truthful.

## Problem statement

The server-only refactor combined partially completed migrations, duplicated
runtime paths, a generated native-package template, and tests that do not cover
the active architecture. The result is a server that can appear green while
ignoring its seed, not using Rust in the active path, corrupting or losing
recurrent state under MT, failing to send UI changes to the authoritative
world, and serializing snapshots through a single large in-memory buffer.

This is a recovery project, not a feature expansion. The goal is to establish
one understandable architecture with explicit ownership and tests that fail
when the advertised behavior is absent.

## Goals

- Make a run repeatable from a seed when settings and the step/sequence-assigned
  action log are the same.
- Make simulation time independent of scheduler jitter.
- Make the simulation-speed control represent an honest multiplier.
- Use the Rust N-API kernels in both single-threaded and multi-threaded
  inference.
- Keep each recurrent brain's state attached to its population slot.
- Restore server-authoritative live settings and God Mode operations.
- Give `points_delta_norm` one consistent, documented meaning.
- Store large populations with bounded peak serialization memory.
- Resume the latest compatible saved population on normal startup.
- Preserve legacy snapshot readability without continuing the legacy write
  format.
- Replace silent passes and weak smoke tests with contract-focused coverage.
- Keep the project explicitly local and hobby-oriented.
- Leave a continuously updated handoff record for future contexts.

## Non-goals

- Do not implement a second, full simulation engine in Rust.
- Do not restore the browser-local simulation or browser worker architecture.
- Do not add accounts, authentication, authorization, public hosting support,
  or an enterprise security model.
- Do not blindly restore deleted tests merely to increase the test count.
- Do not promise bit-identical long-horizon behavior between the JS and native
  math backends when their floating-point evaluation order differs.
- Do not redesign gameplay, fitness weights, sensor layout, graph editing, or
  rendering unless a repair requires a narrowly scoped change.
- Do not destructively rewrite or delete existing user databases.
- Do not edit historical files under `docs/todo/archive/`.
- Do not optimize before correctness measurements identify a real bottleneck.

## Locked decisions

### DEC-001: Server authority remains

The Node server owns simulation state. The browser remains a rendering and
control client.

### DEC-002: Rust has one focused responsibility

Rust supplies N-API neural-network kernels for Dense, MLP, GRU, LSTM, and RRU
operations. Rust does not own world state, physics, sensors, spawning,
evolution, persistence, or networking.

### DEC-003: Native kernels are the normal backend

Normal runs require the native addon to load successfully. A JS backend remains
available only through an explicit diagnostic configuration. Missing native
code must not silently produce a supposedly accelerated run.

### DEC-004: Backend and threading are independent axes

The supported diagnostic matrix is:

| Math backend | One thread | Multiple threads |
| --- | --- | --- |
| Native kernels | Supported | Supported |
| JS reference | Supported for diagnosis | Supported for diagnosis |

Enabling MT must not disable native kernels. Disabling MT must not disable
native kernels.

### DEC-005: No silent mid-generation fallback

If an explicitly requested worker pool fails after a generation has begun, the
server enters a faulted simulation state. It rejects the in-flight step, runs
no further authoritative steps, keeps status/control endpoints available, and
broadcasts the structured failure. It must not switch brain implementations or
resume partially mutated in-memory state. Recovery is an explicit Reset, New
Run, or checkpoint resume at a boundary where recurrent state is defined as
zero.

### DEC-006: Stable recurrent ownership

During a pool epoch, each population slot has one long-lived brain instance
owned by one worker, using a deterministic slot-to-worker mapping. Batch
compaction, snake deaths, and changing batch partitions do not transfer
recurrent ownership.

### DEC-007: Reset and seed behavior

Apply/reset restarts from generation one with the current seed and the newly
applied authoritative configuration. It creates a new run ID because it starts
a new evolutionary lineage, but the initial world repeats when seed and
configuration are unchanged. A distinct New Run / New Seed action does the
same with a fresh seed. Seed and run-ID changes are explicit and visible.

### DEC-008: Startup and checkpoint behavior

Normal startup resumes the latest compatible checkpoint. An explicit
`--fresh` or equivalent configuration starts without resuming. Starting fresh
does not delete prior checkpoints. Reset and New Run must create a durable
generation-one run-start checkpoint before the new run is advertised as
current, so a crash cannot silently restore the preceding run.

### DEC-009: Persistence compatibility

New writes use a bounded-memory format. Existing `genomes_blob` snapshots
remain readable through a legacy path. A corrupt latest checkpoint must not be
silently treated as an empty database.

### DEC-010: Local-only operating model

The default bind remains loopback. Input validation and size limits protect the
process from accidental bad data. No authentication work is in scope.

## Engineering decisions owned by the implementation

These decisions are detailed enough to avoid repeated owner interruptions but
may be revised if code evidence invalidates them:

- Use one canonical world-step pipeline instead of maintaining near-duplicate
  sync and async physics paths.
- Keep `World` steps at one constant `baseDt`. Use the existing `SimCore`
  accumulator to schedule zero or more complete steps per wall-clock pump;
  `simSpeed` never enlarges the delta passed to a world step.
- Bound steps per scheduler pump. If throughput cannot meet the requested
  multiplier, discard and report excess wall-clock debt rather than skipping
  simulation state or creating an enlarged/partial step.
- Give population snakes an explicit `populationSlot` rather than treating
  their current array position as a durable identity.
- Use versioned stateful PRNG streams. Keep simulation randomness separate
  from rendering and other cosmetic randomness.
- Assign accepted commands a simulation-step number and monotonic sequence;
  apply them in that total order at a step boundary, never while an MT
  inference promise is in flight.
- Store each genome's Float32 weights as a separate SQLite BLOB row.
- Keep rare non-population neural snakes, such as resurrected or disconnected
  external snakes, on the single-threaded native path.
- Use explicit `batchId`, `poolEpoch`, and `weightEpoch` values in worker
  messages. Keep the evolutionary generation number as a distinct concept.
- Request MT visualization data from the worker that owns the selected slot
  instead of reading stale main-thread brain activations.
- Select the math backend immutably before any brain is constructed. Do not add
  a dependency-injection framework unless the existing module boundary cannot
  enforce that contract.
- Use Protocol 2 for the repaired client/server contract rather than silently
  accepting a Protocol 1 client that lacks authoritative operations.
- Give each new evolutionary lineage a lightweight run ID. Do not add a
  separate run-history service; snapshots remain the durable history.
- Keep MT disabled by default until the correctness matrix passes and local
  benchmarks show that it helps the configured workload. This does not block
  native kernels from being the default math backend.

## Baseline verification

The following was observed locally at the baseline commit:

- Node: `v24.12.0`.
- npm: `11.8.0`.
- Rust: `rustc 1.92.0` and `cargo 1.92.0`.
- Direct TypeScript check passed.
- Direct ESLint check passed.
- Vite production client build passed.
- Rust release tests passed: 3 tests.
- After explicitly building the addon, Vitest passed: 38 files and 152 tests.
- Before building the addon, direct Vitest produced 151 passes and one
  performance-test failure because `native/index.js` was missing.
- In that missing-addon run, all native bridge tests still displayed as passed
  because each test returned early when native loading failed.
- The worktree was clean before the two planning-document edits.

These results describe the current harness; they are not proof that the target
behavior works.

## Evidence about the removed parity tests

Local history establishes the following sequence:

1. A committed `test_output.txt` records MT/SIMD parity diverging at tick 1.
2. Commit `a5cf885` reduced the parity run from 20 ticks to 5 and increased the
   allowed frame tolerance from `0.001` to `2.0`.
3. Commit `afdc4e7` deleted `server/mtParity.test.ts` while introducing the
   native package template.
4. The deletion commit message is `.` and contains no design rationale.
5. `src/worker.test.ts` was deleted when the browser worker path was removed,
   but equivalent server-authoritative behavior tests were not added.

The replacement tests in this plan are derived from runtime contracts. The
deleted files are evidence and reference material, not specifications to copy.

## Defect registry

| ID | Defect | Baseline evidence | Planned phase |
| --- | --- | --- | --- |
| CORE-001 | Sync and async world paths duplicate control and physics logic | `World.update` / `updateAsync` and `_stepPhysics` / `_stepPhysicsAsync` | 1 |
| CORE-002 | Serial inference executes while scanning snakes, but pooled inference waits until all control branches are collected | `_stepPhysics` calls `brain.forward` inline while `_stepPhysicsAsync` dispatches after its collection loop | 1 |
| TIME-001 | Physics depends on measured wall-clock delta | `SimServer.tick` derives `dt` from `performance.now` | 1 |
| TIME-002 | 1x and 12x speed are both capped to 0.01 seconds per server tick | `World.update` clamps scaled time with default `CFG.dtClamp = 0.01` | 1 |
| TIME-003 | Scaled-time substeps resample neural control while external sensors and baseline-bot updates occur only once per outer update | `World.update` / `updateAsync` wrap repeated physics substeps with outer controller/bot calls | 1 |
| DET-001 | `worldSeed` is accepted but unused by `SimCore` and `World` | `SimCoreOptions.worldSeed` is never passed to `World` | 2 |
| DET-002 | Core spawning, genetics, death drops, mutation, crossover, and food use global randomness | `world.ts`, `snake.ts`, `mlp.ts`, `brains/ops.ts`, `utils.ts` | 2 |
| DET-003 | Reset and import discard meaningful RNG continuation state | `SimCore.reset` and `World.importPopulation` | 2 and 7 |
| DET-004 | Baseline-bot RNG continuation is held inside opaque `createRng` closures and cannot be exported, digested, or restored | `src/rng.ts` and `BaselineBotManager.botRngs` | 2 |
| NAT-001 | Active startup does not load the neural kernel bridge before brain construction | `server/index.ts` and active worker path | 3 |
| NAT-002 | Full-world native adapter requires a Rust `World` export that does not exist | `server/native-backend.ts` and `native/src/lib.rs` | 3 |
| NAT-003 | N-API functions enter unsafe pointer code without validating array lengths and dimensions | `native/src/SIMD_Kernals.rs` | 3 |
| NAT-004 | Native package metadata and targets are largely napi-rs template defaults | `native/package.json` | 3 and 9 |
| NAT-005 | Canonical worker message handling can begin before asynchronous native loading completes | `server/worker/inferWorker.ts` installs its handler before the final native-load await | 3 and 4 |
| MT-001 | Two incompatible server brain pools exist | `server/brainPool.ts` and `src/sim/NodeBrainPool.ts` | 4 |
| MT-002 | The active pool begins with zero-filled weights and skips initial synchronization | `NodeBrainPool.init` and `SimServer.mtGeneration` | 4 |
| MT-003 | Active workers allocate private recurrent stores when `stateSize` is omitted | `SimServer.initMT` / `ensureBrainPool` and `src/worker/inferWorker.ts` | 4 |
| MT-004 | Recurrent state is not reset when generation weights change | No active call equivalent to `resetBrains` | 4 |
| MT-005 | Deaths and compacted batches can move recurrent snakes between workers | Batch chunks are assigned by current batch position | 4 |
| MT-006 | Resurrected and released external neural snakes can index beyond population weights | World batch indices are snake-array indices | 4 |
| MT-007 | Worker failures silently disable MT and continue with stale main-thread recurrent brains | `SimServer.tick` catch path | 4 |
| MT-008 | Server startup does not await asynchronous MT initialization | `startServer` calls `simServer.start()` without awaiting | 4 |
| MT-009 | Visualizer data is intentionally suppressed whenever MT is active | `SimServer.buildStats` | 4 |
| MT-010 | The async World path mutates pre-inference state before pooled inference resolves | `World.updateAsync` / `_stepPhysicsAsync` and `SimServer.tick` | 1 and 4 |
| SNS-001 | `prepareForStep` overwrites the score marker immediately before neural sensing | `Snake.prepareForStep` and sensor index 8 | 5 |
| SNS-002 | External and neural controllers sample score delta at different lifecycle points | `World._publishControllerSensors` versus physics control evaluation | 5 |
| UI-001 | Live controls mutate browser `CFG` only | `main.ts liveUpdateFromSlider` | 6 |
| UI-002 | Simulation-speed input refreshes only its displayed label | `main.ts` speed input listener | 6 |
| UI-003 | God Mode kill logs but sends nothing; drag discards coordinates | `main.ts` canvas handlers | 6 |
| UI-004 | Live/reset metadata is private to a DOM-oriented module | `src/settings.ts` | 6 |
| UI-005 | At least two "live" values need explicit derived-state handling | collision cell size and baseline bot respawn delay | 6 |
| CFG-001 | Config hash and seed captured by HTTP/welcome state become stale after changes | `server/index.ts` closure values | 6 and 7 |
| CFG-002 | Config identity depends on raw object property order | `server/hash.ts` hashes `JSON.stringify` output directly | 6 |
| PER-001 | Advertised chunked module does not exist | Documentation versus filesystem | 7 |
| PER-002 | Save converts all weights to JSON arrays, accumulates buffers, concatenates, then gzip-compresses synchronously | `World.exportPopulation` and `server/persistence.ts` | 7 |
| PER-003 | Startup reads snapshot settings but never restores saved genomes | `server/index.ts` | 7 |
| PER-004 | HTTP export recreates the entire population as one JSON response | `server/httpApi.ts` | 7 |
| PER-005 | Automatic checkpoints default to disabled and the current save hook runs after new-generation spawning/random draws | `server/config.ts`, `World._endGeneration`, and server generation-change handling | 2 and 7 |
| TEST-001 | Native tests silently pass without native | `src/brains/nativeBridge.test.ts` | 3 and 8 |
| TEST-002 | Network suites silently return on bind permission errors | server acceptance/integration/system/security suites | 1, 6, and 8 |
| TEST-003 | The main UI test asserts little beyond WebSocket construction | `src/main.test.ts` | 8 |
| TEST-004 | Test category scripts cover filenames, not coherent contracts | `scripts/run-tests.ts` | 8 |
| DOC-001 | README, AGENTS, and API docs describe behavior absent from code | current documentation | 9 |
| DOC-002 | `AGENTS.md` describes `server/config.toml` as an existing defaults file, but the baseline checkout has no such file and `parseConfig` creates it as a startup side effect | `AGENTS.md`, `server/config.ts`, and the baseline tree | 9 |
| DOC-003 | `AGENTS.md` requires `markdown-rules/rules.md`, but that policy file and directory are absent from the checkout | `AGENTS.md` and the baseline tree | 9 |

## Target architecture

### Runtime flow

```text
Browser controls
  -> validated WebSocket command
  -> SimServer pending-command queue
  -> tick boundary
  -> SimCore fixed step
  -> one World step pipeline
  -> sensors and controller selection
  -> eligible population slots -> canonical BrainPool
  -> each stable owner worker -> native N-API kernels
  -> outputs returned in batch order
  -> movement, food, collisions, evolution
  -> serialized frame and stats
```

### State ownership

| State | Owner | Persistence/reset rule |
| --- | --- | --- |
| World, snakes, pellets, generation | Main server thread | Rebuilt from explicit seed/checkpoint |
| Authoritative simulation PRNG | World/main server thread | Versioned state captured at safe checkpoint boundary |
| Population weights | World population plus shared read buffer | Copied with an acknowledged weight epoch |
| Recurrent brain state | Stable worker owning the population slot | Reset on new population/import/reset; never inferred from batch position |
| Extra neural snake state | Main-thread brain instance | Never indexes population shared weights |
| UI view and selection | Browser | Non-authoritative |
| Live setting commands | SimServer queue until tick boundary | Applied and acknowledged atomically |
| Saved genomes | SQLite per-genome rows | Transactional, versioned, legacy-readable |

### Canonical inference components

The intended canonical implementation is:

- `server/brainPool.ts` for Node worker lifecycle, epochs, timeouts, and shared
  buffers.
- `server/worker/inferWorker.ts` for worker-owned brains and native loading.
- A small shared protocol module if parent and worker message types would
  otherwise be duplicated.
- `BatchInferenceRunner` or its replacement as a narrow World-facing
  interface.

The weaker duplicate chain under `src/sim/BaseBrainPool.ts`,
`src/sim/NodeBrainPool.ts`, `src/sim/poolProtocol.ts`, and
`src/worker/inferWorker.ts` is removed after the canonical pool passes its
replacement tests.

## Cross-cutting invariants

### Determinism

- INV-DET-001: No authoritative simulation decision reads `Math.random`.
- INV-DET-002: The same seed, settings, backend, completed-step count, and
  step/sequence-assigned action log produce the same authoritative state.
- INV-DET-003: Rendering randomness cannot advance simulation RNG state.
- INV-DET-004: Wall-clock timestamps affect scheduling and profiling only.
- INV-DET-005: Reset retains the seed unless New Run explicitly replaces it.
- INV-DET-006: RNG algorithm and state formats are versioned.
- INV-DET-007: Commands are replayable by assigned simulation step and a
  deterministic sequence within that step; raw socket arrival time is not the
  replay order.

Exact replay is required within the same code revision, RNG and snapshot
versions, graph definition, runtime/backend build, target architecture, and
supported execution environment. Cross-build or cross-platform exactness is a
compatibility goal only where tests establish it. JS/native kernel parity uses
explicit numeric tolerances.

### Time

- INV-TIME-001: Every authoritative `World` step receives the same configured
  `baseDt`, never measured callback jitter or `baseDt * simSpeed`.
- INV-TIME-002: `simSpeed` controls the requested wall-clock rate at which
  complete fixed steps are scheduled.
- INV-TIME-003: A pump may execute zero or more whole fixed steps. A catch-up
  cap may discard and report wall-clock debt, but it never skips simulation
  state or substitutes a partial/enlarged step.
- INV-TIME-004: Given the same seed, settings, command log, and completed step
  count, state is independent of speed, scheduler jitter, and pump grouping.

### Native and MT

- INV-INF-001: Native and MT flags never disable one another implicitly.
- INV-INF-002: Brain instances are constructed only after their selected math
  backend is ready.
- INV-INF-003: A normal native run fails clearly if the addon is unavailable.
- INV-INF-004: Every population slot has exactly one recurrent-state owner
  while a pool epoch is active.
- INV-INF-005: Initial weights are present before the first MT inference.
- INV-INF-006: A new weight epoch is not used until workers acknowledge reset.
- INV-INF-007: Stale or mismatched batch IDs cannot satisfy current work.
- INV-INF-008: Worker failure cannot cause an implicit recurrent-state switch.
- INV-INF-009: A worker/protocol fault prevents any partially completed step
  from becoming externally authoritative and faults the run until explicit
  boundary recovery.

### UI and protocol

- INV-UI-001: Browser controls never masquerade as authoritative state.
- INV-UI-002: Live updates apply only at a server tick boundary.
- INV-UI-003: Server validation uses the same path/type/range metadata as the
  UI.
- INV-UI-004: Reset-required values cannot be sent through the live path.
- INV-UI-005: God Mode results reflect server application, not optimistic logs.
- INV-UI-006: Dragging translates the whole snake body consistently.
- INV-UI-007: One atomically accepted settings request increments the global
  configuration revision once and broadcasts the normalized authoritative
  result to every UI client.

### Sensors

- INV-SNS-001: `points_delta_norm` means score change since that snake's
  previous sensor sample.
- INV-SNS-002: Sampling updates the marker exactly once per delivered
  observation.
- INV-SNS-003: Neural and external control paths use the same sampling method.

### Persistence

- INV-PER-001: New snapshot writes never build one combined population buffer.
- INV-PER-002: New snapshot writes never convert the entire population to
  plain JSON weight arrays.
- INV-PER-003: Metadata and all genome rows commit atomically.
- INV-PER-004: Legacy snapshots remain read-only compatible.
- INV-PER-005: Normal startup either resumes a valid checkpoint or reports why
  it cannot; it does not silently discard saved evolution.
- INV-PER-006: A population checkpoint is not falsely documented as a complete
  mid-tick world save.
- INV-PER-007: A generation-boundary checkpoint captures the new population,
  zero recurrent-state assumption, RNG/allocator state, and generation number
  before the first spawn, pellet, focus, sensor, or inference operation.
- INV-PER-008: Reset and New Run become current only after their generation-one
  run-start checkpoint commits.

### Testing

- INV-TST-001: A required dependency being absent fails the relevant suite.
- INV-TST-002: Unsupported test environments use an explicit opt-out, not a
  green early return.
- INV-TST-003: Tests assert the selected runtime mode actually ran.
- INV-TST-004: Long-horizon deterministic comparisons use the same math
  backend. JS/native kernel comparisons use explicit numeric tolerances at
  bounded operations.

## Dependencies and execution order

The phases are ordered to avoid building new features on duplicated or
nondeterministic foundations:

0. Establish honest characterization and execution diagnostics.
1. Unify the World step and correct time.
2. Implement authoritative seeded randomness.
3. Make native kernels safe and activate them in the runtime.
4. Consolidate MT inference and correct recurrent-state ownership.
5. Correct score-delta sensor semantics.
6. Restore server-authoritative UI operations and God Mode.
7. Replace persistence and implement actual resume.
8. Reconstruct the test and CI contract around the repaired architecture.
9. Correct documentation and remove remaining migration debris.

Tests are added with the phase they protect. Phase 8 reorganizes and hardens
the overall harness; it does not postpone all testing until the end.

## Phase 0: Honest characterization and execution diagnostics

### Purpose

Create a small set of tests and runtime observations that demonstrate which
path actually ran. Do not leave the repository with intentionally failing
tests at the end of the phase.

### Checklist

- [x] Add a test helper that captures authoritative state or a stable digest
  with first-divergence details from any supplied `World` instance.
- [x] Define canonical digest ordering and Float32 encoding. Prefer durable
  identity/population slot and raw Float32 bits; reject or report non-finite
  values rather than normalizing them. Keep observer-only state out of the
  authoritative digest. Also exclude run IDs, timestamps, and other metadata
  that may legitimately differ between two equivalent replays; test those
  fields through their own contracts.
- [x] Use explicit hand-constructed component fixtures where needed. Do not
  create a supposedly seeded full-World fixture until Phase 2 adds the
  production RNG seam.
- [x] Add an inference-mode record containing requested backend, active
  backend, requested MT state, active worker count, pool epoch, weight epoch,
  graph key, parameter count, seed, and native-addon build identifier.
- [x] Expose that mode through logs and a test-accessible server status path.
- [x] Put broken-behavior reproductions in an unmistakable characterization
  suite. Name each test with its defect ID and record the phase in which it
  must expire or be converted.
- [x] Characterize `TIME-002`/`TIME-003`, including neural, external-controller,
  and baseline-bot cadence under scaled substeps; convert in Phase 1.
- [x] Characterize `MT-002`, proving first-tick pooled weights differ from the
  population; convert in Phase 4.
- [x] Characterize `PER-003`/`PER-005`, including non-restored genomes and the
  current post-spawn checkpoint point; convert in Phase 7.
- [x] Record the exact current control/physics/generation ordering needed to
  judge Phase 1's deliberate behavior changes.
- [x] Record any newly discovered blockers in the defect registry.

### Verification

- [x] Direct TypeScript check passes.
- [x] Focused characterization tests pass with assertions describing current
  behavior.
- [x] No production behavior is changed except additive diagnostics.

### Handoff checkpoint

Record new helper filenames, diagnostic field shapes, and the exact commands
used under "Handoff journal".

### Phase 0 characterization record

#### Current outer-update and substep ordering

The baseline `World.update` and `World.updateAsync` paths currently perform the
following sequence:

1. Begin profiling, compute
   `scaled = clamp(dt * simSpeed, 0, max(0.004, CFG.dtClamp))`, and derive a
   substep count and `stepDt` from the scaled value.
2. Advance generation time and particles, apply the finite-state guard, and
   check the sensor layout.
3. Publish external-controller sensors once for the outer update and update the
   baseline-bot manager once with the full scaled delta.
4. For each physics substep, accumulate/spawn ambient pellets, then visit snakes
   in array order. Each snake runs `prepareForStep`; baseline actions take
   priority, followed by external-controller actions, the zero-action
   `external-only` branch, and due neural sensing/inference.
5. The synchronous path performs neural inference immediately. The asynchronous
   path prepares snakes, mutates pre-inference state, builds sensors and batches,
   and only then awaits pooled inference. This is the characterized `MT-010`
   partial-mutation boundary, not an approved behavior.
6. Apply collected controls and advance snakes in array order, rebuild the
   collision grid, resolve collisions, and repeat for every substep.
7. Update focus, camera, best-score and alive-count state, then end the
   generation when its termination condition is met.

With the default `dt = 1 / 60`, `dtClamp = 0.01`, and
`collision.substepMaxDt = 0.005`, both 1x and 12x requests are capped to 0.01
simulated seconds and execute two 0.005-second physics substeps. External sensor
publication and baseline-manager update occur once, while due neural control can
be sampled in each substep. The Phase 0 tests preserve this defect as evidence
for the Phase 1 conversion; they do not endorse it.

#### Current generation and checkpoint ordering

`World._endGeneration` computes fitness, sorts the population, updates history
and Hall of Fame state, breeds, increments the generation, clears generation
counters and particles, initializes pellets, resets baseline bots, spawns all
new-generation snakes, rebuilds the collision grid, and selects focus. After
`SimCore.update` returns, `SimServer` reassigns controllers and then handles the
generation change. The current automatic checkpoint therefore observes the
already-spawned new world, after random draws, rather than the required Phase 7
pre-spawn boundary.

#### Authoritative-state helper

`server/test/authoritativeWorldDigest.ts` provides schema-version-1 SHA-256
captures and structured first-divergence diagnostics. It uses stable paths,
exact safe integers, raw Float32 bit encodings, population slots when available
with current snake identities as a documented fallback, stable graph-node
ordering, and canonical pellet ordering. A mismatch reports both ticks plus the
population slot, brain family, and recurrent node when available. Non-finite
state is rejected with its exact path. Camera/focus/render data, last sensor and
output displays, history/Hall-of-Fame views, run/session IDs, and timestamps are
excluded.

The helper and its tests use explicit hand-built component fixtures; no seeded
full `World` fixture was introduced before Phase 2. Inspectable baseline-bot
manager fields are captured, but its `createRng` closures expose no continuation
state. That newly found gap is registered as `DET-004` for Phase 2.

#### Inference-mode diagnostics

`InferenceModeRecord` exposes these fields through `SimServer.getInferenceMode`,
the HTTP health status, and the `inference-mode` startup log:

- `requestedBackend: 'js' | 'native' | null`;
- `activeBackend: 'js' | 'native' | 'mixed' | 'unknown'`;
- `requestedMt: boolean`;
- `activeWorkerCount: number`;
- `poolEpoch: number | null`;
- `weightEpoch: number | null`;
- `graphKey: string`;
- `parameterCount: number`;
- `seed: number`;
- `nativeAddonStatus: 'unavailable' | 'loading' | 'ready' | 'failed'`;
- `nativeAddonBuildIdentifier: string | null`.

The baseline truthfully reports a null requested backend because it has no
backend-selection surface, null epochs because its pool has no epoch contract,
and a null build identifier because the addon exports none. A ready
`NodeBrainPool` reports its actual worker count and JS backend. Serial reporting
uses the backend captured on each executing `GraphBrain`; scripted baseline and
`external-only` snakes are excluded because they do not execute a neural math
backend. These are diagnostics only and do not load native code or change
inference selection.

## Phase 1: One simulation step pipeline and truthful time

### Purpose

Remove sync/async semantic drift and make complete fixed simulation steps
independent of scheduler jitter and presentation speed.

### Planned changes

- Refactor `World.update` and `World.updateAsync` into one authoritative update
  flow.
- Refactor `_stepPhysics` and `_stepPhysicsAsync` into one control/physics
  pipeline with an optional batch inference runner.
- Add an explicit population slot to population-owned snakes.
- Keep external control, baseline bots, pooled inference, and serial inference
  as explicit branches within the same control-collection pass.
- Give `World` one fixed-step entry point that always receives `baseDt`.
- Keep scheduling in the existing `SimCore` accumulator. Convert measured wall
  time and `simSpeed` into a budget of zero or more whole World steps.
- Execute every due step through the full sensing, action, movement,
  interaction, and generation pipeline. Never pass `baseDt * simSpeed` into a
  World step.
- Keep wall time only for scheduling, broadcast cadence, rate limiting, and
  profiling.
- Await `SimServer.start` and make shutdown await worker cleanup.
- Remove `CFG.dtClamp` from simulation-time scaling and remove its obsolete
  UI/protocol surface if it has no remaining valid consumer.
- Bound steps per pump. When the process cannot keep up, discard excess
  wall-clock debt and report the achieved multiplier; do not discard
  simulation state.
- Base any lower-level collision subdivision on `baseDt`, maximum movement,
  body geometry, or grid resolution, never on `simSpeed`.

### Canonical step ordering

The final operation-level ordering must be recorded after Phase 0
characterization. The target boundaries are:

1. Drain authoritative commands assigned to this simulation step in monotonic
   command-sequence order.
2. Apply pre-observation time/accounting for the elapsed interval without
   prematurely advancing the score-observation marker repaired in Phase 5.
3. Determine the controlled-snake set in stable order.
4. Sample each due external, baseline, serial-neural, and pooled-neural
   observation exactly once for this fixed step.
5. Collect all due actions without moving a snake; inference may await here.
6. Commit actions and advance snakes by `baseDt` in stable order.
7. Rebuild or update spatial indexes before later queries depend on them.
8. Resolve food, snake collisions, deaths, drops, and other interactions in a
   documented deterministic order.
9. Complete post-step statistics, focus/observer updates, generation
   termination, and the checkpoint transition.

Existing gameplay ordering is preserved where Phase 0 shows it is intentional.
Any deliberate deviation is recorded with its defect ID. Phase 5 changes only
the score-marker semantics, not the rest of this ordering.

The implemented Phase 1 ordering is:

1. `SimCore` converts measured wall time and the requested multiplier into a
   budget of whole fixed steps; it passes only `fixedDt` to `World.step`.
2. `World.step` advances generation time and particles, checks score-summary
   finiteness and sensor layout, and calls `prepareForStep` once for each alive
   snake.
3. Ambient food due for the fixed interval is spawned, external-controller
   sensors are sampled and published once in stable snake-array order, and the
   baseline manager is updated once. A baseline respawn is prepared once.
4. One stable control pass chooses baseline, external, external-only,
   serial-neural, or population-pooled control. Neural sensors are sampled
   once; serial work and pooled work are collected without movement.
5. Serial inference runs in collected order, then pooled inference is awaited.
   Moving serial inference after collection is the deliberate `CORE-002`
   correction that makes the two inference paths share the same observation
   boundary.
6. All collected controls are applied in stable snake-array order.
7. Movement is integrated by `fixedDt`; collision-safety substeps depend only
   on `fixedDt` and `collision.substepMaxDt`. Each substep advances all alive
   snakes, rebuilds the collision grid, then resolves collisions.
8. Focus, camera, population score summary, and generation termination run
   once. The World and SimCore tick ids commit only after the whole step
   succeeds.

If awaited inference rejects after pre-observation mutation, the step does not
apply controls, move, publish a new tick/frame/stats, or checkpoint. `SimServer`
faults the run at the last committed tick and will not step that World again;
an explicit reset replaces the World before clearing the fault.

### Detailed checklist

- [x] Define the canonical World step API and document its ordering.
- [x] Move duplicated pre-step bookkeeping into the canonical path.
- [x] Move duplicated control selection and batching into the canonical path.
- [x] Move duplicated advancement, grid rebuild, and collision resolution into
  the canonical path.
- [x] Move duplicated focus, camera, fitness summary, and generation-end logic
  into one post-step path.
- [x] Add `populationSlot` or equivalent durable inference identity.
- [x] Update population, baseline, resurrected, and external spawn paths with
  explicit identity semantics.
- [x] Move `simSpeed` into `SimCore`'s scheduling budget and pass only `baseDt`
  to each World step.
- [x] Make 1x request one simulated second per scheduled real second and 12x
  request twelve, subject to compute throughput and the explicit catch-up cap.
- [x] Record achieved multiplier and dropped scheduling debt without changing
  authoritative state for a completed-step count.
- [x] Remove or repurpose `dtClamp` only after all references and UI metadata
  are accounted for.
- [x] Await server start/stop lifecycle methods.
- [x] Make bind failure fail the Phase 1 lifecycle suite; do not silently
  return from a required network test.
- [x] Ensure an inference failure publishes no incremented step/frame or
  checkpoint. If pre-await in-memory mutation cannot be rolled back, fault the
  run and prohibit resuming that object, as finalized in Phase 4.
- [x] Delete no duplicate path until focused equivalence tests pass.

### Required tests

- [x] 0.1x, 1x, and 12x requested/achieved advancement tests.
- [x] One hundred fixed steps run singly produce exactly the same state as the
  same hundred steps grouped into pumps representing 12x speed.
- [x] Scheduler jitter changes when steps occur in wall time, but not their
  fixed delta or result after the same completed-step count.
- [x] Serial control-order regression test.
- [x] External controller and baseline bot ordering tests.
- [x] Collision safety test for one `baseDt`; speed itself causes no extra
  collision subdivision.
- [x] Server lifecycle test that observes MT initialization failure.

### Acceptance gate

- [x] Only one production physics/control implementation remains.
- [x] No authoritative state calculation consumes measured callback delta.
- [x] State after a completed-step count is invariant to speed, jitter, and
  scheduler-pump grouping.
- [x] Speed multiplier assertions pass at the advertised values when throughput
  permits, and shortfall is reported honestly when it does not.

## Phase 2: Authoritative seeded randomness

### Purpose

Make world construction, evolution, and gameplay random decisions reproducible
and persistable.

### Target RNG model

Add a versioned stateful PRNG abstraction in `src/rng.ts` or a narrowly scoped
replacement. It must provide:

- seed normalization;
- next uniform value;
- bounded float and integer helpers;
- Gaussian sampling;
- state export and restore;
- an algorithm/version identifier.

Define state with explicit-width integer operations and a lossless serialized
representation. Version both the uniform algorithm and Gaussian transform. If
Gaussian sampling caches a second value, persist that value and its valid flag.

Derive streams by stable labels such as `world`, `evolution`, `observer`, and
`baseline:<slot>` from the run seed. Do not create a stream by consuming an
arbitrary number of values from another stream: adding an observer or bot must
not shift evolution. Cosmetic client particles remain outside authoritative
server RNG entirely.

### Detailed checklist

- [ ] Pass `worldSeed` from server startup into `SimCore` and `World`.
- [ ] Store the active seed and every authoritative derived-stream state on the
  authoritative core/world.
- [ ] Add the reusable seeded full-World fixture deferred from Phase 0 and use
  normal production construction paths without monkeypatching globals.
- [ ] Replace authoritative `rand`, `randInt`, and `gaussian` calls with
  injected PRNG operations.
- [ ] Add optional RNG parameters to genome and brain-weight initialization.
- [ ] Inject RNG into `Genome.random`.
- [ ] Inject RNG into crossover, recurrent crossover, tournament selection,
  mutation, and Gaussian mutation.
- [ ] Inject RNG into snake spawn position/heading.
- [ ] Inject RNG into death-pellet value/jitter and boost-pellet jitter.
- [ ] Inject RNG into ambient pellet generation and fallback spawning.
- [ ] Replace random resurrect IDs with a deterministic, collision-safe
  allocator.
- [ ] Derive baseline-bot streams from the run seed and durable bot identity;
  verify they do not consume the world/evolution streams accidentally.
- [ ] Move camera/focus randomness to a derived observer stream or a
  deterministic selection rule.
- [ ] Keep render particles and client effects off the authoritative stream.
- [ ] Generate an unspecified new run seed with a system entropy source, not
  `Math.random`.
- [ ] Generate session IDs independently from simulation randomness.
- [ ] Give each lineage a non-randomness-consuming run ID used to group its
  checkpoints.
- [ ] Make Apply/Reset create generation one from the same seed and current
  authoritative configuration, with a new run ID and zero recurrent state.
- [ ] Add an explicit New Run API that performs the same restart with a new
  entropy-derived seed and new run ID.
- [ ] In this phase, prove deterministic in-memory restart and identity
  semantics. Do not claim Reset/New Run crash durability until Phase 7 commits
  the required run-start checkpoint.
- [ ] Give authoritative generated IDs an exported/restored allocator state or
  derive them from stable run/generation/step/counter fields.
- [ ] Add a lint restriction or focused static guard preventing
  `Math.random` in authoritative modules.

### Checkpoint semantics

Automatic resumable checkpoints represent a population at one exact generation
boundary, not arbitrary mid-tick world state. At that program point the prior
generation has ended; selection/crossover/mutation have produced and assigned
the new population; the new generation number is set; recurrent state is
defined as zero; and no new-generation spawn, heading, pellet, focus, sensor,
or inference operation has occurred. Capture RNG and deterministic allocator
state for every authoritative stream immediately before the first random draw
used to construct that world.
Phase 2 creates the boundary hook; Phase 7 persists and resumes it. Manual
population export remains a population transfer unless a later format
explicitly includes full world state.

### Required tests

- [ ] Known PRNG sequence test.
- [ ] PRNG state export/restore test.
- [ ] Gaussian cache state export/restore test if the algorithm caches a value.
- [ ] Same seed creates identical initial genomes, snakes, pellets, and focus.
- [ ] Different seeds diverge.
- [ ] Same seed and action log produce identical state for many fixed ticks.
- [ ] Normal reset reproduces the same initial state.
- [ ] New Run produces a new visible seed and divergent state.
- [ ] Evolution selection, crossover, and mutation reproduce across runs.
- [ ] Death and boost pellet placement reproduce across runs.
- [ ] Cosmetic rendering calls do not change authoritative state.
- [ ] Adding observer/cosmetic work does not shift world, evolution, or bot
  stream sequences.
- [ ] Canonical digest ordering is unaffected by incidental array/batch order.

### Acceptance gate

- [ ] Authoritative modules contain no unapproved `Math.random` calls.
- [ ] Seeded replay passes without monkeypatching globals.
- [ ] Seed and RNG version/state are available to persistence.

## Phase 3: Native kernel safety and runtime activation

### Purpose

Make the existing Rust code useful in the normal runtime while keeping its
scope narrow and its unsafe boundary checked.

### Detailed checklist

- [ ] Delete `server/native-backend.ts` and the `PhysicsBackend` branch after
  confirming no supported caller remains.
- [ ] Remove `SLITHER_NATIVE_BACKEND` full-world behavior and replace it with a
  clear math-backend configuration if needed.
- [ ] Select and load an immutable backend before creating any main-thread
  `GraphBrain`; diagnostics must come from the backend actually attached to the
  brain, not only from requested configuration.
- [ ] Load the selected worker backend before installing a worker message
  handler or compiling/creating worker brains.
- [ ] Make normal native startup fail with a concise actionable error if the
  addon is absent or incompatible.
- [ ] Support an explicit JS diagnostic backend without pretending native ran.
- [ ] Export or derive a native-addon build identifier from crate/package
  version plus source revision; do not report package metadata as proof that a
  stale binary did not load.
- [ ] Record requested/active backend and addon build ID in runtime diagnostics.
- [ ] Validate dimensions, strides, multiplication overflow, and all input,
  output, scratch, weight, and recurrent-state lengths in Rust before entering
  unsafe pointer code.
- [ ] Validate zero-sized/invalid dimensions and unsupported buffer overlap or
  aliasing assumptions before unsafe code.
- [ ] Return structured N-API errors for invalid calls.
- [ ] Keep `unsafe` blocks narrowly scoped with accurate safety comments.
- [ ] Add invalid-length and invalid-dimension tests through the exported
  N-API boundary.
- [ ] Remove native-test early returns in this phase. A missing addon fails the
  required-native suite; only the explicitly named JS suite can run without it.
- [ ] Rename the misspelled `SIMD_Kernals.rs` to `simd_kernels.rs` if the
  rename is isolated and does not complicate active work.
- [ ] Remove unused Rust dependencies.
- [ ] Reduce `native/package.json` from generated template metadata to the
  packages, targets, and scripts this repository actually supports.
- [ ] Keep only x86_64 targets while the crate has an x86_64 compile error.
- [ ] Strengthen the repository's existing Ubuntu/Windows CI matrix for addon
  build/load coverage rather than adding a duplicate Windows job.

### Parity contract

- JS/native kernel tests compare one bounded operation with explicit absolute
  and relative tolerances.
- Same-backend repeated runs within one runtime/addon build and supported
  execution environment must be exact where Float32 writes are exact.
- Long-horizon full-world JS/native frame equality is not a valid gate because
  small legal floating-point differences can become chaotic trajectory
  differences.

### Required tests

- [ ] Dense native versus JS reference.
- [ ] MLP native versus JS reference.
- [ ] GRU native versus JS reference across multiple recurrent steps.
- [ ] LSTM native versus JS reference across multiple recurrent steps.
- [ ] RRU native versus JS reference across multiple recurrent steps.
- [ ] Invalid buffers fail safely rather than reading out of bounds.
- [ ] Unsupported aliasing/overlap fails safely, or is explicitly proven safe.
- [ ] A missing addon fails the native-required suite instead of returning.
- [ ] Runtime integration asserts native mode is active.
- [ ] Explicit JS diagnostic mode asserts native is inactive.

### Acceptance gate

- [ ] Normal single-threaded server inference uses Rust kernels.
- [ ] No code expects a Rust `World` export.
- [ ] Unsafe exported calls validate their contracts.

## Phase 4: Canonical MT inference with stable recurrent state

### Purpose

Run native kernels across worker threads without losing weights, state, or
failure visibility.

### Worker ownership design

For worker count `W`, population slot `S` is owned by a stable deterministic
mapping such as `S % W`. A worker constructs brains only for its owned slots.
It processes an entry only when it owns the entry's population slot. Deaths
and compacted control batches therefore do not migrate brain memory.

Slots are dense integers from zero through population size minus one for a
pool epoch. A population-owned snake keeps its genome's slot for the entire
generation; a death never shifts another snake into that slot. Batch entries
carry slots but inputs/outputs remain associated with batch positions.
Baseline, resurrected, disconnected external, or manually created neural
snakes have no population slot unless an explicit population operation assigns
one. Worker count, population size, architecture, and backend are immutable
within a pool epoch and can change only at a recurrent-reset boundary.

### Detailed checklist

- [ ] Promote `server/brainPool.ts` as the canonical lifecycle foundation.
- [ ] Define one parent/worker message protocol with batch IDs, pool
  epoch, weight epoch, and reset acknowledgement. Keep evolutionary generation
  separate from pool lifecycle.
- [ ] Cap automatic worker count by CPU count and population size; add a
  conservative upper bound pending benchmarks.
- [ ] Allocate shared input, output, index, and population-weight buffers with
  validated capacities.
- [ ] Copy initial population weights during pool initialization.
- [ ] Construct worker brains only after native/JS backend readiness.
- [ ] Construct only the slots owned by each worker.
- [ ] Dispatch batches without changing slot ownership.
- [ ] Route noncontiguous batch entries explicitly: recurrent state indexes by
  population slot while output indexes by batch position.
- [ ] Ignore or reject stale completion messages by batch ID and epoch.
- [ ] Add inference timeout, init timeout, reset timeout, worker error, and
  unexpected-exit handling.
- [ ] Prevent concurrent in-flight batches.
- [ ] On new generation, copy new weights, advance weight epoch, reset all
  recurrent brains, and await acknowledgement before inference.
- [ ] On import/reset/architecture change, rebuild the pool deliberately.
- [ ] Apply worker-count and backend changes only while rebuilding at a
  recurrent-reset boundary.
- [ ] Keep MLP, GRU, LSTM, and RRU behavior under the same pool abstraction.
- [ ] Batch only snakes with a valid population slot.
- [ ] Run resurrected and unowned external neural snakes through their own
  main-thread native brains.
- [ ] On worker error, unexpected exit, inference timeout, epoch mismatch, or
  unrecoverable protocol violation, reject the in-flight step and enter the
  faulted state defined by DEC-005. Keep status available; allow only explicit
  Reset, New Run, or checkpoint-resume recovery.
- [ ] Never rebuild a failed pool and continue mid-generation. Pool rebuild is
  safe only at a new-generation, import, reset, resume, architecture, backend,
  or worker-count boundary where recurrent state is defined as zero.
- [ ] Await pool shutdown during server close.
- [ ] Add a rate-limited visualization request for the selected slot and return
  activations tagged with slot, simulation step, pool epoch, and weight epoch
  from its owner worker. Discard delayed mismatched responses.
- [ ] Remove the duplicate active chain under `src/sim` and `src/worker` only
  after replacement tests pass.
- [ ] Remove tests that cover only the deleted duplicate, replacing them with
  canonical-pool tests.

### Required test matrix

Run each relevant row with 1, 2, and 4 workers:

| Brain | Single native | MT native | Single JS diagnostic | MT JS diagnostic |
| --- | --- | --- | --- | --- |
| MLP | Required | Required | Required | Required |
| GRU | Required | Required | Required | Required |
| LSTM | Required | Required | Required | Required |
| RRU | Required | Required | Required | Required |

Scenarios:

- [ ] Initial inference uses nonzero expected population weights.
- [ ] Shuffled batch order preserves outputs.
- [ ] Pool lifecycle, weights, shuffled batches, and worker-count equality pass
  for all four brain families.
- [ ] Deaths and shrinking batches preserve GRU/LSTM/RRU recurrent history.
- [ ] A recurrent snake absent for several batches retains its own state.
- [ ] Generation transition applies new weights and zeroes old state.
- [ ] Architecture change rebuilds buffers and brains.
- [ ] Resurrected snake does not overflow population buffers.
- [ ] Released external snake uses a valid serial brain.
- [ ] Worker crash rejects in-flight work and surfaces server failure.
- [ ] A worker crash publishes no successful step/frame, causes no fallback,
  and prevents further steps until explicit boundary recovery.
- [ ] Stale completion cannot finish a later batch.
- [ ] Native is reported active inside every native worker.
- [ ] Selected-brain visualization is available under MT.

### Acceptance gate

- [ ] Within one code/runtime/addon build, the same native seed/run is identical
  across 1, 2, and 4 workers.
- [ ] Recurrent tests survive deaths and repartitioning.
- [ ] Only one production Node brain pool remains.
- [ ] MT and native are demonstrably active together.

## Phase 5: Score-delta sensor semantics

### Purpose

Give sensor index 8 a useful and consistent temporal meaning.

### Target behavior

`points_delta_norm` reports the score change accumulated since the same snake's
previous delivered sensor sample. It includes survival, food, kill, and boost
cost changes that occurred between observations. It does not depend on whether
the next action is external, serial neural, or pooled neural.

### Detailed checklist

- [ ] Rename `prevPointsScore` to a name that describes the observation
  boundary, such as `pointsAtLastSensorSample`.
- [ ] Remove score-marker overwrite from `prepareForStep`.
- [ ] Keep pure sensor construction separate from marker mutation.
- [ ] Add one sampling method that builds sensors and then commits the score
  marker exactly once.
- [ ] Use that method for serial neural control.
- [ ] Use that method for pooled neural control.
- [ ] Use that method for external controller sensor messages.
- [ ] Define first-sample behavior explicitly.
- [ ] Define behavior when control cadence skips physics substeps.
- [ ] Update sensor protocol documentation.

### Required tests

- [ ] First observation has the documented delta.
- [ ] Survival reward appears once.
- [ ] Food gained after one observation appears in the next.
- [ ] Kill reward appears in the next observation.
- [ ] Boost spending produces a negative delta.
- [ ] A repeated sample without score change returns zero.
- [ ] Skipped control intervals accumulate rather than discard changes.
- [ ] External, serial neural, and pooled neural sampling agree.

### Acceptance gate

- [ ] No path writes the marker immediately before reading it.
- [ ] Sensor contract and API documentation use the same wording.

## Phase 6: Server-authoritative live controls and God Mode

### Purpose

Restore the UI operations broken by the server-only conversion without
reintroducing client authority.

### Shared settings contract

Extract pure setting metadata from the DOM module so both client and server can
use:

- path;
- value type;
- min/max and integer rules;
- live versus reset-required classification;
- any server-side derived-state hook.

The DOM-building code may import this schema, but the server must not import a
browser module.

### Planned protocol additions

This repair is Protocol 2. A Protocol 1 client is rejected with a clear version
error because it cannot participate safely in authoritative controls. Message
names may change during implementation, but the contracts must cover:

- live settings request with request ID and one or more path/value updates;
- settings-applied response containing authoritative normalized values,
  revision, and current config hash;
- God Mode kill and move requests;
- God Mode result response with applied/rejected status;
- New Run request and response containing the new seed;
- updated welcome/status fields for seed, config revision, and inference mode.

Phase 6 defines and tests the New Run protocol surface, but a production
success response is not sent until Phase 7 makes the run-start checkpoint
durable. Until then, keep the operation explicitly unavailable rather than
acknowledging a crash-unsafe transition.

`requestId` correlates one client request and result. The server also assigns
accepted commands a monotonic arrival sequence and, when draining at a
boundary, an authoritative simulation step. For settings, one accepted
multi-path request is validated/applied atomically against the state produced
by earlier accepted requests in sequence, then increments `configRevision`
once. The requester receives a result and every joined UI receives the same
normalized authoritative patch/state. A rejected request does not increment
the revision.

### Detailed checklist

- [ ] Extract shared setting definitions from `src/settings.ts`.
- [ ] Audit every currently live setting for derived state.
- [ ] Mark genuinely reset-required settings as such.
- [ ] Add a safe live reconfiguration path for collision-grid cell size or
  reclassify it as reset-required.
- [ ] Add a live update method for baseline bot respawn delay.
- [ ] Remove obsolete `dtClamp` setting if Phase 1 makes it meaningless.
- [ ] Add strict server protocol validation.
- [ ] Increment the handshake and message schema to Protocol 2; reject Protocol
  1 explicitly.
- [ ] Add `WsClient` send methods and response handling.
- [ ] Debounce/coalesce high-frequency slider changes by path.
- [ ] Coalesce only one client's unsent updates for the same path. An optimistic
  display may move immediately, but accepted/stored state comes from the
  server result or broadcast.
- [ ] Queue accepted live updates in `SimServer`.
- [ ] Assign queued commands a total order and drain them before their fixed
  step, never during awaited inference.
- [ ] Validate each atomic request against prior accepted requests in order;
  acknowledge only values actually applied and increment revision once.
- [ ] Broadcast normalized authoritative results to every UI and update browser
  `CFG` from results/broadcasts, including server clamping.
- [ ] Send sim-speed changes through the same authoritative mechanism.
- [ ] Recompute a versioned canonical config hash from explicitly included,
  recursively sorted fields after relevant changes. Do not hash raw object
  insertion order; keep content hash distinct from monotonic revision.
- [ ] Replace captured HTTP seed/hash values with getters from active state.
- [ ] Add God Mode message routing for joined UI clients.
- [ ] Queue God Mode operations at a tick boundary.
- [ ] Kill through the normal `Snake.die` behavior.
- [ ] Move by translating head and all body points by the same delta.
- [ ] Validate finite coordinates and clamp/reject the translation delta so the
  entire body, not only the head, remains within defined world bounds.
- [ ] Rebuild or invalidate spatial state after a move before it is queried.
- [ ] Throttle drag messages and always send a final mouse-up position.
- [ ] Update the log from server results, not optimistic console text.
- [ ] Extract transport/control logic from the 4,000-line `main.ts` into small
  testable modules.
- [ ] Keep selection/frame parsing client-side and unchanged unless a test
  proves an offset bug.

### Required tests

- [ ] Each live UI interaction emits the expected message.
- [ ] Reset-required paths are rejected by the live validator.
- [ ] Server clamps/normalizes values from shared metadata.
- [ ] Update arriving during worker inference waits until the next boundary.
- [ ] Sim speed visibly and authoritatively changes generation-time rate.
- [ ] Config hash/revision changes when appropriate.
- [ ] God kill changes the serialized alive set and applies normal drops.
- [ ] God move translates the entire body and leaves valid collision queries.
- [ ] Invalid IDs and coordinates return a rejected result.
- [ ] Drag coalescing applies the final requested position.
- [ ] Multiple UI clients receive coherent authoritative setting state.
- [ ] Two clients racing updates converge by config revision and authoritative
  broadcast order.
- [ ] One accepted multi-path request increments revision once; a rejected
  request does not increment it.
- [ ] Canonically equivalent configs produce the same hash regardless of
  property insertion order.

### Acceptance gate

- [ ] No live control only mutates browser-private state.
- [ ] God Mode kill and drag work against serialized server frames.
- [ ] UI tests exercise extracted behavior rather than only module startup.

## Phase 7: Bounded-memory persistence and actual resume

### Purpose

Replace the false chunking claim with a simple SQLite design that does not
construct the whole population as one JSON/string/buffer object.

### Proposed schema

Keep `population_snapshots` for one metadata row per checkpoint. Add a
versioned child table similar to:

```sql
CREATE TABLE snapshot_genomes (
  snapshot_id INTEGER NOT NULL,
  slot INTEGER NOT NULL,
  arch_key TEXT NOT NULL,
  brain_type TEXT NOT NULL,
  fitness REAL NOT NULL,
  weight_count INTEGER NOT NULL,
  weights_blob BLOB NOT NULL,
  weights_checksum TEXT NOT NULL,
  PRIMARY KEY (snapshot_id, slot),
  FOREIGN KEY (snapshot_id) REFERENCES population_snapshots(id)
    ON DELETE CASCADE
);
```

Add metadata columns or versioned JSON fields for format version, seed, PRNG
algorithm/state, run ID, graph spec, settings, config hash/revision,
deterministic allocator state, checkpoint-boundary type, population count, and
generation-boundary semantics. Add a per-genome checksum using built-in runtime
crypto; do not add a custom container or checksum dependency.

`weights_blob` is a sequence of IEEE-754 Float32 values in little-endian byte
order. `weight_count * 4` must equal its byte length. Readers reject non-finite
weights, parameter counts inconsistent with the graph, unsupported brain/graph
metadata, checksum mismatches, missing/non-dense slots, and population-count
mismatches.

### Write path

- Insert snapshot metadata and genome rows in one SQLite transaction.
- Iterate typed genomes one at a time.
- Encode and bind one little-endian Float32 BLOB at a time; do not first create
  population-sized number arrays.
- Reuse prepared statements within the transaction and release each genome's
  temporary byte view/buffer before moving to the next.
- Do not `Buffer.concat` the population.
- Do not synchronously gzip one population-sized buffer.
- Keep peak serialization memory proportional to one genome plus database
  overhead.
- Enable and verify `PRAGMA foreign_keys = ON` on every relevant connection so
  declared cascades and referential checks actually apply.

### Read and bootstrap path

- Prefer new child rows ordered by slot.
- Construct typed genomes directly.
- Bootstrap `World` with the saved population and generation-start RNG state
  so it does not create and discard a random population first.
- If no child rows exist, use the legacy `genomes_blob` reader.
- Legacy loading may still require the old large allocation; warn that it is a
  compatibility path and never write that format again.

### Exact checkpoint boundary and durability

A generation checkpoint is captured only after the preceding generation has
ended and evolution has assigned the new population and generation number, but
before any spawn positions/headings, ambient pellets, focus choice, recurrent
inference, or sensor sampling for that generation. Recurrent state is defined
as zero. RNG and ID-allocator state are captured immediately before the first
random operation in new-generation construction. Resume enters the same
generation-construction routine at that point.

Automatic checkpointing defaults to every generation once the new path exists;
the interval remains configurable. An explicit disabled value is allowed for
diagnostics but logs that crash resume can lose progress. Reset, New Run, and
`--fresh` always require a generation-one run-start checkpoint regardless of
the interval. A required checkpoint must commit before the server starts or
advertises the new generation/run. Failure faults the transition and status
distinguishes the in-memory generation from the last durable generation.

Startup modes are explicit: `--fresh`, `--resume latest` (the normal default),
and `--resume <snapshot-id>`. Resume restores the snapshot's experiment graph,
settings, seed, run ID, RNG version/state, and allocator state. Operational
options such as bind address, port, database path, requested backend, and worker
count remain startup configuration. A conflicting experiment override requires
`--fresh` rather than silently mutating a resumed experiment. If the selected
snapshot is corrupt or incompatible, startup fails with its ID/reason and lists
valid alternatives that can be selected explicitly.

### Detailed checklist

- [ ] Separate internal typed snapshot models from JSON transport DTOs.
- [ ] Add idempotent schema migration for format/version fields and child
  rows.
- [ ] Enable and assert SQLite foreign-key enforcement.
- [ ] Add prepared insert/select statements for genome rows.
- [ ] Add transactional per-genome save.
- [ ] Add typed per-genome load.
- [ ] Encode Float32 BLOBs as little-endian and validate byte length, checksum,
  finiteness, graph parameter count, brain type, slot density, and population
  count on read.
- [ ] Preserve legacy blob read.
- [ ] Stop new writes to `genomes_blob`.
- [ ] Avoid `World.exportPopulation` on automatic/internal save paths.
- [ ] Capture the generation-boundary RNG state required for exact
  reconstruction of that generation's initial world.
- [ ] Split the current post-spawn `_endGeneration`/server save flow at the
  exact pre-spawn checkpoint boundary defined above.
- [ ] Include run ID, active seed, RNG version/state, allocator state, graph
  spec, settings, boundary type, and config identity.
- [ ] Change the automatic-checkpoint default from disabled to every generation
  when the bounded path is ready; retain a visible diagnostic opt-out.
- [ ] Make normal startup select and validate the latest checkpoint candidate;
  do not silently skip a corrupt/incompatible latest candidate.
- [ ] Add `--fresh`, `--resume latest`, and `--resume <snapshot-id>` behavior
  with the configuration precedence defined above.
- [ ] Make a corrupt/incompatible latest checkpoint produce an actionable
  error and list available alternatives.
- [ ] Do not delete or overwrite older snapshots during migration.
- [ ] Make Reset and New Run commit a generation-one run-start checkpoint
  before switching current run state, without deleting saved history.
- [ ] Update import so an imported seed is either applied explicitly or
  clearly treated as metadata; do not silently ignore it.
- [ ] Make HTTP dependencies query current seed/hash rather than startup
  constants.
- [ ] Stream large export responses incrementally if retaining JSON.
- [ ] Preserve the current user-visible JSON export shape if practical while
  serializing one genome at a time; do not require `Content-Length` if that
  would force whole-response buffering.
- [ ] Avoid adding a new external file format without recording the user-facing
  compatibility decision in this plan.
- [ ] Apply explicit size/count limits with errors that identify the offending
  snapshot or genome.
- [ ] For legacy gzip, report snapshot ID, compressed size, expected
  uncompressed size/population where known, and enforce pre-decompression and
  bounded-output limits against corrupt or excessive data.
- [ ] Measure save duration and peak memory with a synthetic large population.
- [ ] Add a structural test proving automatic save consumes the typed
  per-genome path and never calls the population JSON-export DTO. Use a
  subprocess for process-memory measurement only if the direct harness is too
  noisy.
- [ ] Only add a background persistence worker if measurement shows the
  per-row synchronous transaction causes unacceptable tick stalls.

### Required tests

- [ ] New-format round trip with multiple architectures' metadata.
- [ ] Float32 weights round trip exactly.
- [ ] BLOB byte order, byte-length, checksum, finite-value, and slot-continuity
  validation failures are reported clearly.
- [ ] Foreign-key cascades work on an actual opened connection.
- [ ] Hundreds of synthetic genomes save/load without a combined buffer path.
- [ ] Transaction rollback leaves no partial snapshot.
- [ ] Legacy blob snapshot still loads.
- [ ] Corrupt metadata or genome row fails clearly.
- [ ] Startup resumes saved genomes and generation.
- [ ] Resumed generation start matches the checkpoint's reconstructed state.
- [ ] `--fresh` ignores but preserves existing snapshots.
- [ ] `--resume <snapshot-id>` selects an older valid alternative explicitly.
- [ ] New Run is immediately restart-resumable even before completing its first
  evolved generation and cannot restore the prior run.
- [ ] Checkpoint reconstruction begins before the first new-generation random
  spawn draw with recurrent state in its defined zero condition.
- [ ] A required checkpoint failure prevents the new run/generation from being
  advertised as durable or continuing silently.
- [ ] Current seed/hash appear in save/export after reset or New Run.
- [ ] Export path does not call one giant `JSON.stringify` or `Buffer.concat`.

### Acceptance gate

- [ ] New saves have bounded serialization memory.
- [ ] Restart restores the evolved population.
- [ ] Legacy databases remain usable.
- [ ] Documentation distinguishes population checkpoints from full mid-tick
  saves.

## Phase 8: Test and CI reconstruction

### Purpose

Make green builds mean that the selected architecture actually worked. This
phase reorganizes and completes the harness; it does not defer missing-addon,
bind-failure, or other failure semantics needed by earlier phase gates.

### Test layers

#### Pure/unit

- PRNG and RNG state.
- Genetics with injected RNG.
- Sensor observation markers.
- Shared setting validation.
- God Mode world operations.
- Snapshot binary encoding/decoding.
- Native wrapper argument validation.

#### Component

- Canonical brain pool lifecycle.
- Worker stable ownership.
- Weight/reset epochs.
- WebSocket client outgoing commands.
- Protocol parsing and acknowledgements.
- SQLite new and legacy formats.

#### Integration

- Fixed-step serial server.
- Native single-thread server.
- Native MT server for MLP/GRU/LSTM/RRU.
- UI live update through WebSocket to serialized state.
- God Mode through WebSocket to serialized state.
- Checkpoint save, close, restart, and resume.

#### System/manual

- Launch with normal local scripts.
- Confirm displayed runtime mode and seed.
- Exercise 0.1x, 1x, and 12x.
- Change representative live settings.
- Apply a reset-required setting.
- Kill and drag snakes.
- View brain activation while MT is enabled.
- Save, close, relaunch, and confirm generation/population resume.
- Start New Run and confirm a different seed without lost checkpoints.

### Detailed checklist

- [ ] Verify the Phase 3 removal of every `if (!hasNative) return` pass and keep
  required-native tests separate from explicit-JS tests.
- [ ] Verify required network suites hardened in Phases 1 and 6 fail normally
  on bind errors.
- [ ] Allow network-suite skipping only through an explicit environment
  opt-out whose use is visible in output.
- [ ] Replace the old parity test with contract-specific kernel, worker-count,
  recurrent-state, and deterministic replay tests.
- [ ] Replace the monolithic fake-DOM main test with extracted module tests and
  a small startup smoke test.
- [ ] Rework category scripts so names correspond to real suites rather than
  sparse filename suffixes.
- [ ] Ensure MT integration explicitly sets MT on; current defaults must not
  accidentally make the test serial.
- [ ] Assert selected backend, worker count, and weight epoch in integration
  tests.
- [ ] Assert requested versus active backend/MT state, pool epoch, graph, seed,
  and native-addon build ID come from the runtime that actually executed.
- [ ] Build native once per CI job rather than repeatedly in build and test.
- [ ] Run Rust unit tests, formatting, and clippy.
- [ ] Run TypeScript, ESLint, Vitest layers, and Vite build.
- [ ] Keep both Ubuntu and Windows as first-class paths in the existing CI
  matrix; build and load the addon and run native+MT assertions in the same job
  that compiled it.
- [ ] Keep performance tests informational until a stable baseline is
  recorded.
- [ ] After baseline, set regression thresholds broad enough for shared CI but
  narrow enough to detect a disabled accelerator.

### Correct parity rules

- Same seed + same backend + different worker counts: exact authoritative
  equality expected.
- Same seed + same backend + restart: exact generation-boundary reconstruction
  expected.
- JS versus native single kernel call: numeric tolerance expected.
- JS versus native long-running world: compare bounded control outputs and
  discrete contracts, not arbitrary whole-frame equality after chaotic drift.

### CI acceptance gate

- [ ] CI cannot be green when native failed to load in a native-required job.
- [ ] CI cannot be green when a server system suite failed to bind.
- [ ] At least one CI path runs native and MT simultaneously.
- [ ] All four brain families run lifecycle/weight/worker-count tests; GRU,
  LSTM, and RRU additionally run recurrent-history tests.

## Phase 9: Documentation and migration-debris cleanup

### Purpose

Make the repository describe what it now does and remove obsolete code that
would mislead the next maintainer.

### Detailed checklist

- [ ] Update `README.md` with honest local setup, runtime mode, seed/reset,
  New Run, speed, settings, God Mode, and save/resume behavior.
- [ ] Rewrite the affected `AGENTS.md` sections from verified final code.
- [ ] Update `docs/API-instructions.md` with protocol messages and local-only
  scope.
- [ ] Remove the false chunked-module documentation.
- [ ] Remove the full-world Rust architecture claim.
- [ ] Document native and threading as independent axes.
- [ ] Leave a short durable architecture-decision record for kernel-only Rust
  (DEC-002) and independent backend/threading axes (DEC-004), so those choices
  do not depend only on this recovery checklist.
- [ ] Document checkpoint versus population export semantics.
- [ ] Document exact and tolerance-based determinism boundaries.
- [ ] Document worker failure behavior.
- [ ] Document the actual test commands and suite meanings.
- [ ] Clean native package template files and metadata that no longer serve the
  project.
- [ ] Remove dead imports that drag server-only brain/native code into the
  browser bundle.
- [ ] Remove dead local-worker comments and interfaces.
- [ ] Remove obsolete environment variables and config fields.
- [ ] Confirm archived plans remain untouched.
- [ ] Keep the superseded banner on `native_refactor_plan.md` or archive it in
  a separately approved documentation operation.
- [ ] Resolve or remove the broken `markdown-rules/rules.md` reference.

### Final manual QA

- [ ] Fresh install/build instructions work on the owner's Windows machine.
- [ ] Normal launcher starts native single-thread or configured native MT mode.
- [ ] Browser clearly shows connected server, seed, and inference mode.
- [ ] Representative live controls work.
- [ ] Reset-required controls work after Apply.
- [ ] Sim speed is visibly different at 0.1x, 1x, and 12x.
- [ ] God Mode select, kill, and drag work.
- [ ] Visualizer works with MT enabled.
- [ ] Save/restart resumes.
- [ ] New Run changes seed and preserves saves.
- [ ] No false public-hosting or security claims remain.

## Protocol and data-model migration notes

### Protocol versioning

The baseline protocol version is 1. The repaired contract is Protocol 2 because
it adds live settings, God Mode, New Run, authoritative acknowledgements and
broadcasts, command ordering, and runtime metadata. Reject Protocol 1 with an
explicit incompatibility error; do not silently ignore or reinterpret it.

### Population identity

`populationSlot` is an internal stable identity for weights and recurrent
state. It is not interchangeable with:

- snake array index;
- user-visible snake ID;
- baseline bot index;
- external controller ID.

Add explicit conversion only where required.

Slots are dense for one population/pool epoch, never shift after a death, and
are reassigned deterministically only with a new population/weight epoch.
Population size is immutable within the pool epoch. A non-population neural
snake has no slot and owns independent weights/recurrent state.

### Snapshot versioning

New snapshots need format, RNG, and boundary versions plus a run ID. The reader
selects:

1. new per-genome rows when present and version-compatible;
2. legacy `genomes_blob` otherwise;
3. a clear error when neither is valid.

Do not mutate legacy blobs in place.

### Configuration identity

Define what the config hash covers and keep it stable:

- brain graph and sensor layout;
- settings that change experiment behavior;
- core layer/population settings;
- format/protocol versions where compatibility depends on them.

The run seed remains an explicit adjacent field rather than being hidden in the
hash. Build the hash from a versioned canonical representation of the explicit
fields with recursively sorted keys. `configRevision` is a monotonic runtime
sequence; the hash is content identity, so returning to an older configuration
may reproduce an older hash at a newer revision.

## Error-handling policy

- Native required but missing: fail startup with build instructions.
- Explicit JS diagnostic mode: log that acceleration is disabled.
- MT init failure before run: fail the requested mode.
- MT failure during run: enter the faulted state, publish no successful
  step/frame/checkpoint for the failed work, run no more steps, and require
  explicit Reset, New Run, or checkpoint-resume recovery. Do not resume any
  partially mutated in-memory World.
- Invalid live setting: reject only that request with path and reason.
- Invalid God Mode target: reject with a result message; keep server running.
- Corrupt latest checkpoint: report checkpoint ID and reason; do not silently
  create a new population.
- Legacy checkpoint load memory failure: report legacy limitation and preserve
  the database.
- Required checkpoint disk-full/write error: fault before advertising or
  constructing the new run or new generation's world. Status preserves and
  identifies the last durable checkpoint; never claim the transition
  succeeded.

## Observability requirements

At startup log one concise structured mode record containing:

- run ID;
- seed;
- resumed checkpoint ID or fresh-run reason;
- requested and active math backend;
- native addon status and build identifier;
- requested MT state and active worker count;
- pool epoch and weight epoch;
- architecture key;
- population and parameter counts;
- config revision/hash.

On generation/pool transitions log:

- generation;
- simulation step;
- weight epoch;
- pool epoch;
- pool rebuild reason;
- recurrent reset acknowledgement;
- checkpoint ID, last durable generation, and save duration;
- fault state/reason when present.

Scheduler diagnostics report requested and achieved speed multiplier plus any
dropped wall-clock debt. These values are operational measurements, not inputs
to authoritative World state.

Avoid per-tick noise. Diagnostic first-divergence reports should include tick,
population slot, brain type/node if known, and expected/actual values.

## Performance policy

Correctness gates default enablement. After correctness:

1. Measure native single-thread and native MT with identical seeds/settings.
2. Warm up before timing.
3. Test representative populations and all brain families.
4. Record median and p95 tick/inference duration.
5. Compare 1, 2, 4, and a bounded auto worker count.
6. Choose the smallest worker count that provides a stable benefit.
7. Do not claim a speedup from a test that did not assert native/MT activation.

Persistence measurement must report snapshot size, genome/weight counts, save
duration, and peak process memory before/after.

## Risk register

| Risk | Impact | Mitigation |
| --- | --- | --- |
| RNG injection changes historical evolution sequences | Old seeds will not reproduce old broken runs | Version the algorithm and promise determinism only from the repaired format |
| Native/JS floating-point order differs | Invalid whole-world parity failures | Test kernels with tolerance and same-backend worlds exactly |
| Canonical step refactor changes ordering | Gameplay regression | Characterize exact operation ordering before deletion |
| Stable worker ownership increases per-worker scanning | Performance cost | Measure; optimize routing only after correctness |
| Worker failure policy stops a run | Less apparent availability | Prefer explicit integrity over silent state corruption |
| Collision-grid live resizing is unsafe | Missed collisions | Reallocate derived grid state or reclassify as reset-required |
| Dragging a body leaves stale spatial entries | Incorrect sensors/collisions | Rebuild/invalidate grids at the command boundary |
| Legacy snapshot is huge | Compatibility load may still spike memory | Isolate warning-heavy legacy reader; never write legacy again |
| SQLite per-row transaction still stalls | Visible generation pause | Measure before adding a background persistence worker |
| Protocol expansion desynchronizes clients | Broken UI | Version handshake and integration-test both sides |
| Scope grows into general cleanup | Recovery never finishes | Enforce phase gates and non-goals |

## User consultation gates

The owner has delegated ordinary implementation details. Stop and ask only
when one of these occurs:

- A proposed change would reintroduce a full Rust world/physics implementation.
- Exact resume would require full mid-tick world-state persistence rather than
  generation-boundary population checkpoints.
- A new user-visible export file format or extension is required.
- Legacy snapshot compatibility would need to be dropped.
- A gameplay/sensor/fitness meaning beyond `points_delta_norm` must change.
- MT would be made default despite failing a correctness or performance gate.
- Local-only scope would be expanded to LAN/public hosting with security
  implications.
- A destructive database migration or snapshot deletion is proposed.
- A new large dependency or framework is required.

## Acceptance criteria mapping

| Acceptance ID | Outcome | Primary verification |
| --- | --- | --- |
| AC-001 | Same seed/settings/actions repeat | Deterministic World and server replay suites |
| AC-002 | Reset repeats; New Run diverges | Reset/New Run integration tests |
| AC-003 | Speed is honest | Generation-time tests at 0.1x/1x/12x |
| AC-004 | Native works with and without MT | Runtime mode assertions and matrix |
| AC-005 | Recurrent state survives deaths/repartition | Multi-step GRU/LSTM/RRU pool tests |
| AC-006 | Initial/new-generation weights and state are correct | Weight epoch/reset tests |
| AC-007 | Extra snakes do not overflow MT buffers | Resurrect/release integration tests |
| AC-008 | Live settings reach authoritative state | WS-to-World setting tests |
| AC-009 | God kill/drag mutate serialized world | Protocol and serialized-frame tests |
| AC-010 | Score delta observes prior changes once | Sensor lifecycle tests |
| AC-011 | New persistence uses bounded memory | Schema, round-trip, and memory harness |
| AC-012 | Restart restores evolved population | Save/close/restart system test |
| AC-013 | Legacy snapshots remain readable | Legacy fixture test |
| AC-014 | Native tests cannot silently skip | Required-native suite behavior |
| AC-015 | CI exercises real native MT | CI job mode assertion |
| AC-016 | Docs match runtime and local scope | Final documentation review/manual QA |
| AC-017 | Fixed-step results ignore speed/jitter/pump grouping | Grouped-step scheduler replay tests |
| AC-018 | One documented production World pipeline remains | Ordering characterization and focused pipeline tests |
| AC-019 | Invalid native buffers/dimensions/aliasing fail safely | N-API boundary safety tests |
| AC-020 | MT faults stop without fallback or partial external commit | Worker-failure integration tests |
| AC-021 | Runtime diagnostics report what actually executed | Requested/active mode and worker assertions |
| AC-022 | Config revision/hash track authoritative live state | Multi-client ordering and canonical-hash tests |
| AC-023 | New Run is immediately resumable | Run-start checkpoint crash/restart test |
| AC-024 | Checkpoint resumes at the exact pre-spawn boundary | RNG/allocator/recurrent reconstruction test |

## Definition of done

The recovery is complete only when:

- all phase acceptance gates are checked;
- the final verification matrix is green;
- normal runtime reports native active;
- configured MT reports native active in every worker;
- same-backend deterministic replay and worker-count equality pass;
- completed-step state is invariant to speed, scheduler jitter, and pump
  grouping;
- worker faults enter the defined faulted state without fallback or partial
  external commit;
- live settings, speed, God Mode, and MT visualization pass manual QA;
- new persistence is bounded-memory and restart resumes saved evolution;
- legacy snapshot fixtures still load;
- no required suite silently returns on missing dependencies or bind failure;
- README, AGENTS, and API docs describe the verified architecture;
- obsolete duplicate pools and full-world native adapter are gone;
- this plan's live status says complete and includes the final validation
  commands/results.

## Live execution status

- Current phase: Phase 1 is complete; Phase 2 has not started.
- Active checklist item: none. Stop at the Phase 1 boundary and await the next
  owner instruction before beginning Phase 2.
- Last completed work: full verification of the canonical fixed World
  pipeline, durable population identity, truthful scheduler, awaited
  lifecycle, bind-failure propagation, and inference-fault boundary.
- Source implementation status: Phases 0 and 1 are complete and fully verified
  in the uncommitted worktree.
- Current blocker: none.
- Next action: when authorized, begin Phase 2 from this handoff without
  importing assumptions from superseded or archived plans.
- Current implementation HEAD:
  `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`.
- Last fully verified HEAD:
  `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`.
- Verification scope note: the complete Phase 1 worktree atop that unchanged
  HEAD is fully verified; no commit was created.
- Last successful phase acceptance gate: Phase 1 one-step-pipeline and truthful
  time gate.
- Exact dirty-file summary:
  - preserved pre-existing modified file:
    `docs/todo/native_refactor_plan.md`;
  - Phase 0-only modified files: `src/brains/graph/runtime.ts`,
    `src/brains/nativeBridge.ts`, `src/brains/types.ts`, and
    `src/sim/NodeBrainPool.ts`;
  - files modified across Phase 0 and Phase 1: `server/httpApi.ts`,
    `server/index.ts`, and `server/simServer.ts`;
  - Phase 1 modified files: `server/inferenceMode.test.ts`,
    `server/performance.test.ts`, `server/system.test.ts`, `src/config.ts`,
    `src/protocol/settings.ts`, `src/render.test.ts`, `src/settings.ts`,
    `src/sim/SimCore.ts`, `src/snake.ts`, `src/world.test.ts`, and
    `src/world.ts`;
  - preserved/updated untracked authoritative plan:
    `docs/todo/project-recovery-plan.md`;
  - Phase 0 untracked files: `server/authoritativeWorldDigest.test.ts`,
    `server/inferenceMode.test.ts`, `server/inferenceMode.ts`,
    `server/recoveryPhase0.characterization.test.ts`,
    `server/recoveryPhase0Startup.characterization.test.ts`,
    and `server/test/authoritativeWorldDigest.ts`;
  - Phase 1 untracked files: `server/recoveryPhase1.lifecycle.test.ts`,
    `src/recoveryPhase1.world.test.ts`, and `src/sim/SimCore.test.ts`;
  - the former untracked `src/recoveryPhase0.characterization.test.ts` was
    converted into the narrower Phase 1 suites and removed rather than
    preserving stale broken-behavior assertions.
- Last full JS test result: 45 files, 177 tests passed.
- Last Rust test result: 3 release tests passed at the baseline; native source
  was unchanged and Rust was not rerun during Phase 0.
- Last TypeScript result: passed.
- Last focused Phase 1 result: 5 files, 17 tests passed.
- Last ESLint result: passed.
- Last client-build result: passed with the existing `node:module`
  browser-externalization warning from `nativeBridge`.

## Handoff journal

### 2026-07-21 — planning handoff

- Checked out the exact remote branch requested by the owner:
  `exclusive-server-mode-refactor` at `cb276cc`.
- Installed dependencies and built the local addon.
- Verified TypeScript, ESLint, Vite, Vitest, and Rust baselines.
- Audited determinism, time, active/duplicate pools, native loading, recurrent
  state, UI settings, God Mode, sensor timing, persistence, tests, and docs.
- Confirmed locally why parity coverage disappeared.
- Owner approved kernel-only Rust and seed/resume behavior.
- No production source files were modified.
- At this handoff, wait for owner authorization before Phase 0. Authorization
  was provided before the later reconciliation handoff below.

### 2026-07-21 — independent review reconciliation

- Compared the independent review against the checked-out source rather than
  accepting its assumptions wholesale.
- Replaced scaled World deltas with repeated complete fixed steps scheduled by
  the existing `SimCore` accumulator.
- Moved the seeded full-World fixture from Phase 0 to the RNG phase.
- Defined Reset/New Run identity and durability, the exact pre-spawn checkpoint
  boundary, Protocol 2 ordering, and one concrete MT fault/recovery state.
- Added native aliasing/load-order checks, existing Windows-matrix hardening,
  stable slot lifecycle, canonical config hashing, little-endian BLOB
  validation, and current/verified-HEAD handoff fields.
- Kept public-server security, full-world Rust, a new DI framework, and other
  unsupported expansion out of scope.
- No production source files were modified. The next context starts at Phase 0.

### 2026-07-21 — Phase 0 implementation handoff

- Verified before editing:
  - repository root:
    `C:/Users/jlow8/source/repos/slither_neuroevo`;
  - branch: `exclusive-server-mode-refactor`;
  - HEAD: `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`;
  - worktree contained exactly the expected pre-existing modified
    `docs/todo/native_refactor_plan.md` and untracked
    `docs/todo/project-recovery-plan.md`.
- Preserved both planning files. No reset, revert, replacement, re-clone,
  commit, or push was performed.
- Added the canonical helper in
  `server/test/authoritativeWorldDigest.ts` and its hand-built-fixture tests in
  `server/authoritativeWorldDigest.test.ts`. It records schema version 1,
  SHA-256, stable paths, raw Float32 words, exact integers, future durable
  population slots with current identity fallbacks, recurrent node context,
  canonical pellets, and first-divergence details.
- Added `server/inferenceMode.ts` and diagnostics in `SimServer`, `/health`, and
  startup logging. `GraphBrain` now records the backend selected when it is
  constructed; `NodeBrainPool` exposes its ready worker count and active JS
  backend; the native bridge exposes loader status without triggering a load.
  Requested backend, pool/weight epochs, and addon build ID remain null where
  the baseline has no truthful source for them.
- Added characterization suites:
  - `src/recoveryPhase0.characterization.test.ts` for `TIME-002`, `TIME-003`,
    `CORE-001`, and `MT-010`;
  - `server/recoveryPhase0.characterization.test.ts` for `MT-002`, `PER-005`,
    and `CORE-001` generation/checkpoint ordering;
  - `server/recoveryPhase0Startup.characterization.test.ts` for `PER-003`.
  Each test names its defect and conversion phase. No deleted parity suite was
  restored wholesale.
- The first inference-mode test run exposed that scripted baseline bots were
  being counted as an unknown neural backend. The diagnostic was corrected to
  exclude baseline bots and `external-only` snakes because neither executes a
  neural math backend. No control or inference behavior changed.
- Newly registered defects:
  - `DET-004`: opaque baseline-bot RNG closures cannot be captured or restored;
  - `DOC-002`: root documentation says `server/config.toml` exists, while the
    baseline tree lacks it and startup creates it as a side effect;
  - `DOC-003`: root documentation requires `markdown-rules/rules.md`, but the
    policy file and its directory are absent from the checkout.
- Verification commands and results:
  - focused pre-change baseline:
    `node .\node_modules\vitest\vitest.mjs run src\world.test.ts server\brainPool.test.ts server\persistence.test.ts server\system.test.ts --reporter=dot`
    passed 4 files and 19 tests after the direct approved invocation; the first
    restricted invocation failed before test collection because Vite/esbuild
    could not access its configured temporary cache;
  - TypeScript:
    `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`
    passed with exit code 0;
  - focused Phase 0:
    `node .\node_modules\vitest\vitest.mjs run server\authoritativeWorldDigest.test.ts server\inferenceMode.test.ts src\recoveryPhase0.characterization.test.ts server\recoveryPhase0.characterization.test.ts server\recoveryPhase0Startup.characterization.test.ts src\world.test.ts server\persistence.test.ts --reporter=dot`
    passed 7 files and 30 tests;
  - ESLint: `node .\node_modules\eslint\bin\eslint.js .` passed with exit
    code 0;
  - full JavaScript suite:
    `node .\node_modules\vitest\vitest.mjs run --reporter=dot` passed 43 files
    and 165 tests;
  - client build: `node .\node_modules\vite\bin\vite.js build` passed, with
    the existing `node:module` browser-externalization warning;
  - tracked diff hygiene: `git diff --check` passed;
  - native/Rust code was unchanged; the recorded baseline release result
    remains 3 tests passed and was not rerun in Phase 0.
- Current implementation HEAD and last fully verified HEAD both remain
  `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`; the Phase 0 worktree atop that
  commit is fully verified but uncommitted.
- Last successful acceptance gate: Phase 0 verification and handoff. There are
  no blockers. Phase 1 was not started.

### 2026-07-21 — Phase 1 focused-verification progress

- Reverified the repository root, branch, unchanged implementation HEAD, and
  the exact Phase 0 handoff worktree before editing. The focused pre-change
  baseline passed 4 files and 21 tests.
- Replaced the duplicate sync/async World update and physics methods with one
  async fixed-step pipeline. A focused equivalence guard passed before the old
  paths were deleted.
- Added durable population slots and explicit null identity for baseline,
  external, reused-external, and resurrected snakes. Pooled outputs map back
  through a separate snake-array index.
- Moved speed multiplication into the SimCore scheduling budget. Every World
  call receives one fixed delta; the scheduler caps whole steps and reports
  requested/achieved speed, pending time, and discarded debt through
  `/health`, with rate-limited warnings when debt is dropped.
- Removed the obsolete `dtClamp` config, UI, and settings-protocol surface
  after confirming no references remained.
- Made startup and shutdown awaitable. A new bind-collision test exposed that
  constructing `WebSocketServer` before HTTP listen forwarded `EADDRINUSE`
  through a second uncaught emitter while startup remained pending. WebSocket
  and simulation resources now attach only after a successful HTTP bind.
- Added a simulation fault boundary: rejected inference commits no World or
  SimCore tick and prevents frame, stats, and checkpoint publication. Because
  pre-observation state can already have mutated, the server records the last
  committed tick and refuses further steps on that World until reset replaces
  it.
- Newly confirmed discrepancy: `TEST-002` included a second WebSocket error
  path, not merely tests that returned early. The Phase 1 lifecycle repair and
  regression test cover both failure modes.
- Focused verification commands and results:
  - `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`
    passed;
  - `node .\node_modules\vitest\vitest.mjs run
    src\sim\SimCore.test.ts src\recoveryPhase1.world.test.ts
    server\recoveryPhase1.lifecycle.test.ts server\system.test.ts
    server\inferenceMode.test.ts --reporter=dot` passed 5 files and 17 tests;
  - `rg` confirmed no production or test references to `World.update`,
    `World.updateAsync`, `_stepPhysics`, `_stepPhysicsAsync`, or `dtClamp`;
  - `git diff --check` passed before the final lifecycle-test additions and
    will be rerun at the broad gate.
- Current implementation HEAD and last fully verified HEAD remain
  `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`. Phase 1 is uncommitted. There
  are no blockers, and Phase 2 has not started.

### 2026-07-21 — Phase 1 completion handoff

- Repository state at completion:
  - root: `C:/Users/jlow8/source/repos/slither_neuroevo`;
  - branch: `exclusive-server-mode-refactor` tracking its same-named origin
    branch;
  - current implementation HEAD:
    `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`;
  - last fully verified HEAD:
    `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`;
  - no commit or push was performed.
- Canonical runtime result:
  - `World.step` is the sole production World control/physics path and always
    consumes a positive fixed `baseDt`;
  - one stable pass collects baseline, external, serial, and population-pooled
    controls before any movement;
  - `populationSlot` is durable pooled-inference identity, while a separate
    snake-array index maps outputs back to the current object;
  - collision safety may subdivide a fixed step, but speed never changes that
    subdivision;
  - `SimCore` alone converts wall time and speed into whole fixed steps, caps
    work per pump, and exposes requested/achieved speed plus pending/dropped
    scheduling time;
  - start and stop are awaited, bind errors reject startup, and shutdown waits
    for the active loop and worker cleanup;
  - a rejected authoritative inference step commits no tick and reaches no
    frame, stats, controller reassignment, generation checkpoint, or other
    post-step publication. The server faults at the last committed tick until
    reset replaces the World.
- Exact dirty-file summary:
  - preserved pre-existing modified file:
    `docs/todo/native_refactor_plan.md`;
  - Phase 0-only modified files: `src/brains/graph/runtime.ts`,
    `src/brains/nativeBridge.ts`, `src/brains/types.ts`, and
    `src/sim/NodeBrainPool.ts`;
  - files modified across Phases 0 and 1: `server/httpApi.ts`,
    `server/index.ts`, and `server/simServer.ts`;
  - Phase 1 modified files: `server/performance.test.ts`,
    `server/system.test.ts`, `src/config.ts`, `src/protocol/settings.ts`,
    `src/render.test.ts`, `src/settings.ts`, `src/sim/SimCore.ts`,
    `src/snake.ts`, `src/world.test.ts`, and `src/world.ts`;
  - preserved and continuously updated untracked authoritative plan:
    `docs/todo/project-recovery-plan.md`;
  - Phase 0 untracked files, with `server/inferenceMode.test.ts` extended in
    Phase 1: `server/authoritativeWorldDigest.test.ts`,
    `server/inferenceMode.test.ts`, `server/inferenceMode.ts`,
    `server/recoveryPhase0.characterization.test.ts`,
    `server/recoveryPhase0Startup.characterization.test.ts`, and
    `server/test/authoritativeWorldDigest.ts`;
  - Phase 1 untracked files: `server/recoveryPhase1.lifecycle.test.ts`,
    `src/recoveryPhase1.world.test.ts`, and `src/sim/SimCore.test.ts`;
  - the prior untracked `src/recoveryPhase0.characterization.test.ts` was
    converted into the focused Phase 1 suites and removed. No tracked or
    user-owned file was discarded.
- Test and verification commands with results:
  - focused pre-change characterization:
    `node .\node_modules\vitest\vitest.mjs run src\world.test.ts
    src\recoveryPhase0.characterization.test.ts
    server\recoveryPhase0.characterization.test.ts server\system.test.ts
    --reporter=dot` passed 4 files and 21 tests;
  - early canonical-step and scheduler guard:
    `node .\node_modules\vitest\vitest.mjs run
    src\sim\SimCore.test.ts src\recoveryPhase1.world.test.ts --reporter=dot`
    passed 2 files and 9 tests at that point;
  - the first five-file lifecycle gate produced 16 passes and one timed-out
    bind test plus an uncaught `EADDRINUSE`. This was retained as defect
    evidence and led to attaching WebSocket resources only after successful
    HTTP bind;
  - targeted corrected lifecycle:
    `node .\node_modules\vitest\vitest.mjs run
    server\recoveryPhase1.lifecycle.test.ts --reporter=dot` passed 1 file and
    4 tests;
  - final focused Phase 1:
    `node .\node_modules\vitest\vitest.mjs run
    src\sim\SimCore.test.ts src\recoveryPhase1.world.test.ts
    server\recoveryPhase1.lifecycle.test.ts server\system.test.ts
    server\inferenceMode.test.ts --reporter=dot` passed 5 files and 17 tests;
  - strict TypeScript:
    `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`
    passed with exit code 0;
  - repository-wide ESLint:
    `node .\node_modules\eslint\bin\eslint.js .` passed with exit code 0;
  - full JavaScript suite:
    `node .\node_modules\vitest\vitest.mjs run --reporter=dot` passed 45 files
    and 177 tests;
  - client build: `node .\node_modules\vite\bin\vite.js build` passed, with
    the already-recorded `node:module` browser-externalization warning from
    `nativeBridge`;
  - stale-reference scan found no `World.update`, `World.updateAsync`,
    `_stepPhysics`, `_stepPhysicsAsync`, or `dtClamp` references in production,
    tests, README, or package metadata;
  - final `git diff --check` passed. Its only output was Git's existing LF to
    CRLF conversion warnings for touched tracked files;
  - native/Rust source was unchanged in Phase 1, so the recorded baseline of 3
    passing release tests remains the last Rust result and was not rerun.
- Defect/handoff notes:
  - `CORE-002` was corrected by deferring serial inference until the stable
    control scan completes, matching the pooled observation boundary;
  - `TEST-002` had both silent network-test returns and a second uncaught
    WebSocket bind-error path. The required system test no longer returns
    early, and the lifecycle suite now proves `EADDRINUSE` rejection;
  - no new blocker remains. Known later-phase defects remain registered and
    were not repaired during Phase 1.
- Last successful acceptance gate: Phase 1 one-step-pipeline and truthful time
  gate. All Phase 1 checklist, required-test, and acceptance items are checked.
  Phase 2 has not started.

## Verification command reference

Use direct binaries when the npm/PowerShell wrapper obscures completion:

```powershell
node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false
node .\node_modules\eslint\bin\eslint.js .
node .\node_modules\vite\bin\vite.js build
npm --prefix native run build
cargo test --manifest-path native\Cargo.toml --release
node .\node_modules\vitest\vitest.mjs run --reporter=dot
git status --short --branch
```

If a command is changed during Phase 8, update this reference and the README or
AGENTS workflow documentation together.
