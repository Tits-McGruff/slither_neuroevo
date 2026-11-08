//! Offline tactical fixtures, joined by production prefix/control/physics primitives.
//! Fixture construction and result copies are outside production and are not cost gates.

use super::baseline::{BaselineLifecycleConfig, BaselineLifecycleState};
use super::baseline_v2::{BaselineV2Mode, BaselineV2Settings, BaselineV2State};
use super::control::NeuralControlPipeline;
use super::control_phase::{
    ControlCommitWorkspace, ControlPhaseInputs, ControlPhaseWorkspace, V4BoundaryContext,
};
use super::evolved_lives::{EvolvedDeathCause, EvolvedExperimentConfig, EvolvedGenerationState};
use super::fixed_step::{FixedStepPrefixInputs, FixedStepPrefixWorkspace};
use super::graph::{
    compile_graph, GraphEdge, GraphNodeKind, GraphNodeSpec, GraphOutputRef, GraphSpec,
};
use super::inference::GraphExecutionPlan;
use super::movement::{radius_for_length, MovementConfig};
use super::physics::PhysicsStepKey;
use super::ray_work::RayWorkLimits;
use super::rng::{derive_baseline_v2_behaviors, labelled_stream, StatefulRng};
use super::sensor_contract::SensorProfile;
use super::sensors::{SensorConfig, SensorEvaluator, SensorGenerationState};
use super::state::*;
use super::world_step::{WorldStepConfig, WorldStepWorkspace};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::f64::consts::{FRAC_PI_2, PI};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const DT: f64 = 1.0 / 60.0;
const MAX_SNAKES: usize = 16;
#[cfg(test)]
const BINS: usize = 16;

/// Bounded matrix choices; cadence is explicit and never changes frozen v1.
#[derive(Clone, Copy)]
pub struct TacticalOptions {
    pub seed_count: u32,
    pub steps: u32,
    pub workers: usize,
    pub cadence_seconds: f64,
    /// Explicit admitted angular width; ordinary production defaults are unaffected.
    pub bins: usize,
}

/// Exact first action on identical initial physical geometry for policy comparison.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirstAction {
    pub turn: f32,
    pub boost: bool,
    pub urgent_sectors: Option<u32>,
    pub unsafe_route: Option<bool>,
    pub escape_or_pursuit: Option<bool>,
    pub turn_absolute_difference_from_v1: f64,
    pub boost_agrees_with_v1: bool,
}

/// Last actual delivered decisions of a failed v2 trial; encoded clearances copy actual sampler values.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TacticalDecision {
    pub step: u32,
    pub position: [f64; 2],
    pub heading: f64,
    pub turn: f32,
    pub boost: bool,
    pub committed_heading: Option<f64>,
    pub commitment_remaining_seconds: f64,
    pub forward_body_encoded: f32,
    pub forward_ray_encoded: f32,
    pub forward_wall_encoded: f32,
    pub forward_head_encoded: f32,
    pub urgent_sectors: u32,
    pub unsafe_route: bool,
}

/// One complete physical trajectory; deaths end the trial before any replacement.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TacticalTrial {
    pub scenario: &'static str,
    pub seed: u32,
    pub policy: &'static str,
    pub steps: u32,
    pub alive: bool,
    pub death_cause: Option<&'static str>,
    pub death_contact_flags: u8,
    pub alive_seconds: f64,
    pub food_earned: f64,
    pub kills: u64,
    pub smaller_target_intercepted: Option<bool>,
    pub boost_points_spent: f64,
    pub observations: u64,
    pub incomplete_observations: u64,
    pub policy_world_reads: u64,
    pub behavior_draws: u64,
    pub shared_batch_budget_held: bool,
    pub first_action: FirstAction,
    pub trajectory_witness_sha256: String,
    /// At most 32 delivered v2 decisions; empty for surviving or frozen-v1 trials.
    pub decision_tail: Vec<TacticalDecision>,
}

/// No universal v2 superiority or real-time performance verdict is implied.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TacticalReport {
    pub version: u32,
    pub source_sha256: String,
    pub target: String,
    pub build_class: String,
    pub rustc: String,
    pub math_backend: &'static str,
    /// Frozen v3, Basic and Motion, in that order.
    pub sensor_sha256: [String; 3],
    pub workers: usize,
    pub seed_count: u32,
    pub maximum_steps: u32,
    pub v2_cadence_seconds: f64,
    pub bins: usize,
    pub ordinary_ray_limits: [u32; 3],
    pub saturation_ray_limits: [u32; 3],
    pub caveat: &'static str,
    pub trials: Vec<TacticalTrial>,
}

#[derive(Clone, Copy)]
enum Scenario {
    Wander,
    Approaching,
    Receding,
    Crossing,
    HeadOn,
    Corridor,
    CrowdOpening,
    WallEscape,
    FoodTrap,
    SmallerIntercept,
    LargerThreat,
    BoostEscape,
    PredictedCrossing,
    Saturation,
}
const SCENARIOS: [Scenario; 14] = [
    Scenario::Wander,
    Scenario::Approaching,
    Scenario::Receding,
    Scenario::Crossing,
    Scenario::HeadOn,
    Scenario::Corridor,
    Scenario::CrowdOpening,
    Scenario::WallEscape,
    Scenario::FoodTrap,
    Scenario::SmallerIntercept,
    Scenario::LargerThreat,
    Scenario::BoostEscape,
    Scenario::PredictedCrossing,
    Scenario::Saturation,
];
impl Scenario {
    fn label(self) -> &'static str {
        match self {
            Self::Wander => "open-wander",
            Self::Approaching => "approaching",
            Self::Receding => "receding",
            Self::Crossing => "crossing",
            Self::HeadOn => "head-on",
            Self::Corridor => "corridor",
            Self::CrowdOpening => "crowd-opening",
            Self::WallEscape => "wall-escape",
            Self::FoodTrap => "food-body-trap",
            Self::SmallerIntercept => "smaller-intercept",
            Self::LargerThreat => "larger-threat",
            Self::BoostEscape => "boost-escape",
            Self::PredictedCrossing => "predicted-crossing",
            Self::Saturation => "saturation",
        }
    }
    fn limits(self) -> Result<RayWorkLimits> {
        Ok(if matches!(self, Self::Saturation) {
            RayWorkLimits::new(1, 1, 256, MAX_SNAKES)?
        } else {
            RayWorkLimits::new(1, 256, 4096, MAX_SNAKES)?
        })
    }
}
#[derive(Clone, Copy)]
enum Policy {
    V1,
    BasicDet,
    BasicStoch,
    MotionDet,
    MotionStoch,
}
const POLICIES: [Policy; 5] = [
    Policy::V1,
    Policy::BasicDet,
    Policy::BasicStoch,
    Policy::MotionDet,
    Policy::MotionStoch,
];
impl Policy {
    fn label(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::BasicDet => "basic-v2-deterministic",
            Self::BasicStoch => "basic-v2-stochastic",
            Self::MotionDet => "motion-v2-deterministic",
            Self::MotionStoch => "motion-v2-stochastic",
        }
    }
    fn profile(self) -> SensorProfile {
        match self {
            Self::V1 => SensorProfile::V3,
            Self::BasicDet | Self::BasicStoch => SensorProfile::Basic,
            _ => SensorProfile::Motion,
        }
    }
    fn mode(self) -> Option<BaselineV2Mode> {
        match self {
            Self::V1 => None,
            Self::BasicDet | Self::MotionDet => Some(BaselineV2Mode::Deterministic),
            _ => Some(BaselineV2Mode::Stochastic),
        }
    }
}
#[derive(Debug)]
struct Fixture {
    world: WorldState,
    rng: RngStateBundle,
    allocators: AllocatorState,
    lifecycle: BaselineLifecycleState,
    brains: Vec<BrainRuntimeState>,
}
fn point(x: f64, y: f64) -> WorldPoint {
    WorldPoint { x, y }
}

/// A production Dense graph with zero turn and optional constant boost for opponents.
fn plan(width: usize) -> Result<GraphExecutionPlan> {
    let graph = compile_graph(
        &GraphSpec {
            nodes: vec![
                GraphNodeSpec {
                    id: "input".into(),
                    kind: GraphNodeKind::Input { output_size: width },
                },
                GraphNodeSpec {
                    id: "head".into(),
                    kind: GraphNodeKind::Dense {
                        input_size: width,
                        output_size: 2,
                    },
                },
            ],
            edges: vec![GraphEdge {
                from: "input".into(),
                to: "head".into(),
                from_port: None,
                to_port: None,
            }],
            outputs: vec![GraphOutputRef {
                node_id: "head".into(),
                port: None,
            }],
            output_size: 2,
        },
        &super::fresh_run::stage6a_p0_graph_limits(),
    )?;
    Ok(GraphExecutionPlan::build(&graph)?)
}
fn add_snake(
    fixture: &mut Fixture,
    position: WorldPoint,
    heading: f64,
    length: usize,
    plan: &GraphExecutionPlan,
    boost: bool,
) {
    let heading = heading.sin().atan2(heading.cos());
    let id = fixture.world.snakes.len() as u64 + 1;
    let subject = id == 1;
    let movement = MovementConfig::typescript_defaults();
    let start = fixture.world.body_points.len();
    for offset in 0..length {
        fixture.world.body_points.push(point(
            position.x - heading.cos() * offset as f64 * movement.snake_spacing,
            position.y - heading.sin() * offset as f64 * movement.snake_spacing,
        ));
    }
    let brain = (!subject).then_some(BrainHandle { id, epoch: 1 });
    if let Some(handle) = brain {
        let mut weights = vec![0.0; plan.total_parameters()];
        if boost {
            *weights.last_mut().unwrap() = 1.0;
        }
        fixture.brains.push(BrainRuntimeState {
            handle,
            owner: BrainOwner::Entity(id),
            retained_origin: None,
            non_population_weights: Some(weights.into_boxed_slice()),
            recurrent: vec![0.0; plan.total_state_size()].into_boxed_slice(),
        });
    }
    fixture.world.snakes.push(SnakeState {
        id,
        frame_v1_id: id as u32,
        kind: if subject {
            SnakeKind::Baseline
        } else {
            SnakeKind::Resurrected
        },
        alive: true,
        population_slot: None,
        brain,
        baseline_slot: subject.then_some(0),
        baseline_strategy: subject.then_some(BaselineStrategyState::Roam),
        position,
        previous_position: position,
        direction: heading,
        radius: radius_for_length(length, movement),
        speed: if boost { 500.0 } else { 165.0 },
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 100.0,
        kills: 0,
        target_length: length as f64,
        fitness: 0.0,
        turn: 0.0,
        previous_turn: 0.0,
        input_boost: false,
        previous_input_boost: false,
        control_accumulator_seconds: 0.06,
        delivered_observation_points: 100.0,
        observation_delivered: false,
        completed_step_heading_delta: 0.0,
        completed_step_speed_delta: 0.0,
        completed_step_applied_turn: 0.0,
        body: BodyRange { start, len: length },
        skin: 0,
    });
}
fn fixture(
    scenario: Scenario,
    seed: u32,
    policy: Policy,
    plan: &GraphExecutionPlan,
) -> Result<Fixture> {
    let stream = |label| labelled_stream(f64::from(seed), label).export_state();
    let mut fixture = Fixture {
        world: WorldState::default(),
        rng: RngStateBundle {
            version: if policy.mode().is_some() {
                RNG_BUNDLE_WITH_BASELINE_V2_VERSION
            } else {
                RNG_BUNDLE_VERSION
            },
            world: stream("world"),
            evolution: stream("evolution"),
            external_controller: stream("external"),
            baselines: vec![BaselineRngState {
                slot: 0,
                state: stream("baseline"),
            }],
            evolved_respawns: vec![],
            baseline_behaviors: if policy.mode().is_some() {
                derive_baseline_v2_behaviors(seed, 1, 1, 1, false)?
            } else {
                vec![]
            },
        },
        allocators: AllocatorState {
            version: ALLOCATOR_VERSION,
            next_entity_id: 10000,
            next_brain_id: 10000,
            next_genome_id: 10000,
            next_controller_lease_id: 10000,
            next_frame_v1_id: 10000,
            next_external_id: 1_000_000_010_000,
            next_baseline_id: 2_000_000_010_000,
            next_resurrected_id: 3_000_000_010_000,
        },
        lifecycle: BaselineLifecycleState::generation_boundary(),
        brains: vec![],
    };
    let mut jitter = StatefulRng::new(f64::from(seed));
    let side = if seed.is_multiple_of(2) { 1.0 } else { -1.0 };
    let dy = (jitter.next_f64() - 0.5) * 8.0;
    let origin = if matches!(scenario, Scenario::WallEscape) {
        point(3410.0, dy)
    } else {
        point(0.0, dy)
    };
    let subject_length = if matches!(
        scenario,
        Scenario::Wander
            | Scenario::Crossing
            | Scenario::HeadOn
            | Scenario::PredictedCrossing
            | Scenario::Saturation
    ) {
        5
    } else {
        50
    };
    add_snake(&mut fixture, origin, 0.0, subject_length, plan, false);
    let mut opponent = |x, y, h, length, boost| {
        add_snake(
            &mut fixture,
            point(
                x + (jitter.next_f64() - 0.5) * 12.0,
                (y + (jitter.next_f64() - 0.5) * 6.0) * side + dy,
            ),
            h * side,
            length,
            plan,
            boost,
        )
    };
    match scenario {
        Scenario::Wander | Scenario::WallEscape => {}
        Scenario::Approaching => opponent(110.0, 0.0, PI, 5, false),
        Scenario::Receding => opponent(110.0, 0.0, 0.0, 5, false),
        Scenario::Crossing => opponent(50.0, 50.0, -FRAC_PI_2, 5, false),
        Scenario::HeadOn => opponent(45.0, 0.0, PI, 5, false),
        Scenario::PredictedCrossing => opponent(95.0, 80.0, -FRAC_PI_2, 5, false),
        Scenario::Corridor => {
            opponent(130.0, 45.0, 0.0, 50, false);
            opponent(130.0, -45.0, 0.0, 50, false);
        }
        Scenario::CrowdOpening => {
            opponent(100.0, 35.0, FRAC_PI_2, 8, false);
            opponent(40.0, -95.0, 0.0, 8, false);
            opponent(-90.0, 65.0, PI, 8, false);
        }
        Scenario::FoodTrap => opponent(90.0, 90.0, FRAC_PI_2, 25, false),
        Scenario::SmallerIntercept => opponent(550.0, 25.0, 0.0, 5, false),
        Scenario::LargerThreat => opponent(550.0, 25.0, PI, 100, false),
        Scenario::BoostEscape => opponent(-145.0, 40.0, -0.18, 100, true),
        Scenario::Saturation => {
            for index in 0..8 {
                let angle = index as f64 * PI / 4.0;
                opponent(
                    240.0 * angle.cos(),
                    240.0 * angle.sin(),
                    angle + FRAC_PI_2,
                    80,
                    false,
                );
            }
        }
    }
    if matches!(
        scenario,
        Scenario::FoodTrap | Scenario::Corridor | Scenario::CrowdOpening
    ) {
        for (index, (x, y)) in [(140.0, 0.0), (55.0, 95.0), (60.0, -100.0)]
            .into_iter()
            .enumerate()
        {
            fixture.world.pellets.push(PelletState {
                id: 1000 + index as u64,
                position: point(x, y * side + dy),
                value: 3.0,
                kind: 0,
                color: 0,
                owner: None,
            });
        }
    }
    fixture.lifecycle = BaselineLifecycleState::initialize_after_complete_spawn(
        BaselineLifecycleConfig {
            slot_count: 1,
            ..BaselineLifecycleConfig::typescript_defaults()
        },
        &fixture.world,
    )?;
    if policy.mode().is_some() {
        fixture.lifecycle.slots[0].v2_policy = Some(BaselineV2State::default());
    }
    Ok(fixture)
}

fn config(policy: Policy, cadence: f64) -> WorldStepConfig {
    let mut c = WorldStepConfig::typescript_defaults();
    c.prefix.maximum_snakes = MAX_SNAKES;
    c.prefix.maximum_body_points = 4096;
    c.prefix.maximum_pellets = 4096;
    c.prefix.ambient.target_count = 1;
    c.prefix.ambient.spawn_per_second = 0.0;
    c.prefix.baseline.slot_count = 1;
    c.prefix.baseline.respawn_delay_seconds = 60.0;
    c.control.maximum_snakes = MAX_SNAKES;
    c.control.maximum_brains = MAX_SNAKES;
    c.control.maximum_external_observations = 0;
    c.control.baseline.maximum_world_snakes = MAX_SNAKES;
    c.control.baseline_v2 = policy.mode().map(|mode| BaselineV2Settings {
        mode,
        control_interval_seconds: cadence,
    });
    c.control.sensor_index.maximum_body_entries = 16384;
    c.control.sensor_index.maximum_pellet_entries = 4096;
    c.physics.maximum_body_points = 4096;
    c.physics.maximum_pellets = 4096;
    c.physics.maximum_pellet_index_entries = 4096;
    c.baseline = c.prefix.baseline;
    c.external_replacement.maximum_snakes = MAX_SNAKES;
    c.external_replacement.maximum_body_points = 4096;
    c.external_replacement.maximum_brains = MAX_SNAKES;
    c
}
fn death_label(cause: EvolvedDeathCause) -> &'static str {
    match cause {
        EvolvedDeathCause::Wall => "wall",
        EvolvedDeathCause::HeadToHead => "head-to-head",
        EvolvedDeathCause::Body => "body",
        EvolvedDeathCause::GodMode => "god-mode",
    }
}

fn trial(
    scenario: Scenario,
    seed: u32,
    policy: Policy,
    options: TacticalOptions,
) -> Result<TacticalTrial> {
    trial_with_subject(scenario, seed, policy, options, None, None)
}

fn trial_with_subject(
    scenario: Scenario,
    seed: u32,
    policy: Policy,
    options: TacticalOptions,
    mut corpus: Option<&mut super::tactical_corpus::TacticalCorpusWriter>,
    student: Option<&TacticalStudent>,
) -> Result<TacticalTrial> {
    let mut pending_observation = [0.0f32; 24 + 15 * super::sensor_layout::MAX_SENSOR_BINS];
    let sensor_config = SensorConfig {
        bins: options.bins,
        ..SensorConfig::default()
    };
    let sensor = if policy.profile() == SensorProfile::V3 {
        SensorEvaluator::new(sensor_config)?
    } else {
        SensorEvaluator::new_v4(
            sensor_config,
            MovementConfig::typescript_defaults(),
            DT,
            policy.profile(),
        )?
    };
    let input_width = sensor.layout().input_size;
    let plan = if let Some(student) = student {
        GraphExecutionPlan::build(&student.artifact.graph)?
    } else {
        plan(sensor.layout().input_size)?
    };
    let mut state = fixture(scenario, seed, policy, &plan)?;
    if let Some(student) = student {
        student.install(&mut state, scenario)?;
    }
    let pipeline = NeuralControlPipeline::try_new(
        MAX_SNAKES,
        sensor,
        plan,
        options.workers,
        512 * 1024 * 1024,
    )?;
    let mut control = ControlPhaseWorkspace::new(pipeline)?;
    if policy.mode().is_some() && student.is_none() {
        control.configure_teacher_capture(&[0])?;
    }
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let mut commits = ControlCommitWorkspace::new();
    let mut physics = WorldStepWorkspace::new();
    physics.track_qualification_subject(1);
    let mut config = config(policy, options.cadence_seconds);
    if student.is_some() {
        config.prefix.baseline.slot_count = 0;
        config.baseline = config.prefix.baseline;
        config.control.baseline_v2 = None;
        config.control.neural_control_interval_seconds = options.cadence_seconds;
    }
    let lives = EvolvedGenerationState::default();
    let mut generation = SensorGenerationState::new();
    // The generation-best boundary is initialized before the first delivered sample.
    generation.update_after_step(&state.world)?;
    let limits = scenario.limits()?;
    let mut witness = Sha256::new();
    witness.update(b"slither/native-tactical-trajectory/v1\0");
    let mut report = TacticalTrial {
        scenario: scenario.label(),
        seed,
        policy: if student.is_some() {
            "canonical-student"
        } else {
            policy.label()
        },
        steps: 0,
        alive: true,
        death_cause: None,
        death_contact_flags: 0,
        alive_seconds: 0.0,
        food_earned: 0.0,
        kills: 0,
        smaller_target_intercepted: None,
        boost_points_spent: 0.0,
        observations: 0,
        incomplete_observations: 0,
        policy_world_reads: 0,
        behavior_draws: 0,
        shared_batch_budget_held: true,
        first_action: FirstAction {
            turn: 0.0,
            boost: false,
            urgent_sectors: None,
            unsafe_route: None,
            escape_or_pursuit: None,
            turn_absolute_difference_from_v1: 0.0,
            boost_agrees_with_v1: true,
        },
        trajectory_witness_sha256: String::new(),
        decision_tail: Vec::with_capacity(32),
    };
    for step in 0..options.steps {
        let key = PhysicsStepKey::new(1, 1, u64::from(step), 1, 1, [1; 32], u64::from(step) + 1);
        let prefix = prefix_workspace.prepare(FixedStepPrefixInputs {
            key,
            world: &state.world,
            rng: &state.rng,
            allocators: &state.allocators,
            generation_elapsed_seconds: f64::from(step) * DT,
            ambient_accumulator: 0.0,
            baseline_lifecycle: &state.lifecycle,
            config: config.prefix,
        })?;
        let phase = control.prepare(ControlPhaseInputs {
            prefix,
            generation: &generation,
            assessment_points: None,
            v4: policy.mode().map(|_| V4BoundaryContext {
                context: super::sensors_v4::V4ObservationContext {
                    generation_elapsed_seconds: prefix.generation_elapsed_seconds(),
                    evolved: &lives,
                    experiment: EvolvedExperimentConfig::legacy(240.0),
                },
                limits,
            }),
            population: &[],
            brains: &state.brains,
            wall_now_ms: u64::from(step) * 17,
            config: config.control,
        })?;
        let student_action = if student.is_some() {
            phase
                .control_updates()
                .iter()
                .find(|action| action.snake_id() == 1)
                .copied()
        } else {
            None
        };
        let mut charged = 0u64;
        for action in phase.baseline_controls() {
            report.observations += 1;
            report.incomplete_observations +=
                u64::from(action.sensor_diagnostics().incomplete_rays != 0);
            report.policy_world_reads += action.diagnostics().world_snakes as u64;
            report.behavior_draws += action.diagnostics().uniform_draws as u64;
            charged += action.sensor_diagnostics().ray_work.charged();
            if step == 0 {
                report.first_action.turn = action.next_slot().turn;
                report.first_action.boost = action.next_slot().boost;
            }
        }
        if step == 0 {
            if let Some(decision) = phase.teacher_samples().decisions.first() {
                report.first_action.urgent_sectors = Some(decision.urgent_sectors);
                report.first_action.unsafe_route = Some(decision.unsafe_route);
                report.first_action.escape_or_pursuit = Some(decision.escape_or_pursuit);
            }
        }
        let delivered = phase.teacher_samples();
        if let Some(decision) = delivered.decisions.first() {
            let row = delivered
                .observation(0)
                .ok_or("missing delivered tactical row")?;
            let subject = prefix
                .world()
                .snakes
                .iter()
                .find(|snake| snake.id == 1)
                .ok_or("missing tactical subject")?;
            let continuation = phase
                .baseline_controls()
                .first()
                .and_then(|control| control.next_slot().v2_policy)
                .ok_or("missing tactical policy continuation")?;
            if report.decision_tail.len() == 32 {
                report.decision_tail.remove(0);
            }
            let front = super::sensors::angle_to_centered_bin(0.0, options.bins);
            report.decision_tail.push(TacticalDecision {
                step: step + 1,
                position: [subject.position.x, subject.position.y],
                heading: subject.direction,
                turn: decision.applied_turn,
                boost: decision.boost_request,
                committed_heading: continuation.committed_heading,
                commitment_remaining_seconds: continuation.commitment_remaining_seconds,
                forward_body_encoded: row[24 + 2 * options.bins + front],
                forward_ray_encoded: row[24 + 3 * options.bins + front],
                forward_wall_encoded: row[24 + 4 * options.bins + front],
                forward_head_encoded: row[24 + 5 * options.bins + front],
                urgent_sectors: decision.urgent_sectors,
                unsafe_route: decision.unsafe_route,
            });
        }
        // The exact delivered row is pending until all physical substeps succeed.
        let pending_decision = if corpus.is_some() {
            let delivered = phase.teacher_samples();
            if let Some(decision) = delivered.decisions.first() {
                let row = delivered
                    .observation(0)
                    .ok_or("missing pending corpus row")?;
                if row.len() != input_width {
                    return Err("invalid corpus row width".into());
                }
                pending_observation[..input_width].copy_from_slice(row);
                Some(*decision)
            } else {
                None
            }
        } else {
            None
        };
        let committed = commits.prepare(phase)?;
        let prepared = physics.prepare(committed, config)?;
        state.world = prepared.world().clone();
        state.rng = prepared.rng().clone();
        state.allocators = prepared.allocators().clone();
        state.lifecycle = prepared.baseline_lifecycle().clone();
        state.brains = prepared.brains().to_vec();
        generation = prepared.sensor_generation();
        let _ = prepared;
        let outcome = physics
            .qualification_outcome()
            .ok_or("unpublished tactical outcome")?;
        if outcome.snake_id != 1 {
            return Err("missing physical-subject outcome".into());
        }
        if let (Some(writer), Some(decision)) = (corpus.as_deref_mut(), pending_decision) {
            if decision.physical_snake_id != outcome.snake_id {
                return Err("corpus subject outcome mismatch".into());
            }
            writer.committed(
                step + 1,
                &pending_observation[..input_width],
                decision.applied_turn,
                decision.boost_request,
            )?;
        }
        if let Some(action) = student_action {
            if step == 0 {
                report.first_action.turn = action.turn();
                report.first_action.boost = action.boost();
            }
            if action.source() == super::control_phase::SelectedControlSource::NeuralEvaluated {
                let batch = control
                    .qualification_batch()
                    .ok_or("missing committed student batch")?;
                let index = batch
                    .work()
                    .iter()
                    .position(|work| work.snake_id() == 1)
                    .ok_or("missing committed student work")?;
                report.observations += 1;
                report.incomplete_observations +=
                    u64::from(batch.diagnostics()[index].incomplete_rays != 0);
            }
        }
        report.steps += 1;
        report.alive = outcome.alive_after;
        report.alive_seconds += DT;
        report.food_earned += outcome.food;
        report.kills += outcome.kills;
        report.boost_points_spent += outcome.boost_points_spent;
        report.death_cause = outcome.death.map(death_label);
        report.death_contact_flags |= outcome.death_flags;
        // Also includes neural opponent work, read only from this exact completed prepare.
        if policy.mode().is_some() {
            let work = control.qualification_sensor_work();
            let total: u64 = work.iter().map(|work| work.ray_charged_units).sum();
            report.shared_batch_budget_held &=
                total <= u64::from(limits.batch()) && charged <= total;
        }
        // Debug value formatting is a within-build witness, not a persistence/replay format.
        witness.update(format!("{state:?}{generation:?}").as_bytes());
        if !report.alive {
            break;
        }
    }
    if matches!(scenario, Scenario::SmallerIntercept) {
        report.smaller_target_intercepted = Some(
            report.kills > 0
                && state
                    .world
                    .snakes
                    .iter()
                    .any(|snake| snake.id == 2 && !snake.alive),
        );
    }
    if report.alive {
        report.decision_tail.clear();
    }
    report.trajectory_witness_sha256 = format!("{:x}", witness.finalize());
    Ok(report)
}

fn validate_tactical_options(options: TacticalOptions) -> Result<()> {
    if !(1..=64).contains(&options.seed_count)
        || !(1..=600).contains(&options.steps)
        || !(8..=32).contains(&options.bins)
        || options.workers == 0
        || options.workers > std::thread::available_parallelism()?.get()
    {
        return Err(
            "tactical bounds: seeds 1..64, steps 1..600, bins 8..32, workers 1..available".into(),
        );
    }
    BaselineV2Settings {
        mode: BaselineV2Mode::Deterministic,
        control_interval_seconds: options.cadence_seconds,
    }
    .validate()?;
    Ok(())
}

/// Execute fixed seeded paired fixtures. This does not select production defaults.
pub fn run_tactical_qualification(options: TacticalOptions) -> Result<TacticalReport> {
    run_tactical_qualification_with_corpus(options, None)
}

/// Optionally export exact committed Motion teacher rows into an exclusive offline directory.
pub fn run_tactical_qualification_with_corpus(
    options: TacticalOptions,
    corpus_directory: Option<&std::path::Path>,
) -> Result<TacticalReport> {
    run_tactical_qualification_with_profiled_corpus(
        options,
        corpus_directory,
        SensorProfile::Motion,
    )
}

/// Export one explicitly selected Basic/Motion teacher profile at the requested width.
/// Separate profile corpora retain their actual labels/trajectories; they do not imply
/// identical cross-profile datasets or select any production sensor default.
pub fn run_tactical_qualification_with_profiled_corpus(
    options: TacticalOptions,
    corpus_directory: Option<&std::path::Path>,
    corpus_profile: SensorProfile,
) -> Result<TacticalReport> {
    validate_tactical_options(options)?;
    if !matches!(corpus_profile, SensorProfile::Basic | SensorProfile::Motion) {
        return Err("tactical corpus requires v4 Basic or Motion".into());
    }
    let mut corpus = corpus_directory
        .map(|root| {
            super::tactical_corpus::TacticalCorpusWriter::new(root, options, corpus_profile)
        })
        .transpose()?;
    let mut trials = Vec::new();
    for scenario in SCENARIOS {
        for seed in 1..=options.seed_count {
            let mut reference = None;
            for policy in POLICIES {
                let mut result = if policy.profile() == corpus_profile {
                    if let Some(writer) = corpus.as_mut() {
                        writer.begin();
                    }
                    let result =
                        trial_with_subject(scenario, seed, policy, options, corpus.as_mut(), None)?;
                    if let Some(writer) = corpus.as_mut() {
                        writer.end(&result)?;
                    }
                    result
                } else {
                    trial(scenario, seed, policy, options)?
                };
                if let Some((turn, boost)) = reference {
                    result.first_action.turn_absolute_difference_from_v1 =
                        (f64::from(result.first_action.turn) - f64::from(turn)).abs();
                    result.first_action.boost_agrees_with_v1 = result.first_action.boost == boost;
                } else {
                    reference = Some((result.first_action.turn, result.first_action.boost));
                }
                trials.push(result);
            }
        }
    }
    if let Some(writer) = corpus {
        writer.finish(options)?;
    }
    Ok(TacticalReport{version:2,source_sha256:crate::native_addon_source_sha256(),target:crate::native_addon_build_target(),build_class:crate::native_addon_build_class(),rustc:crate::native_addon_rustc_version(),math_backend:super::inference::InferenceMathBackend::detect().label(),sensor_sha256:[super::sensor_contract::SensorContractIdentity::new(SensorProfile::V3,options.bins)?.sha256_hex()?,super::sensor_contract::SensorContractIdentity::new(SensorProfile::Basic,options.bins)?.sha256_hex()?,super::sensor_contract::SensorContractIdentity::new(SensorProfile::Motion,options.bins)?.sha256_hex()?],workers:options.workers,seed_count:options.seed_count,maximum_steps:options.steps,v2_cadence_seconds:options.cadence_seconds,bins:options.bins,ordinary_ray_limits:[1,256,4096],saturation_ray_limits:[1,1,256],caveat:"Synthetic paired initial geometry; native prefix, actual delivered control/inference, all three physics substeps, food, collisions, effects and baseline death handling. Neural opponents use zero-turn constant Dense policies, with an explicit boosted pursuer in boost-escape. Trials stop on subject death; no respawn, evolution, server, dataset or student agreement is qualified. First-action differences compare identical initial geometry to frozen v1, whose cadence remains fixed-step; later trajectories may diverge. Debug trajectory witnesses apply only within a matching source/build, not across platforms or revisions. Copies and reporting are unmeasured; this is not a real-time cost or universal policy-superiority gate.",trials})
}

/// One independently decoded portable artifact, with an explicit constant-opponent head.
pub struct TacticalStudent {
    artifact: super::brain_artifact::ValidatedBrainArtifact,
    opponent_boost_bias: usize,
}
impl TacticalStudent {
    /// Ordinary portable admission plus v4 Basic/Motion B8..32 and the bounded parameter cap.
    pub fn from_artifact(bytes: &[u8]) -> Result<Self> {
        Self::from_validated(super::brain_artifact::validate_brain_artifact(bytes)?)
    }
    fn from_validated(artifact: super::brain_artifact::ValidatedBrainArtifact) -> Result<Self> {
        let expected = super::sensor_contract::SensorContractIdentity::new(
            artifact.sensors.profile,
            artifact.sensors.bins,
        )?
        .sha256_hex()?;
        if artifact.sensors.profile == SensorProfile::V3
            || artifact.canonical_manifest.sensor_sha256 != expected
            || artifact.weights.len() > crate::contracts::MAX_BRAIN_PARAMETERS
        {
            return Err(
                "student tactical cohort requires v4 Basic/Motion B8..32 and at most one million parameters"
                    .into(),
            );
        }
        let outputs = &artifact.graph.outputs;
        if outputs.len() != 1 || outputs[0].port.unwrap_or(0) != 0 {
            return Err("student tactical cohort requires one terminal Dense2 output".into());
        }
        let head = artifact
            .graph
            .nodes
            .iter()
            .find(|node| node.id == outputs[0].node_id)
            .ok_or("missing student output node")?;
        if head.node_type != super::graph::CompiledNodeType::Dense || head.output_size != 2 {
            return Err("student tactical cohort requires one terminal Dense2 output".into());
        }
        let opponent_boost_bias = head.parameter_offset + head.parameter_length - 1;
        Ok(Self {
            artifact,
            opponent_boost_bias,
        })
    }
    /// The validated artifact owns the student width when the CLI omits an override.
    pub const fn sensor_bins(&self) -> usize {
        self.artifact.sensors.bins
    }

    fn install(&self, state: &mut Fixture, scenario: Scenario) -> Result<()> {
        // Opponents remain zero-turn, with only the explicit escape pursuer boosting.
        for brain in &mut state.brains {
            let weights = brain
                .non_population_weights
                .as_mut()
                .ok_or("missing opponent weights")?;
            weights.fill(0.0);
            if matches!(scenario, Scenario::BoostEscape) && brain.handle.id == 2 {
                weights[self.opponent_boost_bias] = 1.0;
            }
        }
        let subject = &mut state.world.snakes[0];
        subject.kind = SnakeKind::Resurrected;
        subject.baseline_slot = None;
        subject.baseline_strategy = None;
        let handle = BrainHandle { id: 1, epoch: 1 };
        subject.brain = Some(handle);
        let origin = super::retained_origin::RetainedNeuralOrigin::SavedBrain {
            brain_id: Sha256::digest(self.artifact.artifact_sha256.as_bytes()).into(),
            weight_digest: super::retained_origin::packed_weight_digest(&self.artifact.weights),
            source_kind: self.artifact.provenance.source_kind,
        };
        origin.validate_weights(&self.artifact.weights)?;
        state.brains.push(BrainRuntimeState {
            handle,
            owner: BrainOwner::Entity(1),
            retained_origin: Some(std::sync::Arc::new(origin)),
            non_population_weights: Some(self.artifact.weights.clone().into_boxed_slice()),
            recurrent: vec![0.0; self.artifact.graph.total_state_size].into_boxed_slice(),
        });
        state.brains.sort_by_key(|brain| brain.handle.id);
        state.rng.baselines.clear();
        state.rng.baseline_behaviors.clear();
        state.lifecycle = BaselineLifecycleState::initialize_after_complete_spawn(
            BaselineLifecycleConfig {
                slot_count: 0,
                ..BaselineLifecycleConfig::typescript_defaults()
            },
            &state.world,
        )?;
        Ok(())
    }
}

/// Actual student-owned physical trajectories, separate from teacher agreement/parity.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudentTacticalReport {
    pub version: u32,
    pub kind: &'static str,
    pub source_sha256: String,
    pub target: String,
    pub workers: usize,
    pub seed_count: u32,
    pub maximum_steps: u32,
    pub first_seed: u32,
    pub candidate: super::brain_artifact::BrainArtifactDescriptorV1,
    pub caveat: &'static str,
    pub reference_v1: Vec<TacticalTrial>,
    pub trials: Vec<TacticalTrial>,
}

/// Deploy one complete portable student through normal neural control and all physics substeps.
pub fn run_student_tactical_qualification(
    options: TacticalOptions,
    student: &TacticalStudent,
    first_seed: u32,
) -> Result<StudentTacticalReport> {
    validate_tactical_options(options)?;
    if first_seed == 0 || first_seed > 64 || options.seed_count > 65 - first_seed {
        return Err("student tactical seed range must stay within 1..64".into());
    }
    if options.bins != student.sensor_bins() {
        return Err("student sensor bins differ from tactical request".into());
    }
    if options.cadence_seconds != student.artifact.deployment.control_interval_seconds {
        return Err("student deployment cadence differs from tactical request".into());
    }
    let mut reference_v1 = Vec::new();
    let mut trials = Vec::new();
    for scenario in SCENARIOS {
        for seed in first_seed..first_seed + options.seed_count {
            let reference = trial(scenario, seed, Policy::V1, options)?;
            let mut result = trial_with_subject(
                scenario,
                seed,
                match student.artifact.sensors.profile {
                    SensorProfile::Basic => Policy::BasicDet,
                    SensorProfile::Motion => Policy::MotionDet,
                    SensorProfile::V3 => return Err("unsupported student profile".into()),
                },
                options,
                None,
                Some(student),
            )?;
            result.first_action.turn_absolute_difference_from_v1 =
                (f64::from(result.first_action.turn) - f64::from(reference.first_action.turn))
                    .abs();
            result.first_action.boost_agrees_with_v1 =
                result.first_action.boost == reference.first_action.boost;
            reference_v1.push(reference);
            trials.push(result);
        }
    }
    Ok(StudentTacticalReport {version: 1, kind: "native-student-tactics-v1", source_sha256: crate::native_addon_source_sha256(),
        target: crate::native_addon_build_target(), workers: options.workers, seed_count: options.seed_count,
        maximum_steps: options.steps, first_seed, candidate: student.artifact.descriptor(),
        caveat: "Synthetic paired initial native geometry with student-owned neural control, actual sensors, held cadence, zero initial recurrence and complete physics. Opponents preserve zero-turn constant outputs and the explicit boosted pursuer through a terminal Dense2 head. Trials stop at death or horizon. First actions compare identical initial geometry to frozen v1; later trajectories diverge. No population distribution, throughput, training-quality superiority or production-default qualification is implied.",
        reference_v1, trials})
}

#[cfg(test)]
mod tests;
