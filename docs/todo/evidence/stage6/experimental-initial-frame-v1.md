# Stage 6 experimental initial Rust frame-v1 publication

Date: 2026-08-30
Approved plan revision: `2026-07-29-draft-4`

## Outcome and scope

The explicitly experimental fixed-P0 production-addon session can now publish
exactly one neutral-view display frame after its already-proved managed-file,
SQLite-metadata, exact-acknowledgement and running-authority barriers. Rust
packs the bytes directly from the retained `AuthoritativeState`. JavaScript
supplies no camera, world, entity identity, population value, frame statistic
or length. The frame result contains only the replaceable display bytes and six
canonical 16-digit unsigned-64-bit hexadecimal metadata fields.

This is a one-frame startup/lifecycle proof. It does not route a fixed-step
stream through WebSocket, the browser, LAN or the external trainer, and it does
not complete any Stage 6A or production-cutover gate.

## Current-source proof

- `PendingRunStartTransition::pack_initial_frame_v1` rejects until
  `publish_running_authority` has completed, then invokes the existing direct
  Rust frame-v1 packer with `FrameV1ViewDescriptor::default()`.
- `ExperimentalStage6aFreshRunSession::publish_initial_frame_v1` accepts no
  JavaScript arguments. It shares the session's single CAS-owned coarse
  operation slot with construction, checkpoint publication, acknowledgement
  and activation.
- The worker leaves `initial_frame_published` false on every ordinary error,
  marks it only after successful packing and checked metadata conversion, and
  rejects a second publication without changing authority. A caught panic uses
  the existing permanent bounded first-fault latch.
- Every `usize` frame count is checked into `u64` and crosses N-API as canonical
  hexadecimal text. No U64 value crosses through a JavaScript `Number`.
- The `Vec<u8>` becomes an owning N-API `Uint8Array`; checkpoint population,
  recurrent-state and archive bytes remain in Rust or the managed file.
- `ExperimentalFreshRunSession` strictly rejects unknown output fields,
  non-`Uint8Array` bytes, noncanonical numeric fields, alive counts above total
  counts, Float32/byte-length disagreement and payload-length disagreement.
- The real-addon integration walks the returned payload using
  `readFrameHeader` and the current snake/body/pellet frame-v1 layout through
  the exact final float. It observes generation 1, 65 total/alive snakes,
  world radius 3,500, neutral camera `(0, 0)`, zoom 1 and 3,500 pellets.
- The experimental module remains absent from normal `server/index.ts` and
  `SimServer` startup composition. The normal server authority selection was
  not changed.

## Evidence provenance

### Git-history evidence

At the owner's request, the pre-finalization migration snapshot was protected
as commit `cf807632085308d5c6db70776ec39e7ba44310df` and verified on the existing
GitHub `exclusive-server-mode-refactor` branch before this slice continued.
That backup identity is not used as runtime-correctness proof; the behavior
claims above come from the current source and newly reproduced tests below.

### Prior planning measurements and retained artifacts

No prior performance measurement is used to justify this change. The selected
current-TypeScript frame-v1 fixture, identified from source revision
`7925faf7aef33bd3de3e1b6d3c021c4320a8dd68`, remains the test oracle for exact
layout bits. Its Rust comparison was newly rerun here; it is not a new browser,
LAN or production measurement.

### Derived arithmetic

The reproduced fixed-P0 result reports hexadecimal Float32 length `0x48f6`
(18,678) and byte length `0x123d8` (74,712). The derived equality is
`18,678 * 4 = 74,712`. The 65 initial snakes are the fixed profile's 55 evolved
population slots plus 10 baseline slots. These calculations are labelled
derived; the hexadecimal values and typed-array length were read from the
newly reproduced native result.

### Newly reproduced measurements and validation

All checkpoint files and SQLite databases used below were created under unique
operating-system temporary roots. One additional temporary raw session measured
only frame size by echoing its exact Rust descriptor directly; it removed its
own temporary root and is not cited as persistence evidence. Persistence
ordering is instead proved by the real worker-backed integration suites.

| Command or check | Newly reproduced result |
|---|---|
| Production-addon `experimentalFreshRunSession.native.test.ts` | 2 passed. The main test includes failed SQLite commit, exact retry/acknowledgement, activation, premature frame rejection, full frame walk and duplicate rejection. |
| Focused nine-file persistence/session/source-identity/frame Vitest matrix | 53 passed; 9 documented platform/feature skips. |
| TypeScript wrapper suite | 6 passed, including narrowed, expanded and inconsistent frame-output rejection. |
| `cargo test --manifest-path native\Cargo.toml fresh_run -- --nocapture` | 7 passed; 390 filtered out. |
| `cargo test --manifest-path native\Cargo.toml --release engine::generation::tests -- --nocapture` | 12 passed; 385 filtered out. |
| `cargo test --manifest-path native\Cargo.toml --release` | 397 passed; the one compile-fail doctest passed. |
| `cargo test --manifest-path native\Cargo.toml --release --all-targets --features engine-test-hooks` | 415 library tests and 3 enabled binary tests passed. |
| `cargo check --manifest-path native\Cargo.toml --all-targets --features engine-test-hooks` | Passed. |
| `cargo clippy --manifest-path native\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed with warnings denied. |
| `cargo fmt --manifest-path native\Cargo.toml -- --check` | Passed. |
| `node .\node_modules\tsx\dist\cli.mjs scripts\stage3\checkpoint-handoff-evidence.ts` | 6 real isolated addon/worker assertions passed. |
| `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for the four touched TypeScript files | Passed. |
| `git diff --check` | Passed; only existing line-ending conversion warnings were emitted. |

The rebuilt normal production addon and independent Node calculation agree on
native source SHA-256
`0c78434910fac945d37050bc9901641b548b5315b9c4651c08ffaf60d5b95999`.
The identity covers 67 selected files, 3,458,437 canonical bytes and 9,423
accounted path bytes. The addon reports target `x86_64-pc-windows-msvc`, profile
`release`, build class `production` and build-contract SHA-256
`sha256:3b592e686658f6c9e452b4cd898f12700c455077ecf4da9c2635cbdb342b2cfa`.

## Independent review

One independent strong reviewer inspected the concrete frame/authority boundary
read-only. The reviewer changed no files, used no blocked or wasted turn, and
found no blocker, P1, P2 or P3 defect. Directly checked areas included authority
gating, single-operation ownership, error/retry and caught-panic behavior,
typed-array ownership, exact scalar conversion, TypeScript output invariants,
the current browser layout, tests and normal-startup imports.

The reviewer recorded one remaining uncertainty rather than a defect: no test
forces a panic specifically inside frame packing or an N-API resolution/GC
failure. Existing source and tests prove the general catch-to-permanent-fault
route and successful typed-array lifetime, but not those low-level injected
failure cases.

## Limits and open gates

- Only the first neutral-view frame is exposed. Continuous coordinator-owned
  per-step frame publication is not wired.
- Double-buffer/reuse-until-send-completes, latest-unsent replacement and
  WebSocket backpressure remain open.
- The browser renderer, selection, actual LAN transport, stats, welcome-state
  cached frame length, Protocol 2 observations/actions and the external trainer
  are not connected to this session.
- Reset, New Run, resume/recovery, controller assignment, later generation
  frames and normal production startup are unchanged and remain open.
- No Stage 6 plan checkbox, Stage 6A evidence gate, production cutover,
  Debian/Oxygen performance result or sustained-growth gate is claimed.
- No owner database, save, managed checkpoint directory or
  `/opt/apps/slither_neuroevo/` path was read or written.
