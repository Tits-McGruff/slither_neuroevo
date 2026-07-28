# ADR 0002: Rust owns the authoritative game

- Status: accepted by owner approval of plan `2026-07-29-draft-4`
- Decision date: 2026-07-29
- Implementation state: staged migration in progress

## Context

The current branch runs the authoritative simulation in TypeScript and crosses
into Rust for individual neural kernels. Every evolved snake has different
weights and recurrent state, so production calls those nominally batched
kernels with a count of one while TypeScript continues to schedule graph
nodes, sensing, movement, collision, and evolution.

The deployed program is intended for a Debian VM on the owner's trusted home
LAN. Laptop and desktop browsers are remote rendering/control clients, and a
separate desktop RL trainer connects through Protocol 2.

## Decision

Rust will own one persistent authoritative game:

- fixed-step scheduling and overload state;
- world, snakes, bodies, pellets, scores, and controllers;
- corrected sensing and spatial indexes;
- complete heterogeneous neural inference and recurrent state;
- movement, food, collision, death, spawning, and evolution;
- run identity, authoritative RNG streams, and allocator state;
- generation-boundary checkpoint construction and validation;
- binary display-frame packing.

Node remains a thin TypeScript interface for trusted-LAN HTTP, WebSocket,
static-file and archive-byte routing plus a dedicated SQLite-metadata worker.
The browser remains TypeScript for rendering, UI, camera presentation, and
input collection. The separate RL trainer remains an external Protocol 2
client for the first cutover.

The Rust/Node boundary is coarse. It does not cross per snake, graph node,
neural layer, or fixed-step subsystem.

New large checkpoint state lives in immutable managed files as packed binary
data. SQLite stores metadata, current pointers, compact history, graph/config
records, Hall-of-Fame indexes, and file references. Browser import/export uses
ordinary direct file transfer and never reconstructs a population in browser
JavaScript.

## Migration rule

The current TypeScript game remains a selected reference and test oracle while
each subsystem is studied, characterized, corrected where required, ported,
and integrated. It is not a silent fallback after the Rust-authoritative path
starts, and production cutover occurs only after the approved correctness,
performance, LAN-browser, RL-trainer, persistence, recovery, and overnight
growth gates pass.

Implementation proceeds forward from the current branch. It does not revert
the branch or restore the incomplete historical Rust game.

## Consequences

- ADR 0001 is superseded and cannot be used to preserve the kernel-only
  boundary.
- Existing TypeScript behavior is evidence to inspect, not permission to copy
  known defects into Rust.
- Material changes to gameplay, controller behavior, selection pressure,
  architecture, persistence meaning, retention/deletion, compatibility, the RL
  contract, or user-visible rules require owner review.
- Trusted-LAN operation remains required; public-Internet hardening remains
  outside scope.
