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
/// One staged corrected-sensing and heterogeneous-neural control operation.
pub mod control;
/// Combined corrected-sensing and heterogeneous-control performance evidence.
#[cfg(feature = "engine-test-hooks")]
pub mod control_fixture;
/// Wall-time external-controller leases and staged control-source decisions.
pub mod controllers;
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
/// Deterministic Stage 4 corrected-sensing performance evidence.
#[cfg(feature = "engine-test-hooks")]
pub mod sensing_fixture;
/// Sensor-v3 labels, offsets, and input-size contract.
pub mod sensor_layout;
/// Corrected sensor-v3 construction and delivered-observation boundaries.
pub mod sensors;
/// Complete derived body-segment and pellet spatial indexes.
pub mod spatial;
/// Owned persistent-state and generation-boundary admission contracts.
pub mod state;

pub use coordinator::LifecycleState;
