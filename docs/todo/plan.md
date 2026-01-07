# Plan

Make the brain graph editor feel sane: click-to-edit, drag-to-connect, inferred sizes, and a simple output selector, while keeping the existing graph spec and runtime behavior intact. The work focuses on editor UI/UX and data synchronization in `src/main.ts`, with minimal schema changes.

## Scope

- In: click-to-edit selection, drag-to-connect with highlighting, size inference with read-only inputs, simplified outputs (advanced hidden), slider disablement messaging, template alignment, tests/docs updates.
- Out: brain runtime math changes, protocol changes, persistence format changes, legacy renderer updates.

## Architecture notes

- Editor state and wiring live in `src/main.ts` (graph draft, selection, diagram rendering).
- Graph schema is in `src/brains/graph/schema.ts` and runtime validation in `src/brains/graph/compiler.ts`.
- UI layout and controls are defined in `index.html` and `styles.css`.
- README is user-facing; update the Brain graph editor section to match new behavior.

## Implementation details

### Size inference and read-only inputs

Goal: infer input sizes from edges and enforce read-only input sizes in the UI, avoiding user overrides that drift from wiring.

Planned helper (new in `src/main.ts`):

```ts
/** Inferred sizing for graph editor validation and UI. */
interface GraphSizeInfo {
  inputSize: number | null;
  outputSizes: number[] | null;
}

/** Infer sizes from the draft spec, returning per-node sizes and errors. */
function inferGraphSizes(spec: GraphSpec): {
  sizes: Map<string, GraphSizeInfo>;
  errors: string[];
}
```

Inference rules:

- Input: outputSizes = [CFG.brain.inSize] (or spec input node output size if stored).
- Dense/MLP: outputSizes = [node.outputSize], inputSize inferred from incoming edge.
- GRU/LSTM/RRU: outputSizes = [node.hiddenSize], inputSize inferred from incoming edge.
- Concat: outputSizes = [sum of incoming outputSizes].
- Split: outputSizes = node.outputSizes, inputSize inferred from incoming edge.

Sync strategy:

- On every draft change, compute `sizes` and update node `inputSize` to the inferred size for nodes that require one (Dense/MLP/GRU/LSTM/RRU).
- If input is unresolved or mismatched, surface an error banner and mark the node visually (no silent mismatch).
- Keep input size fields read-only in the inspector and list editor (replace with display-only text).

### Output selection (simple vs advanced)

Goal: make outputs obvious for turn + boost.

Simple outputs UI in `index.html`:

- Select: `Output node` (single select).
- Toggle: `Split into 2 outputs` (off by default).

Behavior:

- If toggle off: `outputs = [{ nodeId, port: 0 }]` and `node.outputSize` must be 2.
- If toggle on: `outputs = [{ nodeId, port: 0 }, { nodeId, port: 1 }]` and node must be a Split with two 1-sized outputs (or show error).
- Keep advanced outputs editor hidden in a `<details>` block; it remains the source of truth for JSON import/export and advanced wiring.

Planned UI wiring (in `src/main.ts`):

```ts
function applySimpleOutputs(spec: GraphSpec, nodeId: string, split: boolean): void;
function refreshSimpleOutputUi(spec: GraphSpec): void;
```

### Click-to-edit and connect UX

Goal: remove the need to toggle Connect mode just to edit, and make connections feel direct.

UX changes:

- Always allow click-to-select nodes/edges/outputs; inspector updates immediately.
- Add drag-to-connect from a node: pointerdown on node starts a connection, pointerup over a target node completes it.
- Add hover highlight for valid targets and show a temporary edge line while dragging.
- Keep the Connect button as a fallback (sets a "connect mode" flag), but selection should remain active.

Planned event flow (in `src/main.ts`):

```ts
function beginGraphConnect(fromId: string, pointer: { x: number; y: number }): void;
function updateGraphConnect(pointer: { x: number; y: number }): void;
function endGraphConnect(targetId: string | null): void;
```

Ports:

- Auto-assign `fromPort` for Split outputs and `toPort` for Concat inputs, as today.
- If ports are ambiguous, show a small inline note in the inspector with the chosen order.

### Slider disablement when graph is active

Goal: avoid confusion when stack-builder sliders are ignored by a custom graph.

Behavior:

- When `customGraphSpec` is active, disable the stack-builder sliders (MLP layer count, layer sizes, recurrent size toggles).
- Keep global brain sliders enabled (control dt, mutation settings).
- Add an inline message: "Custom graph active; stack sliders are ignored."

Planned UI wiring:

```ts
function setGraphModeUiState(isGraphActive: boolean): void;
```

## Action items

[ ] Audit `src/main.ts`, `index.html`, and `styles.css` for current graph editor controls, connect flow, outputs UI, and slider wiring.
[ ] Add `inferGraphSizes` and related helpers, and call them on every draft change to keep node input sizes in sync.
[ ] Replace input size fields with read-only display values in both the inspector and list editor; keep output sizes editable.
[ ] Add the simple outputs UI and wire it to update `spec.outputs`; tuck the advanced outputs editor into a `<details>` block.
[ ] Implement drag-to-connect with hover highlight, keep Connect mode as a fallback, and ensure selection always works by click.
[ ] Add visual error styling for size mismatches and unresolved sizes in the diagram and inspector.
[ ] Add `setGraphModeUiState` to disable stack-builder sliders when a custom graph is active, with clear copy.
[ ] Update templates/default graph creation to align with inferred sizes and simple outputs; verify `validateGraph` passes.
[ ] Update README Brain graph editor section with new behavior and clarify that outputs represent turn + boost.
[ ] Add TSDoc for new/modified functions and any new module-level constants.
[ ] Update tests and add new ones for size inference, output wiring, and UI state changes.

## Tests and validation

- Update `src/main.test.ts` to cover:
  - simple output selection updates `spec.outputs`.
  - read-only input size fields render inferred sizes.
  - graph-active state disables the stack-builder sliders.
- Add unit tests for size inference in a new `src/brains/graph.editor.test.ts` (or extend `src/main.test.ts` if preferred).
- Run `npm test` and ensure graph validation tests still pass.

## Open questions

- None.
