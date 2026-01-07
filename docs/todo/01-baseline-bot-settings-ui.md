# Stage 01: Baseline Bot Settings and UI Controls

## A) Delta vs AGENTS.md

- Changes: add CFG fields and Settings UI controls for baseline bot count and
  seed options; expand SettingsUpdate path union and UI bindings in
  `src/main.ts` to handle seed randomization button and input validation.
- Unchanged: runtime simulation, World update logic, serialization layout,
  renderer behavior, and worker/server message shapes.
- Contract touch points: worker <-> main settings updates under
  `src/protocol/settings.ts` (type-only change, message shape unchanged).

## B) Scope; non-goals; assumptions; constraints and invariants

- Relevant decisions: DEC-003, DEC-004.
- Relevant invariants: INV-002, INV-003.
- Scope: configuration schema, settings UI, and seed control widgets.
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
- Wire up the seed randomize button in `src/main.ts` to update CFG and emit a
  `updateSettings` message for worker mode and `sendReset` updates for server
  mode.

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
- No new localStorage keys and no persistence schema changes in this stage.
- Backward compatibility: older builds ignore unknown paths; this stage does not
  write new persisted formats.

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

## J) Performance considerations

- UI-only changes; no impact on hot paths.

## K) Security and privacy considerations

- Seed values are non-sensitive; do not log at info level.

## L) Observability

- Debug log (optional): `ui.botSeed.randomized { seed }` gated by a debug flag.

## M) Rollout and rollback plan (merge-safe gating)

- Default `baselineBots.count = 0` keeps runtime unchanged.
- Rollback by removing CFG keys and UI fields; no persisted data to migrate.

## N) Testing plan

- Update `src/settings.test.ts` to include new slider/input specs and assert
  `updateCFGFromUI` handles them.
- Update `src/main.test.ts` DOM stubs to include new seed input/button IDs and
  confirm `initWorker` posts updates without throwing.
- Validation commands:
  - `npm run test:unit` (covers settings + main tests)
  - `npm test` (CI parity)
- AC mapping:
  - AC-007: settings export/import retains bot settings -> tests in
    `src/settings.test.ts` and `src/main.test.ts` for settings update paths.

## O) Compatibility matrix

- Server mode: unchanged, ok (settings paths accepted but unused).
- Worker fallback: unchanged, ok (settings paths accepted but unused).
- Join overlay: unchanged, ok.
- Visualizer streaming: unchanged, ok.
- Import/export: unchanged, ok (settings fields included in payload).

## P) Risk register

- Risk: seed input not wired to settings update path -> mitigated by tests in
  `src/settings.test.ts` and `src/main.test.ts`.
