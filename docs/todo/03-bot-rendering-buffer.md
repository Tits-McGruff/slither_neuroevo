# Stage 03: Rendering and Buffer Skin Flag for Robot Bots

## Revision notes

- Added explicit equality semantics for skin flag handling and a negative test
  requirement for unknown skin values.
- Tightened rollback notes to match current renderer behavior and added a
  debug playbook.

## A) Delta vs AGENTS.md

- Changes: extend skin flag domain to include a robot value, update
  serialization and fast renderer to use metallic color + robot eyes, and keep
  God Mode buffer parsing aligned.
- Unchanged: buffer layout (offsets and block sizes), worker/server protocol
  message shapes, and sensor sizing.
- Contract touch points: AGENTS.md “Binary frame format and rendering
  pipeline” (serializer, renderWorldStruct, main-thread parsing).

## B) Scope; non-goals; assumptions; constraints and invariants

- Relevant decisions: DEC-004.
- Relevant invariants: INV-001, INV-003, INV-009.
- Scope: serializer skin flag encoding, renderer color/eye styling, God Mode
  parsing alignment, and tests.
- Non-goals: bot runtime logic, seed behavior, or stats aggregation.
- Assumptions: skin flag remains numeric and is interpreted only via strict
  equality checks; baseline bots emit skin=2 only after this stage lands.

## C) Architecture overview (stage-local)

- Update `src/serializer.ts` to set `skin` from a snake property (e.g.
  `snake.skin` or `snake.role`) and encode `2` for robot bots.
- Update `src/render.ts` to interpret `skin === 2` as metallic silver with
  robot eyes, and keep `skin === 1` as gold.
- Update `src/theme.ts` with robot colors (body, eye, glow) and use them in
  renderer.
- Ensure `src/main.ts` God Mode parsing still reads `skin` at offset 2 and
  remains compatible with the expanded skin values.
- Enforce strict equality checks (`skin === 1`, `skin === 2`) to avoid
  truthiness bugs that could render robot skins as gold.

## D) Alternatives considered with tradeoffs

- Encode robot color via `snake.color` only and keep skin flag binary:
  rejected because eye styling needs a stable, fast-path flag.

## E) Planned modules and functions

### Planned modules/files likely to change

- `src/serializer.ts`
- `src/render.ts`
- `src/theme.ts`
- `src/main.ts` (God Mode parsing checks only)
- Tests: `src/serializer.test.ts`, `src/render.test.ts`, `src/main.test.ts`

### Planned function updates (signatures and contracts)

`src/serializer.ts`

- `WorldSerializer.serialize(world)`
  - Input: SerializableWorld with snakes that may include `skin` or `role`.
  - Output: Float32Array with skin value 0/1/2.
  - Error cases: none; defaults to 0 if missing.

`src/render.ts`

- `getSnakeColor(id: number, skin: number): string`
  - Return metallic color when skin === 2.
  - Use strict equality (`skin === 1`, `skin === 2`) to avoid truthiness bugs.
- `drawSnakeStruct(...)`
  - If skin === 2, draw robot eyes (e.g., square pupils or glow) using
    new theme colors.

Example usage (illustrative only):

```ts
const color = getSnakeColor(meta.id, meta.skin);
const eyeColor = meta.skin === 2 ? THEME.snakeRobotEye : THEME.snakeSelfEye;
```

## F) Data model changes; data flow; migration strategy; backward compatibility

- Buffer layout unchanged; skin flag value 2 added.
- Expand/migrate/contract steps:
  - Expand: update renderer to accept 2 before serializer emits it.
  - Migrate: serializer starts emitting 2 for baseline bots.
  - Contract: not applicable; keep support for 0/1/2.
- Backward compatibility: the current renderer uses `skin === 1` checks, so
  unknown values (including 2 on older builds) fall back to default colors. If
  a legacy renderer used truthy checks, robots could appear gold; this stage
  requires strict equality.

## G) State machine design

### Rendering skin selection

State table

| State | Description | Invariants |
| --- | --- | --- |
| skinDefault | Standard snake | color from hashColor |
| skinGold | HoF/gold skin | gold color |
| skinRobot | Baseline bot skin | metallic + robot eyes |

Transition table

| Event | From | To | Guards | Side effects | Invariants enforced |
| --- | --- | --- | --- | --- | --- |
| skinFlag0 | any | skinDefault | skin === 0 | default colors | layout stable |
| skinFlag1 | any | skinGold | skin === 1 | gold colors | layout stable |
| skinFlag2 | any | skinRobot | skin === 2 | robot colors | layout stable |

## H) Touch points checklist

- Binary frame buffer layout consumers (AGENTS.md “Binary frame format”):
  - `src/serializer.ts` (writer)
  - `src/render.ts` (reader)
  - `src/main.ts` (God Mode parsing)
  - `src/protocol/frame.ts` (unchanged offsets)
- Tests:
  - `src/serializer.test.ts` for skin flag values
  - `src/render.test.ts` for robot eye drawing calls
  - `src/render.test.ts` negative test for unknown skin values
  - `src/main.test.ts` if parsing assumptions need updates

## I) Error handling

- No new runtime errors expected; missing skin values default to 0.

## J) Performance considerations

- No new allocations in render loop; reuse existing drawing calls.
- Avoid per-frame object creation for robot eye styling.

## K) Security and privacy considerations

- None beyond existing rendering pipeline.

## L) Observability

- Optional debug flag to show skin id on selection (God Mode) when enabled.
- Debug log (gated): `render.skin.unknown { skinValue, snakeId }` when a skin
  value is not 0/1/2.

## Debug playbook

- Enable baseline bots and confirm robot skins render with metallic color and
  robot eyes, while gold skins remain unchanged.
- Force a snake to use an unknown skin value in a test harness and confirm the
  renderer falls back to default colors (no gold).

## M) Rollout and rollback plan (merge-safe gating)

- Gated by baseline bot count; skin flag 2 appears only when bots are active.
- Rollback: if renderer reverts, serializer can keep emitting 2 without crash;
  robots will appear as default color with the current equality checks. If
  rolling back to a renderer that uses truthy checks, robots may appear gold;
  mitigate by gating skin=2 emission behind renderer version.

## N) Testing plan

- `src/serializer.test.ts`
  - Add test: serialize robot skin value 2 when snake role is baseline bot.
- `src/render.test.ts`
  - Add test: robot skin triggers robot eye color/shape calls.
  - Add test: unknown skin values do not map to gold and fall back to default.
- `src/main.test.ts`
  - Update if needed to accept skin value 2 in selection parsing.
- Validation commands:
  - `npm run test:unit`
  - `npm test`
  - CI still runs `npm run build` and `npm run typecheck`; stage changes must
    keep them green.
- AC mapping:
  - AC-006 -> `src/serializer.test.ts` / `serializes robot skin value 2` and
    `src/render.test.ts` / `robot skin draws robot eyes` +
    `unknown skin falls back to default`.

## O) Compatibility matrix

- Server mode: ok (buffer layout unchanged, renderer updated).
- Worker fallback: ok (same renderer and buffer path).
- Join overlay: unchanged, ok.
- Visualizer streaming: unchanged, ok.
- Import/export: unchanged, ok.

## P) Risk register

- Risk: renderer and serializer desync on skin values. Mitigation: update both
  in the same stage and assert in tests.
