# Modularization integration and validation

This integrates the accepted P0–P9 implementation; it does not start another modularization pass. The implementation and independent/original-plan acceptance records remain in `docs/modularization.md` and `docs/modularization-postreview.md`.

## Preserved histories and integration strategy

On 8 November 2026 the refreshed remote tips were Slither `c9d3d2f2c458815356b7c7b9ab4b4a183b8473e0` and PyRL `b1c9d8b67f0905571b8f4372ed0a042d744a6575`. Slither had 14 modularization-only commits after `14264e4`; remote main had 13 commits outside that local history. PyRL's accepted `6290fc6502b06fa5c504c1e13ddf163acf224e5f` was one commit ahead, with no remote-only work.

Both repositories preserve local branches `backup/modularization-accepted-20261108`, `backup/modularization-origin-before-fetch-20261108`, and `backup/modularization-origin-fetched-20261108`. Slither also preserves `backup/modularization-reviewed-20261108`, including the previously untracked final review report, committed as `00df48066ecba2ba254ec670c77c821e81bb38b6`. A local copy of the original document remains under `.git/integration-20261108`. No other uncommitted user work existed. These backup branches remain local.

The dedicated branch in each repository is `integration/modularization-20261108`. Slither merge `c7b723e733e75f7d2e2b8a90b72a2ea3374b7541` has parents `00df480` and remote main `c9d3d2f`. This preserves all accepted modularization commits and all remote history without replaying duplicate feature-branch fixes or rewriting either main. PyRL retains its original accepted commit. The default Slither fetch encountered a stale configured remote branch; an explicit fetch of `refs/heads/main:refs/remotes/origin/main` refreshed the intended ref successfully without changing fetch configuration.

## Disposition of all 13 remote-only Slither commits

Every commit below is now an ancestor of the integration merge. None was dropped or rewritten.

| Commit | Purpose and affected responsibilities | Integration disposition |
|---|---|---|
| `8dee6f8` | Full-precision cadence controls and 60 Hz preset; setting definitions/UI/tests, README and native server cadence assertions. | Retained. Newer cadence assertions follow the relocated server scenario; original setting modules were not modularized. |
| `dad192a` | Cryptographic request IDs usable on trusted-LAN HTTP; browser main, brain-library/distillation clients and request-ID tests. | Retained. Main keeps modularized imports plus the new request-ID dependency; finish-generation uses the new helper. |
| `e058d66` | Saved-brain founders: native population construction and memory admission, durable provenance/state encoding, N-API optional argument, server leases/New Run, browser library, protocols/docs and tests. | Retained in the accepted physical layout. This is the substantive behavioral addition relative to the reviewed refactor, already present on remote main. |
| `1057645` | Readable library founder action and styling. | Retained unchanged. |
| `d51e8b2` | Exclude generated test trees; HTTP fixture transfers use fresh sockets and explicit body headers. | Retained; shared transfer helper updated where modularization relocated it. |
| `4a01582` | Native settings fixture supports preset event bindings. | Retained in `rustServer.settings.native.test.ts`. |
| `9e91646` | Bounded comparison of large portable buffers; native lifecycle and founder tests. | Retained: Buffer equality assertions, not object expansion. |
| `c89a6a3` | Equivalent cadence fixture fix on the other feature branch. | Existing merge history retained; fixture behavior appears once. |
| `dca7194` | Equivalent bounded buffer/discovery fixes on the other feature branch. | Existing merge history retained; final behavior appears once. |
| `0768cd4` | Merge the two feature-branch fix histories. | Retained as ancestry; not cherry-picked as a separate patch. |
| `efbd3bd` | Merge PR #15, large-brain lifecycle branch. | Retained as ancestry. |
| `1a75c0c` | Merge PR #16, saved-brain founders. | Retained as ancestry. |
| `c9d3d2f` | Merge PR #17, reward/action repair baseline. | Retained as merge parent; its reward/action behavior was already in local baseline `14264e4`. |

Relative to `14264e4`, remote main changes 39 files (1,150 insertions/64 deletions). Its changes are independent features/fixes, not an alternative modularization. The accepted layout overlaps state/checkpoint/fresh-run and native/server/browser test owners, so a plain merge requires relocation-aware resolution rather than choosing one whole side.

## Conflict resolutions and combined-source review

Seven paths conflicted: `checkpoint.rs`, `fresh_run.rs`, `generation.rs`, `state.rs`, `napi_running_engine.rs`, `rustServer.native.test.ts`, and `main.ts`.

- State founder identity/validation/text accounting follow `state/contracts.rs`, `admission_header.rs`, and `memory_budget.rs`. Fixture defaults remain in physical fixture modules. Authority/publication methods are retained.
- Founder encoding v5 follows `checkpoint/state_wire.rs`; the constant remains at the checkpoint facade. Non-founder encoding remains v4, and the remote bounded/versioned corruption regression is in `roundtrip_and_versions.rs`.
- Fresh-run founder construction/contract checks remain with the existing preparation owner; its new tests join `founders_and_qualification.rs`. Ordinary random initialization passes `None`; founder initialization preserves remote slot-zero weights, ordinary mutation, recurrent zeroing, source/RNG and memory preflight behavior.
- Running N-API tasks retain remote's optional founder argument and existing admission/cancellation/cleanup. The task panic fixture follows its existing physical module.
- The new founder archive/New Run scenario joins `rustServer.archive.native.test.ts`; cadence scenario changes remain in their existing root/settings suites. Fixture HTTP behavior moves to `rustServerFixture.ts`; existing archive-download and large-brain fixtures are imported directly. No suite or assertion is discarded or duplicated.
- Main retains root mutable session/graph ownership, live view ports, remote founder callbacks and the request-ID helper. No eager listener/module initialization is introduced.

A declaration comparison against remote main covers 2,923 parsed items in the examined native owner trees, including retained owner implementations and relocated tests. Residuals are the same reviewed path/import/formatting cases, with no unexplained algorithm or lifecycle difference. TypeScript changes preserve remote test callback bodies and update only imports/fixture routing to the accepted layout. Compilation catches missing fixture imports/fields before final validation; those integration-harness omissions were corrected without weakening tests.

The combined behavior differs from the earlier accepted refactor because remote main adds founders/cadence/request-ID behavior. It does not add a third behavior beyond those two histories. Review specifically checked founder source charging before numeric allocation, existing state adoption/rollback, bounded checkpoint provenance, same SQLite commit-before-publication sequence, founder lease release, browser welcome/disconnect/result ordering, and the unchanged PyRL actor/reward contracts.

## Validation and compatibility

Local validation ran on source-bearing merge `c7b723e733e75f7d2e2b8a90b72a2ea3374b7541`. The following integration-record commit changes documentation only. Native source identity changes because remote source adds founder support; the source selection/admission policy itself is unchanged. The independently recomputed source digest and both rebuilt addon classes match `df9de11e991eb1437b102675200ec0385662b6db46d0ae589da3f81bc8b84390`. Local build identifier is `slither_native/0.1.0+c7b723e733e7.0b789706de41a34e`; CI rebuilds have their own Git/toolchain/target identity and do not imply exact cross-build continuation.

| Local gate | Actual result |
|---|---|
| Hook-enabled all-target Rust compilation | Passed. |
| Production addon release build/load | Passed. |
| Release hook-enabled Rust library suite | 878 passed, zero failures/ignored; includes all five upstream founder regressions. |
| All-target hook-enabled Clippy, warnings denied | Passed. |
| Rustfmt check | Passed. |
| TypeScript check, full ESLint, Vite build | All passed. |
| Full Vitest unit/integration/native run | 1,098 passed, 18 skipped, zero failures; process exit 0. |
| Production/hook JavaScript export and prototype snapshots | Both match the accepted baseline. |
| Production declarations | Only remote main's intended optional `founderBrainId` argument and its explanatory comment differ from the accepted declaration snapshot. |
| Current exact restore | Passed in a disposable run. |
| Baseline/pre-P7 exact rejection and explicitly compatible recovery | Both reject exact restore; each compatible recovery activates a distinct run. |
| Python qualification-helper suite | 29 passed. |
| PyRL default suite | 804 passed, 13 skipped; same default gating as accepted implementation. |
| PyRL live reward/native-evaluator cases against this Slither source | All nine passed. |
| PyRL native artifact/teacher cases | 29 passed, intentionally large `real_250th` case deselected. |
| Canonical optimizer/export/native/reimport parity | All 12 CPU roundtrips passed using tools built from the integrated source. |
| Changed PyRL module CI-options Pylint and compilation | Passed; 10.00/10 lint. |

Logs and generated reports remain local under `.git/integration-20261108`; generated binaries, checkpoints and corpora are not committed. An initial Vitest startup was cancelled before executing the suites while native builds were incomplete; the complete successful run above used the rebuilt source-matching addon. Expected panic-injection diagnostics are covered by the final successful JSON result, not treated as ordinary source failures.

Cross-repository CI will pin the exact published Slither integration revision in a separate PyRL workflow commit. Slither routine CI covers Windows/Node 24, Linux/Node 24, Linux/Node 26 unit compatibility and Linux Rust lint. PyRL Quality covers Linux Python 3.10/3.14 and Python 3.12/Node 24 interoperability. Scientific/manual qualification remains distinct from these routine checks.

Both PRs target remote `main` without automatic merge. CI status and exact final PR heads are recorded in the PR descriptions and integration completion response, so updating a status note does not continually invalidate the checked source revision. Until those jobs complete, local Windows success is not Linux/Node 26/Python-matrix evidence. Long scientific runs, private-filesystem Linux qualification, and the deliberately excluded very-large-model test remain explicit qualification gaps rather than routine local passes.
