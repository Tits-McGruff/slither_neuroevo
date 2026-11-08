use super::*;
use crate::engine::rng::StatefulRng;
use crate::engine::state::{
    BaselineRngState, BaselineStrategyState, BodyRange, ControllerKind, ControllerLease,
    ControllerLeaseStatus, LatestControllerAction, SnakeKind, ALLOCATOR_VERSION,
    BASELINE_ENTITY_ID_START, EXTERNAL_ENTITY_ID_START, RESURRECTED_ENTITY_ID_START,
    RNG_BUNDLE_VERSION,
};

type PhaseWorkspaces = PhysicsPipelineWorkspace;

fn key(operation_epoch: u64) -> PhysicsStepKey {
    PhysicsStepKey::new(2, 3, 40, 7, 11, [0x5a; 32], operation_epoch)
}

fn rng_bundle(baseline_count: usize) -> RngStateBundle {
    RngStateBundle {
        baseline_behaviors: Vec::new(),
        evolved_respawns: Vec::new(),
        version: RNG_BUNDLE_VERSION,
        world: StatefulRng::new(11.0).export_state(),
        evolution: StatefulRng::new(22.0).export_state(),
        external_controller: StatefulRng::new(33.0).export_state(),
        baselines: (0..baseline_count)
            .map(|slot| BaselineRngState {
                slot: u32::try_from(slot).expect("fixture slot should fit"),
                state: StatefulRng::new(44.0 + slot as f64).export_state(),
            })
            .collect(),
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

fn snake(
    id: u64,
    kind: SnakeKind,
    position: WorldPoint,
    direction: f64,
    length: usize,
) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: u32::try_from(id).expect("fixture ID should fit"),
        kind,
        alive: true,
        population_slot: (kind == SnakeKind::Evolved).then_some(0),
        brain: None,
        baseline_slot: (kind == SnakeKind::Baseline).then_some(0),
        baseline_strategy: (kind == SnakeKind::Baseline).then_some(BaselineStrategyState::Roam),
        position,
        previous_position: position,
        direction,
        radius: 9.0,
        speed: 165.0,
        boost: false,
        age_seconds: 0.0,
        food: 0.0,
        points: 10.0,
        kills: 0,
        target_length: length as f64,
        fitness: 0.0,
        turn: 0.0,
        previous_turn: 0.0,
        input_boost: false,
        previous_input_boost: false,
        control_accumulator_seconds: 0.0,
        delivered_observation_points: 0.0,
        observation_delivered: false,
        completed_step_heading_delta: 0.0,
        completed_step_speed_delta: 0.0,
        completed_step_applied_turn: 0.0,
        body: BodyRange {
            start: 0,
            len: length,
        },
        skin: 0,
    }
}

fn line_body(position: WorldPoint, direction: f64, length: usize) -> Vec<WorldPoint> {
    (0..length)
        .map(|index| WorldPoint {
            x: position.x - direction.cos() * index as f64 * 7.5,
            y: position.y - direction.sin() * index as f64 * 7.5,
        })
        .collect()
}

fn pack_world(entries: Vec<(SnakeState, Vec<WorldPoint>)>) -> WorldState {
    let mut world = WorldState::default();
    for (mut snake, body) in entries {
        snake.body = BodyRange {
            start: world.body_points.len(),
            len: body.len(),
        };
        snake.position = body[0];
        snake.previous_position = body[0];
        world.body_points.extend(body);
        world.snakes.push(snake);
    }
    world
}

fn body_collision_world(victim_kind: SnakeKind, reverse: bool) -> WorldState {
    let victim_position = WorldPoint { x: 0.0, y: 0.0 };
    let owner_position = WorldPoint { x: 100.0, y: 0.0 };
    let victim = (
        snake(7, victim_kind, victim_position, 0.0, 12),
        line_body(victim_position, 0.0, 12),
    );
    let owner = (
        snake(20, SnakeKind::Evolved, owner_position, 0.0, 15),
        line_body(owner_position, 0.0, 15),
    );
    if reverse {
        pack_world(vec![owner, victim])
    } else {
        pack_world(vec![victim, owner])
    }
}

fn head_head_world() -> WorldState {
    let position = WorldPoint { x: 0.0, y: 0.0 };
    pack_world(vec![
        (
            snake(1, SnakeKind::Evolved, position, 0.0, 8),
            line_body(position, 0.0, 8),
        ),
        (
            snake(2, SnakeKind::Evolved, position, std::f64::consts::PI, 8),
            line_body(position, std::f64::consts::PI, 8),
        ),
    ])
}

fn killed_body_owner_world() -> WorldState {
    let victim_position = WorldPoint { x: 0.0, y: 0.0 };
    let owner_position = WorldPoint { x: 100.0, y: 0.0 };
    pack_world(vec![
        (
            snake(1, SnakeKind::Evolved, victim_position, 0.0, 5),
            line_body(victim_position, 0.0, 5),
        ),
        (
            snake(2, SnakeKind::Evolved, owner_position, 0.0, 15),
            line_body(owner_position, 0.0, 15),
        ),
        (
            snake(
                3,
                SnakeKind::Evolved,
                owner_position,
                std::f64::consts::PI,
                5,
            ),
            line_body(owner_position, std::f64::consts::PI, 5),
        ),
    ])
}

fn lease(snake_id: u64) -> ControllerLease {
    ControllerLease {
        reward: None,
        identity_key: String::new(),
        id: 1,
        snake_id,
        kind: ControllerKind::Player,
        connection_id: Some(1),
        scope: "run".to_owned(),
        resume_token: "token".to_owned(),
        status: ControllerLeaseStatus::Connected,
        latest_action: LatestControllerAction {
            is_explicit: false,
            turn: 0.0,
            boost: false,
            client_tick: 0,
            arrival_sequence: 1,
            accepted_at_ms: 1,
        },
        last_observed_at_ms: 1,
        disconnected_at_ms: None,
        input_hold_expires_at_ms: None,
        grace_expires_at_ms: None,
        takeover_committed_at_ms: None,
    }
}

fn stage_and_accept(
    step: &mut PhysicsStepWorkspace,
    phases: &mut PhaseWorkspaces,
) -> Result<(), PhysicsError> {
    let current_key = step.key.ok_or(PhysicsError::StepNotStarted)?;
    step.advance_substep(phases, current_key)
}

fn snake_by_id(world: &WorldState, id: u64) -> &SnakeState {
    world
        .snakes
        .iter()
        .find(|snake| snake.id == id)
        .expect("fixture snake should exist")
}

fn body_by_id(world: &WorldState, id: u64) -> &[WorldPoint] {
    let snake = snake_by_id(world, id);
    &world.body_points[snake.body.start..snake.body.start + snake.body.len]
}

fn ecology_policy(
    mode: super::super::evolved_lives::EvolvedCorpseMode,
    maximum: u64,
    overflow: super::super::evolved_lives::PelletOverflowMode,
) -> super::super::evolved_lives::EvolvedExperimentConfig {
    let mut policy = super::super::evolved_lives::EvolvedExperimentConfig::legacy(240.0);
    policy.version = 1;
    policy.enabled = true;
    policy.max_lives = maximum;
    policy.corpse_mode = mode;
    policy.pellet_overflow = overflow;
    policy
}

#[test]
fn ecology_modes_apply_only_to_replaceable_deaths_and_wall_deaths_still_emit_none() {
    use super::super::evolved_lives::{
        EvolvedCorpseMode as Mode, EvolvedGenerationState, PelletOverflowMode,
    };
    for maximum in [1, 2] {
        for mode in [Mode::Normal, Mode::None, Mode::SubtractReplacementMass] {
            let mut source = body_collision_world(SnakeKind::Evolved, false);
            for (slot, snake) in source.snakes.iter_mut().enumerate() {
                snake.population_slot = Some(slot as u32);
            }
            let mut lives = EvolvedGenerationState::initialize(&source, 2, 5).unwrap();
            let mut step = PhysicsStepWorkspace::new();
            let mut phases = PhaseWorkspaces::default();
            step.begin(
                key(1),
                &source,
                &rng_bundle(0),
                &allocators(),
                PhysicsConfig::typescript_defaults(),
                1,
            )
            .unwrap();
            step.advance_substep_with_lifecycle(
                &mut phases,
                key(1),
                Some(EffectLifecyclePolicy {
                    experiment: ecology_policy(mode, maximum, PelletOverflowMode::Fail),
                    lives: &lives,
                }),
            )
            .unwrap();
            lives.apply_physics(step.evolved_deltas()).unwrap();
            assert_eq!(lives.slots[0].deaths, 1);
            assert_eq!(lives.slots[1].total_kills, 1);
            let emitted = lives.slots[0].corpse_emitted_count;
            if maximum == 1 || mode == Mode::Normal {
                assert_eq!(emitted, 4);
            } else if mode == Mode::None {
                assert_eq!(emitted, 0);
            } else {
                assert!(emitted < 4);
            }
            assert_eq!(lives.slots[0].corpse_discarded_count, 0);
        }
    }
    let position = WorldPoint { x: 3490.5, y: 0.0 };
    let source = pack_world(vec![(
        snake(7, SnakeKind::Evolved, position, 0.0, 5),
        line_body(position, 0.0, 5),
    )]);
    let lives = EvolvedGenerationState::initialize(&source, 1, 5).unwrap();
    let mut step = PhysicsStepWorkspace::new();
    let mut phases = PhaseWorkspaces::default();
    let rng = rng_bundle(0);
    step.begin(
        key(1),
        &source,
        &rng,
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    step.advance_substep_with_lifecycle(
        &mut phases,
        key(1),
        Some(EffectLifecyclePolicy {
            experiment: ecology_policy(Mode::Normal, 2, PelletOverflowMode::Fail),
            lives: &lives,
        }),
    )
    .unwrap();
    assert!(!step.world().unwrap().snakes[0].alive);
    assert_eq!(phases.effect_diagnostics().attempted_pellets, 0);
    assert_eq!(step.finish(key(1)).unwrap().rng(), &rng);
}

#[test]
fn saturated_corpse_and_boost_effects_keep_existing_pellets_and_full_rng_continuation() {
    use super::super::evolved_lives::{
        EvolvedCorpseMode, EvolvedGenerationState, PelletOverflowMode,
    };
    for boost in [false, true] {
        let mut source = if boost {
            let position = WorldPoint { x: 0.0, y: 0.0 };
            let mut boosted = snake(7, SnakeKind::Evolved, position, 0.0, 100);
            boosted.input_boost = true;
            boosted.points = 100.0;
            boosted.target_length = 5.0;
            pack_world(vec![(boosted, line_body(position, 0.0, 100))])
        } else {
            body_collision_world(SnakeKind::Evolved, false)
        };
        for (slot, snake) in source.snakes.iter_mut().enumerate() {
            snake.population_slot = Some(slot as u32);
        }
        source.pellets.push(PelletState {
            id: 900,
            position: WorldPoint { x: 3000.0, y: 0.0 },
            value: 1.0,
            kind: 0,
            color: 123,
            owner: None,
        });
        let lives = EvolvedGenerationState::initialize(&source, source.snakes.len(), 5).unwrap();
        let rng = rng_bundle(0);
        let mut full = PhysicsStepWorkspace::new();
        let mut full_phases = PhaseWorkspaces::default();
        full.begin(
            key(1),
            &source,
            &rng,
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        full.advance_substep(&mut full_phases, key(1)).unwrap();
        let reference = full.finish(key(1)).unwrap();
        let mut bounded = PhysicsStepWorkspace::new();
        let mut phases = PhaseWorkspaces::default();
        let mut config = PhysicsConfig::typescript_defaults();
        config.maximum_pellets = 1;
        let mut exhausted = allocators();
        exhausted.next_entity_id = EXTERNAL_ENTITY_ID_START;
        bounded
            .begin(key(1), &source, &rng, &exhausted, config, 1)
            .unwrap();
        bounded
            .advance_substep_with_lifecycle(
                &mut phases,
                key(1),
                Some(EffectLifecyclePolicy {
                    experiment: ecology_policy(
                        EvolvedCorpseMode::Normal,
                        2,
                        PelletOverflowMode::DiscardNewExcess,
                    ),
                    lives: &lives,
                }),
            )
            .unwrap();
        let result = bounded.finish(key(1)).unwrap();
        assert_eq!(result.world().pellets, source.pellets);
        assert_eq!(result.rng(), reference.rng());
        assert_eq!(result.allocators(), &exhausted);
        let emissions = phases.effect_diagnostics();
        assert_eq!(
            emissions.attempted_pellets,
            reference.world().pellets.len() - 1
        );
        assert_eq!(emissions.discarded_pellets, emissions.attempted_pellets);
        assert_eq!(emissions.emitted_value, 0.0);
        assert!(emissions.discarded_value > 0.0);
        if boost {
            assert!(emissions.attempted_pellets > config.maximum_pellets);
        } else {
            assert_eq!(bounded.evolved_deltas()[0].corpse_discarded_count, 4);
        }
    }
}

#[test]
fn body_death_and_owner_credit_commit_only_to_the_working_transaction() {
    let source = body_collision_world(SnakeKind::Evolved, false);
    let source_copy = source.clone();
    let rng = rng_bundle(0);
    let allocators = allocators();
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng,
        &allocators,
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .expect("step should begin");
    let mut phases = PhaseWorkspaces::default();
    stage_and_accept(&mut step, &mut phases).expect("substep should join");

    assert_eq!(source, source_copy);
    assert!(snake_by_id(&source, 7).alive);
    assert_eq!(snake_by_id(&source, 20).kills, 0);
    assert_eq!(snake_by_id(step.world().unwrap(), 20).points, 410.0);

    let prepared = step.finish(key(1)).expect("step should finish");
    assert!(!snake_by_id(prepared.world(), 7).alive);
    assert_eq!(snake_by_id(prepared.world(), 20).kills, 1);
    assert_eq!(snake_by_id(prepared.world(), 20).points, 410.0);
    assert_eq!(prepared.world().pellets.len(), 4);
    assert_eq!(prepared.allocators().next_entity_id, 1_004);
    assert_ne!(prepared.rng().world, rng.world);
    assert_eq!(prepared.rng().evolution, rng.evolution);
    assert_eq!(prepared.diagnostics().completed_substeps, 1);
}

#[test]
fn accepted_substep_deltas_match_actual_boost_food_kill_and_death_chronology() {
    let mut source = body_collision_world(SnakeKind::Evolved, false);
    for (slot, snake) in source.snakes.iter_mut().enumerate() {
        snake.population_slot = Some(slot as u32);
    }
    source.snakes[1].input_boost = true;
    source.pellets.push(PelletState {
        id: 900,
        position: source.snakes[1].position,
        value: 1.0,
        kind: 0,
        owner: None,
        color: 0,
    });
    let original = source.clone();
    let mut ledger =
        super::super::evolved_lives::EvolvedGenerationState::initialize(&source, 2, 5).unwrap();
    let mut step = PhysicsStepWorkspace::new();
    let mut config = PhysicsConfig::typescript_defaults();
    config.substep_dt = 1.0 / 180.0;
    step.begin(key(1), &source, &rng_bundle(0), &allocators(), config, 2)
        .unwrap();
    let mut phases = PhaseWorkspaces::default();
    stage_and_accept(&mut step, &mut phases).unwrap();
    let killer = step
        .evolved_deltas()
        .iter()
        .find(|delta| delta.snake_id == 20)
        .unwrap();
    assert!(killer.boost_points_spent > 0.0);
    assert_eq!(killer.food, 1.0);
    assert_eq!(killer.food_count, 1);
    assert_eq!(killer.food_credits, config.food.points_per_food);
    assert_eq!(killer.kill_credits, config.points_per_kill);
    assert_eq!(killer.kills, 1);
    assert_eq!(
        killer.credits,
        config.food.points_per_food + config.points_per_kill
    );
    assert_eq!(
        killer.wallet_after,
        10.0 - killer.boost_points_spent + config.food.points_per_food + config.points_per_kill
    );
    ledger.apply_physics(step.evolved_deltas()).unwrap();
    let first_spend = ledger.slots[1].ledger.life_boost_spent;
    assert_eq!(ledger.slots[0].deaths, 1);
    assert!(ledger.slots[0].corpse_emitted_count > 0);
    assert!(ledger.slots[0].corpse_emitted_value > 0.0);
    assert_eq!(ledger.slots[0].last_body_owner_id, Some(20));
    let record = ledger.transaction_life_ends[0];
    assert_eq!(record.physical_id, source.snakes[0].id);
    assert_eq!(record.cause, EvolvedDeathCause::Body);
    assert_eq!(record.body_owner_id, Some(20));
    assert_eq!(record.contact_flags, 4);
    assert_eq!(
        record.corpse_emitted_count,
        ledger.slots[0].corpse_emitted_count
    );
    assert_eq!(
        record.corpse_emitted_value,
        ledger.slots[0].corpse_emitted_value
    );
    assert_eq!(
        record.life_net_points,
        ledger.slots[0].ledger.life_net_points()
    );

    stage_and_accept(&mut step, &mut phases).unwrap();
    ledger.apply_physics(step.evolved_deltas()).unwrap();
    assert_eq!(ledger.slots[0].deaths, 1);
    assert!(ledger.slots[1].ledger.life_boost_spent > first_spend);
    ledger
        .validate(step.finish(key(1)).unwrap().world(), 2)
        .unwrap();
    assert_eq!(source, original);
    let capacity = step.evolved_deltas.capacity();
    step.discard();
    assert!(step.evolved_deltas().is_empty());
    assert_eq!(step.evolved_deltas.capacity(), capacity);
}

#[test]
fn external_reward_closures_use_real_food_kill_and_terminal_physics() {
    use crate::contracts::{ExternalRewardEndReason, ExternalRewardTransitionKind};
    use crate::engine::external_reward::{
        ExternalRewardLedger, RewardBoundary, RewardLifeIdentity,
    };

    // Both externally controlled lives eat. The victim dies on the owner's
    // body in the same substep; the owner receives that actual kill credit.
    let mut source = body_collision_world(SnakeKind::External, false);
    source.snakes[1].kind = SnakeKind::External;
    source.snakes[1].population_slot = None;
    for index in 0..2 {
        source.pellets.push(PelletState {
            id: 900 + index as u64,
            position: source.snakes[index].position,
            value: 1.0,
            kind: 0,
            owner: None,
            color: 0,
        });
    }
    let begin = RewardBoundary {
        completed_step: 0,
        elapsed_seconds: 0.0,
    };
    let end = RewardBoundary {
        completed_step: 1,
        elapsed_seconds: 1.0 / 60.0,
    };
    let ledgers: Vec<_> = source
        .snakes
        .iter()
        .enumerate()
        .map(|(index, snake)| {
            let mut ledger = ExternalRewardLedger::new(
                RewardLifeIdentity {
                    world_epoch: key(1).world_epoch(),
                    lease_id: index as u64 + 1,
                    physical_snake_id: snake.id,
                    frame_snake_id: snake.frame_v1_id,
                },
                begin,
                snake.body.len,
                snake.points,
            )
            .unwrap();
            let initial = ledger.prepare(index as u64 + 1, begin, None).unwrap();
            ledger.commit_delivery(&initial).unwrap();
            ledger
        })
        .collect();
    for (index, reward) in ledgers.into_iter().enumerate() {
        let mut controller = lease(source.snakes[index].id);
        controller.id = index as u64 + 1;
        controller.kind = ControllerKind::ReinforcementLearning;
        controller.connection_id = Some(index as u64 + 1);
        controller.scope = "reward-fixture".into();
        controller.reward = Some(reward);
        source.controller_leases.push(controller);
    }
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng_bundle(0),
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    let mut phases = PhaseWorkspaces::default();
    stage_and_accept(&mut step, &mut phases).unwrap();
    // Production substep admission updates the staged lease ledgers. The
    // test no longer supplies a parallel accounting path beside physics.
    let mut ledgers: Vec<_> = step
        .world()
        .unwrap()
        .controller_leases
        .iter()
        .map(|controller| controller.reward.unwrap())
        .collect();
    let victim = ledgers[0]
        .prepare(3, end, Some(ExternalRewardEndReason::Generation))
        .unwrap();
    let victim_wire = victim.envelope("reward-fixture").unwrap();
    assert_eq!(
        victim_wire.transition_kind,
        ExternalRewardTransitionKind::Terminal
    );
    assert_eq!(victim_wire.deltas.food_value_delta, 1.0);
    assert_eq!(victim_wire.deltas.food_count_delta, 1);
    assert_eq!(victim_wire.deltas.food_points_delta, 20.0);
    assert_eq!(victim_wire.deltas.length_delta, 1);
    assert_eq!(victim_wire.deltas.retained_growth_delta, 1);
    assert_eq!(victim_wire.deltas.kills_delta, 0);
    assert_eq!(victim_wire.state.starting_length, 12);
    assert_eq!(victim_wire.state.body_length, 13);
    assert_eq!(victim_wire.state.retained_growth, 1);
    let owner = ledgers[1]
        .prepare(4, end, Some(ExternalRewardEndReason::Generation))
        .unwrap();
    let owner_wire = owner.envelope("reward-fixture").unwrap();
    assert_eq!(
        owner_wire.transition_kind,
        ExternalRewardTransitionKind::Truncated
    );
    assert_eq!(owner_wire.deltas.food_value_delta, 1.0);
    assert_eq!(owner_wire.deltas.kills_delta, 1);
    assert_eq!(owner_wire.deltas.kill_points_delta, 400.0);
    assert_eq!(owner_wire.deltas.points_delta, 420.0);
    assert_eq!(owner_wire.state.starting_length, 15);
    assert_eq!(owner_wire.state.body_length, 16);
    assert_eq!(owner_wire.state.retained_growth, 1);
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../contracts/slither_contracts/external-reward-v1-fixture.json");
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture).unwrap()).unwrap();
    assert_eq!(serde_json::json!([victim_wire, owner_wire]), expected);
    ledgers[0].commit_delivery(&victim).unwrap();
    ledgers[1].commit_delivery(&owner).unwrap();
    assert!(ledgers[0].commit_delivery(&victim).is_err());
    // Physics staging leaves the authoritative source untouched.
    assert!(source
        .snakes
        .iter()
        .all(|snake| snake.alive && snake.food == 0.0));
}

#[test]
fn teacher_outcomes_retain_early_boost_after_exhaustion_and_actual_death() {
    use crate::engine::teacher_capture::{record_teacher_physics, TeacherPhysicalOutcomes};
    let position = WorldPoint { x: 0.0, y: 0.0 };
    let mut source = pack_world(vec![(
        snake(7, SnakeKind::Baseline, position, 0.0, 8),
        line_body(position, 0.0, 8),
    )]);
    source.snakes[0].input_boost = true;
    let original = source.clone();
    let mut config = PhysicsConfig::typescript_defaults();
    config.substep_dt = 1.0 / 180.0;
    config.movement.boost_points_cost_per_second = 10_000.0;
    let mut step = PhysicsStepWorkspace::new();
    step.begin(key(1), &source, &rng_bundle(1), &allocators(), config, 3)
        .unwrap();
    let mut phases = PhaseWorkspaces::default();
    let mut outcomes = TeacherPhysicalOutcomes::default();
    for substep in 0..3 {
        stage_and_accept(&mut step, &mut phases).unwrap();
        record_teacher_physics(&mut outcomes, &source, step.evolved_deltas());
        assert_eq!(
            step.evolved_deltas()[0].boost_points_spent > 0.0,
            substep == 0
        );
    }
    assert!(!step.finish(key(1)).unwrap().world().snakes[0].boost);
    assert!(outcomes.0[0].realized_boost);
    assert!(!outcomes.0[0].post_step_death);
    assert_eq!(outcomes.0[0].physical_snake_id, 7);
    assert_eq!(source, original);

    let source = body_collision_world(SnakeKind::Baseline, false);
    let mut step = PhysicsStepWorkspace::new();
    step.begin(key(2), &source, &rng_bundle(1), &allocators(), config, 3)
        .unwrap();
    let mut outcomes = TeacherPhysicalOutcomes::default();
    for _ in 0..3 {
        stage_and_accept(&mut step, &mut phases).unwrap();
        record_teacher_physics(&mut outcomes, &source, step.evolved_deltas());
    }
    assert!(outcomes.0[0].post_step_death);
    assert!(!outcomes.0[0].realized_boost);
    assert_eq!(outcomes.0[0].physical_snake_id, 7);
    assert!(!step.finish(key(2)).unwrap().world().snakes[0].alive);
}

#[test]
fn simultaneous_heads_die_without_any_kill_credit() {
    let source = head_head_world();
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng_bundle(0),
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    let mut phases = PhaseWorkspaces::default();
    stage_and_accept(&mut step, &mut phases).unwrap();
    let prepared = step.finish(key(1)).unwrap();
    assert!(prepared.world().snakes.iter().all(|snake| !snake.alive));
    assert!(prepared.world().snakes.iter().all(|snake| snake.kills == 0));
    assert!(prepared
        .world()
        .snakes
        .iter()
        .all(|snake| snake.points == 10.0));
    assert_eq!(prepared.world().pellets.len(), 4);
}

#[test]
fn body_owner_receives_one_credit_even_when_the_same_snapshot_kills_it() {
    let source = killed_body_owner_world();
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng_bundle(0),
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    step.advance_substep(&mut PhaseWorkspaces::default(), key(1))
        .unwrap();
    let prepared = step.finish(key(1)).unwrap();
    assert!(prepared.world().snakes.iter().all(|snake| !snake.alive));
    assert_eq!(snake_by_id(prepared.world(), 2).kills, 1);
    assert_eq!(snake_by_id(prepared.world(), 2).points, 410.0);
    assert_eq!(snake_by_id(prepared.world(), 1).kills, 0);
    assert_eq!(snake_by_id(prepared.world(), 3).kills, 0);
}

#[test]
fn reversing_container_order_preserves_per_id_state_bodies_pellets_rng_and_ids() {
    fn execute(source: &WorldState) -> (WorldState, RngStateBundle, AllocatorState) {
        let mut step = PhysicsStepWorkspace::new();
        step.begin(
            key(1),
            source,
            &rng_bundle(0),
            &allocators(),
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        stage_and_accept(&mut step, &mut PhaseWorkspaces::default()).unwrap();
        let prepared = step.finish(key(1)).unwrap();
        (
            prepared.world().clone(),
            prepared.rng().clone(),
            prepared.allocators().clone(),
        )
    }

    let forward = execute(&body_collision_world(SnakeKind::Evolved, false));
    let reversed = execute(&body_collision_world(SnakeKind::Evolved, true));
    for id in [7, 20] {
        assert_eq!(snake_by_id(&forward.0, id), snake_by_id(&reversed.0, id));
        assert_eq!(body_by_id(&forward.0, id), body_by_id(&reversed.0, id));
    }
    assert_eq!(forward.0.pellets, reversed.0.pellets);
    assert_eq!(forward.1, reversed.1);
    assert_eq!(forward.2, reversed.2);
}

#[test]
fn controlled_death_remains_a_complete_intermediate_physics_result() {
    let mut source = body_collision_world(SnakeKind::External, false);
    source.controller_leases.push(lease(7));
    let source_copy = source.clone();
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng_bundle(0),
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    let mut phases = PhaseWorkspaces::default();
    step.advance_substep(&mut phases, key(1))
        .expect("physics must retain the controlled death for the outer replacement join");
    assert!(phases.substep.ready.is_none());
    let prepared = step.finish(key(1)).expect("complete intermediate physics");
    assert!(!snake_by_id(prepared.world(), 7).alive);
    assert_eq!(prepared.world().controller_leases, source.controller_leases);
    assert_eq!(source, source_copy);
}

#[test]
fn stale_key_components_cannot_advance_or_finish_a_working_step() {
    let source = body_collision_world(SnakeKind::Evolved, false);
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng_bundle(0),
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    let mut phases = PhaseWorkspaces::default();
    assert_eq!(
        step.advance_substep(&mut phases, key(2)),
        Err(PhysicsError::StepKeyMismatch {
            field: PhysicsStepKeyField::OperationEpoch,
        })
    );
    assert_eq!(step.diagnostics().completed_substeps, 0);
    let wrong_world = PhysicsStepKey::new(3, 3, 40, 7, 11, [0x5a; 32], 1);
    assert_eq!(
        step.advance_substep(&mut phases, wrong_world),
        Err(PhysicsError::StepKeyMismatch {
            field: PhysicsStepKeyField::WorldEpoch,
        })
    );
    let wrong_revision = PhysicsStepKey::new(2, 3, 40, 7, 12, [0x5a; 32], 1);
    assert_eq!(
        step.advance_substep(&mut phases, wrong_revision),
        Err(PhysicsError::StepKeyMismatch {
            field: PhysicsStepKeyField::ConfigRevision,
        })
    );
    let wrong_hash = PhysicsStepKey::new(2, 3, 40, 7, 11, [0x5b; 32], 1);
    assert_eq!(
        step.advance_substep(&mut phases, wrong_hash),
        Err(PhysicsError::StepKeyMismatch {
            field: PhysicsStepKeyField::ConfigHash,
        })
    );
    assert_eq!(step.diagnostics().completed_substeps, 0);
    step.advance_substep(&mut phases, key(1)).unwrap();
    assert!(matches!(
        step.finish(key(2)),
        Err(PhysicsError::StepKeyMismatch {
            field: PhysicsStepKeyField::OperationEpoch,
        })
    ));
}

#[test]
fn multi_substep_step_carries_rng_allocator_and_awards_only_once() {
    let source = body_collision_world(SnakeKind::Evolved, false);
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng_bundle(0),
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        2,
    )
    .unwrap();
    let mut phases = PhaseWorkspaces::default();
    stage_and_accept(&mut step, &mut phases).unwrap();
    assert!(matches!(
        step.finish(key(1)),
        Err(PhysicsError::IncompleteSubsteps {
            completed: 1,
            expected: 2
        })
    ));
    let after_first_rng = step.rng().unwrap().clone();
    let after_first_allocator = step.allocators().unwrap().clone();
    stage_and_accept(&mut step, &mut phases).unwrap();
    let prepared = step.finish(key(1)).unwrap();
    assert_eq!(snake_by_id(prepared.world(), 20).kills, 1);
    assert_eq!(snake_by_id(prepared.world(), 20).points, 410.0);
    assert_eq!(prepared.rng(), &after_first_rng);
    assert_eq!(prepared.allocators(), &after_first_allocator);
    assert_eq!(prepared.world().pellets.len(), 4);
    assert!(matches!(
        step.next_substep_key(),
        Err(PhysicsError::TooManySubsteps { expected: 2 })
    ));
}

#[test]
fn failure_after_one_substep_cannot_expose_a_partial_fixed_step() {
    let source = body_collision_world(SnakeKind::Evolved, false);
    let source_copy = source.clone();
    let rng = rng_bundle(0);
    let rng_copy = rng.clone();
    let allocators = allocators();
    let allocator_copy = allocators.clone();
    let mut step = PhysicsStepWorkspace::new();
    let mut config = PhysicsConfig::typescript_defaults();
    config.maximum_pellet_index_entries = 1;
    step.begin(key(1), &source, &rng, &allocators, config, 2)
        .unwrap();
    let mut phases = PhaseWorkspaces::default();
    step.advance_substep(&mut phases, key(1)).unwrap();
    assert_eq!(step.diagnostics().completed_substeps, 1);

    let error = step
        .advance_substep(&mut phases, key(1))
        .expect_err("second substep should reject its larger pellet index");
    assert!(matches!(
        error,
        PhysicsError::Spatial(error)
            if matches!(
                *error,
                SpatialIndexError::EntryLimitExceeded {
                    kind: "pellet",
                    required: 4,
                    maximum: 1,
                }
            )
    ));
    assert!(matches!(
        step.finish(key(1)),
        Err(PhysicsError::IncompleteSubsteps {
            completed: 1,
            expected: 2,
        })
    ));
    assert_eq!(source, source_copy);
    assert_eq!(rng, rng_copy);
    assert_eq!(allocators, allocator_copy);
}

#[test]
fn kill_overflow_failure_keeps_step_unmodified_and_valid_retry_clears_scratch() {
    let mut source = body_collision_world(SnakeKind::Evolved, false);
    snake_by_id_mut(&mut source, 20).kills = u64::MAX;
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng_bundle(0),
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    let before = step.world().unwrap().clone();
    let mut phases = PhaseWorkspaces::default();
    let error = step
        .advance_substep(&mut phases, key(1))
        .expect_err("kill overflow should reject scratch");
    assert_eq!(error, PhysicsError::KillCountOverflow { killer_id: 20 });
    assert!(phases.substep.ready.is_none());
    assert_eq!(step.world().unwrap(), &before);

    snake_by_id_mut(&mut source, 20).kills = 0;
    step.begin(
        key(2),
        &source,
        &rng_bundle(0),
        &allocators(),
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    stage_and_accept(&mut step, &mut phases).unwrap();
    assert_eq!(
        snake_by_id(step.finish(key(2)).unwrap().world(), 20).kills,
        1
    );
}

fn snake_by_id_mut(world: &mut WorldState, id: u64) -> &mut SnakeState {
    world
        .snakes
        .iter_mut()
        .find(|snake| snake.id == id)
        .expect("fixture snake should exist")
}

#[test]
fn baseline_wall_death_accumulates_one_respawn_event_without_rng_draw() {
    let position = WorldPoint { x: 3_495.0, y: 0.0 };
    let source = pack_world(vec![(
        snake(9, SnakeKind::Baseline, position, 0.0, 8),
        line_body(position, 0.0, 8),
    )]);
    let rng = rng_bundle(1);
    let allocators = allocators();
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng,
        &allocators,
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    stage_and_accept(&mut step, &mut PhaseWorkspaces::default()).unwrap();
    let prepared = step.finish(key(1)).unwrap();
    assert_eq!(
        prepared.baseline_deaths(),
        &[BaselineDeathEvent {
            slot: 0,
            snake_id: 9,
        }]
    );
    assert!(prepared.world().pellets.is_empty());
    assert_eq!(prepared.rng(), &rng);
    assert_eq!(prepared.allocators(), &allocators);
    let baseline_deaths = prepared.prepared_baseline_deaths();
    assert_eq!(baseline_deaths.key(), key(1));
    assert!(std::ptr::eq(baseline_deaths.world(), prepared.world()));
    assert_eq!(baseline_deaths.events(), prepared.baseline_deaths());
}

#[test]
fn normal_baseline_death_uses_its_stream_and_emits_one_respawn_event() {
    let source = body_collision_world(SnakeKind::Baseline, false);
    let rng = rng_bundle(1);
    let allocators = allocators();
    let mut step = PhysicsStepWorkspace::new();
    step.begin(
        key(1),
        &source,
        &rng,
        &allocators,
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    step.advance_substep(&mut PhaseWorkspaces::default(), key(1))
        .unwrap();
    let prepared = step.finish(key(1)).unwrap();
    assert_eq!(
        prepared.baseline_deaths(),
        &[BaselineDeathEvent {
            slot: 0,
            snake_id: 7,
        }]
    );
    assert_eq!(prepared.world().pellets.len(), 4);
    assert_eq!(prepared.allocators().next_entity_id, 1_004);
    assert_ne!(prepared.rng().baselines, rng.baselines);
    assert_eq!(prepared.rng().world, rng.world);
    assert_eq!(prepared.rng().evolution, rng.evolution);
    assert_eq!(prepared.rng().external_controller, rng.external_controller);
}

#[test]
fn repeated_steps_reuse_all_reported_physics_capacities() {
    let mut source = body_collision_world(SnakeKind::Baseline, false);
    source.pellets.extend((0..32).map(|offset| PelletState {
        id: 100 + offset,
        position: WorldPoint {
            x: 2_000.0 + offset as f64,
            y: 2_000.0,
        },
        value: 1.0,
        kind: 0,
        color: 0,
        owner: None,
    }));
    let rng = rng_bundle(1);
    let allocators = allocators();
    let mut step = PhysicsStepWorkspace::new();
    let mut phases = PhaseWorkspaces::default();
    step.begin(
        key(1),
        &source,
        &rng,
        &allocators,
        PhysicsConfig::typescript_defaults(),
        1,
    )
    .unwrap();
    stage_and_accept(&mut step, &mut phases).unwrap();
    step.finish(key(1)).unwrap();
    let mut first_step = None;
    let mut first_substep = None;
    let mut first_pellet_index = None;
    for operation_epoch in 2..=25 {
        step.begin(
            key(operation_epoch),
            &source,
            &rng,
            &allocators,
            PhysicsConfig::typescript_defaults(),
            1,
        )
        .unwrap();
        stage_and_accept(&mut step, &mut phases).unwrap();
        let prepared = step.finish(key(operation_epoch)).unwrap();
        let step_diagnostics = prepared.diagnostics();
        let substep_diagnostics = phases.substep_diagnostics();
        let pellet_index_diagnostics = phases.pellet_index_diagnostics();
        assert_eq!(pellet_index_diagnostics.pellets, 32);
        assert!(pellet_index_diagnostics.estimated_bytes > 0);
        for (capacity, required) in [
            (step_diagnostics.snake_capacity, step_diagnostics.snakes),
            (
                step_diagnostics.body_point_capacity,
                step_diagnostics.body_points,
            ),
            (step_diagnostics.pellet_capacity, step_diagnostics.pellets),
            (
                step_diagnostics.baseline_event_capacity,
                step_diagnostics.baseline_deaths,
            ),
            (
                step_diagnostics.controlled_snake_capacity,
                step_diagnostics.controlled_snakes,
            ),
            (
                step_diagnostics.validation_order_capacity,
                step_diagnostics.snakes,
            ),
            (
                step_diagnostics.validation_range_capacity,
                step_diagnostics.snakes,
            ),
            (
                step_diagnostics.pellet_order_capacity,
                step_diagnostics.pellets,
            ),
        ] {
            assert!(capacity >= required);
        }
        if let Some(expected) = first_step {
            assert_eq!(step_diagnostics, expected);
        } else {
            first_step = Some(step_diagnostics);
        }
        if let Some(expected) = first_substep {
            assert_eq!(substep_diagnostics, expected);
        } else {
            first_substep = Some(substep_diagnostics);
        }
        if let Some(expected) = first_pellet_index {
            assert_eq!(pellet_index_diagnostics, expected);
        } else {
            first_pellet_index = Some(pellet_index_diagnostics);
        }
    }
}
