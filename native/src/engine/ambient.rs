//! Deterministic fixed-step ambient-pellet generation.
//!
//! The current TypeScript world accumulates a fractional spawn count once per
//! fixed step, caps realized pellets by the configured target deficit, and
//! generates each due pellet from the world RNG before any controller samples
//! the boundary. This module preserves that timing and draw order while
//! staging new exact-ID pellet records without mutating authoritative state.

use super::physics::{PhysicsStepKey, PhysicsStepKeyField};
use super::rng::{RngError, SerializedRngState, StatefulRng};
use super::state::{AllocatorState, PelletState, StateError, WorldPoint, WorldState};
use std::error::Error;
use std::f64::consts::TAU;
use std::fmt::{Display, Formatter};

/// First Rust ambient-pellet algorithm identity.
pub const AMBIENT_PELLET_ALGORITHM_VERSION: u32 = 1;
/// Stable frame-v1 pellet-kind value for ambient food.
pub const AMBIENT_PELLET_KIND: u32 = 0;
/// TypeScript-reference rejection-attempt count.
const REJECTION_RETRIES: usize = 8;
/// TypeScript-reference world-time drift multiplier.
const TIME_DRIFT_SCALE: f64 = 0.04;

/// Versioned ambient-pellet settings projected from one admitted config.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientPelletConfig {
    /// Versioned generation formula and draw-order identity.
    pub algorithm_version: u32,
    /// Desired ambient population before death/boost additions.
    pub target_count: usize,
    /// Fractional ambient pellets accumulated per simulated second.
    pub spawn_per_second: f64,
    /// Circular world radius.
    pub world_radius: f64,
    /// Food value assigned to every ambient pellet.
    pub food_value: f64,
    /// Whether density fades toward the arena edge.
    pub edge_falloff_enabled: bool,
    /// Radius fraction where the edge fade begins.
    pub edge_fade_start: f64,
    /// Edge smoothstep exponent.
    pub edge_fade_power: f64,
    /// Ridged-filament contrast exponent.
    pub filament_power: f64,
    /// Domain-warp frequency.
    pub warp_frequency: f64,
    /// Domain-warp scale as a fraction of world radius.
    pub warp_scale_fraction: f64,
    /// Large interference frequency.
    pub large_frequency: f64,
    /// Medium interference frequency.
    pub medium_frequency: f64,
    /// Small interference frequency.
    pub small_frequency: f64,
    /// High-frequency dust contribution.
    pub dust_strength: f64,
}

impl AmbientPelletConfig {
    /// Current TypeScript defaults.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            algorithm_version: AMBIENT_PELLET_ALGORITHM_VERSION,
            target_count: 3_500,
            spawn_per_second: 170.0,
            world_radius: 3_500.0,
            food_value: 1.0,
            edge_falloff_enabled: true,
            edge_fade_start: 0.35,
            edge_fade_power: 2.6,
            filament_power: 4.2,
            warp_frequency: 0.0013,
            warp_scale_fraction: 0.08,
            large_frequency: 0.0026,
            medium_frequency: 0.0042,
            small_frequency: 0.0068,
            dust_strength: 0.35,
        }
    }

    pub(crate) fn validate(self, maximum_pellets: usize) -> Result<(), AmbientError> {
        for (field, value) in [
            ("spawn_per_second", self.spawn_per_second),
            ("world_radius", self.world_radius),
            ("food_value", self.food_value),
            ("edge_fade_start", self.edge_fade_start),
            ("edge_fade_power", self.edge_fade_power),
            ("filament_power", self.filament_power),
            ("warp_frequency", self.warp_frequency),
            ("warp_scale_fraction", self.warp_scale_fraction),
            ("large_frequency", self.large_frequency),
            ("medium_frequency", self.medium_frequency),
            ("small_frequency", self.small_frequency),
            ("dust_strength", self.dust_strength),
        ] {
            if !value.is_finite() {
                return Err(AmbientError::InvalidConfig { field });
            }
        }
        if self.algorithm_version != AMBIENT_PELLET_ALGORITHM_VERSION
            || self.target_count > maximum_pellets
            || self.spawn_per_second < 0.0
            || self.world_radius <= 0.0
            || self.food_value <= 0.0
            || !(0.0..=0.95).contains(&self.edge_fade_start)
            || self.edge_fade_power < 0.1
            || self.filament_power < 0.1
            || self.warp_frequency < 0.0
            || self.warp_scale_fraction < 0.0
            || self.large_frequency < 0.0
            || self.medium_frequency < 0.0
            || self.small_frequency < 0.0
            || !(0.0..=1.0).contains(&self.dust_strength)
        {
            return Err(AmbientError::InvalidConfig {
                field: "ambient ranges",
            });
        }
        Ok(())
    }
}

/// Sizes and retained allocation for the latest ambient preparation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AmbientDiagnostics {
    /// Ambient pellets generated for the fixed step.
    pub generated: usize,
    /// Number of source pellets considered when computing target deficit.
    pub source_pellets: usize,
    /// Rejection candidates examined across all generated pellets.
    pub candidate_attempts: usize,
    /// Retained generated-pellet capacity.
    pub generated_capacity: usize,
    /// Retained serialized world-RNG string capacity.
    pub rng_text_capacity: usize,
}

/// Read-only complete ambient proposal for a later whole-step transaction.
#[derive(Clone, Copy, Debug)]
pub struct PreparedAmbient<'ambient, 'source> {
    key: PhysicsStepKey,
    source_world: &'source WorldState,
    source_world_rng: &'source SerializedRngState,
    source_allocators: &'source AllocatorState,
    source_accumulator: f64,
    generation_time_after_advance: f64,
    fixed_dt: f64,
    config: AmbientPelletConfig,
    maximum_pellets: usize,
    generated: &'ambient [PelletState],
    next_rng: &'ambient SerializedRngState,
    next_allocators: &'ambient AllocatorState,
    next_accumulator: f64,
    diagnostics: AmbientDiagnostics,
}

impl<'ambient, 'source> PreparedAmbient<'ambient, 'source> {
    /// Complete authority/config/operation identity prepared.
    #[must_use]
    pub const fn key(self) -> PhysicsStepKey {
        self.key
    }

    /// Immutable source boundary used to compute the proposal.
    #[must_use]
    pub const fn source_world(self) -> &'source WorldState {
        self.source_world
    }

    /// Revalidate every source input before a coordinator accepts the result.
    #[allow(clippy::too_many_arguments)]
    pub fn validate_current(
        self,
        current_key: PhysicsStepKey,
        current_world: &WorldState,
        current_world_rng: &SerializedRngState,
        current_allocators: &AllocatorState,
        current_accumulator: f64,
        current_generation_time_after_advance: f64,
        current_fixed_dt: f64,
        current_config: AmbientPelletConfig,
        current_maximum_pellets: usize,
    ) -> Result<(), AmbientError> {
        if let Some(field) = self.key.first_mismatch(current_key) {
            return Err(AmbientError::StepKeyMismatch { field });
        }
        if !std::ptr::eq(self.source_world, current_world) {
            return Err(AmbientError::SourceChanged { field: "world" });
        }
        if self.source_world_rng != current_world_rng {
            return Err(AmbientError::SourceChanged { field: "world RNG" });
        }
        if self.source_allocators != current_allocators {
            return Err(AmbientError::SourceChanged {
                field: "allocators",
            });
        }
        if self.source_accumulator.to_bits() != current_accumulator.to_bits() {
            return Err(AmbientError::SourceChanged {
                field: "spawn accumulator",
            });
        }
        if self.generation_time_after_advance.to_bits()
            != current_generation_time_after_advance.to_bits()
        {
            return Err(AmbientError::SourceChanged {
                field: "generation time",
            });
        }
        if self.fixed_dt.to_bits() != current_fixed_dt.to_bits() {
            return Err(AmbientError::SourceChanged {
                field: "fixed delta",
            });
        }
        if self.config != current_config {
            return Err(AmbientError::SourceChanged { field: "config" });
        }
        if self.maximum_pellets != current_maximum_pellets {
            return Err(AmbientError::SourceChanged {
                field: "pellet capacity",
            });
        }
        Ok(())
    }

    /// New ambient pellets in exact generation/ID order.
    #[must_use]
    pub const fn generated(self) -> &'ambient [PelletState] {
        self.generated
    }

    /// World-RNG continuation after every attempted rejection candidate.
    #[must_use]
    pub const fn next_rng(self) -> &'ambient SerializedRngState {
        self.next_rng
    }

    /// Allocator continuation after reserving exact pellet IDs.
    #[must_use]
    pub const fn next_allocators(self) -> &'ambient AllocatorState {
        self.next_allocators
    }

    /// Fractional due-pellet accumulator retained after realized spawns.
    #[must_use]
    pub const fn next_accumulator(self) -> f64 {
        self.next_accumulator
    }

    /// Current size and retained-capacity diagnostics.
    #[must_use]
    pub const fn diagnostics(self) -> AmbientDiagnostics {
        self.diagnostics
    }
}

/// Complete initial ambient fill prepared at an exact pre-spawn boundary.
#[derive(Clone, Copy, Debug)]
pub struct PreparedInitialAmbient<'ambient> {
    generated: &'ambient [PelletState],
    next_rng: &'ambient SerializedRngState,
    next_allocators: &'ambient AllocatorState,
    diagnostics: AmbientDiagnostics,
}

impl<'ambient> PreparedInitialAmbient<'ambient> {
    /// Initial ambient pellets in exact allocation/draw order.
    #[must_use]
    pub const fn generated(self) -> &'ambient [PelletState] {
        self.generated
    }

    /// World-RNG continuation after the complete initial fill.
    #[must_use]
    pub const fn next_rng(self) -> &'ambient SerializedRngState {
        self.next_rng
    }

    /// Allocator continuation after reserving every initial pellet identity.
    #[must_use]
    pub const fn next_allocators(self) -> &'ambient AllocatorState {
        self.next_allocators
    }

    /// Current work and retained-capacity diagnostics.
    #[must_use]
    pub const fn diagnostics(self) -> AmbientDiagnostics {
        self.diagnostics
    }
}

/// Reusable, non-authoritative ambient generation scratch.
#[derive(Debug, Default)]
pub struct AmbientWorkspace {
    generated: Vec<PelletState>,
    next_rng: Option<SerializedRngState>,
    next_rng_gaussian_spare: String,
    next_allocators: Option<AllocatorState>,
    next_accumulator: f64,
    ready: bool,
    source_pellets: usize,
    candidate_attempts: usize,
}

impl AmbientWorkspace {
    /// Construct empty reusable scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage every ambient pellet due at one stable pre-control boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'ambient, 'source>(
        &'ambient mut self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_world_rng: &'source SerializedRngState,
        source_allocators: &'source AllocatorState,
        source_accumulator: f64,
        generation_time_after_advance: f64,
        fixed_dt: f64,
        config: AmbientPelletConfig,
        maximum_pellets: usize,
    ) -> Result<PreparedAmbient<'ambient, 'source>, AmbientError> {
        self.clear();
        validate_step_key(key)?;
        config.validate(maximum_pellets)?;
        for (field, value) in [
            ("source_accumulator", source_accumulator),
            (
                "generation_time_after_advance",
                generation_time_after_advance,
            ),
            ("fixed_dt", fixed_dt),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(AmbientError::InvalidBoundary { field });
            }
        }
        if fixed_dt == 0.0 {
            return Err(AmbientError::InvalidBoundary { field: "fixed_dt" });
        }
        if source_world.pellets.len() > maximum_pellets {
            return Err(AmbientError::SourcePelletCapacityExceeded {
                actual: source_world.pellets.len(),
                maximum: maximum_pellets,
            });
        }

        let accrued = config.spawn_per_second * fixed_dt;
        let accumulated = source_accumulator + accrued;
        if !accrued.is_finite() || !accumulated.is_finite() || accumulated < 0.0 {
            return Err(AmbientError::NonFiniteGenerated {
                field: "spawn accumulator",
            });
        }
        let deficit = config
            .target_count
            .saturating_sub(source_world.pellets.len());
        let generated = if accumulated >= deficit as f64 {
            deficit
        } else {
            accumulated.floor() as usize
        };
        let next_accumulator = accumulated - generated as f64;
        if !next_accumulator.is_finite() || next_accumulator < 0.0 {
            return Err(AmbientError::NonFiniteGenerated {
                field: "remaining spawn accumulator",
            });
        }
        let total = source_world.pellets.len().checked_add(generated).ok_or(
            AmbientError::ArithmeticOverflow {
                context: "post-ambient pellet count",
            },
        )?;
        if total > maximum_pellets {
            return Err(AmbientError::PelletCapacityExceeded {
                required: total,
                maximum: maximum_pellets,
            });
        }
        self.stage_generated(
            source_world_rng,
            source_allocators,
            generated,
            generation_time_after_advance,
            config,
        )?;
        self.next_accumulator = next_accumulator;
        self.source_pellets = source_world.pellets.len();
        self.ready = true;
        self.prepared(
            key,
            source_world,
            source_world_rng,
            source_allocators,
            source_accumulator,
            generation_time_after_advance,
            fixed_dt,
            config,
            maximum_pellets,
        )
    }

    /// Fill an exact pre-spawn boundary to the configured ambient target.
    ///
    /// This is the construction counterpart to [`Self::prepare`]. It starts at
    /// generation time zero with no accumulated spawn credit and therefore
    /// creates exactly `target_count` pellets without inventing a fake fixed
    /// delta or accumulator. The world RNG continues after prior evolved-snake
    /// placement, matching the current TypeScript construction order.
    pub fn prepare_initial_fill<'ambient>(
        &'ambient mut self,
        source_world_rng: &SerializedRngState,
        source_allocators: &AllocatorState,
        config: AmbientPelletConfig,
        maximum_pellets: usize,
    ) -> Result<PreparedInitialAmbient<'ambient>, AmbientError> {
        self.clear();
        config.validate(maximum_pellets)?;
        self.stage_generated(
            source_world_rng,
            source_allocators,
            config.target_count,
            0.0,
            config,
        )?;
        self.next_accumulator = 0.0;
        self.source_pellets = 0;
        self.ready = true;
        Ok(PreparedInitialAmbient {
            generated: &self.generated,
            next_rng: self.next_rng.as_ref().ok_or(AmbientError::ShapeMismatch)?,
            next_allocators: self
                .next_allocators
                .as_ref()
                .ok_or(AmbientError::ShapeMismatch)?,
            diagnostics: self.diagnostics(),
        })
    }

    /// Whether the latest preparation produced a complete proposal.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Current sizes and retained allocation, including after rejection.
    #[must_use]
    pub fn diagnostics(&self) -> AmbientDiagnostics {
        AmbientDiagnostics {
            generated: self.generated.len(),
            source_pellets: self.source_pellets,
            candidate_attempts: self.candidate_attempts,
            generated_capacity: self.generated.capacity(),
            rng_text_capacity: self.next_rng.as_ref().map_or(
                self.next_rng_gaussian_spare.capacity(),
                |state| {
                    serialized_rng_text_capacity(state)
                        .saturating_add(self.next_rng_gaussian_spare.capacity())
                },
            ),
        }
    }

    fn stage_generated(
        &mut self,
        source_world_rng: &SerializedRngState,
        source_allocators: &AllocatorState,
        generated: usize,
        generation_time: f64,
        config: AmbientPelletConfig,
    ) -> Result<(), AmbientError> {
        if !generation_time.is_finite() || generation_time < 0.0 {
            return Err(AmbientError::InvalidBoundary {
                field: "generation_time",
            });
        }
        reserve_for(&mut self.generated, generated, "ambient pellets")?;

        let mut next_allocators = source_allocators.clone();
        let generated_u64 =
            u64::try_from(generated).map_err(|_| AmbientError::ArithmeticOverflow {
                context: "ambient pellet ID count",
            })?;
        let reservation = next_allocators
            .reserve_entity_ids(generated_u64)
            .map_err(AmbientError::Allocator)?;
        copy_serialized_rng_reusing(
            &mut self.next_rng,
            source_world_rng,
            &mut self.next_rng_gaussian_spare,
        )?;
        if generated != 0 {
            let reservation = reservation.ok_or(AmbientError::ShapeMismatch)?;
            let mut next_id = reservation.first;
            let mut rng = StatefulRng::from_state(source_world_rng).map_err(AmbientError::Rng)?;
            for _ in 0..generated {
                let (position, attempts) = generate_position(&mut rng, generation_time, config)?;
                self.candidate_attempts = self.candidate_attempts.checked_add(attempts).ok_or(
                    AmbientError::ArithmeticOverflow {
                        context: "ambient candidate attempts",
                    },
                )?;
                self.generated.push(PelletState {
                    id: next_id,
                    position,
                    value: config.food_value,
                    kind: AMBIENT_PELLET_KIND,
                    color: 0,
                    owner: None,
                });
                next_id = next_id
                    .checked_add(1)
                    .ok_or(AmbientError::ArithmeticOverflow {
                        context: "ambient pellet ID continuation",
                    })?;
            }
            let expected_next =
                reservation
                    .last
                    .checked_add(1)
                    .ok_or(AmbientError::ArithmeticOverflow {
                        context: "ambient pellet reservation end",
                    })?;
            if next_id != expected_next {
                return Err(AmbientError::ShapeMismatch);
            }
            rng.export_state_into_reusing(
                self.next_rng.as_mut().ok_or(AmbientError::ShapeMismatch)?,
                &mut self.next_rng_gaussian_spare,
            );
        }
        match &mut self.next_allocators {
            Some(current) => current.clone_from(&next_allocators),
            None => self.next_allocators = Some(next_allocators),
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn prepared<'ambient, 'source>(
        &'ambient self,
        key: PhysicsStepKey,
        source_world: &'source WorldState,
        source_world_rng: &'source SerializedRngState,
        source_allocators: &'source AllocatorState,
        source_accumulator: f64,
        generation_time_after_advance: f64,
        fixed_dt: f64,
        config: AmbientPelletConfig,
        maximum_pellets: usize,
    ) -> Result<PreparedAmbient<'ambient, 'source>, AmbientError> {
        if !self.ready {
            return Err(AmbientError::ResultNotReady);
        }
        Ok(PreparedAmbient {
            key,
            source_world,
            source_world_rng,
            source_allocators,
            source_accumulator,
            generation_time_after_advance,
            fixed_dt,
            config,
            maximum_pellets,
            generated: &self.generated,
            next_rng: self.next_rng.as_ref().ok_or(AmbientError::ShapeMismatch)?,
            next_allocators: self
                .next_allocators
                .as_ref()
                .ok_or(AmbientError::ShapeMismatch)?,
            next_accumulator: self.next_accumulator,
            diagnostics: self.diagnostics(),
        })
    }

    fn clear(&mut self) {
        self.generated.clear();
        self.next_accumulator = 0.0;
        self.ready = false;
        self.source_pellets = 0;
        self.candidate_attempts = 0;
    }
}

fn generate_position(
    rng: &mut StatefulRng,
    generation_time: f64,
    config: AmbientPelletConfig,
) -> Result<(WorldPoint, usize), AmbientError> {
    generate_position_from(&mut || rng.next_f64(), generation_time, config)
}

fn generate_position_from(
    next_uniform: &mut impl FnMut() -> f64,
    generation_time: f64,
    config: AmbientPelletConfig,
) -> Result<(WorldPoint, usize), AmbientError> {
    let drift = generation_time * TIME_DRIFT_SCALE;
    let warp_scale = config.warp_scale_fraction * config.world_radius;
    if !drift.is_finite() || !warp_scale.is_finite() {
        return Err(AmbientError::NonFiniteGenerated {
            field: "ambient drift or warp",
        });
    }
    let mut best_probability = -1.0;
    let mut best = WorldPoint { x: 0.0, y: 0.0 };
    for attempt in 0..REJECTION_RETRIES {
        let angle = next_uniform() * TAU;
        let radius = next_uniform().sqrt() * config.world_radius;
        let x = angle.cos() * radius;
        let y = angle.sin() * radius;

        let warp_x = (y * config.warp_frequency + drift * 0.7).sin() * warp_scale
            + (x * config.warp_frequency * 1.25 - drift * 0.4).cos() * (warp_scale * 0.6);
        let warp_y = (x * config.warp_frequency - drift * 0.5).cos() * warp_scale
            + (y * config.warp_frequency * 1.1 + drift * 0.8).sin() * (warp_scale * 0.6);
        let warped_x = x + warp_x;
        let warped_y = y + warp_y;

        let first = ((warped_x + warped_y) * config.large_frequency + drift).sin()
            + ((warped_x - warped_y) * config.large_frequency - drift * 0.7).cos();
        let second = (warped_x * config.medium_frequency - drift * 1.1).sin()
            + (warped_y * config.medium_frequency + drift * 0.9).cos();
        let third = (warped_x * config.small_frequency + drift * 1.6).sin()
            * (warped_y * config.small_frequency - drift * 1.3).cos();

        let ridge_a = clamp(1.0 - first.abs() / 2.0, 0.0, 1.0);
        let ridge_b = clamp(1.0 - second.abs() / 2.0, 0.0, 1.0);
        let ridge_c = clamp(1.0 - third.abs(), 0.0, 1.0);
        let web_a = ridge_a.powf(config.filament_power);
        let web_b = ridge_b.powf(config.filament_power * 0.95);
        let dust = ridge_c.powf(2.2) * config.dust_strength;
        let mut probability = clamp(web_a.max(web_b) + dust, 0.0, 1.0);

        if config.edge_falloff_enabled {
            let edge_t = radius / config.world_radius;
            let edge_ramp = clamp(
                (edge_t - config.edge_fade_start) / (1.0 - config.edge_fade_start),
                0.0,
                1.0,
            );
            let edge_smooth = edge_ramp * edge_ramp * (3.0 - 2.0 * edge_ramp);
            let edge_falloff = clamp(1.0 - edge_smooth.powf(config.edge_fade_power), 0.0, 1.0);
            probability *= edge_falloff;
        }
        validate_generated(x, "ambient pellet x")?;
        validate_generated(y, "ambient pellet y")?;
        validate_generated(probability, "ambient acceptance probability")?;
        if probability > best_probability {
            best_probability = probability;
            best = WorldPoint { x, y };
        }
        if next_uniform() < probability {
            return Ok((WorldPoint { x, y }, attempt + 1));
        }
    }
    Ok((best, REJECTION_RETRIES))
}

fn validate_step_key(key: PhysicsStepKey) -> Result<(), AmbientError> {
    if key.world_epoch() == 0
        || key.generation() == 0
        || key.population_epoch() == 0
        || key.config_revision() == 0
        || key.operation_epoch() == 0
    {
        return Err(AmbientError::InvalidStepKey);
    }
    Ok(())
}

fn validate_generated(value: f64, field: &'static str) -> Result<(), AmbientError> {
    if !value.is_finite() {
        return Err(AmbientError::NonFiniteGenerated { field });
    }
    Ok(())
}

fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    value.max(minimum).min(maximum)
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), AmbientError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| AmbientError::AllocationFailed { context, required })?;
    }
    Ok(())
}

fn copy_serialized_rng_reusing(
    target: &mut Option<SerializedRngState>,
    source: &SerializedRngState,
    retained_gaussian_spare: &mut String,
) -> Result<(), AmbientError> {
    if target.is_none() {
        *target = Some(SerializedRngState {
            algorithm: String::new(),
            version: 0,
            state_hex: String::new(),
            gaussian_algorithm: String::new(),
            gaussian_version: 0,
            gaussian_spare_valid: false,
            gaussian_spare_hex: None,
        });
    }
    let target = target.as_mut().ok_or(AmbientError::ShapeMismatch)?;
    reserve_string(
        &mut target.algorithm,
        source.algorithm.len(),
        "RNG algorithm",
    )?;
    reserve_string(&mut target.state_hex, source.state_hex.len(), "RNG state")?;
    reserve_string(
        &mut target.gaussian_algorithm,
        source.gaussian_algorithm.len(),
        "Gaussian algorithm",
    )?;
    match &source.gaussian_spare_hex {
        Some(source_spare) => {
            if target.gaussian_spare_hex.is_none() {
                reserve_string(
                    retained_gaussian_spare,
                    source_spare.len(),
                    "Gaussian spare",
                )?;
                target.gaussian_spare_hex = Some(std::mem::take(retained_gaussian_spare));
            }
            let target_spare = target
                .gaussian_spare_hex
                .as_mut()
                .ok_or(AmbientError::ShapeMismatch)?;
            reserve_string(target_spare, source_spare.len(), "Gaussian spare")?;
            target_spare.clear();
            target_spare.push_str(source_spare);
        }
        None => {
            if let Some(mut spare) = target.gaussian_spare_hex.take() {
                spare.clear();
                if spare.capacity() > retained_gaussian_spare.capacity() {
                    *retained_gaussian_spare = spare;
                }
            }
        }
    }
    target.algorithm.clear();
    target.algorithm.push_str(&source.algorithm);
    target.version = source.version;
    target.state_hex.clear();
    target.state_hex.push_str(&source.state_hex);
    target.gaussian_algorithm.clear();
    target
        .gaussian_algorithm
        .push_str(&source.gaussian_algorithm);
    target.gaussian_version = source.gaussian_version;
    target.gaussian_spare_valid = source.gaussian_spare_valid;
    Ok(())
}

fn reserve_string(
    value: &mut String,
    required: usize,
    context: &'static str,
) -> Result<(), AmbientError> {
    if value.capacity() < required {
        value
            .try_reserve_exact(required.saturating_sub(value.len()))
            .map_err(|_| AmbientError::AllocationFailed { context, required })?;
    }
    Ok(())
}

fn serialized_rng_text_capacity(state: &SerializedRngState) -> usize {
    state
        .algorithm
        .capacity()
        .saturating_add(state.state_hex.capacity())
        .saturating_add(state.gaussian_algorithm.capacity())
        .saturating_add(
            state
                .gaussian_spare_hex
                .as_ref()
                .map_or(0, String::capacity),
        )
}

/// Checked ambient-staging failure. No variant publishes partial authority.
#[derive(Clone, Debug, PartialEq)]
pub enum AmbientError {
    /// One fixed-step identity component is zero or otherwise invalid.
    InvalidStepKey,
    /// A newer complete authority/config/operation identity superseded staging.
    StepKeyMismatch { field: PhysicsStepKeyField },
    /// A non-key source input changed after preparation.
    SourceChanged { field: &'static str },
    /// A projected configuration value is invalid.
    InvalidConfig { field: &'static str },
    /// One fixed-step boundary scalar is invalid.
    InvalidBoundary { field: &'static str },
    /// The source world already violates admitted pellet capacity.
    SourcePelletCapacityExceeded { actual: usize, maximum: usize },
    /// The complete staged pellet set violates admitted capacity.
    PelletCapacityExceeded { required: usize, maximum: usize },
    /// The serialized world RNG could not be restored.
    Rng(RngError),
    /// Exact pellet-ID reservation failed atomically.
    Allocator(StateError),
    /// Checked count or identity arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// A derived scalar became NaN or infinite.
    NonFiniteGenerated { field: &'static str },
    /// Internal staged lengths or reservations disagreed.
    ShapeMismatch,
    /// Reusable scratch could not reserve the required capacity.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
    /// No complete ambient result is currently staged.
    ResultNotReady,
}

impl Display for AmbientError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStepKey => write!(formatter, "invalid ambient fixed-step key"),
            Self::StepKeyMismatch { field } => {
                write!(formatter, "ambient fixed-step key changed at {field:?}")
            }
            Self::SourceChanged { field } => {
                write!(formatter, "ambient source changed at {field}")
            }
            Self::InvalidConfig { field } => write!(formatter, "invalid ambient config: {field}"),
            Self::InvalidBoundary { field } => {
                write!(formatter, "invalid ambient fixed-step boundary: {field}")
            }
            Self::SourcePelletCapacityExceeded { actual, maximum } => write!(
                formatter,
                "source pellet count {actual} exceeds admitted maximum {maximum}"
            ),
            Self::PelletCapacityExceeded { required, maximum } => write!(
                formatter,
                "ambient result requires {required} pellets but maximum is {maximum}"
            ),
            Self::Rng(source) => write!(formatter, "invalid world RNG: {source}"),
            Self::Allocator(source) => {
                write!(formatter, "ambient pellet ID allocation failed: {source}")
            }
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "ambient arithmetic overflow: {context}")
            }
            Self::NonFiniteGenerated { field } => {
                write!(formatter, "ambient generation produced non-finite {field}")
            }
            Self::ShapeMismatch => write!(formatter, "ambient staged result is inconsistent"),
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "ambient scratch allocation failed for {context} ({required} values)"
            ),
            Self::ResultNotReady => write!(formatter, "ambient result is not ready"),
        }
    }
}

impl Error for AmbientError {}

impl From<RngError> for AmbientError {
    fn from(source: RngError) -> Self {
        Self::Rng(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rng::StatefulRng;
    use crate::engine::state::{
        ALLOCATOR_VERSION, BASELINE_ENTITY_ID_START, EXTERNAL_ENTITY_ID_START,
        RESURRECTED_ENTITY_ID_START,
    };

    fn world_with_pellets(count: usize) -> WorldState {
        WorldState {
            pellets: (0..count)
                .map(|index| PelletState {
                    id: 100 + index as u64,
                    position: WorldPoint {
                        x: index as f64,
                        y: 0.0,
                    },
                    value: 1.0,
                    kind: AMBIENT_PELLET_KIND,
                    color: 0,
                    owner: None,
                })
                .collect(),
            ..WorldState::default()
        }
    }

    fn allocators() -> AllocatorState {
        AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 1_000,
            next_brain_id: 1,
            next_genome_id: 1,
            next_controller_lease_id: 1,
            next_frame_v1_id: 1,
            next_external_id: EXTERNAL_ENTITY_ID_START,
            next_baseline_id: BASELINE_ENTITY_ID_START,
            next_resurrected_id: RESURRECTED_ENTITY_ID_START,
        }
    }

    fn step_key() -> PhysicsStepKey {
        PhysicsStepKey::new(1, 2, 3, 4, 5, [6; 32], 7)
    }

    #[test]
    fn fractional_accumulator_realizes_only_complete_due_pellets() {
        let world = world_with_pellets(0);
        let rng = StatefulRng::new(123.0).export_state();
        let source_allocators = allocators();
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.target_count = 10;
        config.spawn_per_second = 2.0;
        let mut workspace = AmbientWorkspace::new();

        let first = workspace
            .prepare(
                step_key(),
                &world,
                &rng,
                &source_allocators,
                0.0,
                0.25,
                0.25,
                config,
                10,
            )
            .expect("half a pellet should stage without RNG draws");
        assert!(first.generated().is_empty());
        assert_eq!(first.next_accumulator(), 0.5);
        assert_eq!(first.next_rng(), &rng);
        assert_eq!(first.next_allocators(), &source_allocators);
        let next_accumulator = first.next_accumulator();

        let second = workspace
            .prepare(
                step_key(),
                &world,
                &rng,
                &source_allocators,
                next_accumulator,
                0.5,
                0.25,
                config,
                10,
            )
            .expect("the completed pellet should stage");
        assert_eq!(second.generated().len(), 1);
        assert_eq!(second.generated()[0].id, 1_000);
        assert_eq!(second.next_allocators().next_entity_id, 1_001);
        assert_eq!(second.next_accumulator(), 0.0);
        assert_ne!(second.next_rng(), &rng);
    }

    #[test]
    fn target_saturation_retains_backlog_without_rng_or_id_draws() {
        let world = world_with_pellets(4);
        let rng = StatefulRng::new(456.0).export_state();
        let source_allocators = allocators();
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.target_count = 3;
        config.spawn_per_second = 10.0;
        let mut workspace = AmbientWorkspace::new();
        let prepared = workspace
            .prepare(
                step_key(),
                &world,
                &rng,
                &source_allocators,
                2.0,
                1.0,
                0.5,
                config,
                10,
            )
            .expect("above-target worlds accrue but do not spawn");
        assert!(prepared.generated().is_empty());
        assert_eq!(prepared.next_accumulator(), 7.0);
        assert_eq!(prepared.next_rng(), &rng);
        assert_eq!(prepared.next_allocators(), &source_allocators);
    }

    #[test]
    fn current_typescript_sampler_fixture_matches_position_attempts_and_uniform_continuation() {
        let world = world_with_pellets(0);
        let rng = StatefulRng::new(0x0a_11_ce as f64).export_state();
        let source_allocators = allocators();
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.target_count = 1;
        config.spawn_per_second = 0.0;
        let mut workspace = AmbientWorkspace::new();
        let prepared = workspace
            .prepare(
                step_key(),
                &world,
                &rng,
                &source_allocators,
                1.0,
                1.0 / 60.0,
                1.0 / 60.0,
                config,
                1,
            )
            .expect("retained TypeScript ambient fixture should stage");
        let pellet = &prepared.generated()[0];
        assert!((pellet.position.x - -83.247_728_720_165_91).abs() <= 1.0e-9);
        assert!((pellet.position.y - -383.354_116_837_452_9).abs() <= 1.0e-9);
        assert_eq!(pellet.value, 1.0);
        assert_eq!(pellet.kind, AMBIENT_PELLET_KIND);
        assert_eq!(pellet.color, 0);
        assert_eq!(pellet.owner, None);
        assert_eq!(prepared.diagnostics().candidate_attempts, 6);
        assert_eq!(prepared.next_rng().state_hex, "0xa262ccb1");
    }

    #[test]
    fn initial_fill_uses_zero_generation_time_and_ignores_live_spawn_rate() {
        let world = world_with_pellets(0);
        let rng = StatefulRng::new(0x51_a7_7e as f64).export_state();
        let source_allocators = allocators();
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.target_count = 3;
        config.spawn_per_second = 0.0;

        let mut initial_workspace = AmbientWorkspace::new();
        let initial = initial_workspace
            .prepare_initial_fill(&rng, &source_allocators, config, 3)
            .expect("generation construction must fill the configured target");
        let initial_pellets = initial.generated().to_vec();
        let initial_rng = initial.next_rng().clone();
        let initial_allocators = initial.next_allocators().clone();
        let initial_diagnostics = initial.diagnostics();

        let mut comparison_workspace = AmbientWorkspace::new();
        let comparison = comparison_workspace
            .prepare(
                step_key(),
                &world,
                &rng,
                &source_allocators,
                3.0,
                0.0,
                1.0 / 60.0,
                config,
                3,
            )
            .expect("equivalent zero-time sampler boundary must stage");

        assert_eq!(initial_pellets, comparison.generated());
        assert_eq!(initial_rng, *comparison.next_rng());
        assert_eq!(initial_allocators, *comparison.next_allocators());
        assert_eq!(initial_diagnostics.generated, 3);
        assert_eq!(initial_diagnostics.source_pellets, 0);
    }

    #[test]
    fn retained_eight_rejection_fixture_uses_the_first_exact_probability_tie() {
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.edge_falloff_enabled = false;
        config.warp_frequency = 0.0;
        config.warp_scale_fraction = 0.0;
        config.large_frequency = 0.0;
        config.medium_frequency = 0.0;
        config.small_frequency = 0.0;
        config.dust_strength = 0.0;
        let draws = [
            0.0,
            1.0 / 9.0,
            0.999_999,
            0.125,
            2.0 / 9.0,
            0.999_999,
            0.25,
            3.0 / 9.0,
            0.999_999,
            0.375,
            4.0 / 9.0,
            0.999_999,
            0.5,
            5.0 / 9.0,
            0.999_999,
            0.625,
            6.0 / 9.0,
            0.999_999,
            0.75,
            7.0 / 9.0,
            0.999_999,
            0.875,
            8.0 / 9.0,
            0.999_999,
        ];
        let mut cursor = 0;
        let (position, attempts) = generate_position_from(
            &mut || {
                let value = draws[cursor];
                cursor += 1;
                value
            },
            0.0,
            config,
        )
        .expect("retained fallback fixture should select a complete candidate");
        assert_eq!(cursor, 24);
        assert_eq!(attempts, 8);
        assert!((position.x - 1_166.666_666_666_666_5).abs() <= 1.0e-12);
        assert!(position.y.abs() <= 1.0e-12);
    }

    #[test]
    fn accumulator_fixture_uses_generation_time_for_density_and_delta_only_for_credit() {
        let world = world_with_pellets(1);
        let rng = StatefulRng::new(0x0a_cc as f64).export_state();
        let source_allocators = allocators();
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.target_count = 2;
        config.spawn_per_second = 2.0;
        let mut workspace = AmbientWorkspace::new();
        let prepared = workspace
            .prepare(
                step_key(),
                &world,
                &rng,
                &source_allocators,
                0.75,
                1.0 / 60.0,
                0.125,
                config,
                2,
            )
            .expect("retained distinct-time accumulator fixture should stage");
        let pellet = &prepared.generated()[0];
        assert!((pellet.position.x - 1_628.749_282_901_049).abs() <= 1.0e-9);
        assert!((pellet.position.y - 1_613.708_614_418_640_8).abs() <= 1.0e-9);
        assert_eq!(prepared.next_accumulator(), 0.0);
        assert_eq!(prepared.diagnostics().candidate_attempts, 2);
        assert_eq!(prepared.next_rng().state_hex, "0x052ebe9a");
    }

    #[test]
    fn complete_source_provenance_rejects_stale_join_inputs() {
        let key = step_key();
        let world = world_with_pellets(0);
        let rng = StatefulRng::new(42.0).export_state();
        let source_allocators = allocators();
        let source_world = world.clone();
        let source_rng = rng.clone();
        let source_allocator_copy = source_allocators.clone();
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.target_count = 1;
        config.spawn_per_second = 0.0;
        let mut workspace = AmbientWorkspace::new();
        let prepared = workspace
            .prepare(
                key,
                &world,
                &rng,
                &source_allocators,
                1.0,
                1.0,
                0.25,
                config,
                1,
            )
            .expect("current provenance should stage");
        prepared
            .validate_current(
                key,
                &world,
                &rng,
                &source_allocators,
                1.0,
                1.0,
                0.25,
                config,
                1,
            )
            .expect("unchanged complete source should validate");

        for (stale, field) in [
            (
                PhysicsStepKey::new(2, 2, 3, 4, 5, [6; 32], 7),
                PhysicsStepKeyField::WorldEpoch,
            ),
            (
                PhysicsStepKey::new(1, 2, 3, 4, 6, [6; 32], 7),
                PhysicsStepKeyField::ConfigRevision,
            ),
            (
                PhysicsStepKey::new(1, 2, 3, 4, 5, [8; 32], 7),
                PhysicsStepKeyField::ConfigHash,
            ),
            (
                PhysicsStepKey::new(1, 2, 3, 4, 5, [6; 32], 8),
                PhysicsStepKeyField::OperationEpoch,
            ),
        ] {
            assert_eq!(
                prepared.validate_current(
                    stale,
                    &world,
                    &rng,
                    &source_allocators,
                    1.0,
                    1.0,
                    0.25,
                    config,
                    1,
                ),
                Err(AmbientError::StepKeyMismatch { field })
            );
        }

        let other_world = world.clone();
        let other_rng = StatefulRng::new(43.0).export_state();
        let mut other_allocators = source_allocators.clone();
        other_allocators.next_entity_id += 1;
        let mut other_config = config;
        other_config.food_value = 2.0;
        for (result, field) in [
            (
                prepared.validate_current(
                    key,
                    &other_world,
                    &rng,
                    &source_allocators,
                    1.0,
                    1.0,
                    0.25,
                    config,
                    1,
                ),
                "world",
            ),
            (
                prepared.validate_current(
                    key,
                    &world,
                    &other_rng,
                    &source_allocators,
                    1.0,
                    1.0,
                    0.25,
                    config,
                    1,
                ),
                "world RNG",
            ),
            (
                prepared.validate_current(
                    key,
                    &world,
                    &rng,
                    &other_allocators,
                    1.0,
                    1.0,
                    0.25,
                    config,
                    1,
                ),
                "allocators",
            ),
            (
                prepared.validate_current(
                    key,
                    &world,
                    &rng,
                    &source_allocators,
                    2.0,
                    1.0,
                    0.25,
                    config,
                    1,
                ),
                "spawn accumulator",
            ),
            (
                prepared.validate_current(
                    key,
                    &world,
                    &rng,
                    &source_allocators,
                    1.0,
                    2.0,
                    0.25,
                    config,
                    1,
                ),
                "generation time",
            ),
            (
                prepared.validate_current(
                    key,
                    &world,
                    &rng,
                    &source_allocators,
                    1.0,
                    1.0,
                    0.5,
                    config,
                    1,
                ),
                "fixed delta",
            ),
            (
                prepared.validate_current(
                    key,
                    &world,
                    &rng,
                    &source_allocators,
                    1.0,
                    1.0,
                    0.25,
                    other_config,
                    1,
                ),
                "config",
            ),
            (
                prepared.validate_current(
                    key,
                    &world,
                    &rng,
                    &source_allocators,
                    1.0,
                    1.0,
                    0.25,
                    config,
                    2,
                ),
                "pellet capacity",
            ),
        ] {
            assert_eq!(result, Err(AmbientError::SourceChanged { field }));
        }
        assert_eq!(world, source_world);
        assert_eq!(rng, source_rng);
        assert_eq!(source_allocators, source_allocator_copy);
    }

    #[test]
    fn capacity_and_rng_failures_leave_no_publishable_result() {
        let world = world_with_pellets(2);
        let rng = StatefulRng::new(789.0).export_state();
        let source_allocators = allocators();
        let mut workspace = AmbientWorkspace::new();
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.target_count = 3;
        config.spawn_per_second = 60.0;

        assert!(matches!(
            workspace.prepare(
                step_key(),
                &world,
                &rng,
                &source_allocators,
                0.0,
                1.0,
                1.0,
                config,
                2,
            ),
            Err(AmbientError::InvalidConfig {
                field: "ambient ranges"
            })
        ));
        assert!(!workspace.is_ready());
        assert_eq!(world.pellets.len(), 2);
        assert_eq!(source_allocators.next_entity_id, 1_000);

        let mut exhausted = source_allocators.clone();
        exhausted.next_entity_id = EXTERNAL_ENTITY_ID_START;
        let mut exhausted_config = config;
        exhausted_config.target_count = 1;
        exhausted_config.spawn_per_second = 0.0;
        assert!(matches!(
            workspace.prepare(
                step_key(),
                &world_with_pellets(0),
                &rng,
                &exhausted,
                1.0,
                1.0,
                1.0,
                exhausted_config,
                3,
            ),
            Err(AmbientError::Allocator(StateError::IdExhausted {
                kind: "entity",
                requested: 1
            }))
        ));
        assert!(!workspace.is_ready());
        assert_eq!(exhausted.next_entity_id, EXTERNAL_ENTITY_ID_START);

        let mut invalid_rng = rng.clone();
        invalid_rng.state_hex = "0x00000000".to_owned();
        assert!(matches!(
            workspace.prepare(
                step_key(),
                &world,
                &invalid_rng,
                &source_allocators,
                0.0,
                1.0,
                1.0,
                config,
                3,
            ),
            Err(AmbientError::Rng(RngError::ZeroXorshiftState))
        ));
        assert!(!workspace.is_ready());
        assert_eq!(world.pellets.len(), 2);
        assert_eq!(source_allocators.next_entity_id, 1_000);
    }

    #[test]
    fn warm_generation_reuses_retained_pellet_capacity() {
        let world = world_with_pellets(0);
        let rng = StatefulRng::new(1_234.0).export_state();
        let source_allocators = allocators();
        let mut config = AmbientPelletConfig::typescript_defaults();
        config.target_count = 32;
        config.spawn_per_second = 32.0;
        let mut workspace = AmbientWorkspace::new();
        let first = workspace
            .prepare(
                step_key(),
                &world,
                &rng,
                &source_allocators,
                0.0,
                1.0,
                1.0,
                config,
                32,
            )
            .expect("warm ambient preparation")
            .diagnostics();
        let first_rng_capacities = {
            let retained = workspace.next_rng.as_ref().expect("warm RNG result");
            (
                retained.algorithm.capacity(),
                retained.state_hex.capacity(),
                retained.gaussian_algorithm.capacity(),
            )
        };
        for _ in 0..24 {
            let next = workspace
                .prepare(
                    step_key(),
                    &world,
                    &rng,
                    &source_allocators,
                    0.0,
                    1.0,
                    1.0,
                    config,
                    32,
                )
                .expect("repeated ambient preparation")
                .diagnostics();
            assert_eq!(next.generated_capacity, first.generated_capacity);
            assert!(next.generated_capacity >= 32);
            let retained = workspace.next_rng.as_ref().expect("reused RNG result");
            assert_eq!(
                (
                    retained.algorithm.capacity(),
                    retained.state_hex.capacity(),
                    retained.gaussian_algorithm.capacity(),
                ),
                first_rng_capacities
            );
        }
    }

    #[test]
    fn gaussian_spare_toggles_reuse_storage_on_idle_and_generated_paths() {
        let world = world_with_pellets(0);
        let source_allocators = allocators();
        let mut rng = StatefulRng::new(1_234.0);
        let _ = rng.gaussian();
        let with_spare = rng.export_state();
        assert!(with_spare.gaussian_spare_hex.is_some());
        let mut without_spare = with_spare.clone();
        without_spare.gaussian_spare_valid = false;
        without_spare.gaussian_spare_hex = None;

        let mut generated_config = AmbientPelletConfig::typescript_defaults();
        generated_config.target_count = 1;
        generated_config.spawn_per_second = 1.0;
        let mut idle_config = generated_config;
        idle_config.target_count = 0;
        idle_config.spawn_per_second = 0.0;
        let mut workspace = AmbientWorkspace::new();
        let first = workspace
            .prepare(
                step_key(),
                &world,
                &with_spare,
                &source_allocators,
                0.0,
                1.0,
                1.0,
                generated_config,
                1,
            )
            .expect("warm generated path with a cached Gaussian spare");
        assert_eq!(
            first.next_rng().gaussian_spare_hex,
            with_spare.gaussian_spare_hex
        );
        let warmed_rng_text_capacity = first.diagnostics().rng_text_capacity;
        assert!(warmed_rng_text_capacity > 0);

        for iteration in 0..24 {
            let generated = iteration % 4 < 2;
            let source_rng = if iteration % 2 == 0 {
                &without_spare
            } else {
                &with_spare
            };
            let prepared = workspace
                .prepare(
                    step_key(),
                    &world,
                    source_rng,
                    &source_allocators,
                    0.0,
                    1.0,
                    1.0,
                    if generated {
                        generated_config
                    } else {
                        idle_config
                    },
                    1,
                )
                .expect("alternating ambient RNG state must prepare");
            assert_eq!(
                prepared.next_rng().gaussian_spare_hex,
                source_rng.gaussian_spare_hex
            );
            assert_eq!(prepared.generated().len(), usize::from(generated));
            assert_eq!(
                prepared.diagnostics().rng_text_capacity,
                warmed_rng_text_capacity
            );
        }
    }
}
