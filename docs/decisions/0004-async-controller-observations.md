# ADR 0004: Ordinary external observations use asynchronous bounded output

- Status: accepted by owner direction
- Decision date: 2026-11-03

## Context

The Rust-authoritative runtime originally treated every ordinary external sensor observation like an ownership-changing controller assignment. A fixed step retained its scheduler ticket until Node attempted each WebSocket send and returned local delivery receipts to Rust. With several PPO controllers this inserted a serial Rust-to-Node-to-Rust barrier after every fixed step, leaving the calculation workers idle even though the observation itself had already been computed.

## Decision

Ordinary controller observations use an ordered asynchronous stream. Before a fixed step mutates authority, the background coordinator waits for and reserves enough bounded Rust reliable-output capacity for the complete possible controller result. It then fully owns and validates the output payload. Once the payload fits the reserved slot, Rust commits the observation delivery markers in the staged world and publishes the event into the reserved slot. A step with only observations also commits authority and the scheduler ticket before exposing that event to Node.

A failed later local WebSocket admission is handled as an exact asynchronous disconnect using the original connection and lease identities. Stale failures cannot disconnect a newer lease. Sensor observations remain FIFO and are never silently coalesced or replaced. Bounded Rust output and bounded per-socket Node queues remain the backpressure mechanism.

Controller ownership-changing operations remain receipt-driven transactions. This includes fresh joins, same-snake reclaims, controlled-death replacement assignments, generation-start assignments and generation publication. A mixed step holds authority and the scheduler ticket until the replacement assignments resolve. Observations for surviving controllers are admitted in the same bounded event and require no receipt.

Protocol 2 clients receive `sensors` payloads and send fire-and-forget held actions. The internal Rust/Node bridge routes admitted `controllerObservations` and receipt-driven replacement `controllerMessages` separately, including when one event carries both.

`points_delta_norm` and external inspector rows use admission to the bounded authoritative observation stream as the delivery boundary. The sensor sample remains the same stable pre-movement observation of that fixed step.

## Consequences

- Routine PPO traffic no longer blocks every authoritative step on Node delivery receipts.
- Output memory stays bounded; a sufficiently slow consumer eventually creates backpressure or disconnects instead of creating an unbounded backlog.
- Controlled replacement and generation ownership semantics retain their existing exact local-send receipts.
- JSON serialization remains on Node and may become the next measurable bottleneck at larger controller counts; changing Protocol 2 encoding is a separate decision.
