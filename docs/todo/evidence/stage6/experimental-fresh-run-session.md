# Stage 6 experimental production-addon fresh-run session evidence

Date: 2026-08-30

Approved plan revision: `2026-07-29-draft-4`

Exact working native source SHA-256:
`be8d142b1ac6b37bbe133fcef220db865e584413029d93fb0a48b4f84ce9de96`

This records one narrow Stage 6A prerequisite. The normal production addon now
contains an explicitly experimental coarse session that retains one real
fixed-P0 Rust fresh-run transition through construction, managed-file
publication, SQLite metadata commit, exact acknowledgement and one running
authority activation. Normal server startup does not import or instantiate the
session. This is not Stage 6A completion or production cutover.

## Evidence classification

### Current-source proof

- `native/src/napi_engine.rs::ExperimentalStage6aFreshRunSession` accepts only
  a bounded run ID, exact eight-digit Uint32 seed hex and exact positive
  16-digit memory-ceiling hex. JavaScript supplies no population, graph,
  genome, RNG, world, snake, pellet or allocator authority.
- Construction, immutable checkpoint publication and collision-safe authority
  activation are `AsyncTask` worker operations. The retained
  `PendingRunStartTransition` remains behind one Rust mutex; N-API returns only
  a checkpoint descriptor and bounded scalar lifecycle metadata.
- `server/rustEngine/experimentalFreshRunSession.ts` performs the production
  addon/source-identity handshake, validates the exact coarse class surface and
  composes publication only through `RunStartPersistenceHandoff` and
  `CheckpointPersistenceClient`. The public commit call accepts only the exact
  operation token.
- Persistence acknowledgement enters Rust as a raw JavaScript object.
  `checkpoint_descriptor_from_napi_object` requires the exact 23-key surface,
  bounds every string through `JsString` before Rust ownership and accepts
  `protocolVersion` only as the finite, integral, exact supported value. U64
  values remain canonical 16-digit lowercase hexadecimal strings.
- A dedicated synchronous acknowledgement CAS/RAII lease is acquired before
  any caller-controlled object inspection. A getter that reenters the addon can
  observe only `acknowledgingPersistence`; an attempted activation cannot own
  the slot. Ordinary parse or mismatch errors release the lease for explicit
  retry.
- Any Rust panic caught in construction, checkpoint publication, persistence
  acknowledgement or activation permanently latches the first diagnostic at
  no more than 512 UTF-8 bytes. A faulted snapshot hides transition/authority
  metadata, and every later mutation or acknowledgement is rejected. Ordinary
  returned admission, I/O or SQLite errors remain retryable.
- The real-addon test retains one Rust transition across an injected SQLite
  failure, exact same-operation retry, exact worker descriptor acknowledgement
  and one activation. Failure leaves no running authority, retry creates one
  managed file, activation produces the expected 65 snakes and 3,500 pellets,
  and duplicate checkpoint/activation attempts are rejected.
- Feature-gated checkpoint/run-start/generation fixture classes remain absent
  from the normal addon. Repository search found no non-test normal-startup
  import of `experimentalFreshRunSession.ts`.

### Git-history evidence

No Git-history claim is used for this slice. The evidence describes the current
intentional uncommitted migration tree and its source-derived addon identity.

### Prior planning measurements

No prior planning measurement is reused as a newly reproduced result.

### Derived arithmetic

No derived performance or capacity arithmetic is used to justify this slice.
The 65-snake and 3,500-pellet values are current fixed-profile source/test
assertions, not timing or memory measurements.

## Newly reproduced local results

These commands ran in the current Windows development workspace, using only
disposable operating-system temporary directories for checkpoint files and
SQLite databases:

| Command | Result |
|---|---|
| `cargo check --manifest-path native\Cargo.toml --all-targets --features engine-test-hooks` | Passed. |
| `cargo test --manifest-path native\Cargo.toml --release` | 397 passed; the one compile-fail doctest also passed. |
| `cargo test --manifest-path native\Cargo.toml --release engine::generation::tests -- --nocapture` | 12 passed; 385 filtered out. |
| `cargo test --manifest-path native\Cargo.toml --release fresh_run -- --nocapture` | 7 passed; 390 filtered out. |
| `cargo fmt --manifest-path native\Cargo.toml -- --check` | Passed. |
| `cargo clippy --manifest-path native\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed with warnings denied. |
| Focused nine-file persistence, handoff, source-identity and experimental-session Vitest matrix | 62 passed; 9 documented platform/feature skips. |
| Real normal-addon `experimentalFreshRunSession.native.test.ts` | 2 passed, including the getter-reentrancy and real file/SQLite/ack/activation path. |
| `node .\node_modules\tsx\dist\cli.mjs scripts\stage3\checkpoint-handoff-evidence.ts` | 6 passed against isolated production and feature-hook addon builds. |
| `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for the touched TypeScript/session/category tests | Passed. |
| `git diff --check` | Passed; only existing line-ending conversion warnings were emitted. |

The final independent Node calculation covers 67 selected native source files,
3,449,034 canonical bytes and 9,423 accounted path bytes. The rebuilt normal
production addon reports the same SHA-256 shown above and build class
`production`.

## Independent-review status

One existing independent reviewer was reused read-only after the concrete
persistence/authority diff; no second reviewer was created and no review turn
was blocked or wasted in this slice. The first review found two P2 defects:
bindgen-owned descriptor inputs could allocate/narrow before validation, and a
caught worker panic left a poisoned or partially mutated session reusable. Raw
bounded parsing and the permanent first-fault latch corrected them.

The focused recheck then found one P2 reentrancy race between the initial idle
check and JavaScript descriptor getters. The synchronous acknowledgement lease
and real-addon getter regression corrected it. The same reviewer's final static
recheck found all three P2s closed and no remaining blocker, P1 or P2 defect.
The reviewer changed no files and did not duplicate the reproduced test matrix.
An actual deliberate Rust panic is not exported through the production addon;
the catch-to-latch route is source-proven and the latch behavior is covered by
a focused Rust unit test.

## Limits and open gates

- The class is explicitly experimental and the thin TypeScript module is not
  wired into `server/index.ts`, `SimServer`, Reset, New Run or resume/recovery.
- This slice does not connect Rust fixed-step execution, controller assignment,
  frame publication or Protocol 2 traffic to the normal server/browser path.
- Normal composition must preserve the checked handoff; Rust proves exact
  descriptor equality but does not independently attest that an arbitrary
  in-process caller obtained a descriptor from SQLite.
- Import/recovery provenance, corrupt-latest recovery, retention/pruning,
  best-50 unique Hall of Fame, pins, the configurable 4-GiB cap, direct
  export/import and legacy compatibility remain open.
- Browser/LAN and external RL vertical slices, Debian `oxygen` measurements,
  P0/P1/P2 gates and overnight database/checkpoint growth remain open.
- No owner database, save, managed checkpoint directory or
  `/opt/apps/slither_neuroevo/` path was read or written. No Stage 6A completion
  or production cutover is claimed.
