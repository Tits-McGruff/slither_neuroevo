# Modularization implementation

The governing plan is the owner's evidence-verified revision dated 7 November 2026. All packets P0–P9 are implemented and verified locally as of 8 November 2026. P9 deliberately retains the documented orchestration exceptions below; no architecture-wide redesign is claimed. Earlier packet paragraphs describe their checkpoint in the sequence and are superseded by this completion record.

Final verification passed 873 release hook-enabled Rust tests and the complete Vitest run (1,089 passed, 18 skipped, no failures; exit 0). Production-library and all-target hook-enabled Clippy, rustfmt, TypeScript, full ESLint, Vite and all 29 Python qualification-helper tests pass. PyRL's independently committed packet passed its full default pytest suite (804 passed, 13 skipped), all nine separately enabled live cross-repository cases, and 29 additional native artifact/teacher cases, as detailed in P8. The same 873 Rust test names remain after the final production extraction. Both addon classes retain their baseline ABI, and production declarations are byte-identical.

Final native source SHA is `7afeb7846d5f7d16ecaf125cf790c92e31e91f788e205fe036f1253bd30f8821`, for Windows x86_64 MSVC release. New→new exact restore passes. The pre-P7 checkpoint (`93fd2b18…`) rejects exact continuation; explicitly compatible recovery activates a distinct run. Source identity/admission policy, formats, algorithms and owner data remain unchanged. File movement intentionally changes source/build identity. Linux, Node 26, long scientific qualification/training and the deliberately excluded very-large-model Python case were not validated by this local refactor, matching the original evidence bundle's platform/scope limits. The unchanged non-hook all-target Clippy baseline issue is recorded in P1; the supported production-library and hook-enabled all-target checks pass.

## P0 baseline and native rollout

Slither started clean at `14264e4d84405e558451b868004ea005f0c2e7d7`; PyRL started clean at `b1c9d8b67f0905571b8f4372ed0a042d744a6575`. Both match the plan's local evidence. The baseline release library run with `engine-test-hooks` passed 873 tests (exit 0). Test discovery, production and hook ABI property/prototype snapshots, generated declarations, the original addons and a disposable managed checkpoint are retained locally under `.git/modularization/`, outside tracked source and owner data.

The baseline production source SHA is `b82620c237fa85cceaed51ffa1d0353843e80489bccddd0b5167996726a44d24`, build `slither_native/0.1.0+7601b3544ca2.7115ff4d0f138fc0`, Windows x86_64 MSVC release with rustc 1.92.0. Old-addon → old-checkpoint exact restore passed (exit 0). No owner database or live run was opened. Rollback uses the original Git revision with its preserved matching binary; never pair that binary with the refactored source layout. Source hashing and exact/compatible admission policy remain unchanged. New-build restore, old-build exact rejection and permitted compatible recovery were subsequently verified as recorded below.

## P1 physical test extraction (verified)

| Production owner | Before lines | After lines | Physical test files | Largest test file lines |
|---|---:|---:|---:|---:|
| `state.rs` | 13,461 | 5,919 | 13 | 1,052 |
| `fresh_run.rs` | 5,992 | 1,973 | 7 | 1,149 |
| `checkpoint.rs` | 7,934 | 5,582 | 6 | 614 |
| `control_phase.rs` | 6,198 | 3,722 | 5 | 925 |
| `coordinator.rs` | 3,296 | 1,717 | 5 | 798 |
| `sensors_v4.rs` | 2,304 | 780 | 5 | 794 |
| `brain_library.rs` | 1,145 | 203 | 4 | 511 |

Every owner retains its original `tests` module and cfg expression. Each now loads `OWNER/tests/mod.rs`, with imports, fixture re-export and topical child declarations. Shared helpers live in `fixtures.rs`; their visibility is limited to the test subtree. Relative engine references become absolute `crate::engine::…` paths to account for the extra module level. Assertions, constants, fixtures, disposable-directory counters and cleanup bodies are retained. The original production source preceding the test declaration matches exactly after newline normalization for all seven owners. All 239 moved test bodies match after path normalization and rustfmt (exit 0). No production public exports change; test names acquire one topical component.

The exact old/new test and fixture symbols and destination paths are recorded in [the P1 move map](modularization-p1-moves.tsv). This is a historical change receipt, not test discovery configuration. The runner continues to discover tests through ordinary Rust module declarations.

Production extraction in state/checkpoint, the remaining test suites, browser, N-API, persistence, selective diagnostics and PyRL remain pending. Large production owners in this packet are intentionally only the first pass; their remaining size is not an exception from the subsequent plan.

### Checks recorded so far

| Check | Result | Exit |
|---|---|---:|
| Baseline release library tests with test hooks | 873 passed | 0 |
| Refactored release library tests with test hooks | 873 passed | 0 |
| Discovery comparison using exact old/new name map | 873 before and after, no missing/unexpected tests | 0 |
| Test body / fixture relocation comparison | All 239 test bodies and all seven fixture payloads match after the documented adjustments and rustfmt | 0 |
| Rust format check | Passed | 0 |
| Production addon build | Passed | 0 |
| Production and test-hook ABI comparison; production generated declarations | Unchanged | 0 |
| Focused native source identity and fresh-run session suites | 13 passed across two files | 0 |
| Native restore probe | New→new exact succeeds; old→new exact rejects; explicitly compatible distinct recovery run activates | 0 |
| Source identity unit tests | 9 passed, 3 platform skips | 0 |
| Clippy all targets with test hooks, warnings denied | Passed | 0 |
| Clippy production library, warnings denied | Passed | 0 |
| Additional Clippy all targets without hooks | Existing unused `ShadowCandidate::synthetic` under cfg(test), with both call sites inside hook-only blocks | 101 |

The last check exposes an unchanged baseline configuration issue: `shadows.rs` is unmodified and both original calling test bodies retain their inner feature gates. The plan's verified all-targets baseline used test hooks. No unrelated dead-code change is folded into this relocation. The build script also reports a local incremental-cache hard-link warning and falls back to copying.

The new production source SHA is `f554a5c57d2b71be60ee2be4d63f5aedccd9717c49be7d358b4593ec638940c7`. The compatibility probe exercises the native startup API in disposable directories: its caller supplies the committed-recovery assertion; it is not a SQLite crash-recovery test. Existing immutable source checkpoint descriptors remain unchanged during recovery. This is compatible recovery, not exact continuation across source identities.

### Physical files and dependency direction

All newly created Rust files are listed below; their symbols are indexed in the move map. Production import fanout is unchanged: no production owner gained an import or state dependency. Each owner gains one out-of-line test root; topical tests and fixtures add one module level beneath the original test module. A representative test change now reads its topical file and, when required, `fixtures.rs`, instead of the original owner plus thousands of unrelated tests. Production state/checkpoint admission work still needs the subsequent production extraction packet.

- `native/src/engine/state/tests/`: `admission.rs`, `controller_leases.rs`, `evolved_lives.rs`, `external_delivery.rs`, `fixtures.rs`, `generation_handoff.rs`, `identity_and_config.rs`, `inspector_and_retained.rs`, `memory_and_capacity.rs`, `mod.rs`, `reward_observations.rs`, `scheduler_and_runtime.rs`, `step_publication.rs`.
- `native/src/engine/fresh_run/tests/`: `capacity_and_identity.rs`, `defaults_and_settings.rs`, `fixtures.rs`, `founders_and_qualification.rs`, `imports_and_restore.rs`, `mod.rs`, `runtime_and_handoff.rs`.
- `native/src/engine/checkpoint/tests/`: `fixtures.rs`, `malformed_archives.rs`, `mod.rs`, `numeric_limits.rs`, `publication_and_recovery.rs`, `roundtrip_and_versions.rs`.
- `native/src/engine/control_phase/tests/`: `commit_and_capacity.rs`, `fixtures.rs`, `mod.rs`, `selection_and_cadence.rs`, `sensors_and_baseline.rs`.
- `native/src/engine/coordinator/tests/`: `controller_delivery.rs`, `fixtures.rs`, `generation_and_runtime.rs`, `metrics.rs`, `mod.rs`.
- `native/src/engine/sensors_v4/tests/`: `baseline_actions.rs`, `fixtures.rs`, `geometry_and_traces.rs`, `mod.rs`, `observations_and_limits.rs`.
- `native/src/engine/brain_library/tests/`: `fixtures.rs`, `mod.rs`, `publication.rs`, `saved_deployment.rs`.

## P2 remaining large engine tests and TypeScript scenario suites (verified)

Twenty-three additional engine owners now load out-of-line tests. The three largest suites (`inference`, `control`, `generation`) have topical children and a shared fixture file; twenty other suites retain their exact Rust module path in one modest `OWNER/tests.rs`. No production logic changes. All 334 moved test bodies, all shared fixtures and all twenty single-file payloads match after the same documented visibility/path/format adjustments. Discovery remains exactly 873 tests, and all 873 release hook-enabled tests pass (exit 0). Rust format and all-targets hook-enabled Clippy pass (exit 0).

| Owner | Before lines | After lines | Physical test files |
|---|---:|---:|---:|
| `inference.rs` | 3,726 | 2,225 | 5 |
| `control.rs` | 2,819 | 1,555 | 4 |
| `generation.rs` | 2,452 | 1,202 | 4 |
| `ambient.rs` | 1,573 | 884 | 1 |
| `baseline.rs` | 1,889 | 1,161 | 1 |
| `baseline_control.rs` | 1,804 | 1,029 | 1 |
| `baseline_qualification.rs` | 1,751 | 1,073 | 1 |
| `collision.rs` | 2,866 | 1,675 | 1 |
| `controllers.rs` | 1,796 | 1,075 | 1 |
| `effects.rs` | 2,295 | 1,363 | 1 |
| `evolution.rs` | 2,560 | 1,402 | 1 |
| `evolved_lives.rs` | 2,017 | 1,382 | 1 |
| `external_replacement.rs` | 3,965 | 2,780 | 1 |
| `fixed_step.rs` | 2,553 | 1,623 | 1 |
| `food.rs` | 1,476 | 864 | 1 |
| `generation_handoff_fixture.rs` | 1,480 | 838 | 1 |
| `graph.rs` | 2,235 | 1,513 | 1 |
| `physics.rs` | 2,915 | 1,768 | 1 |
| `queues.rs` | 2,658 | 1,612 | 1 |
| `sensors.rs` | 2,237 | 1,517 | 1 |
| `sensor_qualification.rs` | 2,333 | 1,550 | 1 |
| `spatial.rs` | 2,465 | 1,635 | 1 |
| `spawn.rs` | 2,065 | 1,221 | 1 |

The TypeScript suites split by responsibility, retaining their original suite titles, test registrations, timeout arguments, gating expressions and callback bodies. `checkpointPersistenceFixture.ts`, `rustServerFixture.ts` and `rustServerArchiveFixture.ts` hold their original shared helpers. The persistence `afterEach` cleanup retains client shutdown before temporary-directory deletion. Vitest configuration is unchanged, including serial native/integration files and default per-file isolation. The Linux private-filesystem qualification command now selects the relocated resource cases; its test-name filter is unchanged. Historical test-file references remain historical.

All 132 original TypeScript registrations and all three fixture payloads match apart from fixture exports and scope-aware import organization. TypeScript typecheck and targeted ESLint pass (exit 0). The persistence integration packet passes 83 tests with one platform skip (exit 0). See the [Rust map](modularization-p2-rust-moves.tsv) and [TypeScript registration map](modularization-p2-ts-moves.tsv); these are change receipts, not test-runner registries. The complete Vitest suite passed against both rebuilt addon classes: 1,088 passed, 18 skipped, zero failed (exit 0). The supplied evidence summary reports 1,083 passed; this receipt records the actual current-run count rather than assuming that summary count. All 132 changed registrations and their case tables are preserved exactly, and the before/after relocation checks account for every changed test body.

Production and hook ABI surfaces and production declarations remain unchanged. Native probes again pass new→new exact restore, old→new exact rejection and compatible distinct-run activation. Both addon classes carry source SHA `cccc9cddacb1dd39945dddcf728609765da69daae4a68daea589fab36c3c323c`. These are source-layout identity changes; admission policy is unchanged. PyRL source and its distillation identity are unchanged in P2.


## P8 independent PyRL packet (verified)

PyRL commit `6290fc6` extracts `gae`, `split_rollout_by_policy_version` and `collate_rollouts` into `pyrl_trainer/rollout_processing.py`, and `format_reward_exposure` into `reward_reporting.py`, preserving all historical `learner` imports. The original four function ASTs are unchanged. The learner drops from 914 to 747 lines; the specialized modules are 148 and 40 lines. Actor, optimizer, policy-fence and distillation checkpoint owners are unchanged. The complete packet receipt is `../PyRL-trainer/docs/modularization.md`.

The default Python 3.12.10 pytest run passed 804 tests with 13 skips. Nine skipped cross-repository tests were separately enabled and all passed, covering the baseline 813 passing cases with four optional skips remaining. Before/after discovery retains all 817 IDs. All 29 additional native artifact/teacher tests passed with the intentional very-large-model qualification case deselected. Changed-module CI-options Pylint, compilation, import compatibility and AST checks passed. Distillation source identity inputs and formats are unchanged. A first pytest command used the wrong repository working directory; it failed during unrelated collection and was corrected, as documented in the PyRL receipt.

## P3 state and checkpoint production extraction (verified)

`state.rs` is now 1,812 lines (from 5,919 after P1). Its existing owner and ordered publication methods remain in the facade; ten specialist files contain the contracts, error type, header/config/world/value validation, hashes, memory estimates, publication checks and owned-buffer swap helpers. `checkpoint.rs` is now 1,458 lines (from 5,582); eight specialist files hold state/graph/population codecs, numeric sources and roles, strict USTAR handling, manifest validation, managed weight operations and the existing restore sequences. These are physical files, not inline modules.

All 273 original top-level functions/types/impl payloads match after visibility, engine-path and rustfmt normalization. This includes the retained authoritative-state impl and the unchanged bodies of the moved restore/publication helpers. All 873 release Rust library tests pass (exit 0); all-target hook-enabled typecheck/Clippy and production-library Clippy pass (exit 0). Existing public/crate/engine facade paths are retained through re-exports. Private sibling helper access uses parent-scoped visibility. Engine-visible DTO fields retain their original effective scope; internal codec fields needed across the checkpoint subtree become parent-visible. Three internal compatibility re-export groups and the hook-used graph estimate carry narrow unused-import allowances so production and hook builds retain the same old paths.

The [P3 symbol map](modularization-p3-moves.tsv) records destinations and declaration visibility. New helpers use their owning facade's types and narrowly visible sibling support; they introduce no state owner, locks, worker, deferred commit, scheduler, transport or allocation policy. Checkpoint constants, numeric/compression choices, manifest field order, archive rules and source-identity policy remain unchanged. Native source identity necessarily changes with the new file layout. A production-only unused re-export warning found during the first addon build was corrected with a facade-local allowance; the final addon rebuild includes that source edit and its source handshake/restore probes pass.

## P4 browser graph extraction (partial packet)

Commit `3aee77a` records the first coherent graph packet. The [P4 map](modularization-p4-moves.tsv) records its symbols. `src/graph_templates.ts` contains the six existing graph builders using explicit settings and brain dimensions. `src/graph_diagram.ts` and `src/graph_inspector.ts` contain the existing rendering and event callback bodies. All six builder bodies and both view bodies match after the explicit dependency substitutions; node IDs, defaults, edges, ports and outputs are unchanged. Diagram types and dimensions move with the view; its clamp uses the existing identical implementation in `utils.ts`.

The entry point retains draft, selection, drag state, connection and authoritative graph ownership. View ports use live getters and narrowly scoped setters/callbacks into that owner, so retained event handlers observe current selection and dimensions rather than a copied mutable state. The immutable DOM references are narrowed locally for callback typing. Imported view modules perform no DOM queries, listener registration or session work at import time. Application registration and view invocation retain their order. There is no import back into `main.ts`.

Typecheck and targeted lint pass. The browser integration suite passes all 26 tests, including one new graph-draft/apply/server-replacement/reset scenario; the existing 25 tests continue to pass. The new test was corrected to match the existing Apply behavior (it submits a reset request) and the actual welcome contract. No production behavior was changed to accommodate the test. Preset, graph/settings projection, archive/camera work and the remaining browser test organization are still pending, as are the remaining N-API, Node and selective native packets.


P4's current physical sizes are `main.ts` 4,003 lines, `graph_templates.ts` 176, `graph_diagram.ts` 337 and `graph_inspector.ts` 363. The entry point remains oversized because presets, graph staging/settings, other form rendering and camera/archive composition are still pending; this is not a final size exception. Its existing local graph Apply button continues to submit a reset request. The regression case verifies that draft selection alone does not submit one, then that an authoritative welcome replaces the staged graph before the next reset. Final P3 production source SHA is `d1845ec35ad83e56ae5882267954db85757dc694cfb4c86875bcc2b798dafe35`; production ABI/declarations and all three restore probes pass. The full native confirmation run passed 209 tests with 12 skips (exit 0); both addon ABIs and source handshakes are unchanged apart from the deliberate new source identity.


P3's first full native run passed 208 cases and failed the committed-import/rejoined-controller heading outcome, with 12 skips. Its unchanged helper polls a real socket-driven heading condition for five seconds. The focused import/reset/New Run variants then passed (3/3), the entire lease file passed (8/8), and a fresh full native confirmation passed 209/209 executed cases with 12 skips. No Rust, server or test edits were made between the failure and these reproductions, and neither the deadline nor the assertion was changed. The failure was not reproduced; its precise scheduling cause is unproven. The initial failed result is retained locally alongside the confirmation rather than represented as an uninterrupted green run.


### P4 remaining graph, settings, archive and camera projections (verified)

Seven more physical modules separate graph copying/projection, graph form rendering, presets, core settings UI, opaque archive transfer, packed-frame/player projections and local camera operations. Their symbols are appended to the P4 move map. All 37 moved helper/listener bodies match after the documented argument/port, immutable-DOM alias and internal-call substitutions. No graph IDs, port order, output count, storage keys, request routes, upload object, response parsing, camera equations or thresholds change. DOM queries and event-listener registration remain in the entry point, with the baseline-control registration still called in its original bootstrap position. Views do not import the entry point.

Connection, pending request, reset/staging and mutable camera state remain with the original root. Camera delegation caches one live port so it adds no per-frame port-object allocation. Packed frame helpers receive the original buffer rather than a world clone. Preset HTTP operations call the existing URL/draft/status owner through a small port and preserve their async sequence. Archive upload sends the original File, and export still creates a direct download anchor. The source-AST archive guards now inspect the extracted implementation and additionally check root delegation; their population-materialization and direct-File assertions remain intact.

The first broader P4 unit/integration run found two guards still anchored to main's former archive branches (878 passed, 2 failed, 6 skipped). After the guard relocation update, the focused archive/main suites passed 29 tests, and the full unit/integration confirmation passed 880 tests with 6 skips and zero failures (exit 0). Typecheck, full ESLint and Vite build pass. Browser integration contributes 26 passing cases, including the graph draft/server replacement case. Rust source, native identity and PyRL distillation identity do not change in this browser packet.

| Current browser source | Lines |
|---|---:|
| `src/main.ts` | 2,916 |
| `src/graph_templates.ts` | 176 |
| `src/graph_diagram.ts` | 337 |
| `src/graph_inspector.ts` | 363 |
| `src/graph_state.ts` | 110 |
| `src/graph_settings.ts` | 456 |
| `src/graph_presets.ts` | 137 |
| `src/archive_transport.ts` | 173 |
| `src/settings_ui.ts` | 203 |
| `src/player_input_and_frames.ts` | 171 |
| `src/camera.ts` | 178 |

The remaining main facade requires the P9 residual review: it still composes DOM registration, server/welcome/reconnect callbacks, graph staging, controller input and the shared UI state. Further moves must justify their navigation benefit rather than accumulate unrelated callbacks into a broad application context. The N-API and Node production packets and selective native helper work remain pending; this receipt does not mark the overall plan complete.

## P5 native boundary extraction (verified)

`napi_engine.rs` drops from 6,464 to 1,078 lines. Seven production children separate DTOs, checkpoint conversion, event conversion, existing fresh-run tasks/helpers, hook-only fixture sessions and the independent coarse probe bridge. Its 460-line test suite retains the original logical module. The fresh-run session's annotated methods, lifecycle, operation admission and synchronous guard remain in the facade. Parent-scoped helper/field visibility permits the original owner to access relocated implementation details; existing public/crate facade paths and every macro/feature attribute are retained. The [P5 map](modularization-p5-moves.tsv) records the moved production declarations.

`napi_running_engine.rs` drops from 2,982 to 1,759 lines. Its cached wire projections and bounded request parsers occupy two 337-line files. Existing task-panic and inventory-wire modules move physically to 538- and 34-line test files while retaining their names and cfg gates. `ExperimentalRunningAuthority`, archive progress/task lifetimes, panic containment, cancellation, atomic ownership and Drop remain together. No execution boundary or bridge granularity changes.

All 199 original fresh-run/probe facade items and all 53 running-authority items match after visibility and rustfmt normalization, including attributes. All three test module payloads and gates match. All 873 release hook-enabled library tests pass (exit 0); production-library and hook-enabled all-target Clippy pass with warnings denied. Both production and hook addons load with exactly the original ABI; production TypeScript declarations are byte-identical. Both builds carry source SHA `93fd2b18d94c020c15620fc3653a340e1f72f29e11acdaac0de0487273818ec0`. New→new exact restore succeeds; P3→new exact restore rejects the changed identity; explicitly compatible recovery activates a distinct run. The same probes also pass against the original baseline descriptor. The complete native integration run passes 209 tests with 12 skips and no failures (exit 0), including task panic, cancellation, restore, server and archive/lease scenarios. Rust format and Vite build pass.

## P6 persistence helper extraction (verified)

The client retains worker creation, request correlation, watchdogs, fault/exit handling and Promise settlement. `response_decode.ts` (467 lines) contains the original strict response/preset decoding and validation; the client is 1,185 lines. The tiny descriptor matcher stays with the client.

The worker retains its one database connection, WAL/FULL setup, original startup call order, active leases, request/progress state, commit transactions and dispatch. `schema.ts` (354 lines) accepts that existing handle. Its Hall-of-Fame initializer receives the existing retention callback, invoked at the same point inside the original immediate transaction. `managed_files.ts` (165) receives the canonical root or descriptor limits explicitly; direct-child, symlink, regular-file, length and in-transaction recheck rules are unchanged. `lineage_reads.ts` (144) accepts the same database for bounded ancestry queries and parameterized filters. `read_models.ts` (208) holds browser history/HoF projections and preset operations. Its browser readers receive the existing handle and one validated-boundary callback, which runs inside the original deferred read transaction. No imported helper opens a connection, acquires a lease, installs a handler or performs schema work at import time. The worker remains 2,548 lines and requires the P9 residual ownership/navigation review.

The [P6 map](modularization-p6-moves.tsv) records symbols and additional explicit dependencies. All 144 original top-level statements, types, functions and owner bodies match after export/parameter, stable dependency-argument and validated-boundary callback normalization; SQL and bounded checks are unchanged after newline normalization. TypeScript typecheck and full ESLint pass. The real-worker persistence suites pass 83 tests with one platform skip (exit 0).

The first broader unit/integration run passed 877 cases and failed three existing WebSocket startup cases with HTTP 503 (six skips). It overlapped the final production-addon rebuild. The same three cases passed after the build without source/test changes; the overlap is a plausible explanation, not a proven diagnosis because that first run did not capture the startup fault detail. The complete confirmation passes 880 cases with six skips and no failures (exit 0); the full native run passes 209 with 12 skips (exit 0). A first focused reproduction selected the integration project, where those unit-suffixed files are not discovered; the corrected unit selection ran all three cases. No deadline, assertion or production behavior was changed to obtain the passing reproduction. Vite build passes. The selective Rust helper packet and final residual review remain pending.

## P7 selective native helpers (verified)

Fourteen physical children separate coordinator health atomics, control/running-step diagnostics, borrowed loop progress reports, stable control lookup/source checks, fresh configuration and existing restore policy, replacement fingerprints/lease-token checks, and bounded archive scanning, inventory validation, legacy decoding/SQLite reads and byte/file primitives. The [P7 map](modularization-p7-moves.tsv) records each moved declaration. Workspaces, authority, measurement cursors, step tickets, archive scratch cleanup, assignment handling and publication remain with their original owners. No arithmetic, constants, backend dispatch, numeric ordering, RNG calls, resource admission, archive rules or restore policy changes.

All 393 production function/type/impl/constant payloads match after engine-path, visibility and rustfmt normalization, including optional trailing function/call argument commas. The retained owner impls are included in that comparison. Existing public/crate paths are re-exported; private child support is parent-visible. All 873 release hook-enabled Rust tests pass on the final source, with exactly the P5 test names (exit 0). Both Clippy configurations pass. The first production-only check identified an ungated re-export of two hook-only timing types; the final facade imports preserve their original feature gate. No hook-only type is added to production. Format passes. Both final addons build/load with unchanged ABI and matching source SHA; production declarations and all three restore probes pass. The complete JavaScript/native run passes 1,089 tests with 18 skips, zero failures and exit 0.

The control/running module introductions now describe their subphases and retained owners. Two stale introductory descriptions of ordinary observations waiting on Node receipts were corrected to describe the existing complete bounded-output admission behavior. The ownership-changing assignment receipt boundary is unchanged. These are comment corrections, not a runtime change.

## P9 measured source shape and residual decision

[Per-file measurements](modularization-file-metrics.tsv) cover 197 changed Slither source files plus two related unchanged review files. Before is the original `14264e4` checkout; after is the current source. Counts include blank lines/comments. Test lines are physical test/fixture files plus top-level Rust test-module spans; small out-of-line declarations count as test lines in their owners. This classification differs slightly from the supplied analyzer's treatment of nested cfg blocks, so it is defined here rather than silently mixing its counts with ours. Production fixture implementations compiled outside test modules remain production source. The table below gives physical facade size; the linked data separates production and tests.

| Source | Before lines | Current lines | Static import prefixes before → after |
|---|---:|---:|---:|
| `engine/state.rs` | 13,461 | 1,812 | 54 → 49 |
| `engine/checkpoint.rs` | 7,934 | 1,458 | 28 → 36 |
| `engine/fresh_run.rs` | 5,992 | 1,654 | 40 → 20 |
| `engine/coordinator.rs` | 3,296 | 1,184 | 23 → 17 |
| `napi_engine.rs` | 6,464 | 1,078 | 32 → 40 |
| `napi_running_engine.rs` | 2,982 | 1,759 | 26 → 25 |
| `src/main.ts` | 4,764 | 2,916 | 30 → 39 |
| `checkpointPersistenceClient.ts` | 1,634 | 1,185 | 11 → 9 |
| `checkpointPersistenceWorker.ts` | 3,373 | 2,548 | 17 → 18 |
| `engine/export_archive.rs` | 4,014 | 2,202 | 21 → 31 |
| `engine/external_replacement.rs` | 3,965 | 2,374 | 16 → 16 |
| `engine/control_phase.rs` | 6,198 | 3,468 | 35 → 22 |
| `engine/running_step.rs` | 3,457 | 3,388 | 18 → 20 |
| `engine/running_loop.rs` | 2,250 | 2,081 | 17 → 19 |
| `engine/inference.rs` | 3,726 | 2,225 | 16 → 12 |

Static import prefixes mean distinct TypeScript import/re-export specifiers, or Rust textual `use` prefixes plus directly declared child modules. They include type imports and test imports. They are a reproducible navigation indicator, not a compile-backed call graph or runtime dependency count. In particular, a Rust child's `use super::*` counts once while depending on its parent's named contracts; it does not prove the child has only one semantic dependency. Increased facade fanout is the explicit cost of composing the specialist files. TypeScript helper dependencies point toward contracts and other specialist helpers, with no import back into the browser entry point or persistence worker. No new connection, SQLite, scheduler or mutable graph owner is introduced.

The largest new specialized production file is 949 lines; all new specialized production files are below 1,000 lines. Extracted Rust test files top out at 1,149 lines. The existing main integration suite is 1,205 lines after its additional regression case. The reviewed unchanged native persistence suite is 1,683 lines; it retains its existing scenario/fixture lifetime and is not represented as a new oversized extraction. Rust production children add one physical/lexical module level (engine facade depth 2, children 3); topical test children reach depth 4. The N-API children have depth 2. Browser and Node helper directory depth is unchanged. PyRL remains one package level, with learner 747 lines, rollout processing 148 and reward reporting 40.

Every remaining production file above 2,000 lines has been reviewed:

| Retained source and production lines | Ownership/navigation reason for the exception |
|---|---|
| `control_phase.rs` — 3,466 | The prepare workspace owns borrowed body/pellet/ray indexes, candidate ordering, sensor scratch and the neural batch key; the commit workspace validates/stages the corresponding complete result. Indexed selection, cadence, recurrent reset and proof validation share that lifecycle. Diagnostics and independent source checks are now separate. Dividing the remaining workspace methods would require readers to follow the same mutable scratch/epoch invariants across several impl files; no independent owner or broader context is justified. |
| `running_step.rs` — 3,388 | `advance_nonterminal`, staged world/control publication, pending observations/assignments, generation checkpoint acknowledgement and generation-start rebind operate on the same staged transaction and retained buffers. Timing/allocation records are separate. Keeping admission, rollback and final swap methods together makes the sequence visible; splitting the held operation is deferred. |
| `src/main.ts` — 2,916 | DOM acquisition/registration, WebSocket welcome/reconnect callbacks, request settlement, graph staging, player reclaim and camera state still share one browser session. Rendering, templates, presets, settings forms, camera math and file transfer are separate. Further callback extraction would need a broad live session port or move ownership/timers; the current role-specific ports avoid that navigation/ownership cost. Gesture registration and server-graph reconciliation still require the root, deliberately. |
| `checkpointPersistenceWorker.ts` — 2,548 | Durable import/checkpoint/recovery commits, current-pointer identity, lease acquisition/release, retention/pruning, progress correlation and shutdown share the single synchronous database and mutable lease state. Stateless/schema/read projections are separate. Moving these mutations would either spread each FULL transaction across modules or introduce a broad worker context; that transaction-focused follow-up is explicitly deferred by the plan. |
| `external_replacement.rs` — 2,372 | One replacement workspace owns source proofs, staged bodies/brains/RNG/allocators, per-assignment receipt resolution and final buffer publication. Immutable digests and lease/token checks are separate. Fresh-join construction uses the same placement/genome rules. Splitting the remaining workspace would distribute validation/rollback/publication of those same buffers without removing a state dependency. |
| `inference.rs` — 2,221 | The execution plan, tensor ranges, heterogeneous batch/recurrent commit checks and private scalar/recurrent kernels share checked layouts and backend/cache contracts. Quantized caches and low-level SIMD already have their own modules. A further private-kernel file is technically possible, but common numerical reviews would then cross another file in addition to plan, cache and SIMD implementations. The modest residual is retained as a navigation tradeoff, not claimed to be irreducible or used to justify an inference redesign. |
| `export_archive.rs` — 2,088 | Scanners, manifest/inventory validation, legacy wire/SQLite readers and archive primitives are separate. The remainder retains import preparation, scratch Drop cleanup, collision-safe publication, role extraction and the leased export-writing sequence. These methods share candidate/file lifetimes and cleanup order. The 114 remaining inline test lines are small, preserved legacy-reader regression cases. Further transaction splitting is deferred. |
| `running_loop.rs` — 2,081 | The retained scheduler ticket, archive reservation, staged import, observation/assignment holds, teacher/inspector boundaries and generation acknowledgement/start methods coordinate the same authority. Progress/value reports are separate. A scheduler or import-method split would make the held-state transitions harder to read together; no second loop or reservation owner is introduced. |

`rustServer.ts` remains 1,769 lines. `startRustServer` is the single server composition and replacement/cancellation owner; storage health, replacement settings, recovery/import notices and upload rejection helpers are already explicit functions. Its staged import → durable metadata commit → publication path is retained. PyRL's `distillation_checkpoint.py` is 714 lines: save and load share the bounded ZIP role limits, exact source/environment validation, replay index, optimizer/RNG state and private job witness. Load validates a new private owner before returning it. That serializer is cohesive and has no size or ownership reason for another split; its source-identity list and formats remain untouched. Actor/policy-fence ownership remains as in P8. No S2 redesign is part of this implementation.

### Repeatable navigation review

This is a static source-reading exercise, not a measured model-token or performance benchmark. The figures are whole implementation-file context envelopes for the named task, including inline tests present in those files but excluding separate test files and unchanged shared contracts. Tight symbol/window retrieval would already read less in the old layout; no claim of proportional model-token savings is made.

| Representative task | Original implementation envelope | Current implementation envelope | Implementation files opened |
|---|---:|---:|---:|
| Locate/change a graph example builder | `main.ts`, 4,764 lines | `graph_templates.ts`, 176 | 1 → 1 |
| Review manifest-role validation | `checkpoint.rs`, 7,934 | `manifest_validation.rs`, 624 | 1 → 1; publication behavior additionally reads the facade |
| Review a normalized config admission check | `state.rs`, 13,461 | `config_validation.rs` + `contracts.rs`, 1,353 | 1 → 2 |
| Adjust camera follow math and frame lookup | `main.ts`, 4,764 | `camera.ts` + `player_input_and_frames.ts`, 349 | 1 → 2; event/session changes still read main |
| Review GAE/policy segmentation | `learner.py`, 914 | `rollout_processing.py`, 148 | 1 → 1; actor data contract is unchanged |

These examples show the intended tradeoff: more files when a responsibility crosses a contract, substantially less unrelated source in the common leaf review, and the original sequential owners retained for transaction review. Final native compatibility/integration gates pass as recorded above. P9 completes the residual decision and measurement work; further ownership or scheduling restructuring requires its own design and authorization.
