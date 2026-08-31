# Stage 6 background Rust running-authority thread checkpoint

Date: 2026-08-31
Approved plan revision: `2026-07-29-draft-4`

## Outcome and scope

The existing pure-Rust `EngineRuntime` background coordinator can now retain
and run one activated `RunningAuthorityLoop`. The authority thread owns its
monotonic `Instant`, waits on the bounded inbound queue without busy polling,
atomically drains the next command boundary and services the retained
fixed-step loop. JavaScript supplies neither per-step clock values nor
authoritative state.

This is a local Rust integration checkpoint. It deliberately adds no N-API,
Node server or browser route. The available command body remains the coarse
Stage 3 probe, frame output is not yet connected to the output queue, and
external-delivery or generation-transition blockers cannot yet be completed
through this runtime. It therefore does not establish normal Rust startup,
Stage 6A completion or production cutover.

## Current-source proof

- `EngineRuntime::new_running_authority` validates both the bounded runtime
  configuration and a pristine retained loop. Every validation failure returns
  the exact loop to its caller rather than reconstructing or dropping
  authority.
- A valid runtime stores the loop in one private Rust slot. `start` captures
  only the slot and metrics in the thread closure; the loop is taken only from
  inside a successfully spawned thread. Thread-spawn failure therefore leaves
  the exact loop retained.
- `RunningAuthorityLoop::validate_background_start` requires wall origin zero,
  ready state, no pending scheduler work and no prior completed step or command
  boundary. The actual authority-thread root creates the `Instant`, excluding
  construction and thread-start delay from scheduler debt.
- The inbound condition variable waits for a scheduler timeout, accepted
  command batch or stop. After each wake, `drain_step_boundary` removes every
  batch accepted before one mutex-release cutoff. A command accepted after
  that cutoff belongs to the following service boundary.
- The coordinator forwards Rust-derived elapsed milliseconds to
  `RunningAuthorityLoop::service_after_command_drain` in background mode. The
  retained-loop invariant still permits at most one complete authoritative
  step per call. If scheduler debt remains, the coordinator immediately starts
  another command-drain boundary before the next step.
- External-delivery and generation-transition pending states wait indefinitely
  on the condition variable rather than spin. No completion command is claimed
  by this slice.
- Bounded atomic health exposes only loop state, generation, completed step,
  scheduler and command-boundary counts, and wait/wake counters. It does not
  expose world, population, recurrent, archive or checkpoint bytes.
- On coordinator exit, the thread moves the same loop value back into the
  private slot before completing root lifecycle handling; no reconstruction
  path exists. Test-only inspection proves that one retained owner remains
  after both orderly and faulted joins.
- The output mutex now totally orders Fault and orderly Stop. A failed stop
  wake rolls the undelivered Stop back before Fault publication, while a Stop
  already drained by a polling consumer remains the terminal result. Once
  either terminal outcome wins, reliable, discrete, stats and frame producers
  all reject later publication.
- Searches of `native/src/napi_engine.rs` and `server/` found no occurrence of
  the running-authority constructor, coordinator or health type. Test-only
  authority-slot inspection remains absent from production builds.

## Evidence provenance

### Git-history evidence

The starting commit was
`7659fef49fdd770162c2f00839dfb6c537261d67`, the retained-running-loop slice.
Before this work it was verified as the exact local and live GitHub
`exclusive-server-mode-refactor` tip with zero ahead and zero behind. That is
history evidence for the starting point only; the behavior below is supported
by current source and newly reproduced checks.

### Prior planning measurements and retained artifacts

No prior timing or performance measurement is used to justify this change.
The existing fixed-P0 constructor, durability barriers, running-step
coordinator, scheduler and retained-loop tests are implementation inputs. The
TypeScript runtime remains a selected reference/test oracle and is not an
automatic production fallback.

### Derived arithmetic

The real-thread correctness test waits for at least three completed steps and
requires at least one timed wait and one command wake. Its three-second test
deadline is only a bounded failure guard; neither the observed step count nor
the wait counters are a throughput, latency or capacity measurement.

### Newly reproduced measurements and validation

All checkpoint files and SQLite databases used by these checks were created in
unique operating-system temporary roots. The owner database, saves and managed
checkpoint directory were not accessed. The Debian `oxygen` VM was under
maintenance and was not contacted; no Debian or target-hardware result is
claimed.

| Command or check | Newly reproduced result |
|---|---|
| `cargo test --manifest-path native\\Cargo.toml engine::queues::tests -- --nocapture` | 18 passed; 389 filtered out. Includes timed wait/drain ordering, stop-wake rollback, Stop/Fault mutex ordering, terminal producer closure and wake re-arm races. |
| `cargo test --manifest-path native\\Cargo.toml engine::runtime::tests -- --nocapture` | 7 passed; 400 filtered out. The injected thread panic is caught and reported as a passing fault-path assertion. |
| `cargo test --manifest-path native\\Cargo.toml fresh_run -- --nocapture` | 12 passed; 395 filtered out. Two real fixed-P0 tests cover autonomous stepping, command wake, orderly stop, caught panic and exact loop retention. |
| `cargo test --manifest-path native\\Cargo.toml --release --quiet` | 407 library tests passed; the compile-fail doctest passed. |
| `cargo test --manifest-path native\\Cargo.toml --release --all-targets --features engine-test-hooks --quiet` | 425 library tests and three enabled target tests passed. |
| `cargo test --manifest-path native\\Cargo.toml --release engine::generation::tests -- --nocapture` | 12 passed; 395 filtered out. |
| Broad `server/rustEngine` Vitest matrix against the rebuilt normal addon | 68 passed; 9 documented platform/feature skips across 10 files. |
| `scripts\\stage3\\checkpoint-handoff-evidence.ts` isolated feature-gated addon/worker run | 6 real Rust-to-managed-file-to-SQLite-to-Rust acknowledgement assertions passed; the normal production addon was restored afterward. |
| Native source-identity unit/integration pair | 10 passed; 3 documented platform skips. The post-restore native integration assertion also passed independently. |
| `cargo check --manifest-path native\\Cargo.toml --all-targets --features engine-test-hooks` | Passed. |
| `cargo clippy --manifest-path native\\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed with warnings denied. |
| `cargo fmt --manifest-path native\\Cargo.toml -- --check` | Passed. |
| `node .\\node_modules\\typescript\\bin\\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for `server/rustEngine` and the isolated handoff runner | Passed. |
| Production-surface search and `git diff --check` | No running-authority N-API/server match; diff check passed. |

The rebuilt normal addon and the independent TypeScript implementation agree
on native source SHA-256
`251036166f694aa1de7fff677ab354e99f7704575f78ce0af53e642b4b907b9a`.
The identity covers 68 selected files, 3,550,540 canonical bytes and 9,568
accounted path bytes. The addon reports target `x86_64-pc-windows-msvc`,
profile `release`, build class `production`, Rust
`rustc 1.92.0 (ded5c06cf 2025-12-08)` and build-contract SHA-256
`sha256:3b592e686658f6c9e452b4cd898f12700c455077ecf4da9c2635cbdb342b2cfa`.

## Independent review

One existing independent strong reviewer inspected the concrete concurrency
and authority-transition diff read-only and changed no files. It found two
related defects: a lifecycle check could race a bridge fault and publish Stop
after Fault, and a failed Stop wake could leave an undelivered Stop queued
before the runtime faulted. The implementation replaced that ordering with one
output-mutex terminal state, wake-failure rollback and focused deterministic
race tests. The main-agent source audit then closed all ordinary output
producers after either terminal result, preventing a later `Started`, probe,
discrete, stats or frame event.

The reviewer reached its separate usage limit before it could inspect the
revised final diff. Per the repository policy, no unchanged-condition
replacement reviewer was created. The initial review was valuable and its
findings were reconciled against source and passing tests, but a completed
independent recheck is still open. This commit is therefore recorded as a
tested remote-backup checkpoint, not as the independently finalized authority
boundary or a passed Stage 6A gate.

## Limits and open gates

- The runtime command contract still executes only the coarse probe. Player,
  Protocol 2 RL, controller lifecycle, persistence acknowledgement and
  reassignment commands are not yet routed through this thread.
- `run_running_coordinator` currently requests no frame from the retained loop.
  Output buffer ownership, latest-unsent replacement, WebSocket backpressure
  and reuse-until-send-completes remain open.
- External-delivery and generation-transition pending work blocks safely, but
  this runtime cannot yet deliver the matching completion and resume it.
- The implementation has no production N-API or Node composition and is not
  selected by normal server startup.
- The independent final-diff recheck remains open until the existing
  reviewer's usage limit resets or another policy-valid condition changes.
- Reset, New Run, resume/recovery, browser rendering and controls, trusted-LAN
  transport, the separate RL trainer, Debian/Oxygen execution, retention,
  export/import, compatibility, corrupt-latest recovery, performance and
  sustained-growth gates remain open.
- No Stage 6A completion, normal production Rust startup or cutover is claimed.
