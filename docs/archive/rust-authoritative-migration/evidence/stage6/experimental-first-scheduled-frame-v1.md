# Stage 6 experimental first Rust-scheduled frame-v1 publication

Date: 2026-08-31
Approved plan revision: `2026-07-29-draft-4`

## Outcome and scope

The explicitly experimental fixed-P0 production-addon session can now execute
one complete Rust-scheduled authoritative step and publish the resulting
frame-v1 payload. The call is input-free: JavaScript supplies no clock,
scheduler debt, controls, world state, IDs, statistics, view values or frame
lengths. Rust derives the first service boundary, prepares and commits the
fixed-step scheduler ticket, executes the real running-step coordinator and
packs the post-step authority into the unchanged browser frame-v1 layout.

This is a bounded one-shot bridge after the already-proved managed-file,
SQLite-metadata, exact-acknowledgement, activation and initial-frame barriers.
It is not a continuous background loop, repeatable per-step N-API surface,
WebSocket/browser/LAN/RL route, normal startup path, Stage 6A completion or
production cutover.

## Current-source proof

- `PendingRunStartTransition::publish_first_scheduled_frame_v1` first requires
  published running authority and then consumes a Rust-owned one-attempt marker
  before constructing the scheduler or coordinator. Every later call rejects,
  including after an in-progress failure.
- Rust constructs `FixedStepScheduler` and `RunningStepCoordinator` from the
  retained authority and admitted work limits. The smallest positive
  whole-millisecond service boundary is derived from the admitted fixed delta
  and requested simulation speed. Exactly one prepared ticket is executed and
  committed.
- Only `RunningStepProgress::Published` is accepted. An unexpected external
  delivery or generation-transition branch returns an error without publishing
  its staged authority.
- The post-step frame is packed from the same retained authority under the same
  session mutex. The frame result exposes only owning display bytes and bounded
  scalar metadata; population, recurrent-state and archive bytes stay in Rust
  or the managed checkpoint file.
- A failure after the operation has begun is permanently retained as the
  session's bounded first fault. If a failure occurs after the authority swap,
  that step is not rolled back, but no retry or hidden second step is possible.
  This slice does not claim a recoverable continuous delivery loop.
- The N-API method accepts no arguments, shares the session's single coarse
  operation slot and additionally requires the initial frame. Generation,
  completed step, counts and lengths cross N-API only as canonical 16-digit
  U64 hexadecimal strings after checked Rust conversion.
- The TypeScript wrapper rejects unknown fields, narrowed or malformed U64
  values, the wrong completed-step boundary, length disagreement and
  inconsistent scheduled-frame snapshots.
- The normal server startup graph does not import or instantiate the
  experimental class. The new method is also runtime-limited to one call, so it
  is not a production per-step JavaScript boundary.

## Evidence provenance

### Git-history evidence

The immediately preceding experimental initial-frame slice is commit
`ecb167e3bc98f5ea63947720fb92da9f192a2f03`, verified locally and on the
existing GitHub `exclusive-server-mode-refactor` branch before this work. That
identity proves the starting history only; current behavior is supported by
the source and newly reproduced checks below.

### Prior planning measurements and retained artifacts

No prior performance measurement is used to justify this change. The selected
current-TypeScript frame-v1 fixture, identified from source revision
`7925faf7aef33bd3de3e1b6d3c021c4320a8dd68`, remains the layout oracle. The
Rust retained test uses seed `0x12345678`; the production-addon integration uses
seed `0x89abcdef`. Their different first-step pellet counts are therefore
separate deterministic observations, not a conflicting reproduction.

### Derived arithmetic

The initial production-addon frame contains hexadecimal Float32 length
`0x48f6` (18,678) and byte length `0x123d8` (74,712). Its post-step frame
contains `0x48e7` (18,663) Float32 values and `0x1239c` (74,652) bytes. The
derived equalities are `18,678 * 4 = 74,712` and
`18,663 * 4 = 74,652`. Five removed pellet records account for 25 fewer floats;
the remaining frame sections add 10 floats, producing the observed net change
of 15 floats or 60 bytes. These calculations are derived from reproduced frame
counts, not independent timing or performance measurements.

### Newly reproduced measurements and validation

All checkpoint files and SQLite databases used by these checks were created in
unique operating-system temporary roots. The owner database, saves, managed
checkpoint directory and Oxygen deployment were not accessed.

| Command or check | Newly reproduced result |
|---|---|
| Production-addon `experimentalFreshRunSession.native.test.ts` | 2 passed. The retained real session covers failed SQLite persistence, exact retry and acknowledgement, activation, initial frame, one scheduled step, complete frame walk and duplicate rejection. |
| Focused real-addon fresh-session/coarse-surface matrix | 8 passed. The normal production addon exposes the explicitly experimental one-shot method while feature-gated test-only exports remain absent. |
| Broad `server/rustEngine` Vitest matrix | 68 passed; 9 documented platform/feature skips across 10 files. |
| TypeScript wrapper suite | 6 passed, including exact completed-step and malformed-output rejection. |
| `cargo test --manifest-path native\\Cargo.toml fresh_run -- --nocapture` | 7 passed. The retained seed reaches generation 1, completed step 1 and 3,496 pellets once. |
| `cargo test --manifest-path native\\Cargo.toml --release engine::generation::tests -- --nocapture` | 12 passed; 385 filtered out. |
| `cargo test --manifest-path native\\Cargo.toml --release` | 397 passed; the compile-fail doctest passed. |
| `cargo test --manifest-path native\\Cargo.toml --release --all-targets --features engine-test-hooks` | 415 library tests and 3 enabled binary tests passed. |
| `cargo check --manifest-path native\\Cargo.toml --all-targets --features engine-test-hooks` | Passed. |
| `cargo clippy --manifest-path native\\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed with warnings denied. |
| `cargo fmt --manifest-path native\\Cargo.toml -- --check` | Passed. |
| `node .\\node_modules\\tsx\\dist\\cli.mjs scripts\\stage3\\checkpoint-handoff-evidence.ts` | 6 real isolated addon/worker assertions passed. |
| `node .\\node_modules\\typescript\\bin\\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for the four touched TypeScript files | Passed. |
| `git diff --check` | Passed; only line-ending conversion warnings were emitted. |

The rebuilt normal production addon and independent Node calculation agree on
native source SHA-256
`1f35726847b85610f9ad291aa5c4d55b3a2608fe371705415ad64bd8e2f37572`.
The identity covers 67 selected files, 3,472,604 canonical bytes and 9,423
accounted path bytes. The addon reports target `x86_64-pc-windows-msvc`, profile
`release`, build class `production` and build-contract SHA-256
`sha256:3b592e686658f6c9e452b4cd898f12700c455077ecf4da9c2635cbdb342b2cfa`.

## Independent review

One reused independent strong reviewer inspected the completed seven-file
source/test diff read-only and changed no files. The reviewer found no blocker,
P1, P2 or P3 defect. It directly checked the one-use transition marker,
scheduler source/ticket/publication correlation, coordinator staging branches,
post-swap fault containment, session operation ownership, exact scalar
conversion, TypeScript invariants, complete frame walk and absence from normal
startup. No review turn was blocked or wasted.

The reviewer retained two uncertainties rather than defects: tests do not
inject a failure specifically between the authority swap and scheduler/frame
completion, and they do not exercise N-API resolution/GC failure. Source
inspection shows both routes cannot retry. External-controller first-step
behavior, Debian execution, continuous scheduling, WebSocket/browser routing
and production startup are also outside this slice.

## Limits and open gates

- The scheduler and coordinator are constructed for one Rust-derived service
  boundary. A retained continuous scheduler, real wall-clock service loop,
  command drain and repeated authoritative steps remain open.
- Double-buffer/reuse-until-send-completes, latest-unsent replacement and
  WebSocket backpressure remain open.
- Browser rendering, selection, welcome-state caching, Protocol 2 external
  observations/actions, real LAN transport and the separate RL trainer are not
  connected to this session.
- Reset, New Run, resume/recovery, controller delivery, later generation
  frames and normal production startup remain open.
- No Stage 6 plan checkbox, Stage 6A evidence gate, production cutover,
  Debian/Oxygen performance result or sustained-growth gate is claimed.
