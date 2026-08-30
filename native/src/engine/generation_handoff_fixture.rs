//! Test-hook-only retained generation persistence and authority handoff.
//!
//! The fixture owns one real terminal [`RunningStepCoordinator`] transition,
//! its unchanged source authority, and a same-run run-start checkpoint. It is
//! deliberately absent from production builds and exposes only bounded scalar
//! records needed by the cross-language integration test.

use super::checkpoint::{
    publish_checkpoint, CheckpointDescriptor, CheckpointLimits, CheckpointOperationId,
};
use super::contract::ENGINE_CONTRACT_VERSION;
use super::generation::GenerationCommitRecord;
use super::graph::{GraphBundle, GraphLimits};
use super::inference::InferenceMathBackend;
use super::inference_fixture::{graph_limits, scenario_graph, Stage4InferenceScenarioName};
use super::rng::labelled_stream;
use super::running_step::{
    ExternalDeliveryEventKind, ExternalDeliveryResult, ExternalDeliveryState,
    ExternalObservationEvent, GenerationReassignmentProgress, RunningStepCoordinator,
    RunningStepInputs, RunningStepProgress,
};
use super::state::{
    estimate_state_memory, normalized_config_hash, normalized_settings_schema_hash, AllocatorState,
    AuthoritativeState, AuthorityPhase, BodyRange, BrainHandle, BrainOwner, BrainRuntimeState,
    ContractVersions, ControllerKind, ControllerLease, ControllerLeaseStatus,
    FixedStepContinuationState, GenerationBoundaryKind, GenerationStartPublication,
    GenerationState, GenomeLineage, LatestControllerAction, NormalizedEngineConfig,
    NormalizedSettingValue, PopulationGenome, RngStateBundle, RunIdentity, SnakeKind, SnakeState,
    StateAdmissionPolicy, StateCandidate, WorldPoint, WorldState, ALLOCATOR_VERSION,
    BASELINE_ENTITY_ID_START, CHECKPOINT_VERSION, ENGINE_STATE_VERSION, EXTERNAL_ENTITY_ID_START,
    GENERATION_BOUNDARY_VERSION, NORMALIZED_CONFIG_VERSION, PROTOCOL_VERSION,
    RESURRECTED_ENTITY_ID_START, RNG_BUNDLE_VERSION, SENSOR_VERSION, SERIALIZER_VERSION,
};
use super::step_config::{fixture_default_settings, RunningStepWorkLimits};
use std::path::Path;
use std::sync::Arc;

/// Stable run identity shared by the run-start and terminal-generation files.
const FIXTURE_RUN_ID: &str = "55555555-6666-4777-8888-999999999999";
/// One evolved genome keeps the retained integration session small.
const FIXTURE_POPULATION_COUNT: usize = 1;
/// Connected socket epoch used by the required reassignment event.
const FIXTURE_CONNECTION_ID: u64 = 7;
/// Stable controller lease identity used by the terminal source world.
const FIXTURE_LEASE_ID: u64 = 1;
/// Duration threshold deliberately crossed by the first complete fixed step.
const FIXTURE_GENERATION_SECONDS: f64 = 8.0;

/// Generation descriptor plus exact Rust-constructed SQLite metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedGenerationHandoff {
    /// Immutable checkpoint descriptor produced by the retained coordinator.
    pub descriptor: CheckpointDescriptor,
    /// Exact summary and elite reference derived during Rust admission.
    pub commit_record: GenerationCommitRecord,
}

/// One retained reliable controller assignment exposed to the thin bridge.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationHandoffAssignment {
    /// Exact source operation epoch.
    pub operation_epoch: u64,
    /// Monotonic external event sequence.
    pub event_sequence: u64,
    /// Connected socket epoch.
    pub connection_id: u64,
    /// Controller assignment epoch.
    pub lease_id: u64,
    /// Fresh successor snake identity.
    pub snake_id: u64,
    /// Fresh opaque resume token generated once by Rust.
    pub resume_token: String,
}

/// Bounded read-only state proving when the final authority changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationHandoffSnapshot {
    /// Current authoritative world epoch.
    pub world_epoch: u64,
    /// Current authoritative generation.
    pub generation: u64,
    /// Current authoritative completed-step count.
    pub completed_step: u64,
    /// Whether one admitted successor remains pending.
    pub transition_pending: bool,
    /// Whether its immutable checkpoint descriptor has published.
    pub checkpoint_published: bool,
    /// Whether the exact durable descriptor acknowledgement was retained.
    pub persistence_acknowledged: bool,
    /// Number of physical immutable-generation publications, excluding retries.
    pub generation_checkpoint_publications: u32,
    /// Number of final old-to-new authority swaps.
    pub authority_publications: u32,
}

/// Feature-gated owner of one real terminal transition across N-API calls.
#[derive(Debug)]
pub struct GenerationHandoffFixtureSession {
    run_start: AuthoritativeState,
    run_start_policy: StateAdmissionPolicy,
    authority: AuthoritativeState,
    coordinator: RunningStepCoordinator,
    checkpoint_limits: CheckpointLimits,
    graph_limits: GraphLimits,
    pending_assignment: Option<ExternalObservationEvent>,
    generation_checkpoint_publications: u32,
    authority_publications: u32,
}

impl GenerationHandoffFixtureSession {
    /// Construct and execute one real terminal fixed step while retaining old authority.
    pub fn new() -> Result<Self, String> {
        if env!("SLITHER_NATIVE_BUILD_CLASS") != "test-hooks" {
            return Err("generation handoff fixture requires a test-hooks native build".to_owned());
        }
        let graph_limits = graph_limits();
        let graph = Arc::new(
            GraphBundle::compile(
                scenario_graph(Stage4InferenceScenarioName::P0),
                &graph_limits,
            )
            .map_err(|error| format!("generation handoff graph failed: {error}"))?,
        );
        let (run_start_candidate, policy) = fixture_run_start(&graph)?;
        let mut running_candidate = run_start_candidate.clone();
        make_terminal_running_candidate(&mut running_candidate, graph.compiled())?;
        let run_start =
            AuthoritativeState::validate_and_own(run_start_candidate, Arc::clone(&graph), &policy)
                .map_err(|error| {
                    format!("generation handoff run start failed admission: {error}")
                })?;
        let mut authority =
            AuthoritativeState::validate_and_own(running_candidate, Arc::clone(&graph), &policy)
                .map_err(|error| {
                    format!("generation handoff running state failed admission: {error}")
                })?;
        let mut coordinator = RunningStepCoordinator::try_new(
            &authority,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .map_err(|error| format!("generation handoff coordinator failed: {error}"))?;
        match coordinator
            .advance_nonterminal(
                &mut authority,
                RunningStepInputs {
                    wall_now_ms: 500,
                    wall_accumulator_seconds: 0.0,
                },
            )
            .map_err(|error| format!("generation handoff terminal step failed: {error}"))?
        {
            RunningStepProgress::GenerationTransitionPending(_) => {}
            other => {
                return Err(format!(
                    "generation handoff fixture expected a terminal transition, got {other:?}"
                ));
            }
        }
        Ok(Self {
            run_start,
            run_start_policy: policy,
            authority,
            coordinator,
            checkpoint_limits: fixture_checkpoint_limits(),
            graph_limits,
            pending_assignment: None,
            generation_checkpoint_publications: 0,
            authority_publications: 0,
        })
    }

    /// Publish the same-run generation-one step-zero checkpoint through the production codec.
    pub fn publish_run_start_checkpoint(
        &self,
        managed_directory: &Path,
        operation_id: CheckpointOperationId,
        transition_epoch: u64,
    ) -> Result<CheckpointDescriptor, String> {
        let boundary = self
            .run_start
            .checkpoint_boundary()
            .map_err(|error| format!("generation handoff run-start boundary failed: {error}"))?;
        publish_checkpoint(
            managed_directory,
            operation_id,
            transition_epoch,
            boundary,
            &self.checkpoint_limits,
            &self.graph_limits,
            &self.run_start_policy,
        )
        .map_err(|error| format!("generation handoff run-start publication failed: {error}"))
    }

    /// Publish or exactly retry the retained generation checkpoint and Rust metadata.
    pub fn publish_generation_checkpoint(
        &mut self,
        managed_directory: &Path,
        operation_id: CheckpointOperationId,
    ) -> Result<PublishedGenerationHandoff, String> {
        let was_published = self
            .coordinator
            .pending_generation_transition()
            .and_then(|transition| transition.checkpoint_descriptor())
            .is_some();
        let descriptor = self
            .coordinator
            .publish_pending_generation_checkpoint(
                &self.authority,
                managed_directory,
                operation_id,
                &self.checkpoint_limits,
                &self.graph_limits,
            )
            .map_err(|error| {
                format!("generation handoff checkpoint publication failed: {error}")
            })?;
        if !was_published {
            self.generation_checkpoint_publications = self
                .generation_checkpoint_publications
                .checked_add(1)
                .ok_or_else(|| "generation checkpoint publication count overflowed".to_owned())?;
        }
        let commit_record = self
            .coordinator
            .pending_generation_transition()
            .ok_or_else(|| "generation transition disappeared after publication".to_owned())?
            .commit_record()
            .to_owned();
        Ok(PublishedGenerationHandoff {
            descriptor,
            commit_record,
        })
    }

    /// Apply one complete worker-echoed descriptor to the real coordinator barrier.
    pub fn acknowledge_generation_persistence(
        &mut self,
        descriptor: &CheckpointDescriptor,
    ) -> Result<(), String> {
        self.coordinator
            .acknowledge_pending_generation_persistence(&self.authority, descriptor)
            .map(|_| ())
            .map_err(|error| format!("generation handoff acknowledgement failed: {error}"))
    }

    /// Stage or reborrow the one required connected-controller assignment.
    pub fn prepare_generation_assignment(&mut self) -> Result<GenerationHandoffAssignment, String> {
        let batch = match self
            .coordinator
            .prepare_acknowledged_generation_reassignments(&self.authority)
            .map_err(|error| format!("generation handoff assignment preparation failed: {error}"))?
        {
            GenerationReassignmentProgress::DeliveryPending(batch) => batch,
            GenerationReassignmentProgress::Ready(_) => {
                return Err(
                    "generation handoff fixture unexpectedly required no delivery".to_owned(),
                );
            }
        };
        if batch.events().len() != 1 || batch.remaining() != 1 {
            return Err("generation handoff fixture expected one pending assignment".to_owned());
        }
        let event = batch.events()[0];
        if !matches!(
            event.delivery_kind,
            ExternalDeliveryEventKind::ReplacementAssignment { .. }
        ) || event.controller_kind != ControllerKind::Player
        {
            return Err("generation handoff fixture emitted the wrong event kind".to_owned());
        }
        let resume_token = batch
            .resume_token(0)
            .ok_or_else(|| "generation assignment omitted its resume token".to_owned())?
            .to_owned();
        self.pending_assignment = Some(event);
        Ok(GenerationHandoffAssignment {
            operation_epoch: event.step_key.operation_epoch(),
            event_sequence: event.event_sequence,
            connection_id: event.connection_id,
            lease_id: event.lease_id,
            snake_id: event.snake_id,
            resume_token,
        })
    }

    /// Resolve the exact retained assignment using Node's local-send outcome.
    pub fn submit_generation_assignment(
        &mut self,
        operation_epoch: u64,
        event_sequence: u64,
        connection_id: u64,
        lease_id: u64,
        accepted: bool,
    ) -> Result<(), String> {
        let event = self
            .pending_assignment
            .ok_or_else(|| "generation assignment has not been prepared".to_owned())?;
        if operation_epoch != event.step_key.operation_epoch()
            || event_sequence != event.event_sequence
            || connection_id != event.connection_id
            || lease_id != event.lease_id
        {
            return Err(
                "generation assignment result does not match the retained event".to_owned(),
            );
        }
        let resolution = self
            .coordinator
            .submit_external_delivery_results(
                &mut self.authority,
                &[ExternalDeliveryResult {
                    step_key: event.step_key,
                    event_sequence,
                    connection_id,
                    lease_id,
                    accepted,
                }],
            )
            .map_err(|error| format!("generation assignment result failed: {error}"))?;
        if resolution.matched_acceptances != usize::from(accepted)
            || resolution.matched_failures != usize::from(!accepted)
            || resolution.ignored_results != 0
            || !matches!(
                resolution.state,
                ExternalDeliveryState::GenerationAssignmentsReady(_)
            )
        {
            return Err("generation assignment result did not resolve exactly once".to_owned());
        }
        Ok(())
    }

    /// Perform the one final authority swap after persistence and delivery barriers.
    pub fn publish_generation_start(&mut self) -> Result<GenerationStartPublication, String> {
        let publication = self
            .coordinator
            .publish_acknowledged_generation_start(&mut self.authority)
            .map_err(|error| format!("generation handoff final publication failed: {error}"))?;
        self.authority_publications = self
            .authority_publications
            .checked_add(1)
            .ok_or_else(|| "generation authority publication count overflowed".to_owned())?;
        Ok(publication)
    }

    /// Inspect only bounded scalar proof of the current authority and barrier.
    #[must_use]
    pub fn snapshot(&self) -> GenerationHandoffSnapshot {
        let transition = self.coordinator.pending_generation_transition();
        GenerationHandoffSnapshot {
            world_epoch: self.authority.world_epoch(),
            generation: self.authority.state().generation.generation,
            completed_step: self.authority.state().generation.completed_step,
            transition_pending: transition.is_some(),
            checkpoint_published: transition
                .and_then(|pending| pending.checkpoint_descriptor())
                .is_some(),
            persistence_acknowledged: transition
                .is_some_and(|pending| pending.persistence_acknowledged()),
            generation_checkpoint_publications: self.generation_checkpoint_publications,
            authority_publications: self.authority_publications,
        }
    }
}

/// Construct the same-run run-start state and its exact source/build policy.
pub(super) fn fixture_run_start(
    graph: &Arc<GraphBundle>,
) -> Result<(StateCandidate, StateAdmissionPolicy), String> {
    let mut settings = fixture_default_settings(FIXTURE_POPULATION_COUNT, 0);
    replace_setting(
        &mut settings,
        "generationSeconds",
        NormalizedSettingValue::Float(FIXTURE_GENERATION_SECONDS),
    )?;
    replace_setting(
        &mut settings,
        "observer.earlyEndMinSeconds",
        NormalizedSettingValue::Float(50.0),
    )?;
    replace_setting(
        &mut settings,
        "pelletCountTarget",
        NormalizedSettingValue::Integer(100),
    )?;
    replace_setting(
        &mut settings,
        "pelletSpawnPerSecond",
        NormalizedSettingValue::Float(5.0),
    )?;
    let settings_schema_sha256 = normalized_settings_schema_hash(&settings)
        .map_err(|error| format!("generation handoff settings schema failed: {error}"))?;
    let config = NormalizedEngineConfig {
        version: NORMALIZED_CONFIG_VERSION,
        settings,
        settings_schema_sha256: settings_schema_sha256.clone(),
        graph_architecture_key: graph.architecture_key.clone(),
        fixed_step_seconds: 1.0 / 60.0,
        requested_sim_speed: 1.0,
        world_radius: 3_500.0,
        population_count: FIXTURE_POPULATION_COUNT,
        baseline_count: 0,
        max_world_snakes: 16,
        max_non_population_brains: 8,
        max_body_points: 10_000,
        max_pellets: 1_000,
        spatial_index_bytes: 16 * 1024 * 1024,
        worker_scratch_bytes: 64 * 1024 * 1024,
        checkpoint_scratch_bytes: 64 * 1024 * 1024,
        controller_input_hold_ms: 500,
        controller_disconnect_grace_ms: 30_000,
    };
    let config_hash = normalized_config_hash(&config)
        .map_err(|error| format!("generation handoff config hash failed: {error}"))?;
    let build_identifier = crate::native_addon_build_identifier();
    let source_sha256 = crate::native_addon_source_sha256();
    let target_triple = crate::native_addon_build_target();
    let build_profile = crate::native_addon_build_profile();
    let build_class = crate::native_addon_build_class();
    let rustc_version = crate::native_addon_rustc_version();
    let build_contract_sha256 = crate::native_addon_build_contract_sha256();
    let brain = BrainHandle { id: 1, epoch: 1 };
    let weights = (0..graph.total_parameters)
        .map(|index| ((index % 257) as f32 - 128.0) / 512.0)
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let candidate = StateCandidate {
        versions: ContractVersions {
            state: ENGINE_STATE_VERSION,
            engine: ENGINE_CONTRACT_VERSION,
            protocol: PROTOCOL_VERSION,
            serializer: SERIALIZER_VERSION,
            sensor: SENSOR_VERSION,
            rng_bundle: RNG_BUNDLE_VERSION,
            checkpoint: CHECKPOINT_VERSION,
            graph_layout: graph.layout_version,
        },
        identity: RunIdentity {
            run_id: FIXTURE_RUN_ID.to_owned(),
            seed: 42,
            config_revision: 1,
            config_hash,
            source_revision: build_identifier.clone(),
            engine_build_id: build_identifier.clone(),
            source_sha256: source_sha256.clone(),
            target_triple: target_triple.clone(),
            build_profile: build_profile.clone(),
            build_class: build_class.clone(),
            rustc_version: rustc_version.clone(),
            build_contract_sha256: build_contract_sha256.clone(),
            math_backend: InferenceMathBackend::Scalar.label().to_owned(),
        },
        config,
        phase: AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart),
        generation: GenerationState {
            boundary_version: GENERATION_BOUNDARY_VERSION,
            generation: 1,
            completed_step: 0,
            population_epoch: 1,
            elapsed_seconds: 0.0,
            wall_accumulator_seconds: 0.0,
            best_fitness_ever: 0.0,
        },
        fixed_step: FixedStepContinuationState::generation_boundary(),
        rng: RngStateBundle {
            version: RNG_BUNDLE_VERSION,
            world: labelled_stream(42.0, "world").export_state(),
            evolution: labelled_stream(42.0, "evolution").export_state(),
            external_controller: labelled_stream(42.0, "external-controller").export_state(),
            baselines: Vec::new(),
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 1,
            next_brain_id: 2,
            next_genome_id: 2,
            next_controller_lease_id: 1,
            next_frame_v1_id: 1,
            next_external_id: EXTERNAL_ENTITY_ID_START,
            next_baseline_id: BASELINE_ENTITY_ID_START,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START,
        },
        population: vec![PopulationGenome {
            slot: 0,
            brain,
            lineage: GenomeLineage {
                genome_id: 1,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: 0.0,
            weights,
        }],
        brains: vec![BrainRuntimeState {
            handle: brain,
            owner: BrainOwner::PopulationSlot(0),
            non_population_weights: None,
            recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
        }],
        world: WorldState::default(),
    };
    let estimate = estimate_state_memory(&candidate, graph)
        .map_err(|error| format!("generation handoff memory estimate failed: {error}"))?;
    let memory_ceiling_bytes = estimate
        .total_bytes
        .checked_add(512 * 1024 * 1024)
        .ok_or_else(|| "generation handoff memory ceiling overflowed".to_owned())?;
    let policy = StateAdmissionPolicy {
        memory_ceiling_bytes,
        expected_source_revision: build_identifier.clone(),
        expected_engine_build_id: build_identifier,
        expected_source_sha256: source_sha256,
        expected_target_triple: target_triple,
        expected_build_profile: build_profile,
        expected_build_class: build_class,
        expected_rustc_version: rustc_version,
        expected_build_contract_sha256: build_contract_sha256,
        expected_math_backend: InferenceMathBackend::Scalar.label().to_owned(),
        expected_settings_schema_sha256: settings_schema_sha256,
    };
    Ok((candidate, policy))
}

/// Turn the admitted run-start shape into one terminal source with a live owner.
fn make_terminal_running_candidate(
    candidate: &mut StateCandidate,
    graph: &super::graph::CompiledGraph,
) -> Result<(), String> {
    candidate.phase = AuthorityPhase::Running;
    candidate.generation.elapsed_seconds =
        FIXTURE_GENERATION_SECONDS - (candidate.config.fixed_step_seconds / 2.0);
    let evolved_position = WorldPoint { x: 0.0, y: 0.0 };
    push_body_snake(
        candidate,
        SnakeState {
            id: 1,
            frame_v1_id: 1,
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(0),
            brain: Some(candidate.population[0].brain),
            baseline_slot: None,
            baseline_strategy: None,
            position: evolved_position,
            previous_position: evolved_position,
            direction: 0.0,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 7.0,
            food: 2.0,
            points: 12.5,
            kills: 1,
            target_length: 5.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 4.0,
            body: BodyRange { start: 0, len: 0 },
            skin: 0,
        },
    )?;
    let external_id = EXTERNAL_ENTITY_ID_START;
    let external_brain = BrainHandle {
        id: candidate.allocators.next_brain_id,
        epoch: candidate.generation.population_epoch,
    };
    candidate.allocators.next_brain_id = candidate
        .allocators
        .next_brain_id
        .checked_add(1)
        .ok_or_else(|| "generation handoff brain identity overflowed".to_owned())?;
    candidate.brains.push(BrainRuntimeState {
        handle: external_brain,
        owner: BrainOwner::Entity(external_id),
        non_population_weights: Some(vec![0.0; graph.total_parameters].into_boxed_slice()),
        recurrent: vec![0.0; graph.total_state_size].into_boxed_slice(),
    });
    let external_position = WorldPoint {
        x: 1_200.0,
        y: -1_200.0,
    };
    push_body_snake(
        candidate,
        SnakeState {
            id: external_id,
            frame_v1_id: 2,
            kind: SnakeKind::External,
            alive: true,
            population_slot: None,
            brain: Some(external_brain),
            baseline_slot: None,
            baseline_strategy: None,
            position: external_position,
            previous_position: external_position,
            direction: 0.0,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 1.0,
            food: 0.0,
            points: 10.0,
            kills: 0,
            target_length: 5.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 2.0,
            body: BodyRange { start: 0, len: 0 },
            skin: 1,
        },
    )?;
    candidate.world.controller_leases.push(ControllerLease {
        id: FIXTURE_LEASE_ID,
        snake_id: external_id,
        kind: ControllerKind::Player,
        connection_id: Some(FIXTURE_CONNECTION_ID),
        scope: FIXTURE_RUN_ID.to_owned(),
        resume_token: "generation-handoff-old-token".to_owned(),
        status: ControllerLeaseStatus::Connected,
        latest_action: LatestControllerAction {
            turn: 0.25,
            boost: false,
            client_tick: 1,
            arrival_sequence: 1,
            accepted_at_ms: 100,
        },
        last_observed_at_ms: 100,
        disconnected_at_ms: None,
        input_hold_expires_at_ms: None,
        grace_expires_at_ms: None,
        takeover_committed_at_ms: None,
    });
    candidate.allocators.next_entity_id = 2;
    candidate.allocators.next_external_id = external_id + 1;
    candidate.allocators.next_controller_lease_id = FIXTURE_LEASE_ID + 1;
    candidate.allocators.next_frame_v1_id = 3;
    candidate
        .fixed_step
        .sensor_generation
        .update_after_step(&candidate.world)
        .map_err(|error| format!("generation handoff sensor state failed: {error}"))?;
    Ok(())
}

/// Append one canonical five-point body and fix its pooled range.
fn push_body_snake(candidate: &mut StateCandidate, mut snake: SnakeState) -> Result<(), String> {
    let start = candidate.world.body_points.len();
    candidate
        .world
        .body_points
        .try_reserve_exact(5)
        .map_err(|_| "generation handoff body allocation failed".to_owned())?;
    candidate
        .world
        .body_points
        .extend((0..5).map(|offset| WorldPoint {
            x: snake.position.x - (f64::from(offset) * 7.5),
            y: snake.position.y,
        }));
    snake.body = BodyRange { start, len: 5 };
    candidate.world.snakes.push(snake);
    Ok(())
}

/// Replace one complete fixture setting while preserving its canonical path order.
fn replace_setting(
    settings: &mut [super::state::NormalizedSetting],
    path: &str,
    value: NormalizedSettingValue,
) -> Result<(), String> {
    let setting = settings
        .iter_mut()
        .find(|setting| setting.path == path)
        .ok_or_else(|| format!("generation handoff setting {path} is missing"))?;
    setting.value = value;
    Ok(())
}

/// Reviewed bounds for the small real generation handoff fixture.
pub(super) fn fixture_checkpoint_limits() -> CheckpointLimits {
    CheckpointLimits {
        max_archive_bytes: 64 * 1024 * 1024,
        max_manifest_bytes: 1024 * 1024,
        max_state_bytes: 4 * 1024 * 1024,
        max_graph_bytes: 4 * 1024 * 1024,
        max_population_index_bytes: 4 * 1024 * 1024,
        max_population_count: 300,
        max_setting_count: 512,
        max_baseline_rng_count: 128,
        max_string_bytes: 256 * 1024,
        max_total_string_bytes: 4 * 1024 * 1024,
        max_weight_floats: 8_000_000,
        max_recurrent_floats: 1_000_000,
        max_numeric_stored_bytes: 64 * 1024 * 1024,
        max_numeric_candidate_bytes: 64 * 1024 * 1024,
        max_total_decoded_bytes: 128 * 1024 * 1024,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_fixture_starts_with_one_real_unpublished_terminal_transition() {
        let fixture = GenerationHandoffFixtureSession::new().expect("fixture must construct");
        let snapshot = fixture.snapshot();
        assert!(snapshot.world_epoch > 0);
        assert_eq!(snapshot.generation, 1);
        assert_eq!(snapshot.completed_step, 0);
        assert!(snapshot.transition_pending);
        assert!(!snapshot.checkpoint_published);
        assert!(!snapshot.persistence_acknowledged);
        assert_eq!(snapshot.generation_checkpoint_publications, 0);
        assert_eq!(snapshot.authority_publications, 0);
    }
}
