# Rust-authoritative runtime factual implementation log

This is the short factual log required by
`docs/todo/rust-authoritative-runtime-plan.md`. It records approvals, commits,
evidence artifacts, measured results, and deviations. It does not restate the
plan or turn an unfinished stage into a completed one.

## Approved plan

- Owner approval received: 2026-07-29.
- Exact approved revision: `2026-07-29-draft-4`.
- Git commit containing the exact reviewed plan:
  `7971ed2ddbda86891c77def31d980aedf96b4236`.
- Plan blob in that commit:
  `bad8dafd8304fd5f81c0f37eb812fba8f2adb2da`.
- Approval scope: the architecture, migration path, persistence/archive
  design, 22 recorded owner decisions, corrections, tests, evidence gates, and
  acceptance requirements in Draft 4.
- Earlier drafts `2026-07-23-draft-1`, `2026-07-28-draft-2`, and
  `2026-07-28-draft-3` were superseded and were not approved.

## Implementation entries

| Date | Stage | Commit | Evidence/result | Known issue or deviation |
|---|---|---|---|---|
| 2026-07-29 | Approval and Stage 1 start | Plan: `7971ed2ddbda86891c77def31d980aedf96b4236` | Owner explicitly approved `2026-07-29-draft-4`; working tree began Stage 1 from implementation base `46c2f634c4cf48b1c1d30b7b55e3373495773d4e`. | None. No earlier draft is approved; no Stage 1 exit gate is yet claimed. |
| 2026-07-29 | Stage 1 architecture record and Git-history evidence | `c489c7e` | Corrected active instructions/README, superseded ADR 0001 and the false recovery-plan authority, added ADR 0002, and retained exact Git commit/blob/command evidence in `docs/todo/evidence/2026-07-29-stage1-git-history.md`. The false-document, historical 10/20-tick controller, and commit `8330065` claims all reproduced from Git and remain distinct from current-source proof. | Runtime repairs and the Stage 1 exit gate remain open. |
| 2026-07-29 | Stage 1 body sensing and collision-index admission | `ac43cde2f0c4913c0b416f41f60077b33f019caf` | Replaced the obsolete body-sensor adapter with the production callback grid contract; neural, baseline, and external observation paths now pass a real-grid body fixture. Collision-grid rebuilds preflight and grow capacity, reject unsafe admission instead of truncating, clear stale references, and expose load/capacity/peak/rebuild/growth/admission/fault diagnostics through stats and health. A 200,001-segment regression retains and queries every entry. Focused tests: 43 passed; world/core/server integration: 27 passed; TypeScript and changed-file ESLint passed. | The 32-Mi-entry ceiling is a temporary TypeScript-reference implementation limit, not a Rust capacity decision. Stage 1 controller, scheduler, browser-input, and reliable-message repairs remain open. |
