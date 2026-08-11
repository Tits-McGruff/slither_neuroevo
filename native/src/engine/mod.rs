//! Rust-owned authoritative simulation components.

/// Deterministic scalar calculation work, scratch, and staged-result contracts.
pub mod calculation;
/// Bounded managed checkpoint-v3 codec and immutable-file publication.
pub mod checkpoint;
/// Deterministic test-hook fixture for the real checkpoint publication boundary.
#[cfg(feature = "engine-test-hooks")]
pub mod checkpoint_fixture;
/// Versioned coarse-command and bounded-queue contracts.
pub mod contract;
/// Background coordinator implementation; only its lifecycle is public.
mod coordinator;
/// Bounded engine faults and stable error codes.
pub mod error;
/// Deterministic graph validation, layout, and compilation contracts.
pub mod graph;
/// Safe scalar complete-graph and heterogeneous-population inference.
pub mod inference;
/// Deterministic Stage 4 whole-population inference evidence.
#[cfg(feature = "engine-test-hooks")]
pub mod inference_fixture;
/// Bounded inbound/outbound queues and coalesced wake contracts.
pub mod queues;
/// Versioned deterministic random-number generation shared by engine state.
pub mod rng;
/// One-shot owner of the background coordinator lifecycle.
pub mod runtime;
/// Owned persistent-state and generation-boundary admission contracts.
pub mod state;

pub use coordinator::LifecycleState;
