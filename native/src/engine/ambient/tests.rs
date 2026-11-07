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
