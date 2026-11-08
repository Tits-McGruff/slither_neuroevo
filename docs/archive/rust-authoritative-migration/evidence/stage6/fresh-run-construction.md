# Stage 6 fixed P0 fresh-run construction evidence

Date: 2026-08-30

Approved plan revision: `2026-07-29-draft-4`

Exact working native source SHA-256:
`60858e7e62e2e48dd3199763911cfa4f4b6191732fad4c3669e050e0e812bff0`

This records one prerequisite for the Stage 6A experimental/fresh path. Rust
constructs the fixed current-default generation-one boundary and returns it
behind the existing run-start durability barrier. This is not normal server
startup, a browser/RL vertical slice, Stage 6A completion, or production
cutover.

## Evidence classification

### Current-source proof

- `native/src/engine/fresh_run.rs::prepare_stage6a_p0_fresh_run` accepts only
  an opaque run ID, exact Uint32 seed, and hard memory ceiling. Rust selects
  the fixed graph and normalized configuration, binds current build/source
  identity, allocates dense population, brain and genome identities, derives
  isolated world/evolution/external-controller and baseline RNG streams, and
  initializes every population genome.
- The constructor first creates a metadata-only population/brain shell. It
  calls `preflight_generation_boundary_allocation` with the complete numeric
  shape before allocating any genome-weight or recurrent-state payload. A
  one-byte ceiling rejects without returning a pending transition.
- Population slots are `0..54`; Rust assigns matching brain and genome IDs
  `1..55`, generation and population epoch one, zero fitness and recurrent
  state, and next brain/genome ID 56. JavaScript supplies none of those values.
- The public result is only `PendingRunStartTransition`. Its real P0 test
  proves activation is rejected before persistence, same-operation checkpoint
  retry returns one exact descriptor, exact acknowledgement is retained, one
  collision-safe authority with 65 snakes and 3,500 pellets activates, and a
  second activation is rejected.
- `scripts/stage6/generate-fresh-run-reference.ts` executes the selected
  TypeScript graph builder, genome initializer and RNG code. The retained
  fixture contains the complete graph spec, TypeScript graph key, compiled
  parameter ranges, every P0 normalized setting as an explicit scalar-kind
  record, 55 per-genome byte digests, one complete-population digest, and the
  world, post-population evolution, external-controller and ten baseline
  uniform continuations. Rust deserializes and compares the graph structures
  and identity, compares setting kinds before values, retains every Float64 as
  exact bits even when its value is integral, and compares every generated
  genome's SHA-256 digest computed over every little-endian Float32 weight
  byte.
- `native/build.rs` and `server/rustEngine/nativeSourceIdentity.ts` both include
  `fixtures/fresh-run-reference.json`. A normal production addon rebuild
  matched Node's independent current-tree calculation and the existing native
  export-surface test confirmed that no test-only fresh-run session was added.

### Git-history evidence

`git diff --exit-code 7925faf7aef33bd3de3e1b6d3c021c4320a8dd68`
over the selected TypeScript config, graph, genome, RNG and baseline-bot source
files returned no differences. This identifies the clean Git source represented
by the executable fixture; it is not runtime or cutover evidence.

### Derived arithmetic

- The compiled fixed graph contains 13,458 Float32 parameters and 16 recurrent
  Float32 values per brain.
- `55 * 13,458 = 740,190` population weight values and `55 * 16 = 880`
  population recurrent values. Tests and checkpoint descriptors validate the
  corresponding exact counts; these products are arithmetic, not measured
  memory or performance results.

No prior planning measurement is used as a reproduced result in this slice.

## Newly reproduced local results

These commands ran in the current Windows development workspace, not on
`oxygen`:

| Command | Result |
|---|---|
| `cargo test --manifest-path native\Cargo.toml --features engine-test-hooks fresh_run -- --nocapture` | 4 passed; 408 filtered out. |
| `cargo test --manifest-path native\Cargo.toml --release engine::generation::tests -- --nocapture` | 12 passed; 382 filtered out. |
| Focused `engine::step_config::tests` | 6 passed; 406 filtered out. |
| `cargo test --manifest-path native\Cargo.toml --release` | 394 passed; the one compile-fail doctest also passed. |
| `cargo fmt --manifest-path native\Cargo.toml -- --check` | Passed. |
| `cargo clippy --manifest-path native\Cargo.toml --all-targets --features engine-test-hooks -- -D warnings` | Passed with warnings denied. |
| Focused Vitest for all Stage 5/6 source fixtures, run-start handoff, native source identity and category registration | 20 passed; 3 Windows platform skips. |
| Normal production-addon source-identity, coarse-export and checkpoint-export integration | 8 passed; 5 feature-gated tests skipped. |
| `node .\node_modules\typescript\bin\tsc -p tsconfig.json --pretty false` | Passed. |
| Targeted ESLint for the new fixture generator/test and changed source-identity/category files | Passed. |
| `git diff --check` | Passed; only existing line-ending conversion warnings were emitted. |

The final source-identity calculation covers 67 selected files, 3,408,676
canonical bytes and 9,423 accounted path bytes. The exact combined population
weight SHA-256 is
`584b196b8be12762e043a4838fca6cc7567dc068ea83f04bf1c54d474af2e711`;
the exact initial world state is `0xe3ccbd27`, the post-population evolution
state is `0xfb3d3d8b`, and the initial external-controller state is
`0x4cd29c09`.

## Independent-review status

One existing independent reviewer was assigned the completed deterministic
authority/persistence diff read-only. Its first turn ended before inspection
because the reviewer hit a separate usage limit and is counted as one
blocked/wasted turn. After the user explicitly requested a retry and the
reviewer became available, it found one P2 retained-oracle/evidence gap: the
fixture did not mechanically compare graph identity/structure, setting kinds,
or the world and external-controller streams. The explicit graph, typed-scalar
and complete uniform-stream comparisons above corrected that finding. The same
reviewer's final static recheck found the P2 closed and no remaining blocker,
P1 or P2 defect. The reviewer changed no files and did not independently rerun
the reported validation. In total, three reviewer turns were used for this
slice: the first was blocked/wasted, the user-requested retry found the P2,
and the correction recheck completed the required review.

## Limits and open gates

- The constructor is a Rust production module but has no N-API or normal
  engine-session/server composition yet. The current Node server cannot start
  this authority through the new constructor.
- Reset and New Run request ordering, retention of an old authority while a
  replacement is staged, thread-count/session policy, controller lifecycle,
  browser/LAN routing and Protocol 2 RL integration remain open.
- Restore/recovery provenance, corrupt-latest recovery, retention/pruning,
  Hall-of-Fame policy, export/import, legacy compatibility and owner-data
  migration remain open.
- No Debian `oxygen`, P0/P1/P2 performance, responsiveness, checkpoint-pause,
  browser/RL latency or overnight-growth measurement is claimed.
- Tests used disposable operating-system temporary directories. No owner
  database, save, managed checkpoint directory, or `/opt/apps/slither_neuroevo/`
  path was read or written.
