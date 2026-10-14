# Portable brain fixtures

These synthetic `.slither-brain` files are frozen protocol fixtures. They are
separate from the five historical source-identity fixtures; no historical bytes
or generators are replaced.

- `portable-brain-v1.slither-brain` retains the existing Basic B8 GRU8/Dense2
  graph, 1,962 parameters and eight recurrent state values.
- `portable-dense-basic8-v1.slither-brain` uses the native qualification graph:
  Input `input` (72 values), Dense `motor` (72 to 2), explicit zero ports,
  Basic B8 sensors, 0.06-second cadence, Preserve, and the canonical native
  initializer with isolated seed 42. It has 146 parameters and no recurrence.
  The Rust `canonical-training-graph` tool produced its manifest and weights;
  PyRL-trainer's `encode_brain_artifact` packaged the original bytes with manual
  test provenance. Its complete SHA-256 is
  `b91462645db00f28e1e2506a685ddee41bb8cb1ac52f8f863b42f19bdc690943`.

Actual addon tests independently recompile and validate both portable contracts.
TypeScript transports these bytes without parsing or executing their models.
