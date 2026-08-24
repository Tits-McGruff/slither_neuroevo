//! Rust-owned authoritative simulation components.

/// Staged once-per-fixed-step generation and live-snake scalar accounting.
pub mod accounting;
/// Staged fixed-step ambient-pellet accumulation and world-RNG generation.
pub mod ambient;
/// Durable baseline-slot lifecycle and pre-control respawn timing.
pub mod baseline;
/// Pure baseline strategy evaluation from shared corrected observations.
pub mod baseline_control;
/// Deterministic scalar calculation work, scratch, and staged-result contracts.
pub mod calculation;
/// Bounded managed checkpoint-v3 codec and immutable-file publication.
pub mod checkpoint;
/// Deterministic test-hook fixture for the real checkpoint publication boundary.
#[cfg(feature = "engine-test-hooks")]
pub mod checkpoint_fixture;
/// Immutable swept collision detection and stable death/award proposals.
pub mod collision;
/// Versioned coarse-command and bounded-queue contracts.
pub mod contract;
/// One staged corrected-sensing and heterogeneous-neural control operation.
pub mod control;
/// Combined corrected-sensing and heterogeneous-control performance evidence.
#[cfg(feature = "engine-test-hooks")]
pub mod control_fixture;
/// Shared corrected observation boundary and exclusive controller selection.
pub mod control_phase;
/// Wall-time external-controller leases and staged control-source decisions.
pub mod controllers;
/// Background coordinator implementation; only its lifecycle is public.
mod coordinator;
/// Deterministic boost/corpse pellet realization and isolated RNG continuation.
pub mod effects;
/// Bounded engine faults and stable error codes.
pub mod error;
/// Reusable corrected prefix of one complete authoritative fixed step.
pub mod fixed_step;
/// Deterministic contested-food claims and post-food body finalization.
pub mod food;
/// Deterministic graph validation, layout, and compilation contracts.
pub mod graph;
/// Safe scalar complete-graph and heterogeneous-population inference.
pub mod inference;
/// Deterministic Stage 4 whole-population inference evidence.
#[cfg(feature = "engine-test-hooks")]
pub mod inference_fixture;
/// Staged steering, boost, movement, and packed body-point proposals.
pub mod movement;
/// Complete multi-substep physics working transaction and stable outcome staging.
pub mod physics;
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
/// Deterministic collision-safe initial-body placement with bounded fallback.
pub mod spawn;
/// Owned persistent-state and generation-boundary admission contracts.
pub mod state;
/// Strict admitted-settings projection for one complete running step.
pub mod step_config;
/// Complete post-control world-step staging before one authority publication.
pub mod world_step;

pub use coordinator::LifecycleState;
