//! Rust-owned authoritative simulation components.

/// Staged once-per-fixed-step generation and live-snake scalar accounting.
pub mod accounting;
/// Cold generation-boundary release of unused allocator pages.
mod allocator_maintenance;
/// Staged fixed-step ambient-pellet accumulation and world-RNG generation.
pub mod ambient;
/// Durable baseline-slot lifecycle and pre-control respawn timing.
pub mod baseline;
/// Pure baseline strategy evaluation from shared corrected observations.
pub mod baseline_control;
/// Declared-information v4 baseline policy with isolated behavior continuation.
pub mod baseline_v2;
/// Deterministic scalar calculation work, scratch, and staged-result contracts.
pub mod calculation;
/// Process-available logical CPU limits for the Rust calculation pool.
pub mod calculation_workers;
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
/// Bounded reliable ordinary-step observations and exact local-send receipts.
mod controller_output;
/// Wall-time external-controller leases and staged control-source decisions.
pub mod controllers;
/// Background coordinator implementation; only its lifecycle is public.
mod coordinator;
/// Reusable committed frame storage and replaceable basic display stats.
pub mod display;
/// Deterministic boost/corpse pellet realization and isolated RNG continuation.
pub mod effects;
/// Bounded engine faults and stable error codes.
pub mod error;
/// Serial TypeScript-compatible fitness, selection, crossover, and mutation.
pub mod evolution;
/// Population-slot physical lives and ordered economic accounting.
pub mod evolved_lives;
/// Private bounded in-place population replacement and publication witnesses.
mod evolved_replacement;
/// Constant-sized committed life diagnostics for replaceable display stats.
pub mod evolved_telemetry;
/// Bounded versioned experiment-history and assessment records.
pub mod experiment_records;
/// Self-contained save archive assembly from one leased managed checkpoint.
pub mod export_archive;
/// One-shot archive completion/validation failures confined to the test addon.
#[cfg(feature = "engine-test-hooks")]
pub(crate) mod export_failure_fixture;
/// Atomic external-controller death replacement staging.
pub mod external_replacement;
/// Reusable corrected prefix of one complete authoritative fixed step.
pub mod fixed_step;
/// Deterministic contested-food claims and post-food body finalization.
pub mod food;
/// Direct browser-compatible binary display-frame v1 packing.
pub mod frame_v1;
/// Bounded current-default generation-one population and durability staging.
pub mod fresh_run;
/// Durable pre-spawn next-generation boundary preparation.
pub mod generation;
/// Retained real generation-persistence handoff integration fixture.
#[cfg(feature = "engine-test-hooks")]
pub mod generation_handoff_fixture;
/// Collision-safe running-world construction after a durable generation boundary.
pub mod generation_start;
/// Versioned TypeScript-compatible random genome initialization.
pub mod genome;
/// Ordered God Mode mutations at clean authoritative boundaries.
pub mod god_mode;
/// Deterministic graph validation, layout, and compilation contracts.
pub mod graph;
/// Bounded shared source-graph JSON decoder used by admission and offline tools.
pub mod graph_json;
/// Safe scalar complete-graph and heterogeneous-population inference.
pub mod inference;
/// Deterministic Stage 4 whole-population inference evidence.
#[cfg(feature = "engine-test-hooks")]
pub mod inference_fixture;
/// Bounded immutable origin of a population imported from an older representation.
pub mod legacy_origin;
/// Atomic runtime-setting replacement at a clean authoritative boundary.
pub mod live_settings;
/// Staged steering, boost, movement, and packed body-point proposals.
pub mod movement;
/// Complete multi-substep physics working transaction and stable outcome staging.
pub mod physics;
/// Bounded inbound/outbound queues and coalesced wake contracts.
pub mod queues;
/// Transaction-staged fair work grants for sensor-v4 capsule rays.
pub mod ray_work;
/// Versioned deterministic random-number generation shared by engine state.
pub mod rng;
/// Durable generation-one checkpoint and running-authority activation barrier.
pub mod run_start;
/// Retained real run-start handoff integration fixture.
#[cfg(feature = "engine-test-hooks")]
pub mod run_start_handoff_fixture;
/// Retained scheduler/coordinator ownership across authoritative steps.
pub mod running_loop;
/// Private complete nonterminal fixed-step orchestration and publication.
mod running_step;
/// One-shot owner of the background coordinator lifecycle.
pub mod runtime;
/// Rust-owned one-step-at-a-time fixed-step scheduler and wall-debt accounting.
pub mod scheduler;
/// Deterministic Stage 4 corrected-sensing performance evidence.
#[cfg(feature = "engine-test-hooks")]
pub mod sensing_fixture;
/// Canonical structural sensor identity, separate from numeric environments.
pub mod sensor_contract;
/// Pure sensor-v4 capsule, wall and projected head-motion geometry.
pub mod sensor_geometry;
/// Sensor-v3 labels, offsets, and input-size contract.
pub mod sensor_layout;
/// Actual native workload timing and bounded committed-observation histograms.
#[cfg(feature = "engine-test-hooks")]
pub mod sensor_qualification;
/// Bounded reusable sensor-only traversal over complete body segments.
pub mod sensor_rays;
/// Corrected sensor-v3 construction and delivered-observation boundaries.
pub mod sensors;
/// Complete Basic/Motion candidate observations for native qualification.
pub mod sensors_v4;
/// Complete derived body-segment and pellet spatial indexes.
pub mod spatial;
/// Deterministic collision-safe initial-body placement with bounded fallback.
pub mod spawn;
/// Owned persistent-state and generation-boundary admission contracts.
pub mod state;
/// Strict admitted-settings projection for one complete running step.
pub mod step_config;
/// Deterministic Stage 5 complete scalar fixed-step performance evidence.
#[cfg(feature = "engine-test-hooks")]
pub mod step_fixture;
/// Thread-local panic injection compiled only into Rust unit tests.
#[cfg(test)]
pub(crate) mod task_panic_fixture;
/// Pending exact teacher observations retained from shared control evaluation.
pub mod teacher_capture;
/// Nonblocking owner handoff to the separate diagnostic transport drain.
pub mod teacher_mailbox;
/// Separate bounded diagnostic queue for immutable teacher chunks.
pub mod teacher_queue;
/// Cold automatic teacher-capture admission from current native authority.
pub mod teacher_runtime;
/// Bounded Rust-owned committed-teacher chunk encoding and structural validation.
pub mod teacher_samples;
/// Committed physical-stream assembly without authority or trainer coupling.
pub mod teacher_stream;
/// Canonical cold training metadata derived from the authoritative graph compiler.
pub mod training_graph;
/// Bounded offline complete-graph inference using the deployed native executor.
#[cfg(any(test, feature = "engine-test-hooks"))]
pub mod training_runner;
/// Worker-local archive byte counters visible to the Node watchdog.
pub mod work_progress;
/// Complete post-control world-step staging before one authority publication.
pub mod world_step;

pub use coordinator::LifecycleState;
pub use running_step::{
    ExternalDeliveryDiagnostics, ExternalDeliveryEventKind, ExternalDeliveryResolution,
    ExternalDeliveryResult, ExternalDeliveryState, ExternalObservationBatch,
    ExternalObservationEvent, GenerationReassignmentProgress, GenerationTransitionBatch,
    GenerationTransitionReason, RunningStepCoordinator, RunningStepError, RunningStepInputs,
    RunningStepOutcome, RunningStepProgress, RUNNING_STEP_COORDINATOR_VERSION,
};
pub use scheduler::{
    FixedStepScheduler, FixedStepSchedulerDiagnostics, FixedStepSchedulerPolicy, ScheduledStep,
    SchedulerError, SchedulerReadiness, SchedulerServiceMode, FIXED_STEP_SCHEDULER_VERSION,
};
pub use state::GenerationStartPublication;
pub use world_step::ExternalDeliveryStatus;
