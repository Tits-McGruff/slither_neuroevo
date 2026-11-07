# Modularization implementation

The governing plan is the owner's evidence-verified revision dated 7 November 2026. The full objective includes packets P0–P9; the initial test extraction alone does not complete it.

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
