# Phase 6 plan: Graph-based brain layouts

## Purpose and scope

This phase introduces graph-based brain definitions that allow arbitrary
layouts such as splits, skips, and reordered layers. The graph is compiled into
an efficient forward pass with preallocated buffers. Structural mutation is not
part of this phase; only weight mutation is supported.

## Architecture narrative

Graph brains are specified as DAGs where nodes represent operations and edges
represent data flow. The compiler validates the graph, allocates parameter
slices in a single weight array, and produces a deterministic execution order.
The runtime executes that order and writes outputs into preallocated buffers to
avoid per-tick allocations. This approach preserves performance while enabling
complex layouts.

## Decisions locked for Phase 6

The graph is a static DAG with no cycles. Supported nodes are Dense, MLP, GRU,
LSTM, RRU, Split, and Concat. Output heads are explicit nodes, weights are
stored in a single Float32Array, and only weight mutation is supported.

## Graph schema

Graph definition format:

```json
{
  "nodes": [
    { "id": "n1", "type": "Input", "outputSize": 24 },
    { "id": "n2", "type": "MLP", "inputSize": 24, "outputSize": 32, "hiddenSizes": [16] },
    { "id": "n3", "type": "GRU", "inputSize": 16, "outputSize": 16 },
    { "id": "n4", "type": "Dense", "inputSize": 16, "outputSize": 2 }
  ],
  "edges": [
    { "from": "n1", "to": "n2" },
    { "from": "n2", "to": "n3" },
    { "from": "n3", "to": "n4" }
  ],
  "outputs": ["n4"]
}
```

Rules: the Input node has no incoming edges, every edge output size matches the
destination input size, and outputs refer to valid nodes.

## Compiler pipeline

1) Validate the DAG using Kahn's algorithm.
2) Validate shape compatibility across edges.
3) Allocate parameter slices for each node in a single weight array.
4) Produce a topological execution order.
5) Allocate output buffers for each node once.

The compiler returns a `CompiledGraph` containing the execution order, parameter
slices, and buffer references used by the runtime.

## Runtime execution

Runtime iterates the execution order. Each node reads inputs from predecessor
buffers, applies its operation, and writes output to its buffer. `Split` writes
into multiple buffers, and `Concat` builds a unified output buffer by copying
inputs in a deterministic order.

## Detailed design notes

Validation is strict because graph errors can be subtle and hard to debug at
runtime. The validator should report the first error with a clear message
including node id and edge details so graph authors can fix issues quickly.
All shape mismatches must be caught at compile time, not at runtime.

Buffer reuse is critical. Every node should own a fixed output buffer that is
allocated once at compile time. Operations like `Concat` should write into a
preallocated buffer in a stable order, and `Split` should write into existing
buffers rather than allocating new ones each tick. This preserves performance
and reduces GC churn in large populations.

Topological ordering must be deterministic across runs for the same graph
definition. If multiple nodes have the same dependency level, order them by
stable id to avoid nondeterminism in evolution experiments.

## Module layout

```text
src/brains/graph/
  schema.ts
  validate.ts
  compiler.ts
  runtime.ts
  ops.ts
```

## Tests

Tests ensure the validator rejects cycles and missing nodes, rejects shape
mismatches, the compiler produces deterministic parameter slices, and the
runtime produces the correct output length.

## Footguns

Avoid per-node allocations inside the runtime. Concat order must be
deterministic across runs, and node ordering must be stable to avoid
nondeterminism in evolution.

## Acceptance criteria

Graph definitions compile without errors, the graph runs inside the World loop,
and the output size matches `CFG.brain.outSize`.

## Execution checklist

- [ ] Define schema types
- [ ] Implement validator
- [ ] Implement compiler
- [ ] Implement runtime
- [ ] Implement ops for each node type
- [ ] Register GraphBrain in the registry
- [ ] Add tests

## Function-by-function pseudocode

### `validate.ts`

```text
function validateGraph(graph):
  ensure nodes list is non-empty
  ensure ids are unique
  build adjacency list
  run Kahn's algorithm to detect cycles
  for each edge:
    ensure from/to nodes exist
    ensure size compatibility
  return ok or error
```

### `compiler.ts`

```text
function compileGraph(graph):
  order = topologicalSort(graph)
  slices = assignParamSlices(order)
  buffers = allocateBuffers(order)
  return { order, slices, buffers }
```

### `runtime.ts`

```text
function runGraph(compiled, input):
  set input buffer
  for node in compiled.order:
    outputs = node.forward(node.inputs, node.params)
    write outputs to node buffer
  return concat outputs of output nodes
```

## Error handling and edge cases

If the validator finds a cycle, it returns a descriptive error including the
cycle path. If a node references a missing input, validation fails before
compile. If an edge output size does not match the destination input size, the
compiler aborts with a detailed error that includes node ids and sizes. Runtime
should never need to check shapes if validation passed.

## Sample payloads and example WS session transcript

Example graph JSON:

```json
{
  "nodes": [
    { "id": "n1", "type": "Input", "outputSize": 24 },
    { "id": "n2", "type": "MLP", "inputSize": 24, "outputSize": 32, "hiddenSizes": [16] },
    { "id": "n3", "type": "GRU", "inputSize": 16, "outputSize": 16 },
    { "id": "n4", "type": "Dense", "inputSize": 16, "outputSize": 2 }
  ],
  "edges": [
    { "from": "n1", "to": "n2" },
    { "from": "n2", "to": "n3" },
    { "from": "n3", "to": "n4" }
  ],
  "outputs": ["n4"]
}
```

Example WS session transcript (no new messages in this phase):

```text
client -> server: {"type":"hello","clientType":"ui","version":1}
server -> client: {"type":"welcome",...}
server -> client: <binary frame>
```

## Test matrix

| Test name | Setup / input | Expected result | Failure cases to verify |
| --- | --- | --- | --- |
| graph_cycle | Graph with cycle A->B->A | Validation error | Compiler hangs or infinite loop |
| shape_mismatch | Edge from 8-dim to 16-dim | Validation error | Runtime crash |
| deterministic_order | Same graph twice | Same execution order | Non-deterministic order |
| output_size | Output nodes combine to CFG.outSize | Output matches | Off-by-one size |
