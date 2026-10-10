# ADR 0003: Retire the TypeScript reference runtime

- Status: accepted by the owner's explicit implementation request on 2026-10-10
- Decision: retain one Rust-authoritative game with TypeScript client/interface/tooling
- Archive commit: `abe20d695a4c8fbd5ae73f50973280c2c0801bbc`
- Frozen branch: `codex/archive/ts-reference-and-rust-2026-10-10`
- Annotated tag: `archive/ts-reference-and-rust-2026-10-10`

## Boundary

Retire TS simulation/neural execution, Node BrainPool and standalone neural
N-API exports. Preserve Rust internal SIMD, pure graph helpers, browser state,
Protocol 2, native persistence/legacy conversion and trusted-LAN operations.
Neutral old config keys warn; active/malformed keys and old flags reject.
Gameplay, numerics, worker policy, formats and recovery behavior remain unchanged.
Deferred laptop/performance work and production experimental naming remain separate.

## Restore the combined source

Use a separate checkout so current owner data and work are not disturbed:

```sh
git fetch origin tag archive/ts-reference-and-rust-2026-10-10
git worktree add --detach ../slither-combined-archive archive/ts-reference-and-rust-2026-10-10
cd ../slither-combined-archive
npm install
npm --prefix native run build
```

Run `npm run server` for archived Rust or `npm run server:reference` for archived
TS; the archived reference also supports deliberate `--backend js` diagnosis.
Use separate disposable database paths when investigating historical behavior.
The tag/branch preserve committed source, not ignored databases/config/builds.

## Evidence

Keep native source-identity fixtures and frozen spawn/movement/death/ambient/
baseline-control expectations with original bytes and provenance. Their TS
generators live at the tag above. Live source comments identify that archive.
Validation uses native regressions, scalar/SSE2, worker continuation, compatibility
and actual client/server contracts. See the factual implementation log for results.
