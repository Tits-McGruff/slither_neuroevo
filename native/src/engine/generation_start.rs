//! Collision-safe construction of a running world from an exact checkpoint boundary.
//!
//! The current TypeScript reference constructs a generation in this order:
//! evolved snakes consume the world RNG, baseline snakes consume their own
//! per-slot RNGs, then the initial ambient target consumes the continued world
//! RNG. This module preserves that ordering while replacing unchecked snake
//! placement with the approved complete-body collision-safe algorithm. It owns
//! reusable staging only and does not authorize checkpoint metadata, authority
//! publication, controller assignment, or scheduler completion.

use super::ambient::{AmbientDiagnostics, AmbientError, AmbientWorkspace};
use super::baseline::{BaselineLifecycleError, BaselineLifecycleState};
use super::run_start::RunStartPersistenceProof;
use super::spawn::{
    SpawnCapacityDiagnostics, SpawnConfig, SpawnDomain, SpawnError, SpawnKey, SpawnPlacement,
    SpawnRequest, SpawnWorkspace,
};
use super::state::{
    AllocatorState, AuthoritativeState, AuthorityPhase, BodyRange, FixedStepContinuationState,
    GenerationBoundaryKind, InitialRunStartReplacement, RngStateBundle, RunStartPublication,
    SnakeKind, SnakeState, StateCandidate, StateError, WorldPoint, WorldState,
};
use super::step_config::{project_running_step_config, RunningStepWorkLimits, StepConfigError};
use super::world_step::{WorldStepConfig, WorldStepError};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// First exact-boundary-to-running-world construction contract.
pub const GENERATION_START_VERSION: u32 = 1;
/// Browser frame-v1 skin used by ordinary evolved snakes.
const EVOLVED_SNAKE_SKIN: u32 = 0;
/// Browser frame-v1 skin used by built-in baseline snakes.
const BASELINE_SNAKE_SKIN: u32 = 2;

/// Complete configuration for one generation construction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GenerationStartConfig {
    /// Versioned phase and draw ordering.
    pub algorithm_version: u32,
    /// Non-gameplay work ceilings used when projecting the admitted source.
    /// Gameplay values are always re-derived from `source.config`.
    work_limits: RunningStepWorkLimits,
}

impl GenerationStartConfig {
    /// Construct from explicit process work ceilings.
    #[must_use]
    pub const fn from_work_limits(work_limits: RunningStepWorkLimits) -> Self {
        Self {
            algorithm_version: GENERATION_START_VERSION,
            work_limits,
        }
    }

    fn project(self, source: &StateCandidate) -> Result<WorldStepConfig, GenerationStartError> {
        if self.algorithm_version != GENERATION_START_VERSION {
            return Err(GenerationStartError::InvalidConfig {
                field: "algorithm_version",
            });
        }
        let world_step = project_running_step_config(&source.config, self.work_limits)
            .map_err(|error| GenerationStartError::StepConfig(Box::new(error)))?
            .world_step;
        world_step
            .validate_shape()
            .map_err(|error| GenerationStartError::WorldStep(Box::new(error)))?;
        if !matches!(source.phase, AuthorityPhase::GenerationBoundary(_)) {
            return Err(GenerationStartError::InvalidSource {
                reason: "generation construction requires an exact pre-spawn boundary",
            });
        }
        if !source.world.snakes.is_empty()
            || !source.world.body_points.is_empty()
            || !source.world.pellets.is_empty()
            || !source.world.controller_leases.is_empty()
        {
            return Err(GenerationStartError::InvalidSource {
                reason: "pre-spawn boundary contains live world data",
            });
        }
        if source.generation.elapsed_seconds.to_bits() != 0.0_f64.to_bits()
            || source.generation.wall_accumulator_seconds.to_bits() != 0.0_f64.to_bits()
            || source.fixed_step.ambient_pellet_accumulator.to_bits() != 0.0_f64.to_bits()
            || source
                .fixed_step
                .sensor_generation
                .best_points_this_generation()
                .to_bits()
                != 0.0_f64.to_bits()
            || !source.fixed_step.baseline_lifecycle.slots.is_empty()
        {
            return Err(GenerationStartError::InvalidSource {
                reason: "generation continuation is not reset",
            });
        }
        if source.population.len() != source.config.population_count
            || source.rng.baselines.len() != source.config.baseline_count
            || world_step.prefix.baseline.slot_count != source.config.baseline_count
        {
            return Err(GenerationStartError::InvalidSource {
                reason: "population or baseline counts differ from admitted configuration",
            });
        }
        let total_snakes = source
            .config
            .population_count
            .checked_add(source.config.baseline_count)
            .ok_or(GenerationStartError::ArithmeticOverflow {
                context: "initial snake count",
            })?;
        if total_snakes > world_step.prefix.maximum_snakes {
            return Err(GenerationStartError::CapacityExceeded {
                storage: "snakes",
                required: total_snakes,
                maximum: world_step.prefix.maximum_snakes,
            });
        }
        let required_body_points = total_snakes
            .checked_mul(world_step.prefix.baseline_spawn.snake_start_len)
            .ok_or(GenerationStartError::ArithmeticOverflow {
                context: "initial body-point count",
            })?;
        if required_body_points > world_step.prefix.maximum_body_points {
            return Err(GenerationStartError::CapacityExceeded {
                storage: "body points",
                required: required_body_points,
                maximum: world_step.prefix.maximum_body_points,
            });
        }
        if world_step.prefix.ambient.target_count > world_step.prefix.maximum_pellets {
            return Err(GenerationStartError::CapacityExceeded {
                storage: "pellets",
                required: world_step.prefix.ambient.target_count,
                maximum: world_step.prefix.maximum_pellets,
            });
        }
        Ok(world_step)
    }
}

/// Aggregated work and retained storage for one complete construction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GenerationStartDiagnostics {
    /// Evolved snakes placed.
    pub evolved_snakes: usize,
    /// Baseline snakes placed.
    pub baseline_snakes: usize,
    /// Random/fallback candidates examined across both RNG domains.
    pub candidates_examined: usize,
    /// Placements supplied by deterministic fallback.
    pub fallback_placements: usize,
    /// Complete wall/body geometry comparisons performed.
    pub geometry_checks: usize,
    /// Initial ambient generation work.
    pub ambient: AmbientDiagnostics,
    /// Retained spawn-workspace sizes after the latest sub-batch.
    pub spawn: SpawnCapacityDiagnostics,
    /// Retained request capacity.
    pub request_capacity: usize,
    /// Retained copied placement capacity.
    pub placement_capacity: usize,
    /// Retained copied evolved-body capacity.
    pub evolved_body_capacity: usize,
    /// Retained one-baseline-body capacity.
    pub baseline_body_capacity: usize,
    /// Retained staged snake capacity.
    pub snake_capacity: usize,
    /// Retained staged body-point capacity.
    pub body_point_capacity: usize,
    /// Retained staged pellet capacity.
    pub pellet_capacity: usize,
}

/// Immutable view of one complete, non-authoritative running-world proposal.
#[derive(Debug)]
pub struct PreparedGenerationStart<'workspace, 'source> {
    source: &'source StateCandidate,
    config: GenerationStartConfig,
    world: &'workspace WorldState,
    rng: &'workspace RngStateBundle,
    allocators: &'workspace AllocatorState,
    fixed_step: &'workspace FixedStepContinuationState,
    diagnostics: GenerationStartDiagnostics,
}

impl<'workspace, 'source> PreparedGenerationStart<'workspace, 'source> {
    /// Exact pre-spawn boundary used for every draw and identity continuation.
    #[must_use]
    pub const fn source(&self) -> &'source StateCandidate {
        self.source
    }

    /// Completely collision-safe world ready for running-state admission.
    #[must_use]
    pub const fn world(&self) -> &'workspace WorldState {
        self.world
    }

    /// RNG bundle after evolved placement, per-baseline placement, and pellets.
    #[must_use]
    pub const fn rng(&self) -> &'workspace RngStateBundle {
        self.rng
    }

    /// Allocator continuation after every initial snake, frame, and pellet ID.
    #[must_use]
    pub const fn allocators(&self) -> &'workspace AllocatorState {
        self.allocators
    }

    /// Initialized baseline/sensor/ambient continuation for the running round.
    #[must_use]
    pub const fn fixed_step(&self) -> &'workspace FixedStepContinuationState {
        self.fixed_step
    }

    /// Aggregated construction work and retained capacities.
    #[must_use]
    pub const fn diagnostics(&self) -> GenerationStartDiagnostics {
        self.diagnostics
    }

    /// Revalidate the borrowed source and projected contract before publication.
    pub fn validate_current(
        &self,
        current: &StateCandidate,
        config: GenerationStartConfig,
    ) -> Result<(), GenerationStartError> {
        if !std::ptr::eq(self.source, current) {
            return Err(GenerationStartError::SourceChanged);
        }
        if self.config != config {
            return Err(GenerationStartError::ConfigChanged);
        }
        config.project(current).map(|_| ())
    }
}

/// Reusable owner of exact-boundary generation-construction scratch.
#[derive(Debug, Default)]
pub struct GenerationStartWorkspace {
    spawn: SpawnWorkspace,
    ambient: AmbientWorkspace,
    world: WorldState,
    rng: Option<RngStateBundle>,
    allocators: Option<AllocatorState>,
    fixed_step: Option<FixedStepContinuationState>,
    requests: Vec<SpawnRequest>,
    copied_placements: Vec<SpawnPlacement>,
    evolved_body_points: Vec<WorldPoint>,
    baseline_body: Vec<WorldPoint>,
    diagnostics: GenerationStartDiagnostics,
    prepared_source_address: Option<usize>,
    prepared_config: Option<GenerationStartConfig>,
    ready: bool,
}

impl GenerationStartWorkspace {
    /// Construct empty reusable generation-start scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepare one complete collision-safe running world without publishing it.
    pub fn prepare<'workspace, 'source>(
        &'workspace mut self,
        source: &'source StateCandidate,
        config: GenerationStartConfig,
    ) -> Result<PreparedGenerationStart<'workspace, 'source>, GenerationStartError> {
        self.clear();
        let world_step = config.project(source)?;

        let population_count = source.config.population_count;
        let baseline_count = source.config.baseline_count;
        let total_snakes = population_count.checked_add(baseline_count).ok_or(
            GenerationStartError::ArithmeticOverflow {
                context: "complete initial snake count",
            },
        )?;
        let start_len = world_step.prefix.baseline_spawn.snake_start_len;
        let total_body_points = total_snakes.checked_mul(start_len).ok_or(
            GenerationStartError::ArithmeticOverflow {
                context: "complete initial body count",
            },
        )?;
        reserve_for(&mut self.world.snakes, total_snakes, "initial snakes")?;
        reserve_for(
            &mut self.world.body_points,
            total_body_points,
            "initial body points",
        )?;
        reserve_for(
            &mut self.world.pellets,
            world_step.prefix.ambient.target_count,
            "initial pellets",
        )?;
        reserve_for(
            &mut self.requests,
            population_count.max(1),
            "spawn requests",
        )?;
        reserve_for(
            &mut self.copied_placements,
            population_count,
            "evolved placements",
        )?;
        reserve_for(
            &mut self.evolved_body_points,
            population_count.checked_mul(start_len).ok_or(
                GenerationStartError::ArithmeticOverflow {
                    context: "temporary evolved bodies",
                },
            )?,
            "temporary evolved bodies",
        )?;
        reserve_for(&mut self.baseline_body, start_len, "one baseline body")?;

        self.rng = Some(source.rng.clone());
        self.allocators = Some(source.allocators.clone());
        let population_u64 = u64::try_from(population_count).map_err(|_| {
            GenerationStartError::ArithmeticOverflow {
                context: "evolved identity count",
            }
        })?;
        let baseline_u64 = u64::try_from(baseline_count).map_err(|_| {
            GenerationStartError::ArithmeticOverflow {
                context: "baseline identity count",
            }
        })?;
        let total_frame_u32 =
            u32::try_from(total_snakes).map_err(|_| GenerationStartError::ArithmeticOverflow {
                context: "initial frame-v1 identity count",
            })?;
        let (evolved_ids, baseline_ids, frame_ids) = {
            let allocators = self
                .allocators
                .as_mut()
                .ok_or(GenerationStartError::InternalShapeMismatch)?;
            let evolved = allocators.reserve_entity_ids(population_u64)?;
            let baseline = allocators.reserve_baseline_ids(baseline_u64)?;
            let frames = allocators.reserve_frame_v1_ids(total_frame_u32)?;
            (evolved, baseline, frames)
        };

        self.prepare_evolved(source, world_step, evolved_ids, frame_ids)?;
        self.prepare_baselines(source, world_step, baseline_ids, frame_ids)?;
        self.prepare_initial_pellets(world_step)?;

        let lifecycle = BaselineLifecycleState::initialize_after_complete_spawn(
            world_step.baseline,
            &self.world,
        )?;
        self.fixed_step = Some(FixedStepContinuationState {
            ambient_pellet_accumulator: 0.0,
            baseline_lifecycle: lifecycle,
            sensor_generation: super::sensors::SensorGenerationState::new(),
        });
        self.diagnostics.evolved_snakes = population_count;
        self.diagnostics.baseline_snakes = baseline_count;
        self.update_capacity_diagnostics();
        self.prepared_source_address = Some(std::ptr::from_ref(source).addr());
        self.prepared_config = Some(config);
        self.ready = true;
        self.prepared(source, config)
    }

    /// Whether the latest attempt produced one complete running-world proposal.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Latest work and retained capacities, including after failure.
    #[must_use]
    pub fn diagnostics(&self) -> GenerationStartDiagnostics {
        let mut diagnostics = self.diagnostics;
        diagnostics.ambient = self.ambient.diagnostics();
        diagnostics.spawn = self.spawn.diagnostics();
        diagnostics.request_capacity = self.requests.capacity();
        diagnostics.placement_capacity = self.copied_placements.capacity();
        diagnostics.evolved_body_capacity = self.evolved_body_points.capacity();
        diagnostics.baseline_body_capacity = self.baseline_body.capacity();
        diagnostics.snake_capacity = self.world.snakes.capacity();
        diagnostics.body_point_capacity = self.world.body_points.capacity();
        diagnostics.pellet_capacity = self.world.pellets.capacity();
        diagnostics
    }

    /// Reborrow the already-complete proposal without repeating RNG draws.
    ///
    /// This is used after a matching persistence acknowledgement has already
    /// been accepted. A retry may reborrow a successful construction, but a
    /// failed construction must run [`Self::prepare`] again from the unchanged
    /// exact boundary.
    pub(crate) fn retained<'workspace, 'source>(
        &'workspace self,
        source: &'source StateCandidate,
        config: GenerationStartConfig,
    ) -> Result<PreparedGenerationStart<'workspace, 'source>, GenerationStartError> {
        config.project(source)?;
        if self.prepared_source_address != Some(std::ptr::from_ref(source).addr()) {
            return Err(GenerationStartError::SourceChanged);
        }
        if self.prepared_config != Some(config) {
            return Err(GenerationStartError::ConfigChanged);
        }
        self.prepared(source, config)
    }

    /// Atomically move one complete proposal into its exact durable run-start authority.
    ///
    /// The workspace must already retain a successful preparation from the same
    /// boundary object and projection. A failed authority admission swaps every
    /// buffer back and leaves this proposal ready for an explicit retry.
    pub(crate) fn publish_initial_run_start(
        &mut self,
        authority: &mut AuthoritativeState,
        config: GenerationStartConfig,
        persistence_proof: &RunStartPersistenceProof,
    ) -> Result<RunStartPublication, GenerationStartError> {
        config.project(authority.state())?;
        if authority.state().phase
            != AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart)
        {
            return Err(GenerationStartError::InvalidSource {
                reason: "initial activation requires a run-start boundary",
            });
        }
        let source_address = std::ptr::from_ref(authority.state()).addr();
        if !self.ready {
            return Err(GenerationStartError::ResultNotReady);
        }
        if self.prepared_source_address != Some(source_address) {
            return Err(GenerationStartError::SourceChanged);
        }
        if self.prepared_config != Some(config) {
            return Err(GenerationStartError::ConfigChanged);
        }

        let publication = {
            let rng = self
                .rng
                .as_mut()
                .ok_or(GenerationStartError::InternalShapeMismatch)?;
            let allocators = self
                .allocators
                .as_mut()
                .ok_or(GenerationStartError::InternalShapeMismatch)?;
            let fixed_step = self
                .fixed_step
                .as_mut()
                .ok_or(GenerationStartError::InternalShapeMismatch)?;
            let mut replacement = InitialRunStartReplacement {
                source_address,
                world: &mut self.world,
                rng,
                allocators,
                fixed_step,
                persistence_proof,
            };
            authority.publish_initial_run_start(&mut replacement)?
        };
        self.rng = None;
        self.allocators = None;
        self.clear();
        Ok(publication)
    }

    /// Whether the retained proposal is exactly the requested boundary/config.
    #[must_use]
    pub(crate) fn retains(&self, source: &StateCandidate, config: GenerationStartConfig) -> bool {
        self.ready
            && self.prepared_source_address == Some(std::ptr::from_ref(source).addr())
            && self.prepared_config == Some(config)
            && config.project(source).is_ok()
    }

    fn prepare_evolved(
        &mut self,
        source: &StateCandidate,
        world_step: WorldStepConfig,
        evolved_ids: Option<super::state::InternalIdReservation>,
        frame_ids: Option<super::state::FrameV1IdReservation>,
    ) -> Result<(), GenerationStartError> {
        let count = source.config.population_count;
        if count == 0 {
            return Err(GenerationStartError::InvalidSource {
                reason: "evolved population is empty",
            });
        }
        let evolved_ids = evolved_ids.ok_or(GenerationStartError::InternalShapeMismatch)?;
        let frame_ids = frame_ids.ok_or(GenerationStartError::InternalShapeMismatch)?;
        self.requests.clear();
        for slot in 0..count {
            self.requests.push(SpawnRequest {
                key: SpawnKey {
                    domain: SpawnDomain::Evolved,
                    slot: u64::try_from(slot).map_err(|_| {
                        GenerationStartError::ArithmeticOverflow {
                            context: "evolved spawn slot",
                        }
                    })?,
                },
            });
        }

        let (next_world_rng, spawn_diagnostics) = {
            let prepared = self.spawn.prepare(
                &source.world,
                &self.requests,
                &source.rng.world,
                world_step.prefix.baseline_spawn,
                count
                    .checked_mul(world_step.prefix.baseline_spawn.snake_start_len)
                    .ok_or(GenerationStartError::ArithmeticOverflow {
                        context: "evolved spawn body count",
                    })?,
            )?;
            self.copied_placements
                .extend_from_slice(prepared.placements());
            self.evolved_body_points
                .extend_from_slice(prepared.body_points());
            (prepared.next_rng().clone(), prepared.diagnostics())
        };
        self.accumulate_spawn_diagnostics(spawn_diagnostics)?;
        self.rng
            .as_mut()
            .ok_or(GenerationStartError::InternalShapeMismatch)?
            .world = next_world_rng;

        for slot in 0..count {
            let placement = *self
                .copied_placements
                .get(slot)
                .ok_or(GenerationStartError::InternalShapeMismatch)?;
            let slot_u32 =
                u32::try_from(slot).map_err(|_| GenerationStartError::ArithmeticOverflow {
                    context: "evolved population slot",
                })?;
            if placement.key
                != (SpawnKey {
                    domain: SpawnDomain::Evolved,
                    slot: u64::from(slot_u32),
                })
            {
                return Err(GenerationStartError::InternalShapeMismatch);
            }
            let population = source
                .population
                .get(slot)
                .filter(|genome| genome.slot == slot_u32)
                .ok_or(GenerationStartError::InvalidSource {
                    reason: "population slots are not dense and canonical",
                })?;
            let body_end = placement.body.start.checked_add(placement.body.len).ok_or(
                GenerationStartError::ArithmeticOverflow {
                    context: "evolved placement body range",
                },
            )?;
            let body = self
                .evolved_body_points
                .get(placement.body.start..body_end)
                .ok_or(GenerationStartError::InternalShapeMismatch)?;
            let body_start = self.world.body_points.len();
            self.world.body_points.extend_from_slice(body);
            let offset =
                u64::try_from(slot).map_err(|_| GenerationStartError::ArithmeticOverflow {
                    context: "evolved identity offset",
                })?;
            let frame_offset =
                u32::try_from(slot).map_err(|_| GenerationStartError::ArithmeticOverflow {
                    context: "evolved frame identity offset",
                })?;
            let snake_id = evolved_ids.first.checked_add(offset).ok_or(
                GenerationStartError::ArithmeticOverflow {
                    context: "evolved snake identity",
                },
            )?;
            let frame_v1_id = frame_ids.first.checked_add(frame_offset).ok_or(
                GenerationStartError::ArithmeticOverflow {
                    context: "evolved frame-v1 identity",
                },
            )?;
            self.world.snakes.push(new_snake(
                snake_id,
                frame_v1_id,
                SnakeKind::Evolved,
                placement,
                BodyRange {
                    start: body_start,
                    len: body.len(),
                },
                Some(slot_u32),
                Some(population.brain),
                None,
                None,
                EVOLVED_SNAKE_SKIN,
                world_step.prefix.baseline_spawn.snake_radius,
                world_step.prefix.baseline_snake_base_speed,
                world_step.control.initial_neural_accumulator_seconds(),
            ));
        }
        Ok(())
    }

    fn prepare_baselines(
        &mut self,
        source: &StateCandidate,
        world_step: WorldStepConfig,
        baseline_ids: Option<super::state::InternalIdReservation>,
        frame_ids: Option<super::state::FrameV1IdReservation>,
    ) -> Result<(), GenerationStartError> {
        let count = source.config.baseline_count;
        if count == 0 {
            if baseline_ids.is_some() {
                return Err(GenerationStartError::InternalShapeMismatch);
            }
            return Ok(());
        }
        let baseline_ids = baseline_ids.ok_or(GenerationStartError::InternalShapeMismatch)?;
        let frame_ids = frame_ids.ok_or(GenerationStartError::InternalShapeMismatch)?;
        for slot in 0..count {
            let slot_u32 =
                u32::try_from(slot).map_err(|_| GenerationStartError::ArithmeticOverflow {
                    context: "baseline slot",
                })?;
            let request = [SpawnRequest {
                key: SpawnKey {
                    domain: SpawnDomain::Baseline,
                    slot: u64::from(slot_u32),
                },
            }];
            let slot_config = remaining_spawn_config(
                world_step.prefix.baseline_spawn,
                self.diagnostics.candidates_examined,
                self.diagnostics.geometry_checks,
                request[0].key,
            )?;
            let source_rng = self
                .rng
                .as_ref()
                .and_then(|rng| rng.baselines.get(slot))
                .filter(|rng| rng.slot == slot_u32)
                .ok_or(GenerationStartError::InvalidSource {
                    reason: "baseline RNG slots are not dense and canonical",
                })?
                .state
                .clone();
            let (placement, next_rng, spawn_diagnostics) = {
                let prepared = self.spawn.prepare(
                    &self.world,
                    &request,
                    &source_rng,
                    slot_config,
                    world_step.prefix.baseline_spawn.snake_start_len,
                )?;
                if prepared.placements().len() != 1 {
                    return Err(GenerationStartError::InternalShapeMismatch);
                }
                let placement = prepared.placements()[0];
                let body = prepared
                    .body_for(&placement)
                    .ok_or(GenerationStartError::InternalShapeMismatch)?;
                self.baseline_body.clear();
                self.baseline_body.extend_from_slice(body);
                (
                    placement,
                    prepared.next_rng().clone(),
                    prepared.diagnostics(),
                )
            };
            self.accumulate_spawn_diagnostics(spawn_diagnostics)?;
            self.rng
                .as_mut()
                .and_then(|rng| rng.baselines.get_mut(slot))
                .ok_or(GenerationStartError::InternalShapeMismatch)?
                .state = next_rng;

            let body_start = self.world.body_points.len();
            self.world
                .body_points
                .extend_from_slice(&self.baseline_body);
            let offset =
                u64::try_from(slot).map_err(|_| GenerationStartError::ArithmeticOverflow {
                    context: "baseline identity offset",
                })?;
            let frame_offset = source
                .config
                .population_count
                .checked_add(slot)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(GenerationStartError::ArithmeticOverflow {
                    context: "baseline frame identity offset",
                })?;
            let snake_id = baseline_ids.first.checked_add(offset).ok_or(
                GenerationStartError::ArithmeticOverflow {
                    context: "baseline snake identity",
                },
            )?;
            let frame_v1_id = frame_ids.first.checked_add(frame_offset).ok_or(
                GenerationStartError::ArithmeticOverflow {
                    context: "baseline frame-v1 identity",
                },
            )?;
            self.world.snakes.push(new_snake(
                snake_id,
                frame_v1_id,
                SnakeKind::Baseline,
                placement,
                BodyRange {
                    start: body_start,
                    len: self.baseline_body.len(),
                },
                None,
                None,
                Some(slot_u32),
                Some(super::state::BaselineStrategyState::Roam),
                BASELINE_SNAKE_SKIN,
                world_step.prefix.baseline_spawn.snake_radius,
                world_step.prefix.baseline_snake_base_speed,
                0.0,
            ));
        }
        Ok(())
    }

    fn prepare_initial_pellets(
        &mut self,
        world_step: WorldStepConfig,
    ) -> Result<(), GenerationStartError> {
        let (source_world_rng, source_allocators) = {
            let rng = self
                .rng
                .as_ref()
                .ok_or(GenerationStartError::InternalShapeMismatch)?;
            let allocators = self
                .allocators
                .as_ref()
                .ok_or(GenerationStartError::InternalShapeMismatch)?;
            (rng.world.clone(), allocators.clone())
        };
        let prepared = self.ambient.prepare_initial_fill(
            &source_world_rng,
            &source_allocators,
            world_step.prefix.ambient,
            world_step.prefix.maximum_pellets,
        )?;
        self.world.pellets.extend_from_slice(prepared.generated());
        self.rng
            .as_mut()
            .ok_or(GenerationStartError::InternalShapeMismatch)?
            .world = prepared.next_rng().clone();
        *self
            .allocators
            .as_mut()
            .ok_or(GenerationStartError::InternalShapeMismatch)? =
            prepared.next_allocators().clone();
        self.diagnostics.ambient = prepared.diagnostics();
        Ok(())
    }

    fn accumulate_spawn_diagnostics(
        &mut self,
        diagnostics: SpawnCapacityDiagnostics,
    ) -> Result<(), GenerationStartError> {
        self.diagnostics.candidates_examined = self
            .diagnostics
            .candidates_examined
            .checked_add(diagnostics.candidates_examined)
            .ok_or(GenerationStartError::ArithmeticOverflow {
                context: "generation spawn candidates",
            })?;
        self.diagnostics.fallback_placements = self
            .diagnostics
            .fallback_placements
            .checked_add(diagnostics.fallback_placements)
            .ok_or(GenerationStartError::ArithmeticOverflow {
                context: "generation fallback placements",
            })?;
        self.diagnostics.geometry_checks = self
            .diagnostics
            .geometry_checks
            .checked_add(diagnostics.geometry_checks)
            .ok_or(GenerationStartError::ArithmeticOverflow {
                context: "generation spawn geometry checks",
            })?;
        self.diagnostics.spawn = diagnostics;
        Ok(())
    }

    fn update_capacity_diagnostics(&mut self) {
        self.diagnostics.spawn = self.spawn.diagnostics();
        self.diagnostics.request_capacity = self.requests.capacity();
        self.diagnostics.placement_capacity = self.copied_placements.capacity();
        self.diagnostics.evolved_body_capacity = self.evolved_body_points.capacity();
        self.diagnostics.baseline_body_capacity = self.baseline_body.capacity();
        self.diagnostics.snake_capacity = self.world.snakes.capacity();
        self.diagnostics.body_point_capacity = self.world.body_points.capacity();
        self.diagnostics.pellet_capacity = self.world.pellets.capacity();
    }

    fn prepared<'workspace, 'source>(
        &'workspace self,
        source: &'source StateCandidate,
        config: GenerationStartConfig,
    ) -> Result<PreparedGenerationStart<'workspace, 'source>, GenerationStartError> {
        if !self.ready {
            return Err(GenerationStartError::ResultNotReady);
        }
        Ok(PreparedGenerationStart {
            source,
            config,
            world: &self.world,
            rng: self
                .rng
                .as_ref()
                .ok_or(GenerationStartError::InternalShapeMismatch)?,
            allocators: self
                .allocators
                .as_ref()
                .ok_or(GenerationStartError::InternalShapeMismatch)?,
            fixed_step: self
                .fixed_step
                .as_ref()
                .ok_or(GenerationStartError::InternalShapeMismatch)?,
            diagnostics: self.diagnostics(),
        })
    }

    fn clear(&mut self) {
        self.world.snakes.clear();
        self.world.body_points.clear();
        self.world.pellets.clear();
        self.world.controller_leases.clear();
        self.requests.clear();
        self.copied_placements.clear();
        self.evolved_body_points.clear();
        self.baseline_body.clear();
        self.fixed_step = None;
        self.diagnostics = GenerationStartDiagnostics::default();
        self.prepared_source_address = None;
        self.prepared_config = None;
        self.ready = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn new_snake(
    id: u64,
    frame_v1_id: u32,
    kind: SnakeKind,
    placement: SpawnPlacement,
    body: BodyRange,
    population_slot: Option<u32>,
    brain: Option<super::state::BrainHandle>,
    baseline_slot: Option<u32>,
    baseline_strategy: Option<super::state::BaselineStrategyState>,
    skin: u32,
    radius: f64,
    speed: f64,
    control_accumulator_seconds: f64,
) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id,
        kind,
        alive: true,
        population_slot,
        brain,
        baseline_slot,
        baseline_strategy,
        position: placement.head,
        previous_position: placement.head,
        direction: placement.direction,
        radius,
        speed,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 0.0,
        kills: 0,
        target_length: body.len as f64,
        fitness: 0.0,
        turn: 0.0,
        previous_turn: 0.0,
        input_boost: false,
        previous_input_boost: false,
        control_accumulator_seconds,
        delivered_observation_points: 0.0,
        body,
        skin,
    }
}

fn remaining_spawn_config(
    mut config: SpawnConfig,
    candidates_used: usize,
    geometry_used: usize,
    key: SpawnKey,
) -> Result<SpawnConfig, GenerationStartError> {
    config.maximum_candidates_per_batch = config
        .maximum_candidates_per_batch
        .checked_sub(candidates_used)
        .filter(|remaining| *remaining != 0)
        .ok_or_else(|| {
            GenerationStartError::Spawn(Box::new(SpawnError::WorkBudgetExceeded {
                key,
                work: "generation spawn candidates",
                required: candidates_used.saturating_add(1),
                maximum: config.maximum_candidates_per_batch,
            }))
        })?;
    config.maximum_geometry_checks_per_batch = config
        .maximum_geometry_checks_per_batch
        .checked_sub(geometry_used)
        .filter(|remaining| *remaining != 0)
        .ok_or_else(|| {
            GenerationStartError::Spawn(Box::new(SpawnError::WorkBudgetExceeded {
                key,
                work: "generation spawn geometry checks",
                required: geometry_used.saturating_add(1),
                maximum: config.maximum_geometry_checks_per_batch,
            }))
        })?;
    Ok(config)
}

fn reserve_for<T>(
    target: &mut Vec<T>,
    required: usize,
    storage: &'static str,
) -> Result<(), GenerationStartError> {
    if required <= target.capacity() {
        return Ok(());
    }
    target
        .try_reserve_exact(required - target.len())
        .map_err(|_| GenerationStartError::AllocationFailed { storage, required })
}

/// Checked generation-construction failure that never publishes partial state.
#[derive(Debug)]
pub enum GenerationStartError {
    /// The supplied boundary is not the exact admitted pre-spawn shape.
    InvalidSource { reason: &'static str },
    /// A projected construction setting is unsupported.
    InvalidConfig { field: &'static str },
    /// Checked size or identity arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// An admitted container ceiling cannot fit the complete construction.
    CapacityExceeded {
        storage: &'static str,
        required: usize,
        maximum: usize,
    },
    /// A reusable staging buffer could not reserve its checked requirement.
    AllocationFailed {
        storage: &'static str,
        required: usize,
    },
    /// Complete-body placement failed.
    Spawn(Box<SpawnError>),
    /// Initial ambient generation failed.
    Ambient(Box<AmbientError>),
    /// Baseline lifecycle initialization failed.
    Baseline(Box<BaselineLifecycleError>),
    /// Deterministic identity allocation failed.
    State(Box<StateError>),
    /// A prepared result was checked against a different boundary object.
    SourceChanged,
    /// A prepared result was checked against a different projection.
    ConfigChanged,
    /// Internal staging shapes disagreed after checked preparation.
    InternalShapeMismatch,
    /// No complete proposal is available after the latest attempt.
    ResultNotReady,
    /// The complete projected world-step contract is invalid.
    WorldStep(Box<WorldStepError>),
    /// The admitted normalized configuration could not be projected exactly.
    StepConfig(Box<StepConfigError>),
}

impl Display for GenerationStartError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSource { reason } => {
                write!(formatter, "invalid generation boundary: {reason}")
            }
            Self::InvalidConfig { field } => write!(formatter, "invalid generation-start {field}"),
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "generation-start arithmetic overflow: {context}")
            }
            Self::CapacityExceeded {
                storage,
                required,
                maximum,
            } => write!(
                formatter,
                "generation-start {storage} requires {required}, maximum {maximum}"
            ),
            Self::AllocationFailed { storage, required } => write!(
                formatter,
                "generation-start {storage} allocation failed for {required} records"
            ),
            Self::Spawn(error) => write!(formatter, "generation spawn failed: {error}"),
            Self::Ambient(error) => write!(formatter, "initial ambient fill failed: {error}"),
            Self::Baseline(error) => {
                write!(
                    formatter,
                    "baseline generation initialization failed: {error}"
                )
            }
            Self::State(error) => {
                write!(formatter, "generation identity allocation failed: {error}")
            }
            Self::SourceChanged => write!(formatter, "generation-start source boundary changed"),
            Self::ConfigChanged => write!(formatter, "generation-start configuration changed"),
            Self::InternalShapeMismatch => {
                write!(formatter, "generation-start staging shape mismatch")
            }
            Self::ResultNotReady => write!(formatter, "generation-start result is not ready"),
            Self::WorldStep(error) => {
                write!(formatter, "generation-start world contract failed: {error}")
            }
            Self::StepConfig(error) => {
                write!(
                    formatter,
                    "generation-start config projection failed: {error}"
                )
            }
        }
    }
}

impl Error for GenerationStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Spawn(error) => Some(error),
            Self::Ambient(error) => Some(error),
            Self::Baseline(error) => Some(error),
            Self::State(error) => Some(error),
            Self::WorldStep(error) => Some(error),
            Self::StepConfig(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SpawnError> for GenerationStartError {
    fn from(error: SpawnError) -> Self {
        Self::Spawn(Box::new(error))
    }
}

impl From<AmbientError> for GenerationStartError {
    fn from(error: AmbientError) -> Self {
        Self::Ambient(Box::new(error))
    }
}

impl From<BaselineLifecycleError> for GenerationStartError {
    fn from(error: BaselineLifecycleError) -> Self {
        Self::Baseline(Box::new(error))
    }
}

impl From<StateError> for GenerationStartError {
    fn from(error: StateError) -> Self {
        Self::State(Box::new(error))
    }
}

impl From<StepConfigError> for GenerationStartError {
    fn from(error: StepConfigError) -> Self {
        Self::StepConfig(Box::new(error))
    }
}
