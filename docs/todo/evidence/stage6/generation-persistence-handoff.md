# Stage 6 generation persistence handoff evidence

Date: 2026-08-29

Approved plan revision: `2026-07-29-draft-4`

Exact working native source SHA-256: `071033da98503cfbc4a714e3a6e834d022a5c1a30b85d2ca95510a5dbc7fa15b`

This records one bounded implementation slice: a real retained Rust generation
transition publishes its managed checkpoint, supplies Rust-constructed compact
metadata to the SQLite worker, accepts only the complete committed descriptor
back, resolves the required controller assignment, and then performs one final
authority swap. It does not record Stage 6A completion or production cutover.

## Evidence classification

### Current-source proof

- `native/src/engine/generation.rs` constructs `GenerationCommitRecord` only
  after admitting the successor boundary. The eight-field summary and
  Hall-of-Fame values originate in Rust evolution output. The successor slot
  and genome ID are read from the exact admitted candidate. Elite parameters
  are compared by their stored Float32 bits, including signed zero.
- `native/src/engine/running_step.rs` retains one
  `PendingGenerationTransition`. Managed-file retry reuses its descriptor;
  premature or mismatched acknowledgement changes no barrier state; only a
  complete matching `CheckpointDescriptor` retains the persistence
  acknowledgement. Generation construction, reassignment and final publication
  remain gated behind that acknowledgement.
- `native/src/engine/generation_handoff_fixture.rs` constructs one real running
  authority, invokes `RunningStepCoordinator::advance_nonterminal` once to
  create its terminal transition, and retains that transition across all N-API
  calls. Publication retry cannot invoke evolution again and records only one
  physical generation-checkpoint publication.
- `native/src/napi_engine.rs` exposes the retained session only with the
  `engine-test-hooks` feature. Generation statistics, slots, snake IDs,
  successor identity and Hall-of-Fame values are outputs, never N-API inputs.
  Every U64 and Float64 bit word crosses as 16 lowercase hexadecimal digits;
  population, graph, recurrent-state and archive bytes do not cross.
- `server/rustEngine/checkpointPersistenceWorker.ts` commits the immutable
  descriptor, current pointer, exact 56-byte history record and exact 56-byte
  Hall-of-Fame reference in one `synchronous=FULL` transaction.
  `checkpointPersistenceClient.ts` strictly parses the worker's complete echoed
  descriptor and compares every field with the pending Rust descriptor before
  returning it for Rust acknowledgement.
- `server/rustEngine/checkpointPersistence.native.test.ts` uses the native
  session's record directly. It does not construct generation statistics or
  successor identities in TypeScript.

No Git-history evidence, prior planning measurement or derived arithmetic is
used to establish the claims in this slice.

### Newly reproduced local measurements

These commands ran on the current Windows development workspace, not on
`oxygen`:

| Command | Result |
|---|---|
| `cargo fmt --manifest-path native\Cargo.toml -- --check` | Passed. |
| `cargo check --manifest-path native\Cargo.toml --features engine-test-hooks` | Passed. |
| `cargo clippy --manifest-path native\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed. |
| `cargo test --manifest-path native\Cargo.toml --release generation -- --nocapture` | 29 passed; 361 filtered out. |
| `cargo test --manifest-path native\Cargo.toml --features engine-test-hooks generation_commit -- --nocapture` | 2 passed; 404 filtered out. |
| `cargo test --manifest-path native\Cargo.toml --features engine-test-hooks retained_fixture_starts_with_one_real_unpublished_terminal_transition -- --nocapture` | 1 passed; 405 filtered out. |
| `node .\node_modules\tsx\dist\cli.mjs scripts\stage3\checkpoint-handoff-evidence.ts` | Isolated production and test-hook addons built; 5 integration assertions passed. |
| `node .\node_modules\vitest\vitest.mjs run server\rustEngine\checkpointPersistence.test.ts server\rustEngine\nativeSourceIdentity.test.ts --reporter=dot` | 27 passed; 4 platform skips, including the Windows symlink-permission skip. |
| `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for the touched persistence, native integration and evidence-runner TypeScript files | Passed. |
| `git diff --check` | Passed; only existing line-ending conversion warnings were emitted. |

Recomputed native source-identity evidence contains 63 selected files and
3,308,354 canonical bytes. Both isolated addons embedded the exact SHA above;
the integration test rejected stale source identity before running assertions.

## Reproduced handoff assertions

- A descriptor acknowledgement before generation checkpoint publication is
  rejected and the old generation-one/step-zero authority remains current.
- Publishing again with the same operation returns the exact same descriptor
  and Rust commit record. A different operation is rejected, the physical
  publication count remains one, and exactly two immutable files exist after
  the run-start and generation publications.
- Attempting the generation SQLite commit before the required run-start pointer
  fails with zero metadata rows and no Rust authority change.
- Generation construction and final publication remain blocked after SQLite
  success until Rust receives the worker-returned descriptor.
- A descriptor with a different logical root is rejected without retaining the
  acknowledgement or changing authority. The exact descriptor is accepted and
  remains accepted on an exact repeat.
- Final authority publication remains blocked until the one real connected
  controller assignment receives an exact local delivery result. A forged
  lease result is rejected; the exact result permits one generation-two/step-one
  swap; a second swap is rejected by the changed world epoch.
- The stored history and Hall-of-Fame blobs match independently reproduced
  little-endian encodings of the exact Rust record byte-for-byte.
- The production addon exposes neither checkpoint fixture nor retained Stage 6
  session.

## Independent review

Exactly one read-only reviewer inspected the concrete persistence and authority
diff. It ran one focused Rust commit-record test and the 18-test persistence
suite, changed no files, and found no blocker, P1 or P2. Its only non-blocking
hardening note was that numeric Float32 equality did not distinguish `-0.0`
from `+0.0`. The validator now compares every elite weight with `to_bits()` and
the added regression proves a signed-zero forgery is rejected without changing
the source authority. The same reviewer statically rechecked that correction
and found no blocker, P1 or P2. No reviewer turn was blocked or wasted.

## Follow-on Node orchestration seam (2026-08-29)

### Current-source proof

- `server/rustEngine/generationPersistenceHandoff.ts` accepts only the pending
  operation ID from its caller. It obtains the descriptor and generation
  commit record directly from the Rust port, reparses both through the strict
  persistence protocol, and forwards them unchanged to
  `CheckpointPersistenceClient`. JavaScript callers cannot supply generation
  statistics, identities, slots or Hall-of-Fame values through this seam.
- The handoff acknowledges Rust only after the persistence client returns the
  exact complete descriptor and matching redundant operation, transition, run
  and checkpoint identities. SQLite rejection or a mismatched result cannot
  reach the acknowledgement call.
- One operation in flight is retry-safe: a duplicate caller shares the same
  promise, while a different operation is rejected before another Rust
  publication. A failed attempt releases only its own token. If the Rust
  acknowledgement itself throws after a durable commit, an explicit retry
  exact-replays the same worker operation and submits the same descriptor
  again.
- Descriptor identity and canonical operation-ID parsing now have one shared
  implementation in `checkpointPersistenceProtocol.ts`; the persistence
  client and the orchestration seam use the same exact-field comparison.
- The retained native integration now routes the failed pre-run-start attempt
  and the successful exact-replay acknowledgement through this production
  Node seam while keeping the real pending `RunningStepCoordinator`
  transition alive. The production addon still exposes no test hook.

No Git-history evidence, prior planning measurement or derived arithmetic is
used for these follow-on claims.

### Newly reproduced local measurements

These follow-on checks ran on the same current Windows workspace, not on
`oxygen`:

| Command | Result |
|---|---|
| Focused Vitest for `generationPersistenceHandoff.test.ts` | 5 passed. |
| Focused Vitest for the new handoff, checkpoint persistence, native source identity and test-category registration | 34 passed; 4 platform skips. |
| `node .\node_modules\tsx\dist\cli.mjs scripts\stage3\checkpoint-handoff-evidence.ts` | Isolated production and test-hook addons built; 5 native integration assertions passed through the real handoff. |
| `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for the handoff, protocol, client, native integration and category files | Passed. |
| `git diff --check` | Passed; only existing line-ending conversion warnings were emitted. |

No native source file changed in this follow-on seam. The isolated rebuild and
source-identity guard continued to use the exact native SHA recorded above.

### Independent follow-on review

The same one independent read-only reviewer was reused after the concrete
orchestration diff. It inspected the handoff, shared parser/equality helpers,
client, worker/native contracts, focused tests and the real native integration.
It changed no files and found no blocker, P1 or P2. Its only non-blocking note
was that the port's synchronous acknowledgement matches the current N-API
contract; a future asynchronous adapter would have to widen and await that
contract explicitly. No review turn was blocked or wasted.

## Fresh run-start durability and activation barrier (2026-08-29)

### Current-source proof

- `PendingRunStartTransition` admits and owns only an exact run-start boundary:
  generation 1, completed step 0 and population epoch 1. The admitted state is
  still an empty generation boundary, not running authority.
- Rust publishes the immutable checkpoint with its own nonzero world epoch as
  the handoff correlation token. The first operation owns the descriptor;
  same-operation retry returns it unchanged and another operation is rejected.
- `CheckpointDescriptor::first_mismatch` compares every descriptor field.
  Acknowledgement before publication or with any mismatched field is rejected;
  only the exact committed descriptor sets the retained persistence barrier.
- `RunStartPersistenceProof` is process-local and constructible only inside the
  Rust run-start module after that barrier. It binds the exact source-state
  address and world epoch. The initial replacement surface carries only the
  collision-safe world, RNG/allocator continuations and reusable fixed-step
  buffers; JavaScript cannot supply population, graph or identity authority.
- Initial-world construction runs outside the Node event loop. Final Rust
  publication rechecks the proof and exact source boundary, swaps the prepared
  buffers atomically, validates the running state and restores the boundary on
  error. One successful publication consumes the retained workspace and a
  second publication is rejected.
- `RunStartPersistenceHandoff` accepts only an operation token. It obtains and
  strictly reparses the descriptor from Rust, requires `run-start`, commits it
  through `CheckpointPersistenceClient` with no generation statistics, and
  passes the worker's exact complete descriptor back to Rust only after the
  shared full-field equality check succeeds. Same-operation overlap coalesces,
  different-operation overlap rejects, and a failed attempt can exact-replay.
- The feature-gated N-API session retains the real Rust transition through
  managed-file publication, a deliberately failed SQLite attempt, durable
  worker commit/replay, mismatched and exact Rust acknowledgements, and one
  final activation. Its normal production-addon counterpart is absent. The
  exposed session data is limited to the checkpoint descriptor and bounded
  scalar proof; population/archive bytes remain in Rust or the managed file.

No Git-history evidence, prior planning measurement or derived arithmetic is
used for these fresh run-start claims.

### Newly reproduced local measurements

These checks ran against the current Windows workspace, not on `oxygen`:

| Command | Result |
|---|---|
| `cargo check --manifest-path native\Cargo.toml --features engine-test-hooks` | Passed. |
| `cargo fmt --manifest-path native\Cargo.toml -- --check` | Passed. |
| `cargo clippy --manifest-path native\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed with warnings denied. |
| Focused feature Rust `run_start` tests | 2 passed; 406 filtered out. |
| Focused release `engine::generation::tests` | 12 passed; 378 filtered out. |
| Focused Vitest for both handoffs, checkpoint persistence, native identity and category registration | 39 passed; 4 platform skips. |
| `node .\node_modules\tsx\dist\cli.mjs scripts\stage3\checkpoint-handoff-evidence.ts` | Rebuilt isolated production and test-hook addons; 6 native integration assertions passed. |
| `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for the touched Rust-engine TypeScript, native integration and category files | Passed. |
| `git diff --check` | Passed; only existing line-ending conversion warnings were emitted. |

The isolated addons used exact current native source SHA-256
`6e68f26a71de46028e86231fc7c2a2d23c3b31db7d7b1367decf93dc5fbf5d08`,
covering 65 selected source files, 3,346,387 canonical source bytes and 9,139
accounted path bytes. This is current-source identity, not a clean-commit claim.

### Independent fresh run-start review

Exactly one existing independent read-only reviewer was reused for this
persistence and authority-transition diff. Its first turn was blocked before
inspection by the reviewer's separate usage limit and was therefore counted as
one wasted turn. After the stated reset time passed, the one permitted retry
inspected the Rust barrier/proof/publication path, N-API feature gating, thin
Node handoff, worker chronology, focused tests and retained native integration.
It changed no files, ran only read-only source/status inspection, and found no
blocker, P1 or P2. Directly supported conclusions were that premature or
mismatched acknowledgements cannot activate authority, failed validation or a
panic restores every swapped buffer and phase, same-operation retry cannot
create a second descriptor, and the normal production addon omits the fixture.
Its remaining uncertainty is the already-open normal production startup and
Reset/New Run composition; it did not independently reproduce Debian results.

## Limits and open gates

- The generation and fresh-run Node modules preserve direct Rust coupling
  through the persistence client, but only feature-gated retained native
  sessions currently implement their Rust ports. The run-start slice admits an
  already-constructed candidate; normal engine-session startup still does not
  build or connect it. Reset/New Run orchestration, old-authority retention and
  any required controller-reassignment delivery are not composed through this
  path. This is not normal production Rust startup, Stage 6A completion or
  cutover.
- The standalone SQLite worker cannot inspect checkpoint population bytes to
  rediscover the successor identity; it relies on the record obtained directly
  from Rust's admitted transition through the proved handoff.
- No owner database, save or `/opt/apps/slither_neuroevo/` path was read or
  written. Tests used disposable operating-system temporary directories and
  databases, which the harness removed.
- Explicit import/recovery provenance, corrupt-latest recovery, retention and
  pruning, best-50 unique Hall of Fame, pins, the configurable 4-GiB automatic
  cap, direct export/upload, legacy compatibility inventory and owner-data
  migration remain open.
- No browser/LAN, Protocol 2 RL trainer, Debian `oxygen`, P0/P1/P2 performance,
  overnight growth or final production-cutover gate is claimed.
