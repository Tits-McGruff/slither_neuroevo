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
const BINS: usize = 16;

/// Bounded matrix choices; cadence is explicit and never changes frozen v1.
#[derive(Clone, Copy)]
pub struct TacticalOptions {
    pub seed_count: u32,
    pub steps: u32,
    pub workers: usize,
    pub cadence_seconds: f64,
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
    let sensor_config = SensorConfig {
        bins: BINS,
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
    let plan = plan(sensor.layout().input_size)?;
    let mut state = fixture(scenario, seed, policy, &plan)?;
    let pipeline = NeuralControlPipeline::try_new(
        MAX_SNAKES,
        sensor,
        plan,
        options.workers,
        512 * 1024 * 1024,
    )?;
    let mut control = ControlPhaseWorkspace::new(pipeline)?;
    if policy.mode().is_some() {
        control.configure_teacher_capture(&[0])?;
    }
    let mut prefix_workspace = FixedStepPrefixWorkspace::new();
    let mut commits = ControlCommitWorkspace::new();
    let mut physics = WorldStepWorkspace::new();
    physics.track_qualification_subject(1);
    let config = config(policy, options.cadence_seconds);
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
        policy: policy.label(),
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
    report.trajectory_witness_sha256 = format!("{:x}", witness.finalize());
    Ok(report)
}

/// Execute fixed seeded paired fixtures. This does not select production defaults.
pub fn run_tactical_qualification(options: TacticalOptions) -> Result<TacticalReport> {
    if !(1..=64).contains(&options.seed_count)
        || !(1..=600).contains(&options.steps)
        || options.workers == 0
        || options.workers > std::thread::available_parallelism()?.get()
    {
        return Err("tactical bounds: seeds 1..64, steps 1..600, workers 1..available".into());
    }
    BaselineV2Settings {
        mode: BaselineV2Mode::Deterministic,
        control_interval_seconds: options.cadence_seconds,
    }
    .validate()?;
    let mut trials = Vec::new();
    for scenario in SCENARIOS {
        for seed in 1..=options.seed_count {
            let mut reference = None;
            for policy in POLICIES {
                let mut result = trial(scenario, seed, policy, options)?;
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
    Ok(TacticalReport{version:1,source_sha256:crate::native_addon_source_sha256(),target:crate::native_addon_build_target(),build_class:crate::native_addon_build_class(),rustc:crate::native_addon_rustc_version(),math_backend:super::inference::InferenceMathBackend::detect().label(),sensor_sha256:[super::sensor_contract::SensorContractIdentity::new(SensorProfile::V3,BINS)?.sha256_hex()?,super::sensor_contract::SensorContractIdentity::new(SensorProfile::Basic,BINS)?.sha256_hex()?,super::sensor_contract::SensorContractIdentity::new(SensorProfile::Motion,BINS)?.sha256_hex()?],workers:options.workers,seed_count:options.seed_count,maximum_steps:options.steps,v2_cadence_seconds:options.cadence_seconds,bins:BINS,ordinary_ray_limits:[1,256,4096],saturation_ray_limits:[1,1,256],caveat:"Synthetic paired initial geometry; native prefix, actual delivered control/inference, all three physics substeps, food, collisions, effects and baseline death handling. Neural opponents use zero-turn constant Dense policies, with an explicit boosted pursuer in boost-escape. Trials stop on subject death; no respawn, evolution, server, dataset or student agreement is qualified. First-action differences compare identical initial geometry to frozen v1, whose cadence remains fixed-step; later trajectories may diverge. Debug trajectory witnesses apply only within a matching source/build, not across platforms or revisions. Copies and reporting are unmeasured; this is not a real-time cost or universal policy-superiority gate.",trials})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> TacticalOptions {
        TacticalOptions {
            seed_count: 1,
            steps: 12,
            workers: 1,
            cadence_seconds: 0.06,
        }
    }
    #[test]
    fn complete_tactical_matrix_is_repeatable_and_parallel_continuations_match() {
        let first = run_tactical_qualification(options()).unwrap();
        assert_eq!(first.trials.len(), 70);
        let mut multi = options();
        multi.workers = 3.min(std::thread::available_parallelism().unwrap().get());
        let parallel = run_tactical_qualification(multi).unwrap();
        assert_eq!(first.trials, parallel.trials);
        for trial in &first.trials {
            assert!(trial.observations > 0 && trial.shared_batch_budget_held);
            assert_eq!(trial.alive, trial.death_cause.is_none());
            if trial.policy != "v1" {
                assert_eq!(trial.policy_world_reads, 0);
            }
            if trial.policy.ends_with("deterministic") {
                assert_eq!(trial.behavior_draws, 0);
            }
        }
        for policy in [Policy::BasicStoch, Policy::MotionStoch] {
            let wander = trial(Scenario::Wander, 1, policy, options()).unwrap();
            assert!(wander.behavior_draws > 0 && wander.alive);
            let saturated = trial(Scenario::Saturation, 1, policy, options()).unwrap();
            assert!(saturated.incomplete_observations > 0);
            assert!(!saturated.first_action.boost);
        }
    }
    #[test]
    fn paired_initial_geometry_is_identical_and_starts_without_foreign_body_contact() {
        let compiled = plan(264).unwrap();
        for scenario in SCENARIOS {
            for seed in 1..=16 {
                let original = fixture(scenario, seed, Policy::V1, &compiled).unwrap();
                let paired = fixture(scenario, seed, Policy::MotionStoch, &compiled).unwrap();
                assert_eq!(original.world, paired.world);
                for head in &original.world.snakes {
                    assert!(head.position.x.hypot(head.position.y) + head.radius < 3500.0);
                    for body in &original.world.snakes {
                        if body.id == head.id {
                            continue;
                        }
                        let points = &original.world.body_points
                            [body.body.start..body.body.start + body.body.len];
                        for segment in points.windows(2) {
                            let dx = segment[1].x - segment[0].x;
                            let dy = segment[1].y - segment[0].y;
                            let t = (((head.position.x - segment[0].x) * dx
                                + (head.position.y - segment[0].y) * dy)
                                / (dx * dx + dy * dy))
                                .clamp(0.0, 1.0);
                            let distance = (head.position.x - segment[0].x - t * dx)
                                .hypot(head.position.y - segment[0].y - t * dy);
                            assert!(
                                distance > 0.82 * (head.radius + body.radius),
                                "{} seed={seed} head={} body={} distance={distance}",
                                scenario.label(),
                                head.id,
                                body.id
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn motion_crossing_escape_changes_the_colliding_velocity_and_survives_mirrored_physics() {
        let extended = TacticalOptions {
            steps: 180,
            cadence_seconds: DT,
            ..options()
        };
        for scenario in [Scenario::Crossing, Scenario::PredictedCrossing] {
            for seed in 1..=4 {
                for policy in [Policy::MotionDet, Policy::MotionStoch] {
                    let result = trial(scenario, seed, policy, extended).unwrap();
                    assert!(result.first_action.turn.abs() >= 0.9, "{result:?}");
                    assert!(result.alive, "{result:?}");
                }
            }
        }
        let escape = trial(Scenario::BoostEscape, 2, Policy::MotionDet, extended).unwrap();
        assert!(escape.first_action.boost && escape.boost_points_spent > 0.0);
    }

    #[test]
    fn actual_wall_and_urgent_crossing_first_actions_keep_qualitative_expectations() {
        let wall = trial(Scenario::WallEscape, 2, Policy::MotionDet, options()).unwrap();
        assert!(wall.first_action.turn.abs() > 0.5 && !wall.first_action.boost);
        let crossing = trial(Scenario::Crossing, 2, Policy::MotionDet, options()).unwrap();
        assert!(
            crossing.first_action.urgent_sectors.unwrap() != 0,
            "{crossing:?}"
        );
        assert_eq!(crossing.first_action.unsafe_route, Some(false));
    }
}
