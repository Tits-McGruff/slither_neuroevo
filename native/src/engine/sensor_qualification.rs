//! Bounded observation evidence from actual controlled-step native runs.
//! The production loop constructs every sample and advances all physics/RNG.
//! This observer runs after publication, outside the measured step, and never
//! probes, consumes a delivery marker or changes a controlling input.

use super::checkpoint::CheckpointOperationId;
use super::frame_v1::FrameV1ViewDescriptor;
use super::fresh_run::{
    prepare_sensor_qualification_run, FreshRunSettingUpdate, Stage6aP0FreshRunRequest,
};
use super::graph::{GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec};
use super::inference::InferenceMathBackend;
use super::ray_work::RayWorkLimits;
use super::running_loop::{
    RunningAuthorityLoop, RunningAuthorityLoopProgress, RunningFramePublication,
};
use super::scheduler::{FixedStepSchedulerPolicy, SchedulerServiceMode};
use super::sensor_contract::{SensorContractIdentity, SensorProfile};
use super::state::NormalizedSettingValue;
use super::step_config::{production_default_settings, RunningStepWorkLimits};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Maximum retained timings; simulation state and histogram storage stay fixed.
pub const MAX_QUALIFICATION_STEPS: usize = 60_000;
/// Fixed histogram resolution, with explicit under/overflow counters.
const HISTOGRAM_BUCKETS: usize = 32;

/// Every resource/work choice is explicit; these are not production defaults.
#[derive(Clone, Debug)]
pub struct SensorQualificationOptions {
    pub population: usize,
    pub profile: SensorProfile,
    pub bins: usize,
    pub large_graph: bool,
    /// Optional immutable portable source for a qualification-only trained-clone cohort.
    pub trained_artifact: Option<PathBuf>,
    pub respawn: bool,
    pub seed: u32,
    pub workers: usize,
    pub math: InferenceMathBackend,
    pub ray_limits: Option<RayWorkLimits>,
    pub baseline: Option<super::baseline_v2::BaselineV2Settings>,
    /// Qualification-only selection within the existing 0.5..60-second setting.
    pub teacher_respawn_delay_seconds: f64,
    pub capture: Option<super::capture_qualification::CaptureQualificationOptions>,
    pub warmup_steps: usize,
    pub measured_steps: usize,
    /// Declared operating environment; never includes connection credentials.
    pub environment: String,
}

/// Fixed-size committed sensing work for one consumer class, without copied observations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerSensorWork {
    pub observations: u64,
    pub v4_observations: u64,
    pub ray_charged_units: u64,
    pub ray_cells: u64,
    pub ray_candidates: u64,
    pub ray_intersections: u64,
    pub incomplete_observations: u64,
    pub incomplete_directions: u64,
    pub pellet_capped_observations: u64,
    pub body_capped_observations: u64,
    pub baseline_action_world_reads: u64,
    pub behavior_uniform_draws: u64,
}
impl ConsumerSensorWork {
    pub(crate) fn record(&mut self, sample: &super::sensors::SensorSampleDiagnostics, v4: bool) {
        self.observations += 1;
        self.v4_observations += u64::from(v4);
        self.ray_charged_units += sample.ray_work.charged();
        self.ray_cells += u64::from(sample.ray_work.cells);
        self.ray_candidates += u64::from(sample.ray_work.candidates);
        self.ray_intersections += u64::from(sample.ray_work.intersections);
        self.incomplete_observations += u64::from(sample.incomplete_rays != 0);
        self.incomplete_directions += sample.incomplete_rays as u64;
        self.pellet_capped_observations += sample.pellet_cap_hits;
        self.body_capped_observations += sample.segment_cap_hits;
    }
    fn merge(&mut self, other: Self) {
        self.observations += other.observations;
        self.v4_observations += other.v4_observations;
        self.ray_charged_units += other.ray_charged_units;
        self.ray_cells += other.ray_cells;
        self.ray_candidates += other.ray_candidates;
        self.ray_intersections += other.ray_intersections;
        self.incomplete_observations += other.incomplete_observations;
        self.incomplete_directions += other.incomplete_directions;
        self.pellet_capped_observations += other.pellet_capped_observations;
        self.body_capped_observations += other.body_capped_observations;
        self.baseline_action_world_reads += other.baseline_action_world_reads;
        self.behavior_uniform_draws += other.behavior_uniform_draws;
    }
}

/// Constant-storage distribution; exact extrema remain visible outside bucket bounds.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundedHistogram {
    pub label: String,
    pub lower: f64,
    pub upper: f64,
    pub samples: u64,
    pub below: u64,
    pub above: u64,
    pub nonfinite: u64,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub buckets: [u64; HISTOGRAM_BUCKETS],
}

impl BoundedHistogram {
    fn new(label: impl Into<String>, lower: f64, upper: f64) -> Self {
        Self {
            label: label.into(),
            lower,
            upper,
            samples: 0,
            below: 0,
            above: 0,
            nonfinite: 0,
            minimum: None,
            maximum: None,
            buckets: [0; HISTOGRAM_BUCKETS],
        }
    }

    fn observe(&mut self, value: f64) {
        self.samples += 1;
        if !value.is_finite() {
            self.nonfinite += 1;
            return;
        }
        self.minimum = Some(self.minimum.map_or(value, |old| old.min(value)));
        self.maximum = Some(self.maximum.map_or(value, |old| old.max(value)));
        if value < self.lower {
            self.below += 1;
        } else if value > self.upper {
            self.above += 1;
        } else {
            let bucket = (((value - self.lower) / (self.upper - self.lower))
                * HISTOGRAM_BUCKETS as f64) as usize;
            self.buckets[bucket.min(HISTOGRAM_BUCKETS - 1)] += 1;
        }
    }
}

/// Complete-step cost distribution; observer analysis is reported separately.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostDistribution {
    pub minimum_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub maximum_ms: f64,
    pub total_ms: f64,
}

pub(crate) fn distribution(values: &mut [f64]) -> CostDistribution {
    let total_ms = values.iter().sum();
    values.sort_unstable_by(f64::total_cmp);
    let quantile = |numerator: usize, denominator: usize| {
        values[(values.len() * numerator)
            .div_ceil(denominator)
            .saturating_sub(1)]
    };
    CostDistribution {
        minimum_ms: values[0],
        median_ms: quantile(1, 2),
        p95_ms: quantile(95, 100),
        maximum_ms: values[values.len() - 1],
        total_ms,
    }
}

/// Disjoint attribution from existing production timers; no new hot-path timing.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePhaseCostTotals {
    pub samples: u64,
    pub total_micros: u64,
    pub index_micros: u64,
    pub sensing_micros: u64,
    pub inference_micros: u64,
    pub other_control_micros: u64,
    pub world_step_micros: u64,
    pub other_micros: u64,
}
impl NativePhaseCostTotals {
    fn record(&mut self, total: u64, cost: super::running_step::RunningStepCostMicros) {
        // Match the production health partition: nested timers never become
        // additive peers, and rounding/inconsistent costs cannot exceed service.
        let control = cost.control_selection.min(total);
        let index = cost.control_index.min(control);
        let neural = cost.control_neural.min(control - index);
        let sensing = cost.control_sensing.min(neural);
        let inference = cost.control_inference.min(neural - sensing);
        let world = cost.world_step.min(total - control);
        self.samples += 1;
        self.total_micros += total;
        self.index_micros += index;
        self.sensing_micros += sensing;
        self.inference_micros += inference;
        self.other_control_micros += control - index - sensing - inference;
        self.world_step_micros += world;
        self.other_micros += total - control - world;
    }

    fn component_total(self) -> u64 {
        self.index_micros
            + self.sensing_micros
            + self.inference_micros
            + self.other_control_micros
            + self.world_step_micros
            + self.other_micros
    }
}

/// Existing hook-only physics timers aggregated outside measured step computation.
/// Collision detail is nested inside collision, so these rows are not additive peers.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativePhysicsPhaseTotals {
    pub samples: u64,
    pub milliseconds: std::collections::BTreeMap<&'static str, f64>,
}
impl NativePhysicsPhaseTotals {
    fn record(&mut self, timing: super::physics::PhysicsPhaseTimings) {
        self.samples += 1;
        for (name, value) in [
            ("pelletIndex", timing.pellet_index_ms),
            ("movement", timing.movement_ms),
            ("food", timing.food_ms),
            ("collision", timing.collision_ms),
            ("collisionIndex", timing.collision_index_ms),
            ("collisionHeadHead", timing.collision_head_head_ms),
            ("collisionHeadBody", timing.collision_head_body_ms),
            ("effects", timing.effects_ms),
            ("resultApplication", timing.result_application_ms),
            ("accept", timing.accept_ms),
        ] {
            *self.milliseconds.entry(name).or_default() += value;
        }
    }
}

/// Admission can fail before a candidate allocates its full packed population.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorQualificationReport {
    pub version: u32,
    pub native_phase_costs: NativePhaseCostTotals,
    /// Same 16,667-microsecond boundary used by production slow-step health.
    pub slow_native_phase_costs: NativePhaseCostTotals,
    /// Hook timings already recorded inside physics; collision detail is nested.
    pub physics_phase_costs: NativePhysicsPhaseTotals,
    pub slow_physics_phase_costs: NativePhysicsPhaseTotals,
    pub capture: Option<super::capture_qualification::CaptureQualificationReport>,
    pub capture_options: Option<super::capture_qualification::CaptureQualificationOptions>,
    pub environment: String,
    pub build_identifier: String,
    pub source_sha256: String,
    pub target: String,
    pub build_class: String,
    pub rustc: String,
    pub available_parallelism: Option<usize>,
    pub cpu_model: Option<String>,
    pub total_memory_bytes: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
    pub parameter_count: Option<usize>,
    /// Omitted for the frozen random-cohort report schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trained_artifact: Option<super::brain_artifact::BrainArtifactDescriptorV1>,
    pub authoritative_memory_reservation_bytes: Option<usize>,
    pub population: usize,
    pub baseline_count: usize,
    pub baseline_mode: String,
    pub baseline_cadence_seconds: Option<f64>,
    pub teacher_respawn_delay_seconds: f64,
    /// Neural, baseline and external classes; only exact committed samples are included.
    pub consumer_work: [ConsumerSensorWork; 3],
    pub minimum_due_consumers: Option<u64>,
    pub maximum_due_consumers: u64,
    pub all_consumer_ray_incomplete_fraction: Option<f64>,
    pub all_consumer_ray_incomplete_below_one_percent: Option<bool>,
    pub shared_batch_budget_held: Option<bool>,
    pub seed: u32,
    pub workers: usize,
    pub math_backend: String,
    pub graph_size: &'static str,
    pub respawn: bool,
    pub sensor_contract: SensorContractIdentity,
    pub sensor_sha256: String,
    pub ray_minimum: Option<u32>,
    pub ray_cap: Option<u32>,
    pub ray_batch: Option<u32>,
    pub admitted: bool,
    pub rejection: Option<String>,
    pub warmup_steps: usize,
    pub measured_steps: usize,
    pub completed_steps: u64,
    /// Same-build/environment world/RNG/allocator/life/weight/recurrent witness.
    pub continuation_sha256: Option<String>,
    pub observations: u64,
    pub ray_charged_units: u64,
    pub ray_cell_visits: u64,
    pub ray_candidate_visits: u64,
    pub ray_intersection_tests: u64,
    pub ray_incomplete_observations: u64,
    pub ray_incomplete_directions: u64,
    pub body_capped_observations: u64,
    pub pellet_capped_observations: u64,
    pub ray_incomplete_fraction: Option<f64>,
    pub minimum_due_observations: Option<usize>,
    pub maximum_due_observations: usize,
    pub step_cost: Option<CostDistribution>,
    pub observer_cost: Option<CostDistribution>,
    /// Necessary native cost evidence, not full-stack/tactical qualification.
    pub native_step_p95_within_60hz: Option<bool>,
    pub ordinary_ray_incomplete_below_one_percent: Option<bool>,
    pub raw: Vec<BoundedHistogram>,
    pub normalized: Vec<BoundedHistogram>,
    pub caveat: &'static str,
}

fn graph(width: usize, large: bool) -> GraphSpec {
    let affine = if large { 256 } else { 64 };
    let recurrent = if large { 96 } else { 16 };
    GraphSpec {
        nodes: vec![
            GraphNodeSpec {
                id: "input".into(),
                kind: GraphNodeKind::Input { output_size: width },
            },
            GraphNodeSpec {
                id: "mlp".into(),
                kind: GraphNodeKind::Mlp {
                    input_size: width,
                    hidden_sizes: vec![affine; if large { 4 } else { 1 }],
                    output_size: affine,
                },
            },
            GraphNodeSpec {
                id: "gru".into(),
                kind: GraphNodeKind::Gru {
                    input_size: affine,
                    hidden_size: recurrent,
                },
            },
            GraphNodeSpec {
                id: "head".into(),
                kind: GraphNodeKind::Dense {
                    input_size: recurrent,
                    output_size: 2,
                },
            },
        ],
        edges: vec![
            GraphEdge {
                from: "input".into(),
                to: "mlp".into(),
                from_port: None,
                to_port: None,
            },
            GraphEdge {
                from: "mlp".into(),
                to: "gru".into(),
                from_port: None,
                to_port: None,
            },
            GraphEdge {
                from: "gru".into(),
                to: "head".into(),
                from_port: None,
                to_port: None,
            },
        ],
        outputs: vec![GraphOutputRef {
            node_id: "head".into(),
            port: None,
        }],
        output_size: 2,
    }
}

/// Build the same complete native workload for controlled-step and real-clock hosts.
/// This admits only P0/P1; callers supply explicit v4 ray budgets and arithmetic.
pub(crate) fn prepare_sensor_workload(
    options: &SensorQualificationOptions,
    request: Stage6aP0FreshRunRequest,
) -> Result<super::run_start::PendingRunStartTransition, super::fresh_run::FreshRunError> {
    let trained = options
        .trained_artifact
        .as_deref()
        .map(load_trained_artifact)
        .transpose()
        .map_err(super::fresh_run::FreshRunError::Settings)?;
    prepare_sensor_workload_with_artifact(options, request, trained.as_ref())
}

fn prepare_sensor_workload_with_artifact(
    options: &SensorQualificationOptions,
    request: Stage6aP0FreshRunRequest,
    trained: Option<&super::brain_artifact::ValidatedBrainArtifact>,
) -> Result<super::run_start::PendingRunStartTransition, super::fresh_run::FreshRunError> {
    let (updates, spec) = sensor_workload_spec(options, trained)?;
    prepare_sensor_qualification_run(
        request,
        &updates,
        spec,
        options.profile,
        options.bins,
        RunningStepWorkLimits {
            calculation_workers: options.workers,
            ray_work: options.ray_limits,
            ..RunningStepWorkLimits::provisional_defaults()
        },
        options.math,
        options.baseline,
        trained,
    )
}

/// Shared normalized settings/graph for ordinary, trained and capacity-only qualification.
pub(crate) fn sensor_workload_spec(
    options: &SensorQualificationOptions,
    trained: Option<&super::brain_artifact::ValidatedBrainArtifact>,
) -> Result<(Vec<FreshRunSettingUpdate>, GraphSpec), super::fresh_run::FreshRunError> {
    if !matches!(options.population, 55 | 300) {
        return Err(super::fresh_run::FreshRunError::Settings(
            "qualification workload must be P0/P1".into(),
        ));
    }
    let contract = SensorContractIdentity::new(options.profile, options.bins)
        .map_err(|error| super::fresh_run::FreshRunError::Settings(error.to_string()))?;
    if let Some(artifact) = trained {
        if options.large_graph
            || artifact.canonical_manifest.sensor_sha256
                != contract
                    .sha256_hex()
                    .map_err(|error| super::fresh_run::FreshRunError::Settings(error.to_string()))?
        {
            return Err(super::fresh_run::FreshRunError::Settings(
                "trained cohort requires matching sensors and no graph override".into(),
            ));
        }
    }
    let mut settings = production_default_settings(options.population, 10);
    for entry in &mut settings {
        match entry.path.as_str() {
            "generationSeconds" => entry.value = NormalizedSettingValue::Float(604_800.0),
            "observer.earlyEndMode" => entry.value = NormalizedSettingValue::Integer(2),
            "evolvedRespawn.enabled" => entry.value = NormalizedSettingValue::Bool(options.respawn),
            "world.pelletOverflow" => entry.value = NormalizedSettingValue::Integer(1),
            "baselineBots.respawnDelay" => {
                entry.value = NormalizedSettingValue::Float(options.teacher_respawn_delay_seconds)
            }
            "sense.bubbleBins" => {
                entry.value = NormalizedSettingValue::Integer(options.bins as i64)
            }
            _ => {}
        }
    }
    if let Some(artifact) = trained {
        for entry in &mut settings {
            match entry.path.as_str() {
                "brain.controlDt" => {
                    entry.value =
                        NormalizedSettingValue::Float(artifact.deployment.control_interval_seconds)
                }
                "evolvedRespawn.recurrentState" => {
                    entry.value =
                        NormalizedSettingValue::Integer(match artifact.deployment.life_state_rule {
                            super::training_graph::TrainingLifeStateRule::Reset => 0,
                            super::training_graph::TrainingLifeStateRule::Preserve => 1,
                        })
                }
                _ => {}
            }
        }
    }
    let updates: Vec<_> = settings
        .into_iter()
        .map(|entry| FreshRunSettingUpdate {
            path: entry.path,
            value: match entry.value {
                NormalizedSettingValue::Bool(value) => f64::from(value),
                NormalizedSettingValue::Integer(value) => value as f64,
                NormalizedSettingValue::Float(value) => value,
                NormalizedSettingValue::Text(_) => unreachable!("numeric fresh schema"),
            },
        })
        .collect();
    Ok((
        updates,
        trained.map_or_else(
            || graph(contract.input_size(), options.large_graph),
            |artifact| artifact.graph.spec().clone(),
        ),
    ))
}

fn raw_histograms() -> Vec<BoundedHistogram> {
    [
        ("body_length", 0.0, 10_000.0),
        ("spendable_points", 0.0, 2_000.0),
        ("delivered_wallet_delta", -100.0, 100.0),
        ("speed", 0.0, 500.0),
        ("life_age_seconds", 0.0, 240.0),
        ("radius", 0.0, 60.0),
        ("food", 0.0, 1_000.0),
        ("kills", 0.0, 20.0),
        ("previous_applied_turn", -1.0, 1.0),
        ("completed_heading_delta", -0.1, 0.1),
        ("completed_speed_delta", -500.0, 500.0),
        ("systematic_pellet_value_subset", 0.0, 64.0),
        ("ray_charged_units", 0.0, 1024.0),
        ("encoded_wall_clearance_metres", 0.0, 1200.0),
        ("encoded_body_clearance_metres", 0.0, 1200.0),
        ("encoded_head_clearance_metres", 0.0, 1200.0),
    ]
    .into_iter()
    .map(|(label, lower, upper)| BoundedHistogram::new(label, lower, upper))
    .collect()
}

fn observe_committed(
    loop_: &RunningAuthorityLoop,
    report: &mut SensorQualificationReport,
) -> Result<(), String> {
    let (batch, world) = loop_
        .qualification_batch()
        .ok_or("missing matching committed observation batch")?;
    let consumers = loop_
        .qualification_sensor_work()
        .ok_or("missing committed consumer work")?;
    let mut total = ConsumerSensorWork::default();
    for (retained, work) in report.consumer_work.iter_mut().zip(consumers) {
        retained.merge(work);
        total.merge(work);
    }
    report.minimum_due_consumers = Some(
        report
            .minimum_due_consumers
            .map_or(total.observations, |previous| {
                previous.min(total.observations)
            }),
    );
    report.maximum_due_consumers = report.maximum_due_consumers.max(total.observations);
    if let Some(batch) = report.ray_batch {
        report.shared_batch_budget_held = Some(
            report.shared_batch_budget_held.unwrap_or(true)
                && total.ray_charged_units <= u64::from(batch),
        );
    }
    let count = batch.work().len();
    report.minimum_due_observations = Some(
        report
            .minimum_due_observations
            .map_or(count, |old| old.min(count)),
    );
    report.maximum_due_observations = report.maximum_due_observations.max(count);
    let width = report.sensor_contract.input_size();
    for (ordinal, unit) in batch.work().iter().enumerate() {
        let snake = world
            .snakes
            .get(unit.snake_index())
            .filter(|snake| snake.id == unit.snake_id())
            .ok_or("pre-control observation source identity changed")?;
        let row = &batch.observations()[ordinal * width..(ordinal + 1) * width];
        for (hist, value) in report.normalized.iter_mut().zip(row) {
            hist.observe(f64::from(*value));
        }
        let raw = [
            snake.body.len as f64,
            snake.points,
            snake.points - snake.delivered_observation_points,
            snake.speed,
            snake.age_seconds,
            snake.radius,
            snake.food,
            snake.kills as f64,
            f64::from(snake.completed_step_applied_turn),
            snake.completed_step_heading_delta,
            snake.completed_step_speed_delta,
        ];
        for (hist, value) in report.raw.iter_mut().zip(raw) {
            hist.observe(value);
        }
        let work = batch.diagnostics()[ordinal];
        report.raw[12].observe(work.ray_work.charged() as f64);
        let near = f64::from_bits(work.radii_bits[0]);
        let bins = report.sensor_contract.bins();
        let offsets = if report.sensor_contract.profile() == SensorProfile::V3 {
            [19 + 2 * bins, 19 + bins, 19 + 3 * bins]
        } else {
            [24 + 4 * bins, 24 + 2 * bins, 24 + 5 * bins]
        };
        for (hist, offset) in report.raw[13..16].iter_mut().zip(offsets) {
            let encoded = row[offset..offset + bins]
                .iter()
                .copied()
                .fold(1.0f32, f32::min);
            hist.observe((f64::from(encoded) + 1.0) * 0.5 * near);
        }
        report.observations += 1;
        report.ray_charged_units += work.ray_work.charged();
        report.ray_cell_visits += u64::from(work.ray_work.cells);
        report.ray_candidate_visits += u64::from(work.ray_work.candidates);
        report.ray_intersection_tests += u64::from(work.ray_work.intersections);
        report.ray_incomplete_observations += u64::from(work.incomplete_rays != 0);
        report.ray_incomplete_directions += work.incomplete_rays as u64;
        report.body_capped_observations += work.segment_cap_hits;
        report.pellet_capped_observations += work.pellet_cap_hits;
    }
    let count = world.pellets.len().min(64);
    for index in 0..count {
        report.raw[11].observe(world.pellets[index * world.pellets.len() / count].value);
    }
    Ok(())
}

/// Operating-system lifetime high water, including admission/checkpoint construction.
#[cfg(target_os = "windows")]
pub(crate) fn process_peak_rss_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let size = u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size,
        ..Default::default()
    };
    // SAFETY: The process pseudo-handle is not owned. The uniquely borrowed initialized
    // output spans exactly `size` bytes, lives through this call, and is not retained.
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) } == 0 {
        return None;
    }
    u64::try_from(counters.PeakWorkingSetSize).ok()
}
#[cfg(not(target_os = "windows"))]
pub(crate) fn process_peak_rss_bytes() -> Option<u64> {
    super::sensing_fixture::linux_process_status_bytes("VmHWM:")
}

/// Execute a bounded actual native run. Workspace must be an exclusively task-owned,
/// empty directory; output/report retention and cleanup belong to the CLI caller.
pub fn run_sensor_qualification(
    options: &SensorQualificationOptions,
    workspace: &Path,
) -> Result<SensorQualificationReport, String> {
    run_with_observer(options, workspace, true)
}

/// Read one finite portable input once, before any candidate population construction.
fn load_trained_artifact(
    path: &Path,
) -> Result<super::brain_artifact::ValidatedBrainArtifact, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    let maximum = super::brain_artifact::MAX_BRAIN_ARTIFACT_BYTES as u64;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err("trained artifact byte admission".into());
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    super::brain_artifact::validate_brain_artifact(&bytes)
}

fn run_with_observer(
    options: &SensorQualificationOptions,
    workspace: &Path,
    observer_enabled: bool,
) -> Result<SensorQualificationReport, String> {
    if !options.teacher_respawn_delay_seconds.is_finite()
        || !(0.5..=60.0).contains(&options.teacher_respawn_delay_seconds)
    {
        return Err("qualification teacher respawn delay requires 0.5..60 seconds".into());
    }
    if let Some(capture) = options.capture {
        capture.validate()?;
        if options.baseline.is_none() || options.profile == SensorProfile::V3 {
            return Err("capture qualification requires baseline v2 and v4".into());
        }
    }
    if !matches!(options.population, 55 | 300)
        || options.warmup_steps > MAX_QUALIFICATION_STEPS
        || options.measured_steps == 0
        || options.measured_steps > MAX_QUALIFICATION_STEPS
        || options
            .warmup_steps
            .checked_add(options.measured_steps)
            .is_none_or(|sum| sum > MAX_QUALIFICATION_STEPS)
    {
        return Err("qualification requires P0/P1 and 1..60000 total steps".into());
    }
    if std::fs::symlink_metadata(workspace)
        .map_err(|error| error.to_string())?
        .file_type()
        .is_symlink()
        || std::fs::read_dir(workspace)
            .map_err(|error| error.to_string())?
            .next()
            .is_some()
    {
        return Err("qualification workspace must be an empty real directory".into());
    }
    let trained = options
        .trained_artifact
        .as_deref()
        .map(load_trained_artifact)
        .transpose()?;
    let contract = SensorContractIdentity::new(options.profile, options.bins)
        .map_err(|error| error.to_string())?;
    let mut report=SensorQualificationReport {
        version:if trained.is_some() {6}else{5},trained_artifact:trained.as_ref().map(|artifact|artifact.descriptor()),native_phase_costs:NativePhaseCostTotals::default(),slow_native_phase_costs:NativePhaseCostTotals::default(),physics_phase_costs:NativePhysicsPhaseTotals::default(),slow_physics_phase_costs:NativePhysicsPhaseTotals::default(),capture:None,capture_options:options.capture,environment:options.environment.clone(),build_identifier:crate::native_addon_build_identifier(),
        source_sha256:crate::native_addon_source_sha256(),target:crate::native_addon_build_target(),
        build_class:crate::native_addon_build_class(),rustc:crate::native_addon_rustc_version(),
        available_parallelism:std::thread::available_parallelism().ok().map(usize::from),
        cpu_model:super::sensing_fixture::system_cpu_model(),total_memory_bytes:super::sensing_fixture::linux_total_memory_bytes(),
        peak_rss_bytes:None,parameter_count:None,authoritative_memory_reservation_bytes:None,population:options.population,baseline_count:10,
        seed:options.seed,workers:options.workers,math_backend:options.math.label().into(),
        baseline_mode: match options.baseline.map(|settings| settings.mode) {None=>"v1",Some(super::baseline_v2::BaselineV2Mode::Deterministic)=>"v2-deterministic",Some(super::baseline_v2::BaselineV2Mode::Stochastic)=>"v2-stochastic"}.into(),
        baseline_cadence_seconds:options.baseline.map(|settings|settings.control_interval_seconds),
        teacher_respawn_delay_seconds:options.teacher_respawn_delay_seconds,
        consumer_work:[ConsumerSensorWork::default();3],minimum_due_consumers:None,maximum_due_consumers:0,
        all_consumer_ray_incomplete_fraction:None,all_consumer_ray_incomplete_below_one_percent:None,shared_batch_budget_held:None,
        graph_size:if trained.is_some() {"trained-artifact"}else if options.large_graph {"large"}else{"modest"},respawn:options.respawn,
        sensor_sha256:contract.sha256_hex().map_err(|error|error.to_string())?,
        ray_minimum:options.ray_limits.map(RayWorkLimits::minimum),ray_cap:options.ray_limits.map(RayWorkLimits::cap),
        ray_batch:options.ray_limits.map(RayWorkLimits::batch),normalized:contract.order().iter().map(|label|BoundedHistogram::new(label,-1.0,1.0)).collect(),
        sensor_contract:contract,raw:raw_histograms(),admitted:false,rejection:None,warmup_steps:options.warmup_steps,
        measured_steps:options.measured_steps,completed_steps:0,continuation_sha256:None,observations:0,ray_charged_units:0,
        ray_cell_visits:0,ray_candidate_visits:0,ray_intersection_tests:0,
        ray_incomplete_observations:0,ray_incomplete_directions:0,body_capped_observations:0,pellet_capped_observations:0,
        ray_incomplete_fraction:None,minimum_due_observations:None,maximum_due_observations:0,
        step_cost:None,observer_cost:None,native_step_p95_within_60hz:None,ordinary_ray_incomplete_below_one_percent:None,
        caveat:"Actual seeded native random-population gameplay with ten explicitly recorded v1/v2 baselines, duration-only completion, optional unlimited evolved respawn, and discard-new overflow. Includes authoritative complete steps and frame packing; excludes startup/checkpoint publication, Node/browser/network and observer analysis. Run-start uses a real immutable checkpoint plus matching benchmark acknowledgement, without SQLite. Simulated wall times are controlled; this does not measure wall-debt scheduling under load. This is not a trained-policy distribution, external-controller workload qualification, tactical agreement, ablation or final production-default selection. Clearance metres invert encoded Float32 channel minima and are censored at the sensing radius; incomplete body values are conservative, not ground truth. Pellet values use a systematic maximum-64 subset per committed boundary. Decaying due counts remain explicit; no zero-sample quality pass.",
    };
    if trained.is_some() {
        report.caveat = "Actual native trained-clone qualification cohort, initialized cold from one independently validated portable graph/weights artifact before the durable run-start boundary. Random initialization RNG draws are retained; no runtime founder/import command is introduced. This is one cloned learned policy, not a genetically diverse mature population. Evolved-life/economic sensor values are authoritative and can differ from the baseline teacher domain. Artifact cadence/life-state rules are recorded settings. Complete steps, frame packing, sensor distributions and optional committed teacher capture are measured; wall-debt scheduling, representative coverage, integrated contention and production defaults are separate gates.";
    }
    let pending = prepare_sensor_workload_with_artifact(
        options,
        Stage6aP0FreshRunRequest {
            run_id: format!(
                "sensor-qualification-{}-{}",
                options.population, options.seed
            ),
            seed: options.seed,
            memory_ceiling_bytes: 4 * 1024 * 1024 * 1024usize,
        },
        trained.as_ref(),
    );
    let mut pending = match pending {
        Ok(pending) => pending,
        Err(error) => {
            report.rejection = Some(error.to_string());
            report.peak_rss_bytes = process_peak_rss_bytes();
            return Ok(report);
        }
    };
    let metadata: serde_json::Value = serde_json::from_str(
        &pending
            .startup_metadata_json()
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    report.parameter_count = metadata["parameterCount"]
        .as_u64()
        .map(|value| value as usize);
    let descriptor = pending
        .publish_checkpoint(
            workspace,
            CheckpointOperationId::parse("1234567890abcdef1234567890abcdef")
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    pending
        .acknowledge_persistence(&descriptor)
        .map_err(|error| error.to_string())?;
    pending
        .publish_running_authority()
        .map_err(|error| error.to_string())?;
    let mut running = pending
        .into_running_loop(FixedStepSchedulerPolicy::provisional_defaults(), 0)
        .map_err(|error| error.to_string())?;
    report.authoritative_memory_reservation_bytes = Some(running.authoritative_memory_bytes());
    let mut frame = Vec::with_capacity(running.admitted_frame_bytes());
    let mut capture = None;
    let run_id = format!(
        "sensor-qualification-{}-{}",
        options.population, options.seed
    );
    let mut steps = Vec::with_capacity(options.measured_steps);
    let mut observer = Vec::with_capacity(options.measured_steps);
    report.admitted = true;
    for ordinal in 0..options.warmup_steps + options.measured_steps {
        if ordinal == options.warmup_steps {
            capture = options
                .capture
                .map(|settings| {
                    super::capture_qualification::CaptureQualificationObserver::start(
                        &mut running,
                        settings,
                        &run_id,
                        options.measured_steps,
                    )
                })
                .transpose()?;
        }
        let wall_ms = ((ordinal as u64 + 1) * 1000).div_ceil(60);
        let started = Instant::now();
        let progress = running
            .service_after_command_drain(
                wall_ms,
                SchedulerServiceMode::Background,
                Some(RunningFramePublication::new(
                    FrameV1ViewDescriptor::default(),
                    &mut frame,
                )),
            )
            .map_err(|error| error.to_string())?;
        let service_duration = started.elapsed();
        let elapsed = service_duration.as_secs_f64() * 1000.0;
        if !matches!(progress, RunningAuthorityLoopProgress::Published { .. })
            || running.completed_step() != ordinal as u64 + 1
        {
            return Err(format!(
                "qualification did not publish exactly one step: {progress:?}"
            ));
        }
        if ordinal >= options.warmup_steps {
            steps.push(elapsed);
            let analysis = Instant::now();
            let micros = u64::try_from(service_duration.as_micros()).unwrap_or(u64::MAX);
            let phases = running.last_step_cost();
            report.native_phase_costs.record(micros, phases);
            let physics = running.last_physics_phase_timings();
            report.physics_phase_costs.record(physics);
            if micros > 16_667 {
                report.slow_native_phase_costs.record(micros, phases);
                report.slow_physics_phase_costs.record(physics);
            }
            if observer_enabled {
                observe_committed(&running, &mut report)?;
            }
            if let Some(capture) = &mut capture {
                capture.observe(&mut running, ordinal - options.warmup_steps + 1)?;
            }
            observer.push(analysis.elapsed().as_secs_f64() * 1000.0);
        }
    }
    if let Some(capture) = capture {
        report.capture =
            Some(capture.finish(&mut running, options.measured_steps, steps.iter().sum())?);
    }
    report.completed_steps = running.completed_step();
    report.continuation_sha256 = Some(running.qualification_continuation_sha256());
    report.peak_rss_bytes = process_peak_rss_bytes();
    let cost = distribution(&mut steps);
    report.native_step_p95_within_60hz = Some(cost.p95_ms <= 1000.0 / 60.0);
    report.step_cost = Some(cost);
    if report.native_phase_costs.component_total() != report.native_phase_costs.total_micros
        || report.slow_native_phase_costs.component_total()
            != report.slow_native_phase_costs.total_micros
    {
        return Err("native phase costs do not partition measured service".into());
    }
    report.observer_cost = Some(distribution(&mut observer));
    if report.observations != 0 && options.profile != SensorProfile::V3 {
        let fraction = report.ray_incomplete_observations as f64 / report.observations as f64;
        report.ray_incomplete_fraction = Some(fraction);
        report.ordinary_ray_incomplete_below_one_percent = Some(fraction < 0.01);
    }
    let mut total = ConsumerSensorWork::default();
    for work in report.consumer_work {
        total.merge(work);
    }
    if total.v4_observations > 0 {
        let fraction = total.incomplete_observations as f64 / total.v4_observations as f64;
        report.all_consumer_ray_incomplete_fraction = Some(fraction);
        report.all_consumer_ray_incomplete_below_one_percent = Some(fraction < 0.01);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exclusive native-only scratch; never a user database or retained archive.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "slither-sensor-qualification-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if std::fs::symlink_metadata(&self.0)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                && self.0.canonicalize().is_ok_and(|path| path == self.0)
            {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }

    #[test]
    fn committed_observer_does_not_change_world_rng_lives_weights_or_recurrence_across_workers() {
        for mode in [
            None,
            Some(super::super::baseline_v2::BaselineV2Mode::Deterministic),
            Some(super::super::baseline_v2::BaselineV2Mode::Stochastic),
        ] {
            let mut options = SensorQualificationOptions {
                population: 55,
                profile: SensorProfile::Motion,
                bins: 8,
                large_graph: false,
                trained_artifact: None,
                respawn: true,
                seed: 42,
                workers: 1,
                math: InferenceMathBackend::Scalar,
                ray_limits: Some(RayWorkLimits::new(1, 256, 16384, 512).unwrap()),
                baseline: mode.map(|mode| super::super::baseline_v2::BaselineV2Settings {
                    mode,
                    control_interval_seconds: 0.06,
                }),
                teacher_respawn_delay_seconds: 20.0,
                capture: mode.map(|_| {
                    super::super::capture_qualification::CaptureQualificationOptions {
                        drain_every_steps: 1,
                        queue_chunks: 1024,
                    }
                }),
                warmup_steps: 1,
                measured_steps: 12,
                environment: "development".into(),
            };
            let observed = run_with_observer(&options, &Scratch::new().0, true).unwrap();
            let unobserved = run_with_observer(&options, &Scratch::new().0, false).unwrap();
            options.workers = 3;
            let parallel = run_with_observer(&options, &Scratch::new().0, true).unwrap();
            assert!(observed.admitted && unobserved.admitted && parallel.admitted);
            assert_eq!(observed.completed_steps, 13);
            assert!(observed.observations > 0);
            assert_eq!(unobserved.observations, 0);
            assert_eq!(observed.continuation_sha256, unobserved.continuation_sha256);
            assert_eq!(observed.continuation_sha256, parallel.continuation_sha256);
            assert_eq!(observed.observations, parallel.observations);
            assert!(observed
                .normalized
                .iter()
                .all(|histogram| histogram.samples == observed.observations
                    && histogram.nonfinite == 0));
            assert!(observed.raw[11].samples > 0);
            assert!(observed.ray_charged_units <= 12 * 16384);
            assert!(observed.step_cost.is_some() && observed.observer_cost.is_some());
            assert_eq!(observed.native_phase_costs.samples, 12);
            assert_eq!(observed.physics_phase_costs.samples, 12);
            assert_eq!(observed.physics_phase_costs.milliseconds.len(), 10);
            assert!(observed
                .physics_phase_costs
                .milliseconds
                .values()
                .all(|value| value.is_finite() && *value >= 0.0));
            assert_eq!(
                observed.native_phase_costs.component_total(),
                observed.native_phase_costs.total_micros
            );
            assert!(observed.native_phase_costs.total_micros > 0);

            assert_eq!(observed.consumer_work, parallel.consumer_work);
            if mode.is_some() {
                let capture = observed.capture.as_ref().unwrap();
                assert_eq!(capture.exact_samples_match, Some(true));
                assert_eq!(
                    capture.samples_observed.iter().sum::<u64>(),
                    observed.consumer_work[1].observations
                );
            }

            assert_eq!(
                observed.consumer_work[0].observations,
                observed.observations
            );
            assert!(observed.consumer_work[1].observations > 0);
            assert_eq!(observed.consumer_work[2].observations, 0);
            assert_eq!(observed.shared_batch_budget_held, Some(true));
            if let Some(mode) = mode {
                assert_eq!(
                    observed.consumer_work[1].observations,
                    observed.consumer_work[1].v4_observations
                );
                assert!(
                    observed.consumer_work[1].observations < 12 * 10,
                    "v2 cadence holds between deliveries"
                );
                assert_eq!(observed.consumer_work[1].baseline_action_world_reads, 0);
                if mode == super::super::baseline_v2::BaselineV2Mode::Deterministic {
                    assert_eq!(observed.consumer_work[1].behavior_uniform_draws, 0);
                }
                // Stochastic mode may take an entirely unsafe/deterministic route;
                // it must not consume a draw merely to advertise that mode.
            } else {
                assert_eq!(observed.consumer_work[1].v4_observations, 0);
            }
        }
    }

    #[test]
    fn trained_cohort_uses_artifact_graph_and_deployment_with_exact_worker_continuation() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/portable-brain-v1.slither-brain");
        let artifact = load_trained_artifact(&path).unwrap();
        let mut options = SensorQualificationOptions {
            population: 55,
            profile: SensorProfile::Basic,
            bins: 8,
            large_graph: false,
            trained_artifact: Some(path),
            respawn: true,
            seed: 42,
            workers: 1,
            math: InferenceMathBackend::Sse2,
            ray_limits: Some(RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
            baseline: Some(super::super::baseline_v2::BaselineV2Settings {
                mode: super::super::baseline_v2::BaselineV2Mode::Stochastic,
                control_interval_seconds: 0.06,
            }),
            teacher_respawn_delay_seconds: 0.5,
            capture: Some(
                super::super::capture_qualification::CaptureQualificationOptions {
                    drain_every_steps: 1,
                    queue_chunks: 1024,
                },
            ),
            warmup_steps: 0,
            measured_steps: 12,
            environment: "development".into(),
        };
        let single = run_sensor_qualification(&options, &Scratch::new().0).unwrap();
        assert!(single.admitted);
        assert_eq!(single.version, 6);
        assert_eq!(single.graph_size, "trained-artifact");
        assert_eq!(single.parameter_count, Some(artifact.weights.len()));
        assert_eq!(
            single.trained_artifact.as_ref().unwrap().artifact_sha256,
            artifact.artifact_sha256
        );
        assert!(
            single.observations > 55 && single.observations < 55 * 12,
            "artifact cadence must hold neural evaluations"
        );
        options.workers = 3.min(std::thread::available_parallelism().unwrap().get());
        let parallel = run_sensor_qualification(&options, &Scratch::new().0).unwrap();
        assert_eq!(single.continuation_sha256, parallel.continuation_sha256);
        assert_eq!(single.consumer_work, parallel.consumer_work);
        assert_eq!(
            serde_json::to_value(&single.normalized).unwrap(),
            serde_json::to_value(&parallel.normalized).unwrap()
        );
        assert_eq!(
            single.capture.as_ref().unwrap().source_sample_sha256,
            parallel.capture.as_ref().unwrap().source_sample_sha256
        );
        assert_eq!(
            single.capture.as_ref().unwrap().exact_samples_match,
            Some(true)
        );
        options.profile = SensorProfile::Motion;
        let mismatch = run_sensor_qualification(&options, &Scratch::new().0).unwrap();
        assert!(
            !mismatch.admitted
                && mismatch
                    .rejection
                    .as_ref()
                    .unwrap()
                    .contains("matching sensors")
        );
        assert_eq!(mismatch.completed_steps, 0);
        options.profile = SensorProfile::Basic;
        options.large_graph = true;
        assert!(
            !run_sensor_qualification(&options, &Scratch::new().0)
                .unwrap()
                .admitted
        );
    }

    #[test]
    fn actual_capture_bytes_and_authority_match_across_fast_slow_and_parallel_runs() {
        use super::super::capture_qualification::CaptureQualificationOptions;
        let mut options = SensorQualificationOptions {
            population: 55,
            profile: SensorProfile::Motion,
            bins: 16,
            large_graph: false,
            trained_artifact: None,
            respawn: true,
            seed: 42,
            workers: 1,
            math: InferenceMathBackend::Sse2,
            ray_limits: Some(RayWorkLimits::new(1, 256, 65536, 512).unwrap()),
            baseline: Some(super::super::baseline_v2::BaselineV2Settings {
                mode: super::super::baseline_v2::BaselineV2Mode::Stochastic,
                control_interval_seconds: 1.0 / 60.0,
            }),
            teacher_respawn_delay_seconds: 0.5,
            capture: None,
            warmup_steps: 0,
            measured_steps: 180,
            environment: "development".into(),
        };
        // Invalid admission must fail before touching a workspace or allocating a run.
        for delay in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            -1.0,
            0.0,
            0.499,
            60.001,
        ] {
            let mut invalid = options.clone();
            invalid.teacher_respawn_delay_seconds = delay;
            assert!(
                run_sensor_qualification(&invalid, Path::new("unused-invalid-delay"))
                    .unwrap_err()
                    .contains("teacher respawn delay")
            );
        }
        let reference = run_with_observer(&options, &Scratch::new().0, true).unwrap();
        options.capture = Some(CaptureQualificationOptions {
            drain_every_steps: 1,
            queue_chunks: 1024,
        });
        let fast = run_with_observer(&options, &Scratch::new().0, true).unwrap();
        let captured = fast.capture.as_ref().unwrap();
        assert_eq!(captured.exact_samples_match, Some(true));
        assert!(captured.samples_observed.iter().all(|count| *count > 0));
        assert_eq!(
            captured.samples_observed.iter().sum::<u64>(),
            fast.consumer_work[1].observations
        );
        assert_eq!(captured.due_teacher_histogram.iter().sum::<u64>(), 180);
        assert!(captured.chunks_drained > 10 && captured.bytes_drained > 0);
        assert_eq!(captured.dropped_chunks, 0);
        assert_eq!(captured.count_limited_dropped_chunks, 0);
        assert_eq!(captured.byte_limited_dropped_chunks, 0);
        assert!(captured.queue_peak_bytes <= captured.queue_payload_budget_bytes);
        assert!(captured.queue_payload_budget_bytes < captured.queue_budget_bytes);
        assert_eq!(reference.continuation_sha256, fast.continuation_sha256);
        options.workers = 3.min(std::thread::available_parallelism().unwrap().get());
        let parallel = run_with_observer(&options, &Scratch::new().0, true).unwrap();
        assert_eq!(fast.continuation_sha256, parallel.continuation_sha256);
        assert_eq!(
            captured.source_sample_sha256,
            parallel.capture.as_ref().unwrap().source_sample_sha256
        );
        assert_eq!(
            captured.drained_sample_sha256,
            parallel.capture.as_ref().unwrap().drained_sample_sha256
        );
        options.capture = Some(CaptureQualificationOptions {
            drain_every_steps: 180,
            queue_chunks: 1,
        });
        let slow = run_with_observer(&options, &Scratch::new().0, true).unwrap();
        assert_eq!(fast.continuation_sha256, slow.continuation_sha256);
        let dropped = slow.capture.unwrap();
        assert_eq!(captured.source_sample_sha256, dropped.source_sample_sha256);
        assert!(dropped.dropped_chunks > 0 && dropped.dropped_samples > 0);
        assert_eq!(dropped.exact_samples_match, None);
        assert!(dropped.queue_accounting_matches);
        assert_eq!(dropped.queue_peak_chunks, 1);
        assert_eq!(dropped.count_limited_dropped_chunks, dropped.dropped_chunks);
        assert_eq!(dropped.byte_limited_dropped_chunks, 0);
        assert_eq!(dropped.production_watermark, dropped.released_through);
    }

    #[test]
    fn native_phase_attribution_partitions_nested_and_inconsistent_timer_values() {
        let cost = super::super::running_step::RunningStepCostMicros {
            control_selection: 200,
            control_index: 50,
            control_neural: 120,
            control_sensing: 70,
            control_inference: 60,
            world_step: 90,
            ..Default::default()
        };
        let mut totals = NativePhaseCostTotals::default();
        totals.record(500, cost);
        assert_eq!(totals.component_total(), 500);
        assert_eq!(totals.inference_micros, 50);
        assert_eq!(totals.other_control_micros, 30);
        assert_eq!(totals.other_micros, 210);
        totals.record(100, cost);
        assert_eq!(totals.samples, 2);
        assert_eq!(totals.component_total(), 600);
        assert_eq!(totals.total_micros, 600);
        assert_eq!(totals.world_step_micros, 90);
    }

    #[test]
    fn histogram_preserves_extrema_overflow_nonfinite_and_exact_boundaries() {
        let mut histogram = BoundedHistogram::new("test", -1.0, 1.0);
        for value in [-2.0, -1.0, 0.0, 1.0, 2.0, f64::NAN] {
            histogram.observe(value);
        }
        assert_eq!(
            (
                histogram.samples,
                histogram.below,
                histogram.above,
                histogram.nonfinite
            ),
            (6, 1, 1, 1)
        );
        assert_eq!(
            (histogram.minimum, histogram.maximum),
            (Some(-2.0), Some(2.0))
        );
        assert_eq!(histogram.buckets.iter().sum::<u64>(), 3);
        assert_eq!(histogram.buckets[0], 1);
        assert_eq!(histogram.buckets[31], 1);
    }

    #[test]
    fn graph_counts_match_the_named_modest_and_large_profiles() {
        for (profile, bins, large, parameters) in [
            (SensorProfile::V3, 16, false, 13458),
            (SensorProfile::Basic, 16, false, 15826),
            (SensorProfile::Motion, 16, false, 25042),
            (SensorProfile::V3, 32, true, 402914),
            (SensorProfile::Basic, 32, true, 420578),
            (SensorProfile::Motion, 32, true, 494306),
        ] {
            let width = SensorContractIdentity::new(profile, bins)
                .unwrap()
                .input_size();
            let compiled = super::super::graph::compile_graph(
                &graph(width, large),
                &super::super::fresh_run::stage6a_p0_graph_limits(),
            )
            .unwrap();
            assert_eq!(compiled.total_parameters, parameters);
        }
    }
}
