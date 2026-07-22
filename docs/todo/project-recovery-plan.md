# Slither Neuroevolution project recovery plan

## Document control

- Status: authoritative, owner-approved implementation plan; Phases 0 through
  8 are complete, and Phase 9 has not started.
- Created: 2026-07-21.
- Branch: `exclusive-server-mode-refactor`.
- Audit baseline commit: `cb276cce8dfc58a2fb3a3fdc3b60659626131ed0`.
- Current implementation HEAD: `308c6f0dd91eca8091bc75dcf08ca87904da2d50`.
- Last fully verified HEAD: `308c6f0dd91eca8091bc75dcf08ca87904da2d50`.
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
- 2026-07-22: The owner committed and pushed the verified Phase 0/1 work as
  `3fe62d0`; began Phase 2 from that clean, remote-synchronized worktree after
  re-reading this plan and the repository instructions.
- 2026-07-22: Completed Phase 2 with versioned stateful randomness, independent
  labeled streams, deterministic Reset/New Run identity semantics, exact
  generation-boundary capture, exported allocator state, and 199 passing
  JavaScript tests. Phase 3 was not started.
- 2026-07-22: The owner committed and pushed the fully verified Phase 2 work as
  `acb634f`; the local branch and upstream are synchronized. Phase 3 remains
  unstarted.
- 2026-07-22: Committed the owner-requested Phase 2 checkpoint metadata as
  `51e5ded`, then began Phase 3 from its clean worktree. The commit is local and
  was pushed immediately before the later Phase 3 implementation commit.
- 2026-07-22: Completed Phase 3 with immutable native/JS backend selection,
  checked Rust N-API boundaries, source-derived addon identity, required-native
  tests, and existing-matrix CI coverage. Phase 4 was not started.
- 2026-07-22: The owner approved the detailed Phase 3 commit message. Staged
  only the verified Phase 3 scope, committed it as `58f85b0`, and pushed it;
  the local branch and upstream are synchronized. Phase 4 remains unstarted.
- 2026-07-22: Began Phase 4 from synchronized HEAD `58f85b0`, preserving the
  post-commit plan checkpoint and content-identical `AGENTS.md` status marker.
- 2026-07-22: Completed Phase 4 with one canonical Node worker pool, stable
  recurrent slot ownership, pool/weight epochs, awaited generation and reset
  boundaries, deterministic native worker parity, explicit fault recovery,
  tagged MT visualization, and 233 passing JavaScript tests. Phase 5 was not
  started.
- 2026-07-22: Committed and pushed the verified Phase 4 work as `7ca46fa`,
  synchronized the local branch with its upstream, and began Phase 5 from that
  committed baseline. The content-identical `AGENTS.md` status artifact remains
  intentionally untouched.
- 2026-07-22: Completed Phase 5 with one explicit score-observation boundary,
  consistent external/serial/pooled delivery, accumulated skipped-cadence
  deltas, aligned protocol/API wording, digest schema v3, and 242 passing
  JavaScript tests. Phase 6 was not started.
- 2026-07-22: Committed and pushed the verified Phase 5 work as `957b76f`,
  synchronized the local branch with its upstream, and began Phase 6 from that
  committed baseline. The content-identical `AGENTS.md` status artifact remains
  intentionally untouched.
- 2026-07-22: Completed and fully verified Phase 6 server-authoritative live
  controls and God Mode. Protocol 2, boundary-queued commands, canonical live
  config identity, extracted client controls, and server-result-driven UI state
  are complete. Phase 7 has not started.
- 2026-07-22: Committed and pushed the verified Phase 6 work as `24b587a`,
  synchronized the local branch with its upstream, and began Phase 7 from that
  committed baseline. The content-identical `AGENTS.md` status artifact remains
  intentionally untouched.
- 2026-07-22: Completed Phase 7 with the normalized SQLite child-row format,
  exact-boundary typed checkpoints, strict current/legacy readers,
  deterministic startup reconstruction, explicit fresh/latest/id selection,
  durable Reset/New Run, incremental HTTP JSON export, and bounded-memory
  measurement harness. All 290 JavaScript tests, strict TypeScript,
  repository-wide ESLint, the production build, and diff hygiene pass. Phase 8
  was not started.
- 2026-07-22: Committed and pushed the fully verified Phase 7 work as
  `308c6f0`, synchronized the local branch with its upstream, and began Phase 8
  from that committed baseline. The content-identical `AGENTS.md` status
  artifact remains intentionally untouched.
- 2026-07-22: Completed Phase 8 with explicit non-overlapping test layers,
  required-native and visible network opt-out contracts, real WebSocket control
  integration, runtime/worker identity assertions, a small startup smoke test,
  measured performance diagnostics, and an Ubuntu/Windows native+MT CI matrix.
  All 60 JavaScript test files/298 tests, the 7-file/45-test required-native
  overlay, Rust tests/fmt/clippy, strict TypeScript, repository-wide ESLint, and
  the production build pass. Phase 9 was not started.

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
| DET-005 | Baseline-bot respawn recreates its RNG from the original seed, rewinding spawn and behavior draws instead of continuing the durable bot stream | `BaselineBotManager.prepareBotSpawn` replaces the live closure with `createRng(seed)` on every respawn | 2 |
| NAT-001 | Active startup does not load the neural kernel bridge before brain construction | `server/index.ts` and active worker path | 3 |
| NAT-002 | Full-world native adapter requires a Rust `World` export that does not exist | `server/native-backend.ts` and `native/src/lib.rs` | 3 |
| NAT-003 | N-API functions enter unsafe pointer code without validating array lengths and dimensions | `native/src/SIMD_Kernals.rs` | 3 |
| NAT-004 | Native package metadata and targets are largely napi-rs template defaults | `native/package.json` | 3 and 9 |
| NAT-005 | Canonical worker message handling can begin before asynchronous native loading completes | `server/worker/inferWorker.ts` installs its handler before the final native-load await | 3 and 4 |
| NAT-006 | A `GraphBrain` records one backend at construction, while GRU/LSTM/RRU primitives recheck global native availability on every step and can execute a different backend than diagnostics report | `src/brains/graph/runtime.ts` and `src/brains/ops.ts` | 3 |
| NAT-007 | The native loader reports ready without validating every required kernel export or a source-derived build identifier, deferring stale/incompatible-addon failure until inference | `src/brains/nativeBridge.ts` | 3 |
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
| MT-011 | Copying a new shared weight epoch does not update graph operators that copied their constructor weights | `GraphBrain` operator construction and the candidate pool reset path | 4 |
| MT-012 | MT visualization can select a baseline or other null-slot focus snake and never request its owning worker | `SimServer.pickVizSnake` and `SimServer.buildStats` | 4 |
| SNS-001 | `prepareForStep` overwrites the score marker immediately before neural sensing | `Snake.prepareForStep` and sensor index 8 | 5 |
| SNS-002 | External and neural controllers sample score delta at different lifecycle points | `World._publishControllerSensors` versus physics control evaluation | 5 |
| UI-001 | Live controls mutate browser `CFG` only | `main.ts liveUpdateFromSlider` | 6 |
| UI-002 | Simulation-speed input refreshes only its displayed label | `main.ts` speed input listener | 6 |
| UI-003 | God Mode kill logs but sends nothing; drag discards coordinates | `main.ts` canvas handlers | 6 |
| UI-004 | Live/reset metadata is private to a DOM-oriented module | `src/settings.ts` | 6 |
| UI-005 | At least two "live" values need explicit derived-state handling | collision cell size and baseline bot respawn delay | 6 |
| UI-006 | Settings UI and snapshot coercion still advertise removed v2/legacy sensor layouts although v3 is the sole runtime contract | `settings.ts`, `protocol/settings.ts`, `settingsSnapshot.ts` | 6 |
| CFG-001 | Config hash and seed captured by HTTP/welcome state become stale after changes | `server/index.ts` closure values | 6 and 7 |
| CFG-002 | Config identity depends on raw object property order | `server/hash.ts` hashes `JSON.stringify` output directly | 6 |
| PER-001 | Advertised chunked module does not exist | Documentation versus filesystem | 7 |
| PER-002 | Save converts all weights to JSON arrays, accumulates buffers, concatenates, then gzip-compresses synchronously | `World.exportPopulation` and `server/persistence.ts` | 7 |
| PER-003 | Startup reads snapshot settings but never restores saved genomes | `server/index.ts` | 7 |
| PER-004 | HTTP export recreates the entire population as one JSON response | `server/httpApi.ts` | 7 |
| PER-005 | Automatic checkpoints default to disabled and the current save hook runs after new-generation spawning/random draws | `server/config.ts`, `World._endGeneration`, and server generation-change handling | 2 and 7 |
| PER-006 | Snapshot restore converts `brain.useMlp` from Boolean `true` to numeric `1`, changing canonical config identity and preventing exact resume | `src/protocol/settings.ts` and `server/settingsSnapshot.ts` | 7 |
| TEST-001 | Native tests silently pass without native | `src/brains/nativeBridge.test.ts` | 3 and 8 |
| TEST-002 | Network suites silently return on bind permission errors | server acceptance/integration/system/security suites | 1, 6, and 8 |
| TEST-003 | The main UI test asserts little beyond WebSocket construction | `src/main.test.ts` | 8 |
| TEST-004 | Test category scripts cover sparse filename suffixes rather than coherent contracts, and the Windows `.cmd` child process fails with `EINVAL` | `scripts/run-tests.ts` | 8 |
| TEST-005 | Acceptance and security tests inherit the production SQLite path, allowing one test run's checkpoint to contaminate later restart behavior and local user state | `server/acceptance.test.ts` and `server/security.test.ts` | 7 and 8 |
| TEST-006 | Phase 4 SimServer fixtures invoke New Run without persistence, so the full suite fails once durable-before-switch is enforced | `server/recoveryPhase4.simServer.test.ts` | 7 |
| TEST-007 | HTTP/config integration sends a live setting after an arbitrary 100 ms reset delay, racing asynchronous reset under full-suite load | `server/integration.test.ts` | 8 |
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

- [x] Pass `worldSeed` from server startup into `SimCore` and `World`.
- [x] Store the active seed and every authoritative derived-stream state on the
  authoritative core/world.
- [x] Add the reusable seeded full-World fixture deferred from Phase 0 and use
  normal production construction paths without monkeypatching globals.
- [x] Replace authoritative `rand`, `randInt`, and `gaussian` calls with
  injected PRNG operations.
- [x] Add optional RNG parameters to genome and brain-weight initialization.
- [x] Inject RNG into `Genome.random`.
- [x] Inject RNG into crossover, recurrent crossover, tournament selection,
  mutation, and Gaussian mutation.
- [x] Inject RNG into snake spawn position/heading.
- [x] Inject RNG into death-pellet value/jitter and boost-pellet jitter.
- [x] Inject RNG into ambient pellet generation and fallback spawning.
- [x] Replace random resurrect IDs with a deterministic, collision-safe
  allocator.
- [x] Derive baseline-bot streams from the run seed and durable bot identity;
  verify they do not consume the world/evolution streams accidentally.
- [x] Move camera/focus randomness to a derived observer stream or a
  deterministic selection rule.
- [x] Keep render particles and client effects off the authoritative stream.
- [x] Generate an unspecified new run seed with a system entropy source, not
  `Math.random`.
- [x] Generate session IDs independently from simulation randomness.
- [x] Give each lineage a non-randomness-consuming run ID used to group its
  checkpoints.
- [x] Make Apply/Reset create generation one from the same seed and current
  authoritative configuration, with a new run ID and zero recurrent state.
- [x] Add an explicit New Run API that performs the same restart with a new
  entropy-derived seed and new run ID.
- [x] In this phase, prove deterministic in-memory restart and identity
  semantics. Do not claim Reset/New Run crash durability until Phase 7 commits
  the required run-start checkpoint.
- [x] Give authoritative generated IDs an exported/restored allocator state or
  derive them from stable run/generation/step/counter fields.
- [x] Add a lint restriction or focused static guard preventing
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

- [x] Known PRNG sequence test.
- [x] PRNG state export/restore test.
- [x] Gaussian cache state export/restore test if the algorithm caches a value.
- [x] Same seed creates identical initial genomes, snakes, pellets, and focus.
- [x] Different seeds diverge.
- [x] Same seed and action log produce identical state for many fixed ticks.
- [x] Normal reset reproduces the same initial state.
- [x] New Run produces a new visible seed and divergent state.
- [x] Evolution selection, crossover, and mutation reproduce across runs.
- [x] Death and boost pellet placement reproduce across runs.
- [x] Cosmetic rendering calls do not change authoritative state.
- [x] Adding observer/cosmetic work does not shift world, evolution, or bot
  stream sequences.
- [x] Canonical digest ordering is unaffected by incidental array/batch order.

### Acceptance gate

- [x] Authoritative modules contain no unapproved `Math.random` calls.
- [x] Seeded replay passes without monkeypatching globals.
- [x] Seed and RNG version/state are available to persistence.

## Phase 3: Native kernel safety and runtime activation

### Purpose

Make the existing Rust code useful in the normal runtime while keeping its
scope narrow and its unsafe boundary checked.

### Detailed checklist

- [x] Delete `server/native-backend.ts` and the `PhysicsBackend` branch after
  confirming no supported caller remains.
- [x] Remove `SLITHER_NATIVE_BACKEND` full-world behavior and replace it with a
  clear math-backend configuration if needed.
- [x] Select and load an immutable backend before creating any main-thread
  `GraphBrain`; diagnostics must come from the backend actually attached to the
  brain, not only from requested configuration.
- [x] Load the selected worker backend before installing a worker message
  handler or compiling/creating worker brains.
- [x] Make normal native startup fail with a concise actionable error if the
  addon is absent or incompatible.
- [x] Support an explicit JS diagnostic backend without pretending native ran.
- [x] Export or derive a native-addon build identifier from crate/package
  version plus source revision; do not report package metadata as proof that a
  stale binary did not load.
- [x] Record requested/active backend and addon build ID in runtime diagnostics.
- [x] Validate dimensions, strides, multiplication overflow, and all input,
  output, scratch, weight, and recurrent-state lengths in Rust before entering
  unsafe pointer code.
- [x] Validate zero-sized/invalid dimensions and unsupported buffer overlap or
  aliasing assumptions before unsafe code.
- [x] Return structured N-API errors for invalid calls.
- [x] Keep `unsafe` blocks narrowly scoped with accurate safety comments.
- [x] Add invalid-length and invalid-dimension tests through the exported
  N-API boundary.
- [x] Remove native-test early returns in this phase. A missing addon fails the
  required-native suite; only the explicitly named JS suite can run without it.
- [x] Rename the misspelled `SIMD_Kernals.rs` to `simd_kernels.rs` if the
  rename is isolated and does not complicate active work.
- [x] Remove unused Rust dependencies.
- [x] Reduce `native/package.json` from generated template metadata to the
  packages, targets, and scripts this repository actually supports.
- [x] Keep only x86_64 targets while the crate has an x86_64 compile error.
- [x] Strengthen the repository's existing Ubuntu/Windows CI matrix for addon
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

- [x] Dense native versus JS reference.
- [x] MLP native versus JS reference.
- [x] GRU native versus JS reference across multiple recurrent steps.
- [x] LSTM native versus JS reference across multiple recurrent steps.
- [x] RRU native versus JS reference across multiple recurrent steps.
- [x] Invalid buffers fail safely rather than reading out of bounds.
- [x] Unsupported aliasing/overlap fails safely, or is explicitly proven safe.
- [x] A missing addon fails the native-required suite instead of returning.
- [x] Runtime integration asserts native mode is active.
- [x] Explicit JS diagnostic mode asserts native is inactive.

### Acceptance gate

- [x] Normal single-threaded server inference uses Rust kernels.
- [x] No code expects a Rust `World` export.
- [x] Unsafe exported calls validate their contracts.

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

- [x] Promote `server/brainPool.ts` as the canonical lifecycle foundation.
- [x] Define one parent/worker message protocol with batch IDs, pool
  epoch, weight epoch, and reset acknowledgement. Keep evolutionary generation
  separate from pool lifecycle.
- [x] Cap automatic worker count by CPU count and population size; add a
  conservative upper bound pending benchmarks.
- [x] Allocate shared input, output, index, and population-weight buffers with
  validated capacities.
- [x] Copy initial population weights during pool initialization.
- [x] Construct worker brains only after native/JS backend readiness.
- [x] Construct only the slots owned by each worker.
- [x] Dispatch batches without changing slot ownership.
- [x] Route noncontiguous batch entries explicitly: recurrent state indexes by
  population slot while output indexes by batch position.
- [x] Ignore or reject stale completion messages by batch ID and epoch.
- [x] Add inference timeout, init timeout, reset timeout, worker error, and
  unexpected-exit handling.
- [x] Prevent concurrent in-flight batches.
- [x] On new generation, copy new weights, advance weight epoch, reset all
  recurrent brains, and await acknowledgement before inference.
- [x] On import/reset/architecture change, rebuild the pool deliberately.
- [x] Apply worker-count and backend changes only while rebuilding at a
  recurrent-reset boundary.
- [x] Keep MLP, GRU, LSTM, and RRU behavior under the same pool abstraction.
- [x] Batch only snakes with a valid population slot.
- [x] Run resurrected and unowned external neural snakes through their own
  main-thread native brains.
- [x] On worker error, unexpected exit, inference timeout, epoch mismatch, or
  unrecoverable protocol violation, reject the in-flight step and enter the
  faulted state defined by DEC-005. Keep status available; allow only explicit
  Reset, New Run, or checkpoint-resume recovery.
- [x] Never rebuild a failed pool and continue mid-generation. Pool rebuild is
  safe only at a new-generation, import, reset, resume, architecture, backend,
  or worker-count boundary where recurrent state is defined as zero.
- [x] Await pool shutdown during server close.
- [x] Add a rate-limited visualization request for the selected slot and return
  activations tagged with slot, simulation step, pool epoch, and weight epoch
  from its owner worker. Discard delayed mismatched responses.
- [x] Remove the duplicate active chain under `src/sim` and `src/worker` only
  after replacement tests pass.
- [x] Remove tests that cover only the deleted duplicate, replacing them with
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

- [x] Initial inference uses nonzero expected population weights.
- [x] Shuffled batch order preserves outputs.
- [x] Pool lifecycle, weights, shuffled batches, and worker-count equality pass
  for all four brain families.
- [x] Deaths and shrinking batches preserve GRU/LSTM/RRU recurrent history.
- [x] A recurrent snake absent for several batches retains its own state.
- [x] Generation transition applies new weights and zeroes old state.
- [x] Architecture change rebuilds buffers and brains.
- [x] Resurrected snake does not overflow population buffers.
- [x] Released external snake uses a valid serial brain.
- [x] Worker crash rejects in-flight work and surfaces server failure.
- [x] A worker crash publishes no successful step/frame, causes no fallback,
  and prevents further steps until explicit boundary recovery.
- [x] Stale completion cannot finish a later batch.
- [x] Native is reported active inside every native worker.
- [x] Selected-brain visualization is available under MT.

### Acceptance gate

- [x] Within one code/runtime/addon build, the same native seed/run is identical
  across 1, 2, and 4 workers.
- [x] Recurrent tests survive deaths and repartitioning.
- [x] Only one production Node brain pool remains.
- [x] MT and native are demonstrably active together.

## Phase 5: Score-delta sensor semantics

### Purpose

Give sensor index 8 a useful and consistent temporal meaning.

### Target behavior

`points_delta_norm` reports the score change accumulated since the same snake's
previous delivered sensor sample. It includes survival, food, kill, and boost
cost changes that occurred between observations. It does not depend on whether
the next action is external, serial neural, or pooled neural.

### Detailed checklist

- [x] Rename `prevPointsScore` to a name that describes the observation
  boundary, such as `pointsAtLastSensorSample`.
- [x] Remove score-marker overwrite from `prepareForStep`.
- [x] Keep pure sensor construction separate from marker mutation.
- [x] Add one sampling method that builds sensors and then commits the score
  marker exactly once.
- [x] Use that method for serial neural control.
- [x] Use that method for pooled neural control.
- [x] Use that method for external controller sensor messages.
- [x] Define first-sample behavior explicitly.
- [x] Define behavior when control cadence skips physics substeps.
- [x] Update sensor protocol documentation.

### Required tests

- [x] First observation has the documented delta.
- [x] Survival reward appears once.
- [x] Food gained after one observation appears in the next.
- [x] Kill reward appears in the next observation.
- [x] Boost spending produces a negative delta.
- [x] A repeated sample without score change returns zero.
- [x] Skipped control intervals accumulate rather than discard changes.
- [x] External, serial neural, and pooled neural sampling agree.

### Acceptance gate

- [x] No path writes the marker immediately before reading it.
- [x] Sensor contract and API documentation use the same wording.

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

- [x] Extract shared setting definitions from `src/settings.ts`.
- [x] Audit every currently live setting for derived state.
- [x] Mark genuinely reset-required settings as such.
- [x] Add a safe live reconfiguration path for collision-grid cell size or
  reclassify it as reset-required.
- [x] Add a live update method for baseline bot respawn delay.
- [x] Remove obsolete `dtClamp` setting if Phase 1 makes it meaningless.
- [x] Add strict server protocol validation.
- [x] Increment the handshake and message schema to Protocol 2; reject Protocol
  1 explicitly.
- [x] Add `WsClient` send methods and response handling.
- [x] Debounce/coalesce high-frequency slider changes by path.
- [x] Coalesce only one client's unsent updates for the same path. An optimistic
  display may move immediately, but accepted/stored state comes from the
  server result or broadcast.
- [x] Queue accepted live updates in `SimServer`.
- [x] Assign queued commands a total order and drain them before their fixed
  step, never during awaited inference.
- [x] Validate each atomic request against prior accepted requests in order;
  acknowledge only values actually applied and increment revision once.
- [x] Broadcast normalized authoritative results to every UI and update browser
  `CFG` from results/broadcasts, including server clamping.
- [x] Send sim-speed changes through the same authoritative mechanism.
- [x] Recompute a versioned canonical config hash from explicitly included,
  recursively sorted fields after relevant changes. Do not hash raw object
  insertion order; keep content hash distinct from monotonic revision.
- [x] Replace captured HTTP seed/hash values with getters from active state.
- [x] Add God Mode message routing for joined UI clients.
- [x] Queue God Mode operations at a tick boundary.
- [x] Kill through the normal `Snake.die` behavior.
- [x] Move by translating head and all body points by the same delta.
- [x] Validate finite coordinates and clamp/reject the translation delta so the
  entire body, not only the head, remains within defined world bounds.
- [x] Rebuild or invalidate spatial state after a move before it is queried.
- [x] Throttle drag messages and always send a final mouse-up position.
- [x] Update the log from server results, not optimistic console text.
- [x] Extract transport/control logic from the 4,000-line `main.ts` into small
  testable modules.
- [x] Keep selection/frame parsing client-side and unchanged unless a test
  proves an offset bug.

### Required tests

- [x] Each live UI interaction emits the expected message.
- [x] Reset-required paths are rejected by the live validator.
- [x] Server clamps/normalizes values from shared metadata.
- [x] Update arriving during worker inference waits until the next boundary.
- [x] Sim speed visibly and authoritatively changes generation-time rate.
- [x] Config hash/revision changes when appropriate.
- [x] God kill changes the serialized alive set and applies normal drops.
- [x] God move translates the entire body and leaves valid collision queries.
- [x] Invalid IDs and coordinates return a rejected result.
- [x] Drag coalescing applies the final requested position.
- [x] Multiple UI clients receive coherent authoritative setting state.
- [x] Two clients racing updates converge by config revision and authoritative
  broadcast order.
- [x] One accepted multi-path request increments revision once; a rejected
  request does not increment it.
- [x] Canonically equivalent configs produce the same hash regardless of
  property insertion order.

### Acceptance gate

- [x] No live control only mutates browser-private state.
- [x] God Mode kill and drag work against serialized server frames.
- [x] UI tests exercise extracted behavior rather than only module startup.

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

- [x] Separate internal typed snapshot models from JSON transport DTOs.
- [x] Add idempotent schema migration for format/version fields and child
  rows.
- [x] Enable and assert SQLite foreign-key enforcement.
- [x] Add prepared insert/select statements for genome rows.
- [x] Add transactional per-genome save.
- [x] Add typed per-genome load.
- [x] Encode Float32 BLOBs as little-endian and validate byte length, checksum,
  finiteness, graph parameter count, brain type, slot density, and population
  count on read.
- [x] Preserve legacy blob read.
- [x] Stop new writes to `genomes_blob`.
- [x] Avoid `World.exportPopulation` on automatic/internal save paths.
- [x] Capture the generation-boundary RNG state required for exact
  reconstruction of that generation's initial world.
- [x] Split the current post-spawn `_endGeneration`/server save flow at the
  exact pre-spawn checkpoint boundary defined above.
- [x] Include run ID, active seed, RNG version/state, allocator state, graph
  spec, settings, boundary type, and config identity.
- [x] Change the automatic-checkpoint default from disabled to every generation
  when the bounded path is ready; retain a visible diagnostic opt-out.
- [x] Make normal startup select and validate the latest checkpoint candidate;
  do not silently skip a corrupt/incompatible latest candidate.
- [x] Add `--fresh`, `--resume latest`, and `--resume <snapshot-id>` behavior
  with the configuration precedence defined above.
- [x] Make a corrupt/incompatible latest checkpoint produce an actionable
  error and list available alternatives.
- [x] Do not delete or overwrite older snapshots during migration.
- [x] Make Reset and New Run commit a generation-one run-start checkpoint
  before switching current run state, without deleting saved history.
- [x] Update import so an imported seed is either applied explicitly or
  clearly treated as metadata; do not silently ignore it.
- [x] Make HTTP dependencies query current seed/hash rather than startup
  constants.
- [x] Stream large export responses incrementally if retaining JSON.
- [x] Preserve the current user-visible JSON export shape if practical while
  serializing one genome at a time; do not require `Content-Length` if that
  would force whole-response buffering.
- [x] Avoid adding a new external file format without recording the user-facing
  compatibility decision in this plan.
- [x] Apply explicit size/count limits with errors that identify the offending
  snapshot or genome.
- [x] For legacy gzip, report snapshot ID, compressed size, expected
  uncompressed size/population where known, and enforce pre-decompression and
  bounded-output limits against corrupt or excessive data.
- [x] Measure save duration and peak memory with a synthetic large population.
- [x] Add a structural test proving automatic save consumes the typed
  per-genome path and never calls the population JSON-export DTO. Use a
  subprocess for process-memory measurement only if the direct harness is too
  noisy.
- [x] Only add a background persistence worker if measurement shows the
  per-row synchronous transaction causes unacceptable tick stalls.

### Required tests

- [x] New-format round trip with multiple architectures' metadata.
- [x] Float32 weights round trip exactly.
- [x] BLOB byte order, byte-length, checksum, finite-value, and slot-continuity
  validation failures are reported clearly.
- [x] Foreign-key cascades work on an actual opened connection.
- [x] Hundreds of synthetic genomes save/load without a combined buffer path.
- [x] Transaction rollback leaves no partial snapshot.
- [x] Legacy blob snapshot still loads.
- [x] Corrupt metadata or genome row fails clearly.
- [x] Startup resumes saved genomes and generation.
- [x] Resumed generation start matches the checkpoint's reconstructed state.
- [x] `--fresh` ignores but preserves existing snapshots.
- [x] `--resume <snapshot-id>` selects an older valid alternative explicitly.
- [x] New Run is immediately restart-resumable even before completing its first
  evolved generation and cannot restore the prior run.
- [x] Checkpoint reconstruction begins before the first new-generation random
  spawn draw with recurrent state in its defined zero condition.
- [x] A required checkpoint failure prevents the new run/generation from being
  advertised as durable or continuing silently.
- [x] Current seed/hash appear in save/export after reset or New Run.
- [x] Export path does not call one giant `JSON.stringify` or `Buffer.concat`.

### Acceptance gate

- [x] New saves have bounded serialization memory.
- [x] Restart restores the evolved population.
- [x] Legacy databases remain usable.
- [x] Documentation distinguishes population checkpoints from full mid-tick
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

- [x] Verify the Phase 3 removal of every `if (!hasNative) return` pass and keep
  required-native tests separate from explicit-JS tests.
- [x] Verify required network suites hardened in Phases 1 and 6 fail normally
  on bind errors.
- [x] Allow network-suite skipping only through an explicit environment
  opt-out whose use is visible in output.
- [x] Replace the old parity test with contract-specific kernel, worker-count,
  recurrent-state, and deterministic replay tests.
- [x] Replace the monolithic fake-DOM main test with extracted module tests and
  a small startup smoke test.
- [x] Rework category scripts so names correspond to real suites rather than
  sparse filename suffixes.
- [x] Ensure MT integration explicitly sets MT on; current defaults must not
  accidentally make the test serial.
- [x] Assert selected backend, worker count, and weight epoch in integration
  tests.
- [x] Assert requested versus active backend/MT state, pool epoch, graph, seed,
  and native-addon build ID come from the runtime that actually executed.
- [x] Build native once per CI job rather than repeatedly in build and test.
- [x] Run Rust unit tests, formatting, and clippy.
- [x] Run TypeScript, ESLint, Vitest layers, and Vite build.
- [x] Keep both Ubuntu and Windows as first-class paths in the existing CI
  matrix; build and load the addon and run native+MT assertions in the same job
  that compiled it.
- [x] Keep performance tests informational until a stable baseline is
  recorded.
- [x] After baseline, set regression thresholds broad enough for shared CI but
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

- [x] CI cannot be green when native failed to load in a native-required job.
- [x] CI cannot be green when a server system suite failed to bind.
- [x] At least one CI path runs native and MT simultaneously.
- [x] All four brain families run lifecycle/weight/worker-count tests; GRU,
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

- Current phase: Phase 8 test and CI reconstruction is complete. Phase 9 has not
  started.
- Active checklist item: none in Phase 8. Await owner review and explicit
  direction before committing, pushing, or starting Phase 9.
- Last completed work: reconstructed the test layers and CI contracts, repaired
  the full-suite reset race exposed by the new manifest, and passed the complete
  Phase 8 verification matrix.
- Source implementation status: the fully verified Phase 8 worktree is
  uncommitted and unstaged. It changes tests, test helpers, package scripts, CI,
  narrow QA workflow documentation, and this plan; it does not change later-
  phase production behavior.
- Current blocker: none. The optional in-app browser spot check could not start
  because the Codex browser runtime was denied access to its own Windows
  `AppData` path. The local native+MT server and Vite client did launch and pass
  HTTP health checks, all temporary processes were stopped, and Phase 9's final
  manual-QA checklist remains intentionally untouched.
- Next action: owner review of the Phase 8 worktree and commit message. Do not
  start Phase 9 in this pass.
- Current implementation HEAD:
  `308c6f0dd91eca8091bc75dcf08ca87904da2d50`.
- Last fully verified HEAD:
  `308c6f0dd91eca8091bc75dcf08ca87904da2d50`.
- Verification scope note: commit `308c6f0` remains the committed baseline; the
  exact unstaged Phase 8 file set below passed all recorded verification. No
  Phase 8 commit or push has occurred.
- Last successful phase acceptance gate: Phase 8 test and CI reconstruction.
- Exact dirty-file summary:
  - content-modified tracked files: `.github/workflows/CI.yml`, `README.md`,
    `docs/todo/project-recovery-plan.md`, `package.json`,
    `scripts/run-tests.ts`, `server/acceptance.test.ts`,
    `server/integration.test.ts`, `server/performance.test.ts`,
    `server/recoveryPhase1.lifecycle.test.ts`,
    `server/recoveryPhase4.simServer.test.ts`, `server/security.test.ts`,
    `server/system.test.ts`, and `src/main.test.ts`;
  - status-only artifact: Git reports `AGENTS.md` modified, but
    `git diff -- AGENTS.md` is empty and its worktree/HEAD blob IDs both equal
    `b7c033c5de793219e590a8382befadb417d77915`; it was not edited;
  - untracked files: `scripts/ci-contract.test.ts`,
    `scripts/test-categories.test.ts`, `scripts/test-categories.ts`,
    `server/test/networkSuites.test.ts`, and
    `server/test/networkSuites.ts`;
  - deleted tracked files: none;
  - staged files: none;
  - branch/upstream divergence: `0 0` at `308c6f0`.
- Pre-change focused baseline:
  `node .\node_modules\vitest\vitest.mjs run
  src\brains\nativeBridge.test.ts
  src\brains\nativeBridge.missing.test.ts
  server\recoveryPhase3.native.test.ts
  server\recoveryPhase4.brainPool.test.ts
  server\recoveryPhase4.simServer.test.ts
  server\inferenceMode.test.ts
  server\recoveryPhase1.lifecycle.test.ts server\integration.test.ts
  server\acceptance.test.ts server\system.test.ts server\security.test.ts
  src\main.test.ts --reporter=dot`; result: 12 files and 56 tests passed.
- The pre-change category runner failed immediately on Windows with
  `spawnSync ... node_modules\.bin\vitest.cmd EINVAL`. The repaired runner uses
  `process.execPath` plus Vitest's ES-module entry point and an explicit file
  manifest; no shell wrapper or filename-suffix discovery remains.
- Network hardening passed 13 tests across 6 files with
  `node .\node_modules\vitest\vitest.mjs run
  server\test\networkSuites.test.ts
  server\recoveryPhase1.lifecycle.test.ts server\integration.test.ts
  server\acceptance.test.ts server\system.test.ts server\security.test.ts
  --reporter=dot`. A separate run with
  `SLITHER_SKIP_NETWORK_TESTS=1` visibly warned, ran 2 non-network tests, and
  skipped exactly 3 TCP-bind tests.
- Explicit category results passed: unit 32 files/131 tests; component 13/99;
  integration 10/61 when included in the final manifest; system 1/1;
  acceptance 1/1; regression 1/1; performance 1/2; security 1/2.
- The complete manifest passed 60 files and 298 tests with
  `node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts all
  --reporter=dot`.
- The required-native overlay passed 7 files and 45 tests with
  `node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts
  native-required --reporter=dot`. It loads the source-identified addon and
  executes native+MT contracts rather than returning when native is missing.
- Focused WebSocket/performance verification passed 7 tests across 2 files
  with `node .\node_modules\vitest\vitest.mjs run
  server\integration.test.ts server\performance.test.ts --reporter=verbose`.
  It proves settings, move, and kill commands cross a real UI WebSocket and
  appear in authoritative binary frames.
- Performance diagnostics recorded 6.455 ms/world frame in isolation and
  15.774 ms/frame under the concurrent full suite against a 40 ms budget.
  Ten dense native batches took 2.291 ms isolated/2.188 ms concurrent, and six
  MLP batches took 2.586 ms isolated/2.824 ms concurrent, each against a broad
  200 ms shared-runner budget. CI retains `continue-on-error` for this layer
  until Ubuntu/Windows history establishes a stable cross-runner baseline.
- The addon was rebuilt once through the compiled napi-rs CLI entry point:
  `node .\node_modules\@napi-rs\cli\dist\cli.js build --platform --release`
  from `native/`. The loaded source identity is
  `slither_native/0.1.0+51e5deda32c5.9f5ea40929585feb`.
- `cargo test --manifest-path native\Cargo.toml --release` passed all 3 Rust
  tests. `cargo fmt --manifest-path native\Cargo.toml -- --check` and
  `cargo clippy --manifest-path native\Cargo.toml -- -D warnings` passed.
- Strict TypeScript passed with
  `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`.
- Repository-wide ESLint passed with
  `node .\node_modules\eslint\bin\eslint.js .`.
- The client production build passed with
  `node .\node_modules\vite\bin\vite.js build`; the existing `node:module`
  browser-externalization warning from `nativeBridge` remains.
- `git diff --check` passed with only the preserved LF-to-CRLF working-copy
  warnings.
- `scripts/ci-contract.test.ts` verifies Ubuntu/Windows and Node 22/24 remain in
  the matrix, native is built exactly once per job, its identity is loaded in
  that job, the native-required MT overlay runs there, every primary layer is
  explicit, no network opt-out is set, and Rust/TypeScript/ESLint/Vite gates
  remain present. The uncommitted workflow has not yet run remotely.
- Local launch smoke started native+MT with two workers and the Vite client on
  ports 5174/5173. `/health` reported the current seed, requested/active native,
  requested MT, two active workers, and the same addon build ID; both endpoints
  returned HTTP 200. The two exact temporary Node processes were stopped and
  both endpoints were confirmed unreachable afterward.
- Local roaming `npm`/`npx` shims are missing `npm-cli.js`/`npx-cli.js`; direct
  commands from this plan were used. The raw napi-rs development launcher also
  conflicts with Node 24 type stripping, so verification used the compiled
  `dist/cli.js` entry point that the real `napi` executable targets.

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

### 2026-07-22 — Phase 2 start

- Re-read this authoritative plan completely, then re-read the repository-root
  `AGENTS.md`. Superseded and archived plans were not consulted for technical
  direction.
- Verified root `C:/Users/jlow8/source/repos/slither_neuroevo`, branch
  `exclusive-server-mode-refactor`, and new HEAD
  `3fe62d0bdec4e9964f7f7c0da9d67ee4249612d2`.
- The worktree was clean and `HEAD...origin/exclusive-server-mode-refactor`
  reported `0 0`, confirming the owner's Phase 0/1 commit was pushed.
- The first Phase 2 action is a source-of-truth audit of all authoritative
  randomness consumers, followed by the smallest focused pre-change RNG/world
  baseline. No Phase 3 work has started.

### 2026-07-22 — Phase 2 implementation checkpoint

- The pre-change RNG/world baseline passed: 4 files and 30 tests.
- Added xorshift32 v1 state with explicit Uint32 hexadecimal continuation,
  versioned polar Box-Muller state, and lossless cached-Gaussian Float64 bits.
- Derived `world`, `evolution`, `observer`, and durable `baseline:<slot>`
  streams directly from the normalized run seed. Production World
  construction no longer uses ambient global randomness.
- Routed genome/brain initialization, selection, recurrent crossover,
  mutation, spawning, food, death/boost drops, focus, and bot behavior through
  their owned streams. Cosmetic particles remain intentionally unseeded and
  outside the canonical gameplay digest.
- Added seed/run identity to `SimCore`, crypto entropy and independent UUIDs at
  the Node server boundary, reproducible same-seed Reset, and an in-memory New
  Run API. Crash-durable acknowledgement remains explicitly deferred to Phase
  7 as required.
- Added export/restore for every stream and generated-id allocator plus the
  exact population-assigned/pre-spawn generation hook. Persistence does not yet
  consume that hook.
- Discovered and fixed `DET-005`: baseline respawn rewound its RNG to the
  original seed. Respawn now continues the durable slot stream.
- Upgraded the canonical digest to schema v2 for seed, gameplay/evolution RNG,
  per-bot RNG, and resurrection allocator state while retaining observer-only
  exclusions.
- Added the seeded production fixture and focused Phase 2 suites. Strict
  TypeScript, repository-wide ESLint, and 21/21 focused Phase 2 tests pass.
- Full JavaScript and production-build verification remain pending. Phase 3
  has not started.

### 2026-07-22 — Phase 2 completion handoff

- Completed every Phase 2 implementation, required-test, and acceptance-gate
  item. Phase 3 has not started, and no Phase 3 production behavior was
  changed.
- The versioned xorshift32 v1 uniform stream and polar Box-Muller v1 Gaussian
  stream preserve lossless integer state and the cached Gaussian value. Stable
  labels derive independent `world`, `evolution`, `observer`, and
  `baseline:<slot>` streams directly from the normalized run seed.
- All audited authoritative consumers now receive their owned stream:
  brain/genome initialization, tournament selection, crossover, mutation,
  snake spawning, ambient/death/boost pellets, baseline bots, and focus. The
  static guard rejects ambient `Math.random` reads in those modules, while
  cosmetic particle randomness remains intentionally non-authoritative.
- `SimCore` and `World` retain the visible seed and run identity. Reset keeps
  the seed and creates a new run ID; New Run uses Node system entropy for a
  different seed and creates another independent run ID. Failed restart
  construction is transactional and leaves the prior world and identity
  intact.
- The generation-boundary hook now observes the assigned new population,
  incremented generation, cleared prior transient world state, zero recurrent
  state, and every authoritative RNG/allocator state before any new-generation
  spawn, pellet, focus, sensor, or inference draw. Phase 7 still owns durable
  checkpoint acknowledgement and crash-resume behavior.
- The canonical digest is schema v2 and includes gameplay/evolution/bot RNG
  state and generated-ID allocator state, while excluding observer-only state
  and run/session identity. The deterministic resurrection allocator is
  collision-safe and exportable/restorable.
- Discovered `DET-005` during implementation: baseline-bot respawn recreated
  its original RNG and rewound the slot's sequence. The fix retains and
  advances the durable per-slot stream, with a regression test proving
  continuation after respawn.
- The first full-suite run exposed one expected hand-built Phase 0 fixture
  mismatch after the new boundary-clear operation was introduced; production
  behavior was correct. The explicit characterization fixture/order was
  updated, and the complete suite then passed.
- Verification commands and results:
  - `node .\node_modules\vitest\vitest.mjs run src\mlp.test.ts src\world.test.ts src\bots\baselineBots.test.ts server\authoritativeWorldDigest.test.ts --reporter=dot`
    passed 4 files and 30 tests before Phase 2 edits;
  - `node .\node_modules\typescript\bin\tsc -p tsconfig.json --noEmit`
    passed after all source and test edits;
  - `node .\node_modules\eslint\bin\eslint.js .` passed after all source and
    test edits;
  - `node .\node_modules\vitest\vitest.mjs run src\rng.test.ts server\recoveryPhase2.determinism.test.ts --reporter=dot`
    passed 2 files and 22 tests;
  - `node .\node_modules\vitest\vitest.mjs run --reporter=dot` passed all 47
    files and 199 tests;
  - `node .\node_modules\vite\bin\vite.js build` passed with 29 modules
    transformed and the existing `node:module` browser-externalization warning
    from `nativeBridge`;
  - the authoritative `Math.random` and legacy random-helper import scans found
    no prohibited use;
  - `git diff --check` passed; its only output was the existing LF-to-CRLF
    working-copy warnings.
- Repository root is `C:/Users/jlow8/source/repos/slither_neuroevo`; branch is
  `exclusive-server-mode-refactor`; both current implementation HEAD and last
  fully verified committed HEAD are
  `3fe62d0bdec4e9964f7f7c0da9d67ee4249612d2`; remote divergence is `0 0`.
- The fully verified Phase 2 worktree remains uncommitted: 13 modified files
  and 4 untracked files, listed exactly under "Live execution status". No file
  was staged, committed, or pushed during this pass. Native/Rust source was not
  changed, so the earlier three-test release result remains the last Rust
  result.
- Current blocker: none. Last successful acceptance gate: Phase 2
  authoritative seeded randomness. The next executor must re-read this plan,
  verify the worktree, and wait for explicit owner direction before Phase 3.

### 2026-07-22 — Phase 2 post-commit synchronization

- Verified repository root `C:/Users/jlow8/source/repos/slither_neuroevo` and
  branch `exclusive-server-mode-refactor`.
- Verified the owner's commit
  `acb634f1af68422b2a18e5d04903e2b140520b68`, with subject `Complete Phase 2
  deterministic authoritative randomness`.
- Verified upstream `origin/exclusive-server-mode-refactor` and divergence
  `0 0`, confirming the commit is pushed and the local branch is synchronized.
- `git status --short --branch` was clean before this metadata update. The
  exact current dirty state is one modified file,
  `docs/todo/project-recovery-plan.md`; there are no staged or untracked files.
- No implementation or test command was rerun because the newly committed
  tree exactly matches the fully verified Phase 2 worktree. The last verified
  results remain TypeScript and ESLint passing, 22 focused Phase 2 tests
  passing, all 199 tests across 47 files passing, the Vite production build
  passing, and `git diff --check` passing.
- Current implementation HEAD and last fully verified HEAD are both
  `acb634f1af68422b2a18e5d04903e2b140520b68`. Last successful acceptance gate
  remains Phase 2 authoritative seeded randomness. There are no new defects or
  blockers, and Phase 3 has not started.

### 2026-07-22 — Phase 3 start

- Staged only `docs/todo/project-recovery-plan.md`, verified the staged diff,
  and created the explicitly requested commit
  `51e5deda32c5861fd34c11ff9c5389ae100e814f` with subject `Record Phase 2
  post-commit checkpoint`. No push was requested or performed.
- Verified root `C:/Users/jlow8/source/repos/slither_neuroevo`, branch
  `exclusive-server-mode-refactor`, a clean post-commit worktree, and upstream
  divergence `1 0`, consisting only of that local plan commit.
- Re-read this authoritative plan completely, then re-read the repository-root
  `AGENTS.md`. The stale-documentation warnings in this plan take precedence;
  the superseded native plan and archive were not consulted.
- Phase 3 is authorized and in progress. The first action is a current-source
  audit of native loading, immutable backend selection, Rust N-API safety,
  package metadata, existing parity tests, and the active CI matrix, followed
  by the smallest direct native pre-change baseline. Phase 4 has not started.
- Current blocker: none. Last successful acceptance gate remains Phase 2
  authoritative seeded randomness.

### 2026-07-22 — Phase 3 completion handoff

- Completed every Phase 3 detailed-checklist, required-test, and
  acceptance-gate item. Phase 4 has not started.
- Current-source audit and implementation result:
  - deleted the unsupported full-World `NativeBackend`, removed the
    `PhysicsBackend` World branch and `SLITHER_NATIVE_BACKEND`, and confirmed
    no supported caller or Rust `World` expectation remains;
  - added one immutable `native`/`js` math-backend selection to server config,
    defaulting normal startup to native with CLI/environment overrides for the
    explicitly named JS diagnostic mode;
  - made server startup prepare and validate the selected backend before any
    World or main-thread brain construction, and threaded that selection
    through `SimCore`, `World`, `Snake`, genome/registry construction, and
    every graph node;
  - made GRU, LSTM, and RRU instances retain the same backend reported by their
    owning `GraphBrain`, fixing newly registered `NAT-006` rather than allowing
    a later global addon load to change an existing brain's execution path;
  - made both current worker chains receive and prepare their selected backend
    before installing a message handler or constructing a brain. Consolidating
    the duplicate pools and recurrent ownership remains Phase 4 work;
  - made loader readiness require every Dense/MLP/GRU/LSTM/RRU export plus a
    nonempty source-derived build identifier, fixing newly registered
    `NAT-007`; normal native failure now reports the direct build instruction
    and names JS as diagnostic-only rather than silently falling back;
  - renamed `SIMD_Kernals.rs` to `simd_kernels.rs`, removed the unused Rust
    dependency, reduced the napi-rs template package to the two supported
    x86_64 targets and actual scripts, and regenerated its lockfile;
  - added a deterministic addon identifier containing crate version, Git
    revision, and a content hash of the native manifest/build/source inputs;
  - moved every public N-API entry behind positive-dimension, stride,
    checked-arithmetic, buffer-length, scratch/state/weight, and writable-alias
    validation. Invalid calls return structured `InvalidArg` N-API errors before
    private raw-pointer kernels run;
  - placed scalar-tail pointer access in explicit unsafe scopes and replaced
    inherited vague comments with exact range/non-overlap safety contracts;
  - strengthened the existing Ubuntu/Windows Node 22/24 matrix to build and
    load the addon, assert its identifier, run Rust release tests, client build,
    typecheck, lint, and the required JavaScript suite without adding a
    duplicate job.
- Test coverage result:
  - Dense and MLP compare native output with bounded JS references;
  - GRU, LSTM, and RRU compare multiple recurrent steps;
  - exported calls reject zero/negative/overflowing dimensions, short inputs,
    outputs, scratch, weights and recurrent state, invalid layers/strides, and
    unsupported writable overlap;
  - the required-native file loads the addon in `beforeAll` and contains no
    missing-addon return, while a fresh subprocess proves a missing addon fails
    with actionable output;
  - runtime integration proves a normal single-threaded server reports
    requested/active native plus the loaded identifier, and explicit JS mode
    reports native inactive and unloaded;
  - the canonical future worker pool has a native-preload initialization and
    inference test without beginning the Phase 4 ownership refactor.
- Verification commands and results:
  - pre-change focused baseline:
    `node .\node_modules\vitest\vitest.mjs run
    src\brains\nativeBridge.test.ts server\inferenceMode.test.ts
    --reporter=dot` passed 2 files and 9 tests;
  - pre-change native baseline and final native unit gate:
    `cargo test --manifest-path native\Cargo.toml --release` each passed all 3
    tests; the final run followed the raw-pointer safety tightening;
  - the repository-local `npm --prefix native run build` wrapper failed
    immediately because its user-level shim targets missing
    `C:\Users\jlow8\AppData\Roaming\npm\node_modules\npm\bin\npm-cli.js`.
    Per the verification-command fallback, no wait/retry loop was used;
  - direct addon build from `native`:
    `node .\node_modules\@napi-rs\cli\dist\cli.js build --platform --release`
    passed after the final native edit;
  - direct addon load/export validation passed for all five kernels and
    `nativeAddonBuildIdentifier`, returning
    `slither_native/0.1.0+51e5deda32c5.9f5ea40929585feb`;
  - `cargo fmt --manifest-path native\Cargo.toml` completed,
    `cargo fmt --manifest-path native\Cargo.toml -- --check` passed, and
    `cargo clippy --manifest-path native\Cargo.toml -- -D warnings` passed;
  - strict TypeScript:
    `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`
    passed;
  - repository-wide ESLint:
    `node .\node_modules\eslint\bin\eslint.js .` passed;
  - final focused Phase 3 gate:
    `node .\node_modules\vitest\vitest.mjs run
    src\brains\nativeBridge.test.ts
    src\brains\nativeBridge.missing.test.ts
    server\recoveryPhase3.native.test.ts server\inferenceMode.test.ts
    server\brainPool.test.ts server\recoveryPhase1.lifecycle.test.ts
    server\recoveryPhase2.determinism.test.ts --reporter=dot` passed 7 files
    and 42 tests;
  - full JavaScript suite:
    `node .\node_modules\vitest\vitest.mjs run --reporter=dot --silent`
    passed all 49 files and 209 tests. An earlier restricted wrapper attempt
    failed before collection when esbuild was denied access while loading the
    Vite config; the direct approved invocation completed normally;
  - client build: `node .\node_modules\vite\bin\vite.js build` passed with 29
    modules transformed and the previously recorded `node:module`
    browser-externalization warning from the Node-only loader branch;
  - static scans found no production `NativeBackend`, `PhysicsBackend`,
    `SLITHER_NATIVE_BACKEND`, `setBackend`, or `native-backend` reference, and
    no skip/early-return pattern in the required-native tests;
  - `git diff --check` passed with only the existing LF-to-CRLF working-copy
    warnings; its final rerun after the completed handoff edit also passed.
- Repository state:
  - root: `C:/Users/jlow8/source/repos/slither_neuroevo`;
  - branch: `exclusive-server-mode-refactor`;
  - current implementation HEAD and last fully verified committed HEAD:
    `51e5deda32c5861fd34c11ff9c5389ae100e814f`;
  - upstream divergence: `1 0`, consisting of the earlier local plan-only
    commit; no Phase 3 commit or push was performed;
  - the exact unstaged worktree is listed under "Live execution status".
- Newly discovered defects `NAT-006` and `NAT-007` were fixed and verified in
  this phase. The absent `markdown-rules/rules.md` reference was reconfirmed as
  existing registered defect `DOC-003`, not rediscovered as new work. No new
  blocker remains.
- Last successful acceptance gate: Phase 3 native kernel safety and runtime
  activation. Await explicit owner direction before Phase 4.

### 2026-07-22 — Phase 3 post-commit synchronization

- Pushed the existing local plan checkpoint
  `51e5deda32c5861fd34c11ff9c5389ae100e814f` to
  `origin/exclusive-server-mode-refactor`, then verified divergence `0 0`.
- After the owner approved the proposed Phase 3 message, inspected the complete
  unstaged name/status and diff summary. Explicitly staged the 33 deliberate
  Phase 3 paths, including the Rust filename replacement and obsolete adapter
  deletion. The content-identical `AGENTS.md` status artifact was excluded.
- `git diff --cached --check` passed, and the staged summary contained only the
  reviewed Phase 3 source, tests, native package, CI, and recovery-plan scope.
- Created commit `58f85b009dbc461702e7f571a4ef0ab964b4a134` with subject
  `Complete Phase 3 native safety and activation` and the owner-reviewed body.
  The commit records 33 files, 2,685 insertions, and 4,954 deletions.
- Pushed `58f85b0` to `origin/exclusive-server-mode-refactor` and verified the
  local branch and upstream are synchronized at divergence `0 0`.
- No validation command was rerun after committing because the commit tree is
  exactly the already verified staged tree. The last results remain 42 focused
  Phase 3 tests, all 209 JavaScript tests across 49 files, 3 Rust release tests,
  and passing TypeScript, ESLint, rustfmt, Clippy, Vite build, addon-load, and
  diff-hygiene checks.
- Current implementation HEAD and last fully verified HEAD are both
  `58f85b009dbc461702e7f571a4ef0ab964b4a134`. Last successful acceptance gate
  remains Phase 3 native kernel safety and runtime activation.
- Current blocker: none. The exact current worktree is this unstaged plan
  synchronization plus the content-identical `AGENTS.md` status artifact; no
  staged, deleted, or untracked path remains. Phase 4 has not started.

### 2026-07-22 — Phase 4 start

- Owner authorized the next phase and asked that the content-identical
  `AGENTS.md` marker be left alone until later documentation work updates it.
- Verified repository root `C:/Users/jlow8/source/repos/slither_neuroevo`,
  branch `exclusive-server-mode-refactor`, HEAD
  `58f85b009dbc461702e7f571a4ef0ab964b4a134`, and upstream divergence `0 0`.
- Preserved the exact starting worktree: unstaged
  `docs/todo/project-recovery-plan.md` post-commit metadata plus the
  byte-identical `AGENTS.md` status marker. No path was staged, deleted, or
  untracked.
- Re-read the Phase 4 worker-ownership design, detailed checklist, required
  matrix, DEC-005/DEC-006, user consultation gates, and repository-root
  `AGENTS.md`. No consultation gate is active.
- The first Phase 4 action is a local source audit of both pool/worker chains,
  World batch routing, generation/reset boundaries, fault propagation,
  diagnostics, and selected-slot visualization, followed by the smallest
  current MT lifecycle baseline. Phase 5 has not started.
- The audit confirmed that production still uses
  `src/sim/NodeBrainPool.ts`/`src/worker/inferWorker.ts`, while
  `server/brainPool.ts`/`server/worker/inferWorker.ts` is the stronger but
  test-only candidate. The production chain does not copy initial population
  weights, and the candidate constructs every population brain in every worker
  and partitions current batch positions, so neither chain satisfies stable
  recurrent ownership.
- `World` already supplies durable `populationSlot` indices and keeps
  slot-less baseline, resurrected, disconnected, and manual snakes on serial
  inference. The replacement can preserve this existing batch contract.
- `SimCore.update()` can commit more than one fixed step per outer server pump.
  Generation synchronization therefore needs an awaited post-step boundary
  before a subsequent fixed step, not only the existing once-per-pump check.
- Focused pre-change command:
  `node .\node_modules\vitest\vitest.mjs run server\brainPool.test.ts
  server\recoveryPhase0.characterization.test.ts
  server\recoveryPhase1.lifecycle.test.ts server\inferenceMode.test.ts
  --reporter=dot`; result: 4 files and 11 tests passed. It reproduced the
  expected expiring `MT-002` zero-weight behavior.
- Added `server/brainPoolProtocol.ts` as the sole canonical parent/worker
  protocol and replaced the candidate pool/worker internals. Workers now
  construct only modulo-owned slots, scan explicit noncontiguous batch
  positions, acknowledge pool/weight epochs, report active backend/build
  identity, and return tagged activation snapshots from the selected slot.
- `server/recoveryPhase4.brainPool.test.ts` exercises MLP, GRU, LSTM, and RRU
  under JS and native backends for 1, 2, and 4 requested workers. Shuffled and
  shrinking batches, absent recurrent slots, initial nonzero weights, new
  generation reset, architecture reinitialization, stale completions,
  injected worker failure, capacity validation, and MT visualization pass.
- The first matrix run found newly registered `MT-011`: graph operators copy
  their constructor weights, so copying the shared population buffer did not
  update existing recurrent kernels. The reset protocol now rebinds each owned
  brain to its shared slot before zeroing state. The focused rerun passed 16/16
  tests across 2 files; TypeScript and focused ESLint also pass.
- Production integration found newly registered `MT-012`: the existing
  visualizer selector could choose a baseline/null-slot focus under MT and
  therefore issue no worker request. MT selection now prefers an alive,
  uncontrolled population slot, while preserving the serial/unowned fallback.
  The SimServer integration rerun passed 5/5 tests.
- Current blocker: none. Last successful acceptance gate remains Phase 3
  native kernel safety and runtime activation.

### 2026-07-22 — Phase 4 completion handoff

- Completed every Phase 4 detailed-checklist, required-test, and acceptance
  item. Phase 5 has not started.
- Promoted `server/brainPool.ts` into the sole production Node pool and added
  `server/brainPoolProtocol.ts` as the one shared parent/worker contract.
  Pool lifecycle and population weights now use separate monotonic epochs;
  batch completions also carry a monotonic batch id.
- Pool initialization validates exact shared capacities, copies nonzero
  population weights before worker construction, caps workers by available
  CPUs, population size, and a conservative eight-worker ceiling, and requires
  every worker to report the requested backend and native build identity.
- Each worker constructs only slots satisfying the stable modulo ownership
  rule. Every worker scans the explicit current batch positions and evaluates
  only owned durable population slots, so batch shuffling, deaths, shrinking
  batches, and temporarily absent recurrent snakes do not migrate GRU/LSTM/RRU
  state. Slot-less baseline, external, and resurrected snakes remain on their
  own serial main-thread brains.
- New generation synchronization copies the complete population, advances the
  weight epoch, rebinds graph operators to their shared slot, zeroes recurrent
  state, and awaits every reset acknowledgement. `SimCore` now awaits a
  post-commit hook between fixed steps, so a generation transition inside a
  multi-step scheduler pump is synchronized before the next inference.
- Reset, New Run, successful import, and architecture changes execute behind a
  server boundary barrier, await old-pool shutdown, deliberately build a new
  pool epoch, and clear a prior fault only after rebuild succeeds. A failed
  worker, timeout, unexpected exit, future/mismatched epoch, or protocol
  violation never enables serial fallback or mid-generation pool rebuilding.
- Worker faults reject the authoritative step, publish no successful frame,
  stats, or checkpoint from that tick, retain health/status access, and
  broadcast a structured error. A focused test proves later ticks remain
  prohibited until explicit New Run recovery creates a fresh pool epoch.
- MT visualization now requests the selected population slot from its owning
  worker, rate-limits requests, tags activation snapshots with slot, simulation
  step, pool epoch, and weight epoch, and filters stale or mismatched cached
  results. Under MT, selection prefers an alive uncontrolled population slot
  while retaining serial fallback for a genuinely unowned selection.
- Replacement coverage passed before deleting
  `src/sim/BaseBrainPool.ts`, `src/sim/NodeBrainPool.ts`,
  `src/sim/poolProtocol.ts`, and `src/worker/inferWorker.ts`. The expiring
  `MT-002` Phase 0 characterization was converted rather than preserving a
  test for deleted behavior. Static scans find no remaining production
  `NodeBrainPool`, `BaseBrainPool`, `IBrainPool`, or duplicate protocol use.
- Required matrix evidence:
  - MLP, GRU, LSTM, and RRU each match their serial reference under native and
    JS diagnostic backends for 1, 2, and 4 requested workers;
  - shuffled and shrinking batches plus absent slots preserve recurrent
    history and produce identical histories across worker counts;
  - a complete native SimServer seed/run produces the same authoritative
    digest for 1, 2, and 4 requested workers;
  - native backend/build identity is reported by every native worker;
  - initial weights, generation reset, architecture/reset/import rebuild,
    stale completion, concurrent dispatch rejection, init/infer/reset timeout,
    unexpected exit, selected-slot visualization, and fault recovery contracts
    are covered.
- Newly discovered `MT-011` was fixed: graph operators copy constructor
  weights, so reset now rebinds every owned brain after the shared copy and
  before state reset. Newly discovered `MT-012` was fixed: MT visualization no
  longer stalls on a baseline/null-slot focus. No newly discovered blocker
  remains.
- Verification commands and results:
  - pre-change baseline:
    `node .\node_modules\vitest\vitest.mjs run server\brainPool.test.ts
    server\recoveryPhase0.characterization.test.ts
    server\recoveryPhase1.lifecycle.test.ts server\inferenceMode.test.ts
    --reporter=dot`; 4 files and 11 tests passed;
  - canonical pool matrix/lifecycle:
    `node .\node_modules\vitest\vitest.mjs run server\brainPool.test.ts
    server\recoveryPhase4.brainPool.test.ts --reporter=dot`; 2 files and 21
    tests passed;
  - production MT integration:
    `node .\node_modules\vitest\vitest.mjs run
    server\recoveryPhase4.simServer.test.ts --reporter=dot`; 1 file and 6
    tests passed;
  - post-consolidation focused regression command over 9 files passed 58 tests
    before the final lifecycle additions; all those tests are included in the
    later complete suite;
  - `node .\node_modules\vitest\vitest.mjs run --reporter=dot`; all 233 tests
    across 51 files passed;
  - `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`;
    passed;
  - `node .\node_modules\eslint\bin\eslint.js .`; passed;
  - `node .\node_modules\vite\bin\vite.js build`; passed with the existing
    `node:module` browser-externalization warning from `nativeBridge`;
  - `cargo test --manifest-path native\Cargo.toml --release`; 3 tests passed,
    with the existing home-path canonicalization warning;
  - `git diff --check`; passed, with only the existing LF-to-CRLF working-copy
    warnings in the combined final audit;
  - stale-chain scan and remaining-worker-file listing passed: production now
    contains only `server/worker/inferWorker.ts`, while `src/sim` contains only
    `SimCore.ts` and its test.
- Repository state at completion:
  - root: `C:/Users/jlow8/source/repos/slither_neuroevo`;
  - branch: `exclusive-server-mode-refactor`;
  - current implementation HEAD and last fully verified committed HEAD:
    `58f85b009dbc461702e7f571a4ef0ab964b4a134`;
  - upstream divergence: `0 0`;
  - exact modified, deleted, untracked, status-only, and staged paths are listed
    under "Live execution status". `AGENTS.md` remains untouched and
    content-identical to HEAD despite its preserved status marker;
  - no Phase 4 file is staged, committed, or pushed.
- Last successful acceptance gate: Phase 4 canonical MT inference with stable
  recurrent state. Current blocker: none. Await explicit owner direction before
  Phase 5.

### 2026-07-22 — Phase 5 start

- Re-read this authoritative plan completely, then re-read the repository-root
  `AGENTS.md`. The stale-documentation warnings in this plan take precedence;
  the superseded native plan and archive were not consulted.
- Verified repository root `C:/Users/jlow8/source/repos/slither_neuroevo`,
  branch `exclusive-server-mode-refactor`, HEAD
  `7ca46faeaf371ee4d20fcb9a5b60e93cbdf70a57`, and upstream divergence `0 0`.
- Verified that the owner-approved Phase 4 commit is present and pushed. The
  starting worktree contained only the preserved content-identical `AGENTS.md`
  status artifact: its worktree and HEAD blob IDs both equal
  `b7c033c5de793219e590a8382befadb417d77915`, and its diff is empty.
- Phase 5 is authorized and in progress. The first action is a current-source
  audit of every score-marker read/write and every external, serial, and pooled
  sensor-delivery path, followed by the smallest direct pre-change sensor/world
  baseline. Phase 6 has not started.
- The audit confirmed the plan's source assumptions. `Snake.prepareForStep`
  overwrites `prevPointsScore` immediately before adding survival score;
  `buildSensors` then reads that marker. External publication and neural
  control call the pure builder separately, while scripted baseline-bot probes
  also call it but are not delivered score-delta observations.
- The implementation seam is therefore one stateful `Snake.sampleSensors`
  boundary layered over pure `computeSensors`. First-sample behavior is change
  since snake construction: an immediate sample is zero, while score accrued
  before the first delivery is included. Baseline-bot strategy probes remain
  pure and do not advance the delivery marker.
- Focused pre-change command:
  `node .\node_modules\vitest\vitest.mjs run src\sensors.test.ts
  src\snake.test.ts src\recoveryPhase1.world.test.ts
  server\controllerRegistry.test.ts --reporter=dot`; result: 4 files and 31
  tests passed.
- Implemented `pointsAtLastSensorSample` as the authoritative observation
  boundary. `prepareForStep` now adds survival reward without touching it;
  pure `computeSensors` only reads it; `sampleSensors` builds the vector and
  commits the current score only after construction succeeds.
- Routed external publication plus serial and pooled neural inference through
  `sampleSensors`. The legacy one-snake neural update uses the same boundary.
  The three baseline-bot strategy probes remain pure because they are not
  delivered neural/external observations and must not consume accumulated
  score delta.
- Added `src/recoveryPhase5.sensors.test.ts` for first delivery, pure repeated
  construction, survival, food, kill, boost cost, skipped control cadence,
  external/serial/pooled agreement, and source/API wording equality. The first
  focused run passed 56 tests and exposed one invalid boost fixture: the snake
  was exactly at the minimum-length guard and correctly spent nothing. Making
  only that fixture boost-eligible produced a clean 57-test rerun.
- Renamed the marker in the canonical authoritative digest and advanced its
  schema from v2 to v3 because both the state path and its future-observation
  semantics changed. The deterministic regression suite passes with the new
  schema.
- Focused Phase 5 command over the new suite plus sensor, snake, canonical
  step, digest, and deterministic regressions passed 6 files and 57 tests.
  Strict TypeScript and focused ESLint also pass. Static scans find no
  `prevPointsScore` reference and no delivered production path that bypasses
  `sampleSensors`.
- Broad regression verification is active. Phase 6 has not started.
- Current blocker: none. Last successful acceptance gate remains Phase 4
  canonical MT inference with stable recurrent state.

### 2026-07-22 — Phase 5 completion handoff

- Completed every Phase 5 detailed-checklist, required-test, and acceptance
  item. Phase 6 has not started, and no Phase 6 production behavior changed.
- Renamed the old tick-oriented marker to `pointsAtLastSensorSample` and
  removed its overwrite from `prepareForStep`. Survival, food, kill rewards,
  and boost costs now remain accumulated until a delivered observation
  consumes them.
- Kept `computeSensors` pure. The new `sampleSensors` method constructs the
  vector first and then commits the current score exactly once. A failed sensor
  build therefore does not advance the observation boundary.
- Routed external controller publication, serial neural control, pooled neural
  control, and the legacy one-snake neural update through the same sampling
  method. Scripted baseline-bot strategy probes intentionally retain pure
  `computeSensors` calls because they are not delivered neural or external
  observations and do not consume the delta channel.
- Defined the first observation as change since construction. Because a snake
  initializes its marker from its initial score, an immediate sample is zero;
  any survival or other score change accrued before the first delivery appears
  in that delivery. When neural cadence skips fixed steps, all intervening
  changes accumulate until the next due sample.
- Added one exact public description in `src/protocol/sensors.ts` and mirrored
  it in `docs/API-instructions.md`. The focused suite normalizes Markdown
  layout and asserts the wording remains equal.
- Advanced the authoritative World digest from schema v2 to v3 and renamed its
  marker path because the captured state now has an observation-boundary name
  and meaning. Existing deterministic replay and first-divergence coverage
  passes with the new schema.
- Added `src/recoveryPhase5.sensors.test.ts`. It proves first-sample behavior,
  pure repeated construction, single-consumption survival, food and kill
  rewards, negative boost spending, skipped-cadence accumulation,
  external/serial/pooled equality, and protocol/API documentation alignment.
- Verification commands and results:
  - pre-change focused baseline:
    `node .\node_modules\vitest\vitest.mjs run src\sensors.test.ts
    src\snake.test.ts src\recoveryPhase1.world.test.ts
    server\controllerRegistry.test.ts --reporter=dot` passed 4 files and 31
    tests;
  - the first focused Phase 5 run passed 56 tests and failed only the new boost
    fixture because its snake was correctly at the minimum-length no-boost
    guard. Making that fixture boost-eligible without changing production code
    resolved the test;
  - final focused Phase 5:
    `node .\node_modules\vitest\vitest.mjs run
    src\recoveryPhase5.sensors.test.ts src\sensors.test.ts src\snake.test.ts
    src\recoveryPhase1.world.test.ts server\authoritativeWorldDigest.test.ts
    server\recoveryPhase2.determinism.test.ts --reporter=dot` passed 6 files
    and 57 tests;
  - strict TypeScript:
    `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`
    passed;
  - repository-wide ESLint:
    `node .\node_modules\eslint\bin\eslint.js .` passed;
  - full JavaScript suite:
    `node .\node_modules\vitest\vitest.mjs run --reporter=dot` passed all 242
    tests across 52 files;
  - client build: `node .\node_modules\vite\bin\vite.js build` passed with 29
    modules transformed and the existing `node:module`
    browser-externalization warning from `nativeBridge`;
  - static scans found no `prevPointsScore` reference and no delivered World
    path that bypasses `sampleSensors`; only the explicitly pure baseline-bot
    probes and `sampleSensors`'s internal builder call remain;
  - `git diff --check` passed with only existing LF-to-CRLF working-copy
    warnings. Native/Rust source was unchanged, so the prior three-test release
    result remains the last Rust result and was not rerun.
- Repository state at completion:
  - root: `C:/Users/jlow8/source/repos/slither_neuroevo`;
  - branch: `exclusive-server-mode-refactor`;
  - current implementation HEAD and last fully verified committed HEAD:
    `7ca46faeaf371ee4d20fcb9a5b60e93cbdf70a57`;
  - upstream divergence: `0 0`;
  - the exact modified, untracked, status-only, and staged paths are listed
    under "Live execution status"; nothing is staged, committed, or pushed.
- Fixed registered defects `SNS-001` and `SNS-002`. No new production defect
  or blocker was discovered. Last successful acceptance gate: Phase 5
  score-delta sensor semantics.

### 2026-07-22 — Phase 6 start

- Verified repository root `C:/Users/jlow8/source/repos/slither_neuroevo`,
  branch `exclusive-server-mode-refactor`, new HEAD
  `957b76f3734de3f634f12bc921fbaed52a62bb2a`, and upstream divergence `0 0`.
- Verified the owner-approved Phase 5 commit is present and pushed. The
  starting worktree contained only the preserved content-identical `AGENTS.md`
  status artifact: its worktree and HEAD blob IDs both equal
  `b7c033c5de793219e590a8382befadb417d77915`, and its diff is empty.
- The authoritative plan and repository instructions were read before this
  phase transition; the superseded native plan and archive remain outside the
  implementation source of truth.
- Phase 6 is authorized and in progress. The first action is a current-source
  audit of shared setting metadata, Protocol 1 validation/handshake behavior,
  client transport and UI bindings, server command-boundary ordering, config
  identity, and God Mode routing, followed by the smallest relevant pre-change
  baseline. Phase 7 has not started.
- Current blocker: none. Last successful acceptance gate remains Phase 5
  score-delta sensor semantics.

### 2026-07-22 — Phase 6 implementation and focused verification

- Pre-change characterization passed 47 tests across 9 files with the direct
  focused command covering settings, WebSocket transport, protocol,
  integration/system, `main.ts`, baseline bots, `SimCore`, and `World`.
- The source audit confirmed the planned Protocol 1/browser-only control gaps
  and found one additional settings-contract defect: the UI and snapshot
  coercion still advertised v2/legacy sensor layouts although v3 is the only
  valid runtime contract. The obsolete UI control is removed, and all legacy
  numeric markers now normalize to v3 rather than writing stale labels.
- Extracted pure setting definitions from the DOM module, including numeric
  type/range/integer/live/reset metadata. Collision cell size is reset-only;
  baseline respawn delay has an explicit live cached-state method.
- Implemented strict Protocol 2 shapes for live settings, God Mode, New Run,
  authoritative results, revision/hash/settings/inference welcome state, and
  explicit Protocol 1 incompatibility reporting. New Run remains explicitly
  unavailable and does not mutate identity before Phase 7 durability.
- Added a total-order server command queue drained by a new pre-step `SimCore`
  hook. Atomic setting batches normalize before application, increment one
  revision only after success, compute a canonical versioned hash, and
  broadcast one authoritative patch to every joined UI. A test-controlled
  inference wait proves commands arriving mid-inference wait for step 2 rather
  than mutating step 1.
- Added normal-death God Mode kill and whole-body bounded translation with
  collision-grid rebuild. Overflowing but finite coordinates are rejected.
  The serialized alive set, pellet drops, equal point deltas, and post-move
  spatial query are covered.
- Extracted slider debounce/coalescing and drag throttling from `main.ts` into
  `src/net/authoritativeControls.ts`. Browser `CFG` is now updated from welcome
  state or server results/broadcasts, sim speed uses the same live path, and
  the God Mode log is result-driven. Final mouse-up coordinates always send.
- Verification so far:
  - strict TypeScript passed with
    `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`;
  - the first focused run exposed only test-environment native setup and a real
    finite-coordinate overflow hole in the new bounds math; the fixture was
    switched to the explicit JS backend, and production math now rejects
    non-finite intermediate values;
  - the corrected focused command passed 66 tests across 13 files, including
    shared metadata, extracted client behavior, Protocol 2 validation,
    canonical hashing, boundary ordering, multi-client convergence, sim speed,
    God Mode, WebSocket integration, system lifecycle, and main startup.
- Current blocker: none. Phase 6 full lint/build/test and acceptance review are
  pending. Last fully verified HEAD and last successful phase acceptance gate
  remain `957b76f` and Phase 5 respectively; no Phase 6 file is staged,
  committed, or pushed.

### 2026-07-22 — Phase 6 completion handoff

- Completed and verified every Phase 6 detailed-checklist, required-test, and
  acceptance-gate item. Phase 7 has not started.
- Current repository state:
  - root: `C:/Users/jlow8/source/repos/slither_neuroevo`;
  - branch: `exclusive-server-mode-refactor`;
  - current implementation HEAD and last fully verified committed HEAD:
    `957b76f3734de3f634f12bc921fbaed52a62bb2a`;
  - upstream divergence: `0 0`;
  - staged paths: none;
  - deleted tracked paths: none;
  - no Phase 6 commit or push was performed.
- Exact dirty-file summary:
  - content-modified tracked paths:
    `docs/todo/project-recovery-plan.md`, `server/acceptance.test.ts`,
    `server/hash.ts`, `server/httpApi.ts`, `server/index.ts`,
    `server/integration.test.ts`, `server/protocol.test.ts`,
    `server/protocol.ts`, `server/security.test.ts`,
    `server/settingsSnapshot.ts`, `server/simServer.ts`,
    `server/system.test.ts`, `server/wsHub.ts`,
    `src/bots/baselineBots.test.ts`, `src/bots/baselineBots.ts`,
    `src/main.ts`, `src/net/wsClient.test.ts`, `src/net/wsClient.ts`,
    `src/protocol/settings.ts`, `src/settings.test.ts`, `src/settings.ts`,
    `src/sim/SimCore.ts`, and `src/world.ts`;
  - untracked paths: `server/configIdentity.ts`, `server/hash.test.ts`,
    `server/recoveryPhase6.controls.test.ts`,
    `src/net/authoritativeControls.test.ts`,
    `src/net/authoritativeControls.ts`,
    `src/protocol/settingDefinitions.ts`, and
    `src/protocol/settings.test.ts`;
  - status-only artifact: Git reports `AGENTS.md` modified, but its diff is
    empty and its worktree/HEAD blob IDs remain identical. Per owner direction,
    it was left untouched.
- Implemented the shared pure setting schema and atomic live normalizer,
  including bounded core values, explicit derived-state metadata, reset-only
  collision cell size, live baseline-respawn cache updates, and v3-only sensor
  layout normalization.
- Implemented strict Protocol 2 validation and explicit Protocol 1 rejection,
  settings/God Mode/New Run transport, dynamic welcome/health/save/export
  identity, and one monotonic pre-step command queue. New Run is deliberately
  rejected without mutation until Phase 7 can durably checkpoint run start.
- Implemented authoritative browser control flow, per-client/per-path slider
  coalescing, drag throttling with a guaranteed final release position,
  result-driven logs, and server-normalized browser state. Existing
  selection/frame parsing was not changed.
- Implemented normal-death God Mode kill and bounded equal-delta whole-body
  translation with finite-intermediate validation and spatial-grid rebuild.
  Tests verify pellet drops, serialized alive/head state, body coordinates,
  bounds, and collision queries.
- Final verification commands and results:
  - `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false`
    passed;
  - `node .\node_modules\eslint\bin\eslint.js .` passed repository-wide;
  - the focused Phase 6 Vitest command recorded in live status passed 68 tests
    across 12 files;
  - `node .\node_modules\vitest\vitest.mjs run --reporter=dot` passed all 264
    tests across 56 files;
  - `node .\node_modules\vite\bin\vite.js build` passed with the existing
    `node:module` browser-externalization warning from `nativeBridge`;
  - `git diff --check` passed with only LF-to-CRLF working-copy warnings;
  - native/Rust source was unchanged, so the prior three passing release tests
    remain the last Rust result and were not rerun.
- The first full regression run exposed an incomplete Phase 0 startup test
  double after welcome diagnostics began reading architecture information. The
  startup diagnostic was made tolerant of the established mock while retaining
  accurate real-world values. Focused verification then added the serialized
  moved-head assertion and completed with 68 passing tests.
- Newly discovered defect `UI-006` was registered and fixed: stale v2/legacy
  sensor-layout choices are no longer advertised or persisted. Registered
  Phase 6 defects `UI-001` through `UI-005` and `CFG-001` through `CFG-002` are
  also repaired by the verified implementation. No new blocker remains.
- Last successful acceptance gate: Phase 6 server-authoritative live controls
  and God Mode. Await explicit owner direction before Phase 7.

### 2026-07-22 — Phase 6 publish and Phase 7 start

- Inspected the complete Phase 6 name/status and diff summary, staged only the
  30 recorded Phase 6 paths, and excluded the content-identical `AGENTS.md`
  status artifact. `git diff --cached --check` passed before commit.
- Created commit `24b587a0cc541dcee7068d22d86e3a4946777c34` with subject
  `feat: restore server-authoritative controls and God Mode` and the
  owner-reviewed detailed body, including the Protocol 2 breaking-change
  notice. Pushed it to `origin/exclusive-server-mode-refactor`.
- Verified repository root `C:/Users/jlow8/source/repos/slither_neuroevo`,
  branch `exclusive-server-mode-refactor`, HEAD `24b587a`, and upstream
  divergence `0 0` after the push.
- Re-read this authoritative plan completely and then the repository-root
  `AGENTS.md`. This plan's stale-documentation warnings take precedence; the
  superseded native plan and archive were not consulted.
- Preserved the only pre-Phase-7 worktree artifact: Git reports `AGENTS.md`
  modified, but its worktree and HEAD blob IDs remain identical at
  `b7c033c5de793219e590a8382befadb417d77915` and its diff is empty.
- Phase 7 is authorized and in progress. The first action is a current-source
  audit of persistence schema migration, typed versus JSON models, legacy blob
  limits, generation-boundary capture, startup selection/bootstrap, required
  checkpoint durability, import/export, and HTTP streaming, followed by the
  smallest direct persistence baseline. Phase 8 has not started.
- The audit confirms the registered Phase 7 defects still match current source:
  `saveSnapshot` maps every typed weight buffer to JSON, accumulates all genome
  buffers, calls `Buffer.concat`, and gzip-compresses the combined population;
  the legacy reader gunzips without an output cap and catches corruption;
  automatic save runs only after spawn/pellets/focus; startup applies saved
  settings but never the population/seed/RNG/allocator/generation; and HTTP
  export calls one whole-payload `JSON.stringify`.
- The existing Phase 2 boundary hook is correctly placed after population
  assignment and transient-state clearing but before spawn, pellets, focus,
  sensors, and inference. Phase 7 can extend that seam with simulation-step,
  run/config, graph, history, and durability metadata rather than inventing a
  second generation path.
- Focused pre-change command:
  `node .\node_modules\vitest\vitest.mjs run
  server\persistence.test.ts
  server\recoveryPhase0.characterization.test.ts
  server\recoveryPhase0Startup.characterization.test.ts
  server\recoveryPhase2.determinism.test.ts --reporter=dot`; result: 4 files
  and 24 tests passed, including the two Phase 7-expiring characterizations.
- No source discrepancy requires a new defect ID or owner consultation. The
  implementation will use the planned SQLite child-row format and existing
  JSON export extension; no new dependency or user-visible file format is
  required.
- Current blocker: none. Last successful acceptance gate remains Phase 6
  server-authoritative live controls and God Mode.

### 2026-07-22 — Phase 7 completion handoff

- Completed every Phase 7 detailed-checklist, required-test, and acceptance-gate
  item. Phase 8 has not started, and no Phase 8 production or harness design was
  imported into this pass.
- Added separate internal checkpoint types and builders in
  `server/snapshotTypes.ts` and `server/checkpoint.ts`. Automatic and required
  saves iterate typed genomes directly and do not call the population JSON DTO
  path or create one whole-population byte buffer.
- Migrated persistence idempotently to versioned parent metadata plus one
  `snapshot_genomes` child row per dense population slot. Current writes use one
  SQLite transaction, explicit little-endian Float32 encoding, SHA-256 checksums,
  foreign keys, and one reusable genome-sized scratch buffer. Reads validate
  version, boundary, population count, dense slots, architecture key, brain
  type, parameter count, byte length, checksum, and finite values before
  constructing typed genomes.
- Preserved legacy `genomes_blob` as a read-only compatibility path. Legacy
  loading now reports the snapshot ID and compressed/population size context,
  applies compressed and bounded-output limits, and warns about the historical
  combined allocation. New writes explicitly store `genomes_blob` as null and
  never recreate the legacy format.
- Split generation completion at the authoritative Phase 2 seam: the evolved
  population, generation number, committed simulation step, run identity,
  config/graph/settings identity, RNG streams, deterministic allocator state,
  and zero recurrent-state condition are checkpointed before spawn, pellets,
  focus selection, sensors, or inference. Resume bootstraps `World` directly at
  that boundary without constructing and discarding a random population.
- Normal startup now defaults to `--resume latest`; `--fresh`,
  `--resume latest`, and `--resume <snapshot-id>` are explicit. A selected
  corrupt or incompatible candidate fails with its snapshot ID/reason and valid
  alternatives instead of silently skipping evolution. Experiment overrides
  that conflict with resume require `--fresh`; operational bind/database/backend
  options remain startup configuration.
- Automatic checkpointing now defaults to every generation. Setting the
  interval to zero remains a diagnostic opt-out and explicitly warns that crash
  resume can lose progress. Fresh startup, Reset, and New Run always commit a
  generation-one run-start checkpoint before switching or advertising the new
  identity. A
  required New Run failure retains the prior identity, leaves the previous
  durable snapshot current, returns a negative protocol result, and enters the
  defined faulted state.
- `/api/save` creates a typed, non-resumable population-export snapshot; normal
  resume selection ignores that boundary kind while explicit selection rejects
  it clearly. `/api/export/latest` writes JSON incrementally with response
  backpressure and no forced content length, emitting one genome at a time.
  Save/export dependencies query the live seed and config hash. Import reports
  that its seed is metadata-only and does not silently alter the active run.
- Added `scripts/measure-persistence.ts`. An initial measurement exposed
  retained per-genome temporary buffers in the draft implementation; replacing
  them with one exactly sized scratch buffer flattened RSS while payload doubled
  from 55,044,000 to 110,088,000 bytes. Final 500/1,000-genome measurements and
  timings are recorded in live status. The observed generation-boundary cost
  does not justify the complexity of a background persistence worker.
- Converted the Phase 0 persistence/startup characterizations into positive
  resume contracts, expanded `server/persistence.test.ts` to 18 strict format
  tests, and added 7 exact durability/reconstruction tests in
  `server/recoveryPhase7.persistence.test.ts`. Server integration tests now
  verify live seed/hash behavior and explicit import-seed disposition.
- Newly discovered and repaired `PER-006`: Boolean `brain.useMlp` was restored
  as numeric `1`, changing canonical config identity. Shared setting coercion
  now preserves its Boolean type.
- Newly discovered and repaired `TEST-005`: acceptance and security fixtures
  inherited the production SQLite path and could migrate or contaminate ignored
  local state. They now use isolated in-memory databases and explicit fresh JS
  startup. An early run opened the ignored local `data/slither.db`; it was not
  deleted, reverted, or added to Git.
- Newly discovered and repaired `TEST-006`: two Phase 4 server tests invoked
  New Run without persistence. The full suite correctly failed under the new
  durable-before-switch contract; the shared fixture now owns an in-memory
  SQLite store and still verifies native-pool rebuild and fault recovery.
- A legacy-database integration run exposed migration ordering in the draft:
  the resumable index was initially created before older parent tables had the
  new boundary column. Migration now adds all columns before creating the index.
  This draft-only issue was fixed before the acceptance gate and is not a
  remaining blocker.
- Final verification passed: 57 Vitest files/290 tests, strict TypeScript,
  repository-wide ESLint, the Vite production build, and `git diff --check`.
  Focused persistence, exact-resume, server integration, and corrected Phase 4
  fixture results are recorded verbatim in live status. Native/Rust code was
  unchanged, so its prior three passing release tests were not rerun.
- Added a narrow README clarification: restart snapshots are exact
  generation-boundary population checkpoints, not full mid-tick saves, and
  imported seeds remain metadata under the active run identity. Broader stale
  documentation remains assigned to Phase 9.
- Current implementation HEAD and last fully verified committed HEAD are both
  `24b587a0cc541dcee7068d22d86e3a4946777c34`. The fully verified Phase 7
  worktree is uncommitted, unstaged, and has the exact file list recorded in
  live status. Branch/upstream divergence is `0 0`.
- Current blocker: none. Last successful acceptance gate: Phase 7 bounded-
  memory persistence and actual resume. Await explicit owner direction before
  Phase 8.

### 2026-07-22 — Phase 8 completion handoff

- Completed every Phase 8 detailed-checklist and CI-acceptance item without
  restoring deleted parity tests wholesale or changing Phase 9 production and
  cleanup scope. Phase 9 has not started.
- Current-source audit found that required-native behavior was already stronger
  than the old defect description: the native bridge suite loads the addon in
  `beforeAll`, asserts readiness, and has a subprocess test that proves a
  missing addon exits with actionable build instructions. Existing Phase 4
  tests already cover MLP/GRU/LSTM/RRU across JS/native and 1/2/4 worker
  requests, stable recurrent ownership, complete recurrent histories, pool and
  weight epochs, reset/fault paths, deterministic same-backend results, and
  per-worker source build identity. Those contracts were retained and grouped;
  the deleted broad parity tests were not copied back.
- Repaired the remaining `TEST-002` paths. Acceptance, integration, system,
  security, and the network-dependent lifecycle cases now propagate bind/start
  errors normally. `server/test/networkSuites.ts` permits skipping only when
  `SLITHER_SKIP_NETWORK_TESTS=1` and emits a visible warning; mixed suites keep
  their non-network tests active. A focused opt-out run proved 2 tests ran and
  exactly 3 bind tests skipped.
- Replaced the 325-line main-entry fake DOM and obsolete Worker stub with a
  small generic startup smoke that imports the real entry module and asserts
  the resolved `ws://localhost:5174` connection. Extracted authoritative
  control and WebSocket-client behavior remains in focused module tests.
- Replaced suffix discovery with `scripts/test-categories.ts`, an explicit,
  non-overlapping primary manifest for unit/component/integration/system/
  acceptance/regression/performance/security plus an additive
  `native-required` overlay. `scripts/test-categories.test.ts` recursively
  discovers repository tests and fails on omissions or duplicate primary
  assignments. The runner now invokes the Vitest ES module through
  `process.execPath`, forwards CLI arguments, and works on Windows without the
  former `.cmd` `EINVAL` failure.
- Added `scripts/ci-contract.test.ts` and updated CI so each Ubuntu/Windows,
  Node 22/24 matrix job builds native exactly once, verifies source identity,
  runs Rust unit tests and the required native+MT overlay in the same job, then
  runs every explicit JavaScript layer, Vite, TypeScript, and ESLint. The
  separate Rust job still enforces formatting and clippy. No CI network opt-out
  is set. Performance remains visible but informational while cross-runner
  history accumulates.
- Strengthened runtime diagnostics in the native MT `SimServer` integration:
  requested and active backend/MT, requested and active worker counts,
  pool/weight epochs, graph key, seed, addon readiness, source-derived build ID,
  and every worker's matching build ID are asserted from the runtime that
  executed.
- Added a real WebSocket-to-binary-frame integration contract. It waits for a
  post-reset two-snake frame, applies authoritative camera settings, observes
  the changed serialized zoom, moves a selected snake and observes its new
  serialized head, then kills it and observes the ID disappear. This closes the
  live-settings/God-Mode system path without faking the hub.
- Newly discovered and repaired `TEST-007`: the older config/HTTP integration
  sent settings after an arbitrary 100 ms reset delay. The first concurrent
  full-manifest run exposed the race with one timeout (59 files/297 tests
  passed). The test now waits for the protocol-visible post-reset frame before
  sending settings; its focused rerun and the next concurrent full manifest
  both pass.
- Expanded `TEST-004` with the observed Windows failure. Before repair,
  `node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts unit` failed at
  `spawnSync ... vitest.cmd EINVAL`; after repair, unit passed 32 files/131
  tests and the complete manifest passed 60 files/298 tests.
- Performance tests now print auditable measurements and their broad budgets.
  Isolated and concurrent results are recorded in live status. The native
  thresholds are over 68 times the observed concurrent duration, leaving ample
  shared-runner headroom while still flagging a disabled or catastrophically
  regressed accelerator; CI keeps the step informational until matrix history
  is stable.
- Rebuilt the addon once through napi-rs's compiled CLI, verified the exact
  source build identity, and passed all 3 Rust release tests, rustfmt, and
  clippy. The raw development launcher failed under Node 24's built-in type
  stripping, and the machine's roaming npm/npx shims are missing their CLI
  modules; neither wrapper was retried after the direct supported entry points
  succeeded.
- Final automated verification passed: the 7-file/45-test required-native
  overlay, all 60 files/298 JavaScript tests, strict TypeScript,
  repository-wide ESLint, Vite production build, Rust test/fmt/clippy, and diff
  hygiene. Exact commands and layer counts are in live status.
- Local launch smoke independently started the normal Vite client and a fresh
  native+MT server with two workers. Health reported the active seed,
  requested/active native backend, requested MT, two active workers, and the
  source addon ID. In-app browser control then failed before navigation because
  its own sandbox could not read `C:\Users\jlow8\AppData`; no repository
  failure was observed. Both exact temporary processes were stopped and Phase
  9's final manual-QA checkboxes remain untouched.
- Added a narrow README QA note for the changed explicit suite commands, as
  required when the Phase 8 command surface changes. Broader README/AGENTS/API
  reconciliation remains Phase 9 work. `AGENTS.md` was not edited; its empty-
  diff status artifact still has matching worktree/HEAD blob ID
  `b7c033c5de793219e590a8382befadb417d77915`.
- Current implementation HEAD and last fully verified committed HEAD remain
  `308c6f0dd91eca8091bc75dcf08ca87904da2d50`. The verified Phase 8 worktree is
  uncommitted, unstaged, and has the exact file list in live status; branch and
  upstream remain synchronized at divergence `0 0`.
- Current blocker: none. Last successful acceptance gate: Phase 8 test and CI
  reconstruction. Await owner review; do not commit, push, or start Phase 9
  without explicit direction.

## Verification command reference

Use direct binaries when the npm/PowerShell wrapper obscures completion:

```powershell
node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false
node .\node_modules\eslint\bin\eslint.js .
node .\node_modules\vite\bin\vite.js build
cargo test --manifest-path native\Cargo.toml --release
node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts all --reporter=dot
node .\node_modules\tsx\dist\cli.mjs scripts\run-tests.ts native-required --reporter=dot
git status --short --branch
```

When the npm shim cannot start, run the native build from `native/` with:

```powershell
node .\node_modules\@napi-rs\cli\dist\cli.js build --platform --release
```

If a command is changed during Phase 8, update this reference and the README or
AGENTS workflow documentation together.
