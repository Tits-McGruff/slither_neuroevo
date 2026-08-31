# Stage 6 retained generation-transition resume

Date: 2026-09-01

Approved plan revision: `2026-07-29-draft-4`

Authority implementation commit:
`44b5b9d34ab3ed7d97dfb9b173c92998c584fdb0`

Linux portability follow-up commit:
`ba9c0ace77aa3cc0fbaae62f4e1d142d34dd103b`

Exact final native source SHA-256:
`ee129af48ef83975676a4232659a94e9c60e4159ae6dbf5cecc9ed7883580810`

## Outcome and scope

The retained Rust `RunningAuthorityLoop` can now carry one real terminal
generation transition through immutable checkpoint publication, the exact
SQLite metadata acknowledgement, required connected-controller reassignment,
one final successor authority swap and an in-place scheduler/coordinator
rebind. The old generation remains authoritative throughout persistence and
reassignment. Exact retry reuses the admitted transition and managed
descriptor rather than evolving or publishing a second checkpoint.

This is a Rust-core and feature-gated integration boundary. The background
runtime command queue does not yet route these completion operations, and the
normal addon/server startup exposes no generation-handoff test session. This
slice therefore does not establish normal Rust startup, the Stage 6A vertical
slice, any P0/P1/P2 performance gate or production cutover.

## Current-source proof

- `PendingRunStartTransition::into_running_loop` now transfers the admitted
  checkpoint and graph limits into the retained loop. Later generation
  publication therefore uses the same bounded Rust policy that admitted the
  run rather than accepting limits from JavaScript.
- `RunningAuthorityLoop::publish_pending_generation_checkpoint` returns only
  the immutable descriptor and the `GenerationCommitRecord` borrowed from the
  retained `GenerationTransitionBatch`. The compact eight-field summary,
  Hall-of-Fame source identity, exact floating-point bits, elite successor
  slot and successor genome ID remain Rust-constructed outputs.
- `acknowledge_pending_generation_persistence` accepts only the worker's
  complete exact descriptor. A premature or mismatched descriptor returns an
  error without changing authority, retiring the scheduler ticket or losing
  the admitted successor. An exact acknowledgement remains retained.
- `prepare_acknowledged_generation_reassignments` creates the reliable
  assignment batch only after persistence acknowledgement.
  `submit_external_delivery_results` applies exact local results to either an
  ordinary retained step or that assignment batch. Late, duplicate or stale
  results received with no batch are counted as ignored and return `Idle`
  before ticket or frame access; disappearance of an ordinary active blocker
  still faults.
- `publish_acknowledged_generation_start` performs the one final Rust authority
  swap only after every barrier is ready. It validates an in-place coordinator
  rebind, commits the exact terminal scheduler ticket with a post-wait wall
  origin, updates only the coordinator's world epoch, clears the retained
  ticket once and optionally packs a post-swap frame. A repeat call is rejected
  by the changed loop state.
- The in-place rebind preserves the coordinator's admitted workspaces and its
  monotonic external event sequence. The continuation fixture publishes the
  next ordinary generation-two step through that same coordinator, proving
  that finalization neither reconstructs the large workspaces nor repeats the
  terminal transition.
- The retained N-API fixture and its scalar converters remain entirely behind
  `engine-test-hooks`. The isolated production-addon assertion proves those
  exports are absent. Population, recurrent-state and archive bytes do not
  cross N-API.

## Evidence provenance

### Git-history evidence

The starting branch tip was
`dd36aa81e13df22927fba3872eaf5d92fa9058ab`, already verified as the identical
local and live GitHub `exclusive-server-mode-refactor` tip with zero ahead and
zero behind. The reviewed authority change was committed and fast-forwarded as
`44b5b9d34ab3ed7d97dfb9b173c92998c584fdb0`.

Oxygen then compiled a Linux-only `clippy::needless_return` branch that Windows
does not compile. The two expression-only corrections in
`native/src/engine/sensing_fixture.rs` were committed and fast-forwarded as
`ba9c0ace77aa3cc0fbaae62f4e1d142d34dd103b`. They do not change the reviewed
generation barrier. Every exact-final result below uses `ba9c0ac` unless a
different commit is stated explicitly.

### Prior planning measurements and retained artifacts

No prior timing, capacity or persistence measurement is used to justify this
change. Existing scheduler, generation, managed-checkpoint and TypeScript
reference fixtures are implementation inputs. TypeScript remains a separately
selected test oracle and is not an automatic runtime fallback.

### Derived arithmetic

The retained-loop unit fixture uses a one-second fixed delta and serves eight
complete boundaries. Its real terminal ticket therefore represents generation
1 step 8, and its admitted successor is generation 2 step 8; the next ordinary
publication is generation 2 step 9. The feature-gated N-API fixture separately
uses a 1/60-second fixed delta, preloads elapsed time to eight seconds minus
half a step, services its terminal ticket at 500 wall milliseconds and resumes
at 1,000 wall milliseconds after the artificial persistence/assignment wait.
These are deterministic fixture inputs and arithmetic, not timing or
throughput measurements.

### Newly reproduced Windows validation

All managed files and SQLite databases used by integration tests were created
under unique operating-system temporary roots. Generated native binaries are
ignored and were not committed.

| Command or check | Newly reproduced result |
|---|---|
| `cargo test --manifest-path native\\Cargo.toml --release --all-targets --features engine-test-hooks --quiet` | 426 library tests and all three enabled target tests passed at exact final commit `ba9c0ac`. |
| `cargo test --manifest-path native\\Cargo.toml --release engine::generation::tests -- --nocapture` | 12 focused generation/commit-record tests passed; 395 were filtered out. |
| `cargo test --manifest-path native\\Cargo.toml --release --features engine-test-hooks generation_handoff -- --nocapture` | Both retained generation-handoff/rebind tests passed; 424 library tests were filtered out. |
| `scripts\\stage3\\checkpoint-handoff-evidence.ts` at reviewed authority commit `44b5b9d` | Six isolated Rust-to-managed-file-to-SQLite-to-Rust assertions passed. The only later native source change is the Linux-only lint correction, and the same six assertions passed on Oxygen at exact final commit `ba9c0ac`. |
| Broad `server/rustEngine` Vitest matrix against the rebuilt exact-final normal addon | 68 passed; 9 documented platform/feature skips across 10 files. |
| Native source-identity unit/integration pair | 10 passed; 3 documented Windows platform skips. |
| `cargo clippy --manifest-path native\\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed with warnings denied. |
| `cargo fmt --manifest-path native\\Cargo.toml -- --check` | Passed; the local tool emitted only its existing user-path canonicalization warning. |
| `node .\\node_modules\\typescript\\bin\\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for the touched native integration test and isolated evidence runner | Passed. |
| `git diff --check` | Passed. |

The rebuilt Windows production addon and independent TypeScript calculation
agree on the final source SHA above. The identity covers 68 selected files,
3,589,650 canonical bytes and 9,568 accounted path bytes. The addon reports
target `x86_64-pc-windows-msvc`, profile `release`, build class `production`,
Rust `rustc 1.92.0 (ded5c06cf 2025-12-08)` and build-contract SHA-256
`sha256:3b592e686658f6c9e452b4cd898f12700c455077ecf4da9c2635cbdb342b2cfa`.

### Newly reproduced Oxygen validation

Oxygen reported Debian 13 kernel `6.12.94+deb13-amd64`, x86_64 GNU/Linux,
an AMD Ryzen 7 2700, eight available processors and 16,382,176 KiB total
memory. This is environment identity only; the test/build durations below are
not P0/P1/P2 performance measurements.

The exact GitHub branch was cloned into the unique disposable directory
`/tmp/slither-stage6-generation-resume-4zpOlk`. The clone was verified at
`ba9c0ace77aa3cc0fbaae62f4e1d142d34dd103b` before exact-final validation.

| Command or check | Newly reproduced result |
|---|---|
| Strict all-target Clippy with `engine-test-hooks` | Passed with warnings denied at exact final commit `ba9c0ac`. |
| Rustfmt check | Passed. |
| Release all-target suite with `engine-test-hooks` | 427 library tests and all three enabled target tests passed. |
| Focused release `engine::generation::tests` | 12 passed; 396 filtered out. |
| Focused release `generation_handoff` filter with `engine-test-hooks` | Both retained handoff/rebind tests passed; 425 library tests were filtered out. |
| Exact isolated worker-backed checkpoint handoff | Six assertions passed, including production fixture absence, immutable publication, truncate/delete rejection, failed SQLite isolation, exact acknowledgement, assignment and one final swap. |
| Normal production-addon source-identity pair after an ordinary addon build | 13 passed. |

The Linux addon reports the same source SHA as Windows, target
`x86_64-unknown-linux-gnu`, profile `release`, build class `production`, the
same Rust compiler and build-contract SHA-256
`sha256:3bcf1d0a16aeef770a5316c503b16e0a2bf2212f8d360309768b2bc4c2b8f51e`.

After validation, the exact disposable directory was resolved and verified as
a real directory under `/tmp`, removed recursively, and verified absent. The
deployed `/opt/apps/slither_neuroevo/` tree, owner databases and owner saves
were neither read nor written during this slice.

### Superseded diagnostic and setup invocations

- A pre-final Windows debug `cargo test --all-targets --features
  engine-test-hooks` run passed 421 tests and failed five benchmark fixtures
  solely because those fixtures explicitly require a release build. The
  correct final release invocation above passed 426 library tests and all
  three enabled targets; the debug run is not counted as validation.
- One pre-final combined Windows Vitest invocation failed to start because
  esbuild could not traverse a protected parent directory and could not
  resolve `vite.config.ts`. The immediate focused and broad reruns passed; the
  failed launcher invocation is not counted.
- Oxygen's first strict Clippy run at `44b5b9d` found two Linux-only
  `needless_return` diagnostics. Commit `ba9c0ac` corrected exactly those two
  expressions; strict Clippy and the complete release suite then passed.
- The first post-handoff Oxygen native-identity run passed all 12 source-only
  tests but could not load `native/index.js`, which a fresh clone had not yet
  generated. A direct N-API build first failed before compilation because the
  non-login SSH shell did not expose `cargo` by name. Running the documented
  build under Oxygen's configured login shell succeeded, and the unchanged
  identity pair then passed 13/13. Neither setup invocation is counted as a
  source/test failure.

## Independent review

One existing independent strong reviewer inspected the concrete persistence,
authority-transition and scheduler-rebind diff read-only. No additional agent
was used. The reviewer changed no files and found no blocker, P1, P2 or P3.

The review covered Rust-owned generation metadata and successor identity,
exact descriptor acknowledgement, retry identity, old-authority retention,
assignment delivery, late-result handling, one final swap, scheduler ticket
retirement, in-place coordinator continuation, frame-failure ordering,
feature-gated N-API converters, the thin Node persistence handoff and native
source coverage. It independently reproduced both handoff tests, nine release
fresh-run tests, 12 focused generation tests, all six isolated worker-backed
assertions, strict Clippy, rustfmt and diff checks. No review turn was blocked
or wasted.

The reviewer retained one test limitation rather than a defect: the optional
frame failure after generation finalization is supported by source ordering,
frame preflight and the ordinary post-publication injected-failure fixture, but
does not have a separate generation-specific injection test.

## Limits and open gates

- The background `EngineRuntime` still routes only its bounded probe command.
  Persistence acknowledgement, controller/RL delivery, generation resume and
  frame output are not yet connected through its production queue surface.
- The complete worker-backed bridge remains a feature-gated integration
  session. The normal addon/server startup has no equivalent session route and
  JavaScript cannot call these loop methods in production.
- This slice proves one retained generation transition and the next ordinary
  step. It does not prove normal startup/resume, browser or trusted-LAN play,
  the external Protocol 2 trainer, output backpressure, or a complete
  generation through the actual Node process.
- Import/recovery branch provenance, corrupt-latest recovery, retention and
  pruning, best-50 unique Hall of Fame, pins, the configurable 4 GiB automatic
  cap, direct export/upload import, legacy compatibility and overnight growth
  remain open.
- The Oxygen checks are build/correctness portability evidence, not the real
  browser/LAN/RL vertical slice or the mandatory Debian P0/P1/P2 performance
  gates.
- No Stage 6A completion, normal production Rust startup or production cutover
  is claimed.
