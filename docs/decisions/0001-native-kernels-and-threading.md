# ADR 0001: Keep Rust kernel-only and threading independent

- Status: accepted
- Decision date: 2026-07-21
- Last verified: 2026-07-22

## Context

The recovery audit found two competing ideas in the repository: a native addon
that already provided neural-network math kernels, and an unfinished proposal
for a second implementation of the complete simulation world in Rust. It also
found paths where enabling worker threads could silently change or disable the
selected math backend.

The Node server is the authoritative owner of world state, command ordering,
physics, sensors, evolution, persistence, and networking. Maintaining another
world implementation would duplicate those contracts and make deterministic
replay harder to reason about.

## Decision

Rust has one production responsibility: N-API kernels for Dense, MLP, GRU,
LSTM, and RRU operations. Rust does not own the world, physics, sensors,
spawning, evolution, persistence, rendering, or networking. The browser is a
rendering and control client and does not run a local simulation.

The math backend and worker threading are independent configuration axes:

| Math backend | No worker pool | Worker pool |
| --- | --- | --- |
| Native kernels | Supported and normal | Supported |
| JavaScript reference | Diagnostic only | Diagnostic only |

Normal startup selects `native` and fails with build instructions if the addon
cannot be loaded. `--backend js` is an explicit diagnostic choice; it is not a
silent fallback. `--mt` enables the canonical Node worker pool without changing
the selected backend, and `--mt-workers N` requests its bounded worker count.

Each population slot has one stable recurrent-state owner for a pool epoch. A
worker failure or protocol mismatch faults the authoritative run: the failed
step publishes no frame, stats, or checkpoint, and the server does not switch
to another backend. Recovery requires an explicit Reset, New Run, or process
restart from a valid generation-boundary checkpoint.

## Determinism boundary

Exact whole-world replay is required only for the same source revision,
versioned RNG and snapshot formats, graph and settings, backend build, target
architecture, supported environment, completed-step count, and ordered action
log. Native-versus-JavaScript comparisons cover bounded kernel operations with
declared numeric tolerances because floating-point evaluation order can differ.

## Consequences

- Native safety work stays concentrated at the N-API buffer boundary.
- World behavior has one implementation and one fixed-step ordering contract.
- CI must prove native and multi-threaded execution together rather than infer
  activation from configuration.
- A future full-world Rust rewrite or a silent fallback policy requires a new
  architecture decision; it is not an incremental extension of this design.
