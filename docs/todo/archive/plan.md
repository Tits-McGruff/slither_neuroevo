# TypeScript conversion plan (full conversion)

Goal: convert the full runtime to TypeScript to maximize static context, while keeping runtime behavior and performance identical. This plan stages the work to preserve the buffer contract and minimize regressions.

## Guiding principles

- Behavior and performance remain unchanged; types should not alter logic.
- Preserve the worker/frame contract exactly (offsets and ordering).
- Convert in dependency order so compile errors surface early.
- Keep typed arrays and hot loops intact; do not introduce allocations.
- Every phase ends with tests and a manual smoke check.

## Scope definition

- Convert everything under `src/` to `.ts`, including hot loops (`world`, `snake`, `mlp`).
- Add shared protocol/contract types in `src/protocol/`.
- Add `tsconfig.json` and typecheck scripts.

## Phase 0: Prep and constraints

Checklist:

- [x] Add `tsconfig.json` with strict settings, `noEmit`, `allowJs: false`.
- [x] Add `tsconfig.json` with strict settings, `noEmit`, `allowJs: false`.
- [x] Add `npm run typecheck` script and wire into CI.
- [x] Add a short TS policy section to `.github/copilot-instructions.md`.

Deliverable: TS toolchain is configured but no files converted yet.

## Phase 1: Protocol and shared types

Create a new `src/protocol/` folder for shared types/constants used by both main thread and worker.

Checklist:

- [x] Create `src/protocol/messages.ts` defining all worker message payloads and stats shapes.
- [x] Create `src/protocol/frame.ts` defining frame header offsets + typed reader helpers.
- [x] Create `src/protocol/settings.ts` with slider path unions and `SettingsUpdate` types.
- [x] Update tests to import shared protocol types/constants where helpful.

Acceptance: no runtime changes yet; tests pass.

## Phase 2: Boundary modules (IO + serialization)

Convert the modules that define contracts and external IO first.

Checklist:

- [x] Convert `src/serializer.js` → `src/serializer.ts`.
- [x] Convert `src/render.js` → `src/render.ts`.
- [x] Convert `src/settings.js` → `src/settings.ts`.
- [x] Convert `src/storage.js` → `src/storage.ts`.
- [x] Ensure JSON import/export payload types are explicit and stable.

Acceptance: buffer layout unchanged; tests pass; visual output unchanged.

## Phase 3: Main + worker

Convert the main thread and worker modules to enforce message contracts.

Checklist:

- [x] Convert `src/worker.js` → `src/worker.ts`.
- [x] Convert `src/main.js` → `src/main.ts`.
- [x] Make message switch statements exhaustive and typed.
- [x] Type `stats` payload, `fitnessHistory`, and HoF sync paths.

Acceptance: worker/main behavior unchanged; no missing message cases.

## Phase 4: Simulation core (hot loops)

Convert the core simulation while preserving logic and memory behavior.

Checklist:

- [x] Convert `src/world.js` → `src/world.ts`.
- [x] Convert `src/snake.js` → `src/snake.ts`.
- [x] Convert `src/mlp.js` → `src/mlp.ts`.
- [x] Ensure typed arrays stay as `Float32Array` and avoid new allocations.
- [x] Keep performance-sensitive helpers inline where they already are.

Acceptance: full simulation parity; no new GC churn.

## Phase 5: Remaining modules

Convert the rest of the codebase for completeness.

Checklist:

- [x] Convert `src/utils.js`, `src/sensors.js`, `src/spatialHash.js`, `src/particles.js`, `src/theme.js`, `src/BrainViz.js`, `src/FitnessChart.js`, `src/chartUtils.js`, `src/hallOfFame.js`, and all tests.
- [x] Update import paths and ensure build/test consistency.

Acceptance: project builds/tests with only `.ts` sources.

## Phase 6: Documentation updates

Update docs to reflect the TS conversion.

Checklist:

- [x] Update `.github/copilot-instructions.md` with TS policy, new `src/protocol/` folder, and new file extensions.
- [x] Update `README.md` only for user-facing paths if needed (avoid dev-only details).
- [x] Remove or update references to `.js` filenames where they changed.

Acceptance: docs match code layout and no stale references remain.

## Risk map / pitfalls

- Buffer contract must not change; validate offsets with tests.
- Worker messages must remain JSON-serializable.
- Avoid widening types to `any` in core hot paths.
- Beware circular type imports between protocol and runtime modules.

## Verification matrix

- Unit tests: `npm test`.
- Manual runtime: open app, ensure frames arrive, visualizer renders, HoF updates, import/export works.
- Regression checks: camera zoom, God Mode selection, pellet colors/types, boosting trails.

## Rollout checklist (per phase)

- [x] Build passes (`npm run build`).
- [x] Tests pass (`npm test`).
- [x] No buffer layout change.
- [x] No new allocations in hot loops.
- [x] Docs updated when file extensions change.
