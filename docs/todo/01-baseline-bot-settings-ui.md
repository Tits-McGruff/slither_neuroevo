# Stage 01: Baseline Bot Settings and UI Controls

## Revision notes

- Clarified import/export behavior for baseline bot settings and aligned worker
  import flow with server-mode reset semantics.
- Added debug playbook and tightened AC-007 test mappings with concrete test
  targets.

## A) Delta vs AGENTS.md

- Changes: add CFG fields and Settings UI controls for baseline bot count and
  seed options; expand SettingsUpdate path union; add path validation for
  imported settings; update `src/main.ts` import flow to apply settings in
  worker mode.
- Unchanged: runtime simulation, World update logic, serialization layout,
  renderer behavior, and worker/server message shapes.
- Contract touch points: worker <-> main settings updates under
  `src/protocol/settings.ts` (type-only change, message shape unchanged).

## B) Scope; non-goals; assumptions; constraints and invariants

- Relevant decisions: DEC-003, DEC-004.
- Relevant invariants: INV-002, INV-003.
- Scope: configuration schema, settings UI, seed control widgets, and import
  application of baseline bot settings.
- Non-goals: bot runtime logic, spawn lifecycle, rendering changes.
- Assumptions: count adds to NPCs; per-bot seeds derived later in runtime.

## C) Architecture overview (stage-local)

- Add `CFG.baselineBots` to `src/config.ts` with defaults that keep bots off by
  default (count 0).
- Extend `src/settings.ts` `SETTING_SPECS` to include a slider for bot count, a
  checkbox for per-generation random seed, a numeric input for base seed, and a
  button to randomize the seed (UI-only action that writes to the seed input
  and triggers a settings update).
- Extend `src/protocol/settings.ts` `SettingsUpdate['path']` union to include
  the new CFG paths and let `setByPath` apply them in worker/server.
- Add a small path-validation helper in `src/main.ts` (or `src/settings.ts`)
  that filters `settings`/`updates` imports to known `SettingsUpdate['path']`
  values before applying them.
- Wire up the seed randomize button in `src/main.ts` to update CFG and emit a
  `updateSettings` message for worker mode and `sendReset` updates for server
  mode.
- Clarify UI semantics: the randomize button only updates the base seed field;
  the per-generation toggle decides whether generation influences the derived
  seed.
- Align worker-mode import behavior in `src/main.ts` to apply `data.settings`
  and `data.updates` before posting the import message, matching server-mode
  reset semantics.

## D) Alternatives considered with tradeoffs

- Embed seed controls in a custom Settings section in `index.html` only:
  rejected because it bypasses the existing data-path settings update pipeline
  and increases risk of divergence between worker/server modes.

## E) Planned modules and functions

### Planned modules/files likely to change

- `src/config.ts`
- `src/settings.ts`
- `src/protocol/settings.ts`
- `src/main.ts`
- `index.html`
- `styles.css`
- Tests: `src/settings.test.ts`, `src/main.test.ts`

### Planned new helpers (signatures and contracts)

`src/main.ts`

- `function applyBaselineSeed(seed: number): void`
  - Input: finite integer >= 0.
  - Output: none; updates seed input and emits settings update.
  - Errors: ignores non-finite or negative values.

- `function randomizeBaselineSeed(): number`
  - Input: none.
  - Output: new seed (32-bit safe integer).
  - Errors: none; uses local RNG (Math.random) only for UI value.

Example usage (illustrative only):

```ts
const seed = randomizeBaselineSeed();
applyBaselineSeed(seed);
```

`src/settings.ts`

- Extend `SETTING_SPECS` with fields:
  - `baselineBots.count` (range slider, integer, default 0).
  - `baselineBots.seed` (numeric input, integer).
  - `baselineBots.randomizeSeedPerGen` (checkbox).

### Validation rules

- Seed must be finite integer; negative values clamp to 0.
- Count clamps to non-negative integer.

## F) Data model changes; data flow; migration strategy; backward compatibility

- New CFG keys under `baselineBots`:
  - `count: number`
  - `seed: number`
  - `randomizeSeedPerGen: boolean`
- SettingsUpdate path union expanded to include these keys.
- Export payloads already include `settings` and `updates` in worker and server
  flows; baseline bot settings are carried via those fields:
  - `settings.baselineBots.count`
  - `settings.baselineBots.seed`
  - `settings.baselineBots.randomizeSeedPerGen`
  - `updates[]` entries with the same paths when sliders or inputs change
- Import behavior:
  - If `settings`/`updates` exist, apply them before import in both worker and
    server modes.
  - If fields are missing, defaults come from CFG_DEFAULT.
  - Unknown settings paths are ignored (validated against the path union).
- No new localStorage keys; `slither_neuroevo_pop` remains genome-only.
- Backward compatibility: older builds ignore unknown settings fields in JSON;
  newer builds accept older files with missing bot settings.

## G) State machine design

### Seed control UI state

State table

| State | Description | Invariants |
| --- | --- | --- |
| Idle | Seed value displayed | value mirrors CFG.baselineBots.seed |
| Editing | User typing seed | invalid input not committed |
| Randomizing | Random seed button pressed | generates finite integer |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| focusSeedInput | Idle | Editing | none | none | no commit yet |
| blurSeedInput | Editing | Idle | valid seed | applyBaselineSeed | CFG updated |
| clickRandomize | Idle | Randomizing | none | applyBaselineSeed | finite seed |
| randomizeDone | Randomizing | Idle | none | none | value shown |

### Import settings application (worker + server)

State table

| State | Description | Invariants |
| --- | --- | --- |
| ImportIdle | No import active | UI reflects current CFG |
| ImportSettingsApply | Applying file settings | only known paths applied |
| ImportInProgress | Import message in flight | settings already applied |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| importSelected | ImportIdle | ImportSettingsApply | file parsed | apply settings/updates | path validation |
| applyDone | ImportSettingsApply | ImportInProgress | reset complete | post import | settings locked |
| importDone | ImportInProgress | ImportIdle | worker/server ack | unlock UI | consistency |

## H) Touch points checklist

- Worker <-> main settings update types:
  - `src/protocol/settings.ts` (path union)
  - `src/main.ts` (updateSettings call paths)
  - `src/worker.ts` (setByPath usage remains generic)
- No buffer, protocol, or sensor layout changes in this stage.

## I) Error handling

- Invalid seed input: ignore and revert to previous value; optionally show a
  small inline hint.
- Missing DOM elements (tests): guard null checks to avoid runtime errors.
- Import payloads with unknown settings paths: drop those entries and continue
  import (no fatal errors).

## J) Performance considerations

- UI-only changes; no impact on hot paths.

## K) Security and privacy considerations

- Seed values are non-sensitive; do not log at info level.

## L) Observability

- Debug log (optional): `ui.botSeed.randomized { seed }` gated by a debug flag.
- Debug toggle: CFG.debug.baselineBots (added in Stage 02) or a UI-only flag
  for settings tracing.

## Debug playbook

- Worker mode: set bot count and seed, click randomize, and verify the seed
  input, CFG path updates, and `updateSettings` messages in worker logs.
- Import a file with and without `settings`/`updates` and confirm defaults are
  applied when missing.

## M) Rollout and rollback plan (merge-safe gating)

- Default `baselineBots.count = 0` keeps runtime unchanged.
- Rollback by removing CFG keys and UI fields; no persisted data to migrate.

## N) Testing plan

- Update `src/settings.test.ts` to include new slider/input specs and assert
  `updateCFGFromUI` handles them
  (`it('applies baselineBots settings paths')`).
- Update `src/main.test.ts` DOM stubs to include new seed input/button IDs and
  confirm `initWorker` posts updates without throwing.
- Add `src/main.test.ts` import test: file with bot settings triggers a reset
  in worker mode before `import` is posted
  (`it('import applies bot settings before worker import')`).
- Add negative test: unknown settings paths are ignored (path validation)
  (`it('ignores unknown settings paths on import')`).
- Validation commands:
  - `npm run test:unit` (covers settings + main tests)
  - `npm test` (CI parity)
  - CI still runs `npm run build` and `npm run typecheck`; stage changes must
    keep them green.
- AC mapping:
  - AC-007 -> `src/main.test.ts` / `import applies bot settings before import`
    and `src/settings.test.ts` / `updateCFGFromUI supports baselineBots paths`.

## O) Compatibility matrix

- Server mode: changed, ok (new settings paths accepted; import applies them).
- Worker fallback: changed, ok (new settings paths applied via updateSettings).
- Join overlay: unchanged, ok.
- Visualizer streaming: unchanged, ok.
- Import/export: changed, ok (settings fields applied in worker + server
  imports; verified by `src/main.test.ts`).

## P) Risk register

- Risk: seed input not wired to settings update path -> mitigated by tests in
  `src/settings.test.ts` and `src/main.test.ts`.
