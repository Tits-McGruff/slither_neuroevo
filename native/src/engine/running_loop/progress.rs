//! Borrowed progress and delivery reports; the retained loop owns admission and scheduling.

use super::*;

/// Current ability of one retained running-authority loop to accept a step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunningAuthorityLoopState {
    /// The next drained command boundary may service the scheduler.
    Ready,
    /// A fresh assignment awaits delivery before its snake can become current.
    ControllerJoinPending,
    /// A same-snake reconnect assignment awaits its exact local-send result.
    ControllerReclaimPending,
    /// One complete staged step awaits exact reliable-delivery results.
    ExternalDeliveryPending,
    /// One terminal step awaits checkpoint metadata and successor admission.
    GenerationTransitionPending,
    /// A population-work hold bounds allocation and its later durability transaction.
    ImportPending,
    /// A first unrecoverable loop error permanently ended this instance.
    Faulted,
}

/// Caller-owned optional output for one successful authoritative publication.
pub struct RunningFramePublication<'buffer> {
    /// Presentation-only values echoed into frame-v1 header fields.
    pub view: FrameV1ViewDescriptor,
    /// Reusable storage retained by the future background frame publisher.
    pub output: &'buffer mut Vec<u8>,
}

impl<'buffer> RunningFramePublication<'buffer> {
    /// Bind one presentation descriptor to caller-owned reusable storage.
    #[must_use]
    pub fn new(view: FrameV1ViewDescriptor, output: &'buffer mut Vec<u8>) -> Self {
        Self { view, output }
    }
}

/// Owned scalar result of one post-command-drain service opportunity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RunningAuthorityLoopProgress {
    /// No scheduler ticket exists while a fresh assignment is unresolved.
    ControllerJoinPending,
    /// No scheduler ticket exists while a reconnect assignment is unresolved.
    ControllerReclaimPending,
    /// Population work owns the source hold through preparation and durability.
    ImportPending,
    /// No complete fixed delta is due yet.
    Idle {
        /// Simulated seconds still needed for the next complete step.
        simulation_seconds_until_step: f64,
        /// Equivalent real-wall seconds at the admitted multiplier.
        wall_seconds_until_step: f64,
    },
    /// One exact scheduler ticket and authoritative step published.
    Published {
        /// Monotonic scheduler ticket identity.
        ticket_sequence: u64,
        /// Complete steps represented by retained debt before this publication.
        due_steps: usize,
        /// Exact authority publication correlated with the ticket.
        publication: RunningStepPublication,
        /// Frame metadata when this service boundary requested frame packing.
        frame: Option<FrameV1Metadata>,
    },
    /// A complete staged step is blocked on reliable external delivery.
    ExternalDeliveryPending {
        /// Scheduler ticket retained until matching delivery resolution.
        ticket_sequence: u64,
        /// Exact unresolved event count.
        remaining: usize,
    },
    /// A terminal step is blocked on its durable generation handoff.
    GenerationTransitionPending {
        /// Scheduler ticket retained through persistence and successor admission.
        ticket_sequence: u64,
        /// Exact terminal source identity.
        source_key: PhysicsStepKey,
        /// Rule that ended the generation.
        reason: GenerationTransitionReason,
        /// Fully admitted successor generation identity.
        successor_generation: u64,
        /// Successor completed-step chronology.
        successor_completed_step: u64,
    },
}

/// Immutable managed descriptor and exact Rust-constructed generation record.
///
/// Both values originate from the retained admitted transition. The thin
/// persistence bridge may encode and commit them, but cannot supply or alter
/// generation statistics, identities, slots, or Hall-of-Fame values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningGenerationCheckpointPublication {
    pub workload: Option<crate::engine::generation_work::GenerationWork>,
    /// Descriptor for the immutable managed checkpoint file.
    pub descriptor: CheckpointDescriptor,
    /// Independently retained content-addressed elite weights.
    pub hall_of_fame_weights: HallOfFameWeightsDescriptor,
    /// Exact compact history and Hall-of-Fame reference admitted by Rust.
    pub commit_record: GenerationCommitRecord,
}

/// State reached after applying exact local-send results to a retained batch.
#[derive(Clone, Debug, PartialEq)]
pub enum RunningAuthorityDeliveryState {
    /// No retained delivery batch exists; every supplied result was stale.
    Idle,
    /// One ordinary fixed step still awaits reliable delivery results.
    RunningStepPending {
        /// Retained scheduler ticket identity.
        ticket_sequence: u64,
        /// Exact unresolved event count.
        remaining: usize,
    },
    /// The ordinary fixed step published and its scheduler ticket retired.
    RunningStepPublished {
        /// Retained scheduler ticket identity.
        ticket_sequence: u64,
        /// Complete steps represented by debt at the original service boundary.
        due_steps: usize,
        /// Exact authority publication correlated with the ticket.
        publication: RunningStepPublication,
        /// Frame metadata when post-delivery frame packing was requested.
        frame: Option<FrameV1Metadata>,
    },
    /// Next-generation assignments still await one or more exact results.
    GenerationAssignmentsPending {
        /// Retained terminal scheduler ticket identity.
        ticket_sequence: u64,
        /// Exact unresolved assignment count.
        remaining: usize,
    },
    /// All required generation-start assignments resolved while old authority
    /// remains current until the explicit final publication call.
    GenerationAssignmentsReady {
        /// Retained terminal scheduler ticket identity.
        ticket_sequence: u64,
        /// Exact terminal source identity.
        source_key: PhysicsStepKey,
        /// Fully admitted successor generation identity.
        successor_generation: u64,
        /// Successor completed-step chronology.
        successor_completed_step: u64,
    },
}

/// Exact accounting and owned scalar state after delivery-result submission.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningAuthorityDeliveryResolution {
    /// Previously-unaccepted exact events accepted by this call.
    pub matched_acceptances: usize,
    /// Previously-unresolved exact events whose local send failed.
    pub matched_failures: usize,
    /// Stale, unknown, replaced, or duplicate results ignored.
    pub ignored_results: usize,
    /// Retained or published state after applying the results.
    pub state: RunningAuthorityDeliveryState,
}

/// One complete durable-boundary-to-running publication and scheduler rebind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningGenerationStartResolution {
    /// Retired terminal scheduler ticket identity.
    pub ticket_sequence: u64,
    /// Complete steps represented by debt at the terminal service boundary.
    pub due_steps: usize,
    /// Exact final authority publication.
    pub publication: GenerationStartPublication,
    /// Frame metadata when post-swap frame packing was requested.
    pub frame: Option<FrameV1Metadata>,
}

pub(super) fn generation_pending_progress(
    step: ScheduledStep,
    batch: GenerationTransitionBatch<'_>,
) -> RunningAuthorityLoopProgress {
    let successor = batch.candidate();
    RunningAuthorityLoopProgress::GenerationTransitionPending {
        ticket_sequence: step.sequence(),
        source_key: batch.source_key(),
        reason: batch.reason(),
        successor_generation: successor.generation.generation,
        successor_completed_step: successor.generation.completed_step,
    }
}
