# Stage 6 retained Rust running-authority loop

Date: 2026-08-31
Approved plan revision: `2026-07-29-draft-4`

## Outcome and scope

An activated, durable Rust authority can now move into one retained
`RunningAuthorityLoop`. The loop owns that authority together with one
`FixedStepScheduler` and one `RunningStepCoordinator` across repeated service
boundaries. Each ready service call can publish at most one complete fixed
step, even when scheduler debt contains several steps. Optional frame-v1
packing writes directly into caller-owned reusable storage only after the
matching authority and scheduler ticket publish.

The loop is a Rust core handoff, not a new N-API step call. It is not yet
installed in the existing background thread/queue spine, and no Node,
WebSocket, browser, LAN or RL route invokes it. This slice therefore does not
claim a continuous production loop, normal Rust startup, Stage 6A completion
or production cutover.

## Current-source proof

- `PendingRunStartTransition::into_running_loop` requires already-published
  step-zero authority and rejects a transition whose experimental one-shot
  scheduler attempt has begun. The two owners are mutually exclusive.
- Loop construction is two-phase. `RunningAuthorityLoop::prepare` performs
  every fallible scheduler/coordinator check while the prior transition still
  owns authority. On failure, `RunStartLoopHandoffError` returns the exact
  unchanged boxed transition. Only the subsequent infallible
  `from_prepared` call moves authority.
- A ready service boundary asks the retained scheduler for readiness, prepares
  one exact ticket, advances the complete running-step coordinator once and
  commits that same ticket only after immediate authority publication. A
  backlog cannot turn one service call into multiple world steps.
- External-delivery and generation-transition branches retain the exact
  scheduler ticket and staged coordinator work. Repeated blocked service calls
  reborrow that work without servicing the scheduler, changing authority,
  packing a requested frame or rerunning evolution.
- Any scheduler, coordinator, retained-state or post-publication frame error
  permanently faults the loop. A later call returns `AlreadyFaulted`, so a
  partially completed service cannot be retried as a hidden second step.
- The optional frame is packed after exact authority publication and scheduler
  commit. Its existing preflight leaves caller storage unchanged on error. A
  focused test demonstrates that a frame error retains the already-committed
  step once, leaves sentinel bytes unchanged and faults the loop.
- The loop exposes bounded scalar diagnostics and immutable borrowed pending
  batches. It exposes no mutable world, population, recurrent state, archive
  or checkpoint payload.
- Searches of the final source found no N-API, Node or browser export of the
  loop, handoff or service method. The normal production addon surface is
  unchanged.

## Evidence provenance

### Git-history evidence

The immediately preceding first Rust-scheduled-frame slice is commit
`729f04a76a461be6c4556c1e15eafea11f989169`, verified locally and on the
existing GitHub `exclusive-server-mode-refactor` branch before this work. That
identity proves the starting history only; the behavior below is supported by
the current source and newly reproduced checks.

### Prior planning measurements and retained artifacts

No prior timing or performance measurement is used to justify this change.
The existing fixed-P0 Rust construction, scheduler, running-step coordinator,
generation-transition and frame-v1 packer are retained implementation inputs,
not newly measured production throughput. The TypeScript runtime remains the
selected reference/test oracle and was not introduced as a fallback.

### Derived arithmetic

The terminal-retention fixture uses a one-second fixed delta and an
eight-second generation duration, so its eighth ticket derives successor
generation 2 at completed step 8 while published source authority remains at
generation 1, completed step 7. The separate backlog fixture services a wall
boundary derived as eight times the scheduler-reported one-step boundary; the
reproduced scheduler reports more than one due step, while three separate
service calls publish only steps 1, 2 and 3.

The short-generation fixture raises only its disposable state-admission
ceiling to 4 GiB so current and successor states can coexist during the real
terminal transition. This is a test input, not a production default, capacity
selection or performance result.

### Newly reproduced measurements and validation

All checkpoint files and SQLite databases used by these checks were created in
unique operating-system temporary roots. The owner database, saves, managed
checkpoint directory and Oxygen deployment were not accessed.

| Command or check | Newly reproduced result |
|---|---|
| `cargo test --manifest-path native\\Cargo.toml fresh_run -- --nocapture` | 10 passed; 390 filtered out. The three retained-loop tests cover backlog, recoverable construction failure, reusable frame storage, permanent faulting, post-publication frame failure and one retained terminal candidate. |
| `cargo test --manifest-path native\\Cargo.toml --release` | 400 library tests passed; the compile-fail doctest passed. |
| `cargo test --manifest-path native\\Cargo.toml --release --all-targets --features engine-test-hooks --quiet` | 418 library tests passed; three enabled binary tests and the compile-fail doctest passed. |
| `cargo test --manifest-path native\\Cargo.toml --release engine::generation::tests -- --nocapture` | 12 passed; 388 filtered out. |
| Broad `server/rustEngine` Vitest matrix against the rebuilt normal addon | 68 passed; 9 documented platform/feature skips across 10 files. |
| `scripts\\stage3\\checkpoint-handoff-evidence.ts` isolated feature-gated addon/worker run | 6 real handoff assertions passed; the script restored the normal production addon afterward. |
| `cargo check --manifest-path native\\Cargo.toml --all-targets --features engine-test-hooks` | Passed. |
| `cargo clippy --manifest-path native\\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed with warnings denied. |
| `cargo fmt --manifest-path native\\Cargo.toml -- --check` | Passed. |
| `node .\\node_modules\\typescript\\bin\\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for `server/rustEngine` and the isolated handoff runner | Passed. |
| `git diff --check` | Passed. |

The rebuilt normal production addon and independent Node calculation agree on
native source SHA-256
`24aabb8da47ea011891a675de9c656a0ea3d06b3abcd361c4d8322dcb52ae6d4`.
The identity covers 68 selected files, 3,509,506 canonical bytes and 9,568
accounted path bytes. The addon reports target `x86_64-pc-windows-msvc`,
profile `release`, build class `production` and build-contract SHA-256
`sha256:3b592e686658f6c9e452b4cd898f12700c455077ecf4da9c2635cbdb342b2cfa`.

## Independent review

One reused independent strong reviewer inspected the final five-file Rust
source/test diff read-only and changed no files. The reviewer found no blocker,
P1, P2 or P3 defect. It directly checked two-phase authority transfer,
one-shot/loop exclusion, exact ticket/publication correlation, one step per
ready boundary, blocked-work retention, post-publication fault containment,
unchanged caller bytes on frame failure, retained generation identity and the
absence of a new N-API/Node/browser surface. Its focused Rust tests, rustfmt,
feature check and diff check passed. No review turn was blocked or wasted.

The reviewer retained one implementation boundary rather than a defect: this
loop does not yet expose external-delivery or generation-transition completion
and resume methods. The external retained branch also lacks a dedicated
loop-level test. Those future authority-transition paths require their own
high-impact review when implemented.

## Limits and open gates

- A production Rust background thread still needs to own this loop, read its
  own monotonic clock, drain commands/actions and call the service boundary.
  Current tests supply wall-clock values directly to the Rust method.
- Reliable external-event delivery resolution, generation checkpoint
  publication/acknowledgement, successor reassignment completion, final
  authority swap and scheduler rebind are not forwarded through this loop yet.
  The lower coordinator and feature-gated handoff fixture retain their existing
  independent proofs.
- Caller-owned `Vec` reuse is proved, but output-queue ownership,
  reuse-until-send-completes, latest-unsent replacement, WebSocket
  backpressure and welcome-state caching remain open.
- Reset, New Run, resume/recovery, browser rendering and controls, trusted-LAN
  transport, the separate RL trainer, Debian/Oxygen execution, retention,
  export/import, compatibility and sustained-growth gates remain open.
- No Stage 6 plan checkbox, Stage 6A evidence gate or production cutover is
  claimed.
