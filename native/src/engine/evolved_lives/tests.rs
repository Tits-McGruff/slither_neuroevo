use super::*;

fn runtime() -> EvolvedSlotRuntime {
    let current = LifeAssessment {
        ordinal: 1,
        starting_length: 5,
        final_length: 5,
        peak_length: 5,
        ..LifeAssessment::default()
    };
    EvolvedSlotRuntime {
        population_slot: 0,
        physical_id: 1,
        status: EvolvedLifeStatus::Alive,
        life_ordinal: 1,
        lives_started: 1,
        deaths: 0,
        successful_respawns: 0,
        current,
        best_point_life: current,
        ledger: EvolvedEconomicLedger::default(),
        total_alive_seconds: 0.0,
        total_food: 0.0,
        total_kills: 0,
        completed_life_growth: 0.0,
        generation_peak_length: 5,
        pending_seconds: 0.0,
        respawn_deadline: None,
        respawn_deadline_step: None,
        last_attempt_step: None,
        pending_recurrent_reset: false,
        last_attempt: None,
        fallback_cursor: 0,
        failed_placements: 0,
        last_cause: None,
        last_death_flags: 0,
        last_body_owner_id: None,
        cause_counts: [0; 4],
        corpse_emitted_count: 0,
        corpse_emitted_value: 0.0,
        corpse_discarded_count: 0,
        corpse_discarded_value: 0.0,
    }
}

fn delta(
    runtime: &EvolvedSlotRuntime,
    spent: f64,
    credits: f64,
    length: usize,
    alive: bool,
) -> EvolvedPhysicsDelta {
    EvolvedPhysicsDelta {
        population_slot: Some(runtime.population_slot),
        snake_id: runtime.physical_id,
        boost_points_spent: spent,
        credits,
        length_after: length,
        alive_after: alive,
        wallet_after: runtime.ledger.life_local_wallet + runtime.ledger.prior_bank_remaining
            - spent
            + credits,
        food: credits / 20.0,
        food_count: u64::from(credits > 0.0),
        food_credits: credits,
        kill_credits: 0.0,
        kills: 0,
        death: Some(EvolvedDeathCause::Body),
        death_flags: 4,
        body_owner_id: Some(99),
        corpse_emitted_count: 0,
        corpse_emitted_value: 0.0,
        corpse_discarded_count: 0,
        corpse_discarded_value: 0.0,
    }
}

#[test]
fn all_point_and_nonpoint_bundles_include_unfinished_lives_and_earliest_best_tie() {
    let mut slot = runtime();
    slot.alive_award(2.0, 80.0).unwrap();
    let first = delta(&slot, 50.0, 0.0, 8, false);
    slot.apply_physics(first).unwrap();
    assert_eq!(slot.ledger.generation_gross_earned, 80.0);
    assert_eq!(slot.ledger.generation_net_earned, 30.0);
    assert_eq!(
        slot.assessment(PointAssessmentMode::Reset, NonPointAggregation::FollowScore)
            .points,
        30.0
    );
    assert_eq!(
        slot.assessment(
            PointAssessmentMode::BestLife,
            NonPointAggregation::FollowScore
        )
        .points,
        80.0
    );
    assert_eq!(slot.begin_life(2, 5, false).unwrap(), 0.0);
    slot.alive_award(3.0, 80.0).unwrap();
    let second = delta(&slot, 10.0, 0.0, 9, true);
    slot.apply_physics(second).unwrap();
    assert_eq!(slot.best_point_life.ordinal, 1);
    assert_eq!(slot.best_point_life.alive_seconds, 2.0);
    for (mode, expected) in [
        (PointAssessmentMode::Reset, 70.0),
        (PointAssessmentMode::CumulativeNet, 100.0),
        (PointAssessmentMode::CumulativeGross, 160.0),
        (PointAssessmentMode::BestLife, 80.0),
    ] {
        for aggregation in [
            NonPointAggregation::GenerationTotals,
            NonPointAggregation::FollowScore,
            NonPointAggregation::LegacyTerminal,
        ] {
            let assessment = slot.assessment(mode, aggregation);
            assert_eq!(assessment.points, expected);
            let totals = aggregation == NonPointAggregation::GenerationTotals
                || (aggregation == NonPointAggregation::FollowScore
                    && matches!(
                        mode,
                        PointAssessmentMode::CumulativeNet | PointAssessmentMode::CumulativeGross
                    ));
            let best = aggregation == NonPointAggregation::FollowScore
                && mode == PointAssessmentMode::BestLife;
            assert_eq!(
                assessment.alive_seconds,
                if totals {
                    5.0
                } else if best {
                    2.0
                } else {
                    3.0
                }
            );
            assert_eq!(
                assessment.retained_growth,
                if totals {
                    7.0
                } else if best {
                    3.0
                } else {
                    4.0
                }
            );
        }
    }
    slot.alive_award(1.0, 11.0).unwrap();
    assert_eq!(slot.best_point_life.ordinal, 2);
    assert_eq!(slot.best_point_life.peak_points, 81.0);
    assert_eq!(slot.best_point_life.alive_seconds, 4.0);
}

#[test]
fn peak_growth_is_separate_from_retained_growth_and_death_finalizes_once() {
    let mut slot = runtime();
    let grow = delta(&slot, 0.0, 20.0, 12, true);
    slot.apply_physics(grow).unwrap();
    let shrink = delta(&slot, 10.0, 0.0, 7, false);
    slot.apply_physics(shrink).unwrap();
    assert_eq!(slot.completed_life_growth, 2.0);
    assert_eq!(slot.current.peak_length, 12);
    assert_eq!(
        slot.finalize(EvolvedDeathCause::Body),
        Err(EvolvedLifeError::Lifecycle)
    );
    assert_eq!(slot.deaths, 1);
    slot.begin_life(2, 5, false).unwrap();
    let end = delta(&slot, 0.0, 5.0, 4, false);
    slot.apply_physics(end).unwrap();
    assert_eq!(slot.completed_life_growth, 2.0);
    assert_eq!(slot.generation_peak_length, 12);
    assert_eq!(
        slot.assessment(
            PointAssessmentMode::CumulativeNet,
            NonPointAggregation::GenerationTotals
        )
        .retained_growth,
        2.0
    );
}

#[test]
fn successive_substeps_attribute_spend_before_new_food_and_simultaneous_death_credit() {
    let mut slot = runtime();
    slot.alive_award(1.0, 50.0).unwrap();
    slot.finalize(EvolvedDeathCause::GodMode).unwrap();
    assert_eq!(slot.begin_life(2, 5, true).unwrap(), 50.0);
    let first = delta(&slot, 50.0, 10.0, 6, true);
    slot.apply_physics(first).unwrap();
    assert_eq!(slot.ledger.life_net_points(), 10.0);
    let mut second = delta(&slot, 5.0, 400.0, 6, false);
    second.kills = 1;
    slot.apply_physics(second).unwrap();
    assert_eq!(slot.ledger.life_boost_spent_from_prior_bank, 50.0);
    assert_eq!(slot.ledger.life_boost_spent_from_life_credits, 5.0);
    assert_eq!(slot.ledger.life_net_points(), 405.0);
    assert_eq!(slot.current.kills, 1);
    assert_eq!(slot.deaths, 2);
}

#[test]
fn committed_lifecycle_telemetry_preserves_state_partition_work_and_exact_counters() {
    use super::super::evolved_telemetry::EvolvedLifecycleTelemetry;
    let mut lives = EvolvedGenerationState::default();
    for (slot, status) in [
        EvolvedLifeStatus::Alive,
        EvolvedLifeStatus::Waiting,
        EvolvedLifeStatus::PendingPlacement,
        EvolvedLifeStatus::Exhausted,
        EvolvedLifeStatus::DeadNoRespawn,
    ]
    .into_iter()
    .enumerate()
    {
        let mut owner = runtime();
        owner.population_slot = slot as u32;
        owner.status = status;
        if status != EvolvedLifeStatus::Alive {
            owner.deaths = 1;
            owner.pending_seconds = 2.5;
        }
        owner.failed_placements = slot as u64;
        lives.slots.push(owner);
    }
    lives.placement_work = EvolvedPlacementWork {
        step: 10,
        eligible_slots: 5,
        attempted_slots: 3,
        placed_slots: 1,
        pending_no_placement: 1,
        pending_work_limit: 1,
        candidates: 768,
        geometry_checks: 250_000,
    };
    let prior = lives.clone();
    let report = EvolvedLifecycleTelemetry::capture(&lives, 10).unwrap();
    assert_eq!(
        (
            report.population,
            report.alive,
            report.waiting,
            report.pending_placement,
            report.exhausted,
            report.dead_no_respawn
        ),
        (5, 1, 1, 1, 1, 1)
    );
    assert_eq!(
        (
            report.lives,
            report.deaths,
            report.respawns,
            report.failed_placements
        ),
        (5, 4, 0, 10)
    );
    assert_eq!(report.pending_seconds, 10.0);
    assert_eq!(report.placement, lives.placement_work);
    assert_eq!(lives, prior);
    assert!(std::mem::size_of::<EvolvedLifecycleTelemetry>() <= 256);
    assert!(EvolvedLifecycleTelemetry::capture(&lives, 11).is_err());
    lives.placement_work.candidates = 2049;
    assert!(EvolvedLifecycleTelemetry::capture(&lives, 10).is_err());
    lives.placement_work = EvolvedPlacementWork::default();
    lives.slots[0].lives_started = u64::MAX;
    assert_eq!(
        EvolvedLifecycleTelemetry::capture(&lives, 10),
        Err(EvolvedLifeError::Overflow)
    );
}

#[test]
fn life_end_capture_keeps_simultaneous_contacts_and_actual_economics() {
    let mut state = EvolvedGenerationState {
        slots: vec![runtime()],
        ..EvolvedGenerationState::default()
    };
    state.slots[0].alive_award(2.0, 50.0).unwrap();
    let mut end = delta(&state.slots[0], 10.0, 400.0, 7, false);
    end.death = Some(EvolvedDeathCause::Wall);
    end.death_flags = 7;
    end.body_owner_id = Some(900);
    end.kills = 1;
    end.corpse_emitted_count = 2;
    end.corpse_emitted_value = 3.0;
    end.corpse_discarded_count = 4;
    end.corpse_discarded_value = 5.0;
    state.apply_physics(&[end]).unwrap();
    state.validate_events().unwrap();
    let record = state.transaction_life_ends[0];
    assert_eq!(state.recent_life_ends.front(), Some(&record));
    assert_eq!(record.cause, EvolvedDeathCause::Wall);
    assert_eq!(record.contact_flags, 7);
    assert_eq!(record.body_owner_id, Some(900));
    assert_eq!(record.physical_id, 1);
    assert_eq!(record.life_ordinal, 1);
    assert_eq!(record.life_net_points, 440.0);
    assert_eq!(record.life_gross_earned, 450.0);
    assert_eq!(record.life_boost_spent, 10.0);
    assert_eq!(record.wallet, 440.0);
    assert_eq!(record.food, 20.0);
    assert_eq!(record.kills, 1);
    assert_eq!(record.final_length, 7);
    assert_eq!(record.alive_seconds, 2.0);
    assert_eq!(record.corpse_discarded_value, 5.0);
    let mut altered = state.clone();
    altered.recent_life_ends[0].cause = EvolvedDeathCause::Body;
    assert_eq!(altered.validate_events(), Err(EvolvedLifeError::Lifecycle));
    let mut altered = state.clone();
    altered.transaction_life_ends[0].food += 1.0;
    assert_eq!(altered.validate_events(), Err(EvolvedLifeError::Lifecycle));
}

#[test]
fn population_sized_transaction_retains_only_the_latest_256_ends() {
    let slots = (0..300)
        .map(|index| {
            let mut owner = runtime();
            owner.population_slot = index;
            owner.physical_id = u64::from(index) + 1;
            owner
        })
        .collect();
    let source = EvolvedGenerationState {
        slots,
        ..EvolvedGenerationState::default()
    };
    let mut staged = EvolvedGenerationState::default();
    staged.stage_from(&source).unwrap();
    let deltas: Vec<_> = staged
        .slots
        .iter()
        .map(|owner| delta(owner, 0.0, 0.0, 5, false))
        .collect();
    staged.apply_physics(&deltas).unwrap();
    staged.validate_event_continuation(&source).unwrap();
    assert_eq!(staged.transaction_life_ends.len(), 300);
    assert_eq!(staged.recent_life_ends.len(), MAX_RECENT_LIFE_ENDS);
    assert_eq!(staged.recent_life_ends.front().unwrap().event_ordinal, 45);
    assert_eq!(staged.recent_life_ends.back().unwrap().event_ordinal, 300);
    let transaction_capacity = staged.transaction_life_ends.capacity();
    let ring_capacity = staged.recent_life_ends.capacity();
    let published = staged.clone();
    staged.stage_from(&published).unwrap();
    staged.validate_event_continuation(&published).unwrap();
    assert!(staged.transaction_life_ends.is_empty());
    assert_eq!(
        staged.transaction_life_ends.capacity(),
        transaction_capacity
    );
    assert_eq!(staged.recent_life_ends.capacity(), ring_capacity);
    let mut altered = staged.clone();
    altered.recent_life_ends[0].food = 1.0;
    assert_eq!(
        altered.validate_event_continuation(&published),
        Err(EvolvedLifeError::Lifecycle)
    );
}

#[test]
fn long_life_history_reuses_storage_and_rejects_unrecorded_deaths() {
    let mut published = EvolvedGenerationState {
        slots: vec![runtime()],
        ..EvolvedGenerationState::default()
    };
    let mut staged = EvolvedGenerationState::default();
    for ordinal in 1..=600 {
        staged.stage_from(&published).unwrap();
        if ordinal > 1 {
            staged.slots[0].begin_life(ordinal, 5, false).unwrap();
        }
        staged.slots[0].alive_award(0.25, 0.1).unwrap();
        let end = delta(&staged.slots[0], 0.0, 0.2, 5, false);
        staged.apply_physics(&[end]).unwrap();
        staged.validate_event_continuation(&published).unwrap();
        std::mem::swap(&mut staged, &mut published);
    }
    assert_eq!(published.life_end_count, 600);
    assert_eq!(published.recent_life_ends.len(), 256);
    let report =
        super::super::evolved_telemetry::EvolvedLifecycleTelemetry::capture(&published, 600)
            .unwrap();
    assert_eq!(
        (report.recent_event_count, report.life_end_cursor),
        (256, 600)
    );
    assert_eq!(
        published.recent_life_ends.front().unwrap().life_ordinal,
        345
    );
    assert_eq!(published.recent_life_ends.back().unwrap().physical_id, 600);
    let ring_capacity = published.recent_life_ends.capacity();
    staged.stage_from(&published).unwrap();
    staged.slots[0].begin_life(601, 5, false).unwrap();
    staged.slots[0]
        .finalize(EvolvedDeathCause::GodMode)
        .unwrap();
    assert_eq!(
        staged.validate_event_continuation(&published),
        Err(EvolvedLifeError::Lifecycle)
    );
    assert_eq!(staged.recent_life_ends.capacity(), ring_capacity);
    let boundary = EvolvedGenerationState::default();
    boundary.validate_events().unwrap();
    assert_eq!(boundary.life_end_count, 0);
}

fn lifecycle_fixture(
    count: usize,
) -> (
    EvolvedGenerationState,
    super::super::state::RngStateBundle,
    EvolvedExperimentConfig,
) {
    let slots = (0..count)
        .map(|index| {
            let mut slot = runtime();
            slot.population_slot = index as u32;
            slot.physical_id = index as u64 + 1;
            slot.finalize(EvolvedDeathCause::GodMode).unwrap();
            slot
        })
        .collect();
    let stream = super::super::rng::StatefulRng::new(42.0).export_state();
    let rng = super::super::state::RngStateBundle {
        baseline_behaviors: Vec::new(),
        version: super::super::state::RNG_BUNDLE_WITH_EVOLVED_VERSION,
        world: stream.clone(),
        evolution: stream.clone(),
        external_controller: stream,
        baselines: Vec::new(),
        evolved_respawns: super::super::state::EvolvedRespawnRngState::derive_generation(
            7, 1, count,
        )
        .unwrap(),
    };
    let mut policy = EvolvedExperimentConfig::legacy(240.0);
    policy.version = 1;
    policy.enabled = true;
    (
        EvolvedGenerationState {
            version: EVOLVED_LIVES_VERSION,
            slots,
            ..EvolvedGenerationState::default()
        },
        rng,
        policy,
    )
}

#[test]
fn zero_delay_waits_for_next_boundary_and_uniform_zero_draws_exactly_once() {
    let (mut lives, mut rng, mut policy) = lifecycle_fixture(1);
    policy.delay_mode = RespawnDelayMode::Uniform;
    let original = rng.clone();
    let mut expected = rng.evolved_respawns[0].runtime().unwrap();
    expected.next_f64();
    lives
        .schedule_deaths(policy, &mut rng, 20, 1.0, 1.0 / 60.0)
        .unwrap();
    assert_eq!(
        rng.evolved_respawns[0].uniform_state,
        expected.uniform_state()
    );
    assert_eq!(rng.world, original.world);
    assert_eq!(rng.evolution, original.evolution);
    assert_eq!(lives.slots[0].respawn_deadline_step, Some(21));
    let sampled = rng.clone();
    lives
        .schedule_deaths(policy, &mut rng, 20, 1.0, 1.0 / 60.0)
        .unwrap();
    assert_eq!(rng, sampled);
    let mut due = Vec::new();
    lives.collect_due(20, 1.0 / 60.0, &mut due).unwrap();
    assert!(due.is_empty());
    lives.collect_due(21, 1.0 / 60.0, &mut due).unwrap();
    assert_eq!(due, [0]);
    lives.record_failed_attempt(0, 21, 1.0, 224).unwrap();
    assert_eq!(lives.slots[0].life_ordinal, 1);
    assert_eq!(lives.slots[0].successful_respawns, 0);
    assert_eq!(lives.slots[0].fallback_cursor, 224);
    lives.collect_due(35, 1.0 / 60.0, &mut due).unwrap();
    assert!(due.is_empty());
    lives.collect_due(36, 1.0 / 60.0, &mut due).unwrap();
    assert_eq!(due, [0]);
}

#[test]
fn finite_exhaustion_does_not_sample_a_delay_and_world_uniform_preserves_evolution() {
    let (mut lives, mut rng, mut policy) = lifecycle_fixture(1);
    let original = rng.clone();
    policy.max_lives = 1;
    policy.delay_mode = RespawnDelayMode::Uniform;
    lives
        .schedule_deaths(policy, &mut rng, 1, 0.0, 1.0 / 60.0)
        .unwrap();
    assert_eq!(lives.slots[0].status, EvolvedLifeStatus::Exhausted);
    assert_eq!(rng, original);
    let (mut lives, mut rng, mut policy) = lifecycle_fixture(1);
    let original = rng.clone();
    policy.delay_mode = RespawnDelayMode::Uniform;
    policy.rng_mode = RespawnRngMode::World;
    policy.delay_seconds = 10.0;
    lives
        .schedule_deaths(policy, &mut rng, 1, 100.0, 1.0 / 60.0)
        .unwrap();
    let deadline = lives.slots[0].respawn_deadline.unwrap();
    assert!((105.0..=115.0).contains(&deadline));
    assert_ne!(rng.world, original.world);
    assert_eq!(rng.evolved_respawns, original.evolved_respawns);
    assert_eq!(rng.evolution, original.evolution);
}

#[test]
fn three_hundred_due_slots_get_bounded_ordered_attempts_without_consuming_lives() {
    let (mut lives, mut rng, policy) = lifecycle_fixture(300);
    lives
        .schedule_deaths(policy, &mut rng, 0, 0.0, 1.0 / 60.0)
        .unwrap();
    let mut due = Vec::new();
    for step in 1..=38 {
        lives.collect_due(step, 1.0 / 60.0, &mut due).unwrap();
        assert!(due.len() <= 8);
        for &slot in &due {
            lives
                .record_failed_attempt(slot, step, step as f64 / 60.0, 0)
                .unwrap();
        }
    }
    assert!(lives.slots.iter().all(|slot| slot.failed_placements >= 1
        && slot.life_ordinal == 1
        && slot.successful_respawns == 0));
    lives.collect_due(39, 1.0 / 60.0, &mut due).unwrap();
    assert_eq!(due, [4, 5, 6, 7, 8, 9, 10, 11]);
    let capacity = due.capacity();
    lives.collect_due(39, 1.0 / 60.0, &mut due).unwrap();
    assert_eq!(due.capacity(), capacity);
}

#[test]
fn pending_time_excludes_participating_newborns_and_deadline_overflow_rejects() {
    let (mut lives, mut rng, policy) = lifecycle_fixture(2);
    lives
        .schedule_deaths(policy, &mut rng, 1, 0.0, 1.0 / 60.0)
        .unwrap();
    lives.slots[0].begin_life(100, 5, false).unwrap();
    lives.slots[0].alive_award(1.0 / 60.0, 0.01).unwrap();
    lives.accrue_pending_time(1.0 / 60.0).unwrap();
    assert_eq!(lives.slots[0].pending_seconds, 0.0);
    assert_eq!(lives.slots[1].pending_seconds, 1.0 / 60.0);
    let (mut lives, mut rng, policy) = lifecycle_fixture(1);
    assert_eq!(
        lives.schedule_deaths(policy, &mut rng, u64::MAX, 0.0, 1.0 / 60.0),
        Err(EvolvedLifeError::Overflow)
    );
}

#[test]
fn inherited_burn_before_credit_preserves_life_net() {
    let mut ledger = EvolvedEconomicLedger::default();
    ledger.credit(50.0).unwrap();
    assert_eq!(ledger.begin_life(true).unwrap(), 50.0);
    ledger.spend(50.0).unwrap();
    ledger.credit(10.0).unwrap();
    assert_eq!(ledger.life_net_points(), 10.0);
    assert_eq!(ledger.life_boost_spent_from_prior_bank, 50.0);
    assert_eq!(ledger.life_boost_spent_from_life_credits, 0.0);
    assert_eq!(ledger.generation_net_earned, 10.0);
    ledger.validate().unwrap();
}

#[test]
fn life_credits_spend_first_and_gross_is_never_capital() {
    let mut ledger = EvolvedEconomicLedger::default();
    ledger.credit(50.0).unwrap();
    ledger.begin_life(true).unwrap();
    ledger.credit(10.0).unwrap();
    ledger.spend(15.0).unwrap();
    assert_eq!(ledger.life_local_wallet, 0.0);
    assert_eq!(ledger.prior_bank_remaining, 45.0);
    assert_eq!(ledger.life_boost_spent_from_life_credits, 10.0);
    assert_eq!(ledger.life_boost_spent_from_prior_bank, 5.0);
    assert_eq!(ledger.generation_gross_earned, 60.0);
    assert_eq!(ledger.begin_life(false).unwrap(), 0.0);
    let before = ledger;
    assert_eq!(ledger.spend(1.0), Err(EvolvedLifeError::Overdraw));
    assert_eq!(ledger, before);
}

#[test]
fn invalid_overflow_and_overdraw_leave_the_ledger_unchanged() {
    let mut ledger = EvolvedEconomicLedger::default();
    ledger.credit(1.0).unwrap();
    for amount in [f64::NAN, f64::INFINITY, -1.0] {
        let before = ledger;
        assert!(ledger.credit(amount).is_err());
        assert!(ledger.spend(amount).is_err());
        assert_eq!(ledger, before);
    }
    let before = ledger;
    assert_eq!(ledger.spend(1.0001), Err(EvolvedLifeError::Overdraw));
    assert_eq!(ledger, before);
}

#[test]
fn a_long_accumulation_history_does_not_increase_overdraw_allowance() {
    let mut ledger = EvolvedEconomicLedger::default();
    ledger.credit(1.0e12).unwrap();
    ledger.operations = 100_000_000;
    let before = ledger;
    assert_eq!(ledger.spend(1.0e12 + 1.0), Err(EvolvedLifeError::Overdraw));
    assert_eq!(ledger, before);
}

#[test]
fn repeated_fractional_credit_and_full_burn_remain_coherent() {
    let mut ledger = EvolvedEconomicLedger::default();
    let mut physical_wallet = 0.0;
    for step in 0..200_000 {
        let credit = if step % 17 == 0 { 3.125031 } else { 0.01 };
        physical_wallet += credit;
        ledger.credit(credit).unwrap();
        let spend = physical_wallet.min(0.333333333333);
        physical_wallet -= spend;
        ledger.spend(spend).unwrap();
        assert!(
            (physical_wallet - ledger.life_local_wallet).abs()
                <= ledger.rounding_bound(physical_wallet)
        );
    }
    ledger.validate().unwrap();
}
