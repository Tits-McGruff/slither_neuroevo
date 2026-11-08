use super::*;

#[test]
fn cached_distance_order_preserves_ieee_bits_and_legacy_sort_without_growing_candidates() {
    assert_eq!(size_of::<DistanceOrderKey>(), size_of::<f64>());
    assert_eq!(size_of::<PelletCandidate>(), 2 * size_of::<u64>());
    assert_eq!(size_of::<BodyCandidate>(), 4 * size_of::<u64>());
    let mut bits = vec![
        0,
        1,
        2,
        0x000f_ffff_ffff_ffff,
        0x0010_0000_0000_0000,
        0x3ff0_0000_0000_0000,
        0x7fef_ffff_ffff_ffff,
        0x7ff0_0000_0000_0000,
        0x7ff0_0000_0000_0001,
        0x7ff8_0000_0000_0000,
        0x7fff_ffff_ffff_ffff,
    ];
    bits.extend(bits.clone().into_iter().map(|bits| bits | (1 << 63)));
    let mut state = 0x6f3b_2a19_85c4_07e1_u64;
    for _ in 0..16_384 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bits.push(state);
    }
    for &bits in &bits {
        let value = f64::from_bits(bits);
        assert_eq!(DistanceOrderKey::new(value).value().to_bits(), bits);
    }
    let mut expected = bits.iter().copied().map(f64::from_bits).collect::<Vec<_>>();
    expected.sort_unstable_by(f64::total_cmp);
    let mut actual = bits
        .into_iter()
        .map(f64::from_bits)
        .map(DistanceOrderKey::new)
        .collect::<Vec<_>>();
    actual.sort_unstable();
    assert_eq!(
        actual
            .into_iter()
            .map(|key| key.value().to_bits())
            .collect::<Vec<_>>(),
        expected.into_iter().map(f64::to_bits).collect::<Vec<_>>()
    );
}

#[test]
fn bounded_dense_cell_lookup_matches_sorted_fallback() {
    let cells = vec![
        CellSpan {
            key: CellKey { x: -2, y: 1 },
            start: 0,
            end: 2,
        },
        CellSpan {
            key: CellKey { x: 3, y: 4 },
            start: 2,
            end: 5,
        },
    ];
    let mut lookup = CellLookup::default();
    lookup.rebuild(&cells).expect("bounded lookup should build");
    assert!(lookup.cells() > 0);
    assert_eq!(lookup.find(&cells, cells[0].key), Some(cells[0]));
    assert_eq!(lookup.find(&cells, CellKey { x: 0, y: 0 }), None);

    let sparse = vec![
        CellSpan {
            key: CellKey { x: -100_000, y: 0 },
            start: 0,
            end: 1,
        },
        CellSpan {
            key: CellKey { x: 100_000, y: 0 },
            start: 1,
            end: 2,
        },
    ];
    let mut fallback = CellLookup::default();
    fallback
        .rebuild(&sparse)
        .expect("sparse fallback should build");
    assert_eq!(fallback.cells(), 0);
    assert_eq!(fallback.find(&sparse, sparse[1].key), Some(sparse[1]));

    let overflow_envelope = vec![
        CellSpan {
            key: CellKey {
                x: i32::MIN,
                y: i32::MIN,
            },
            start: 0,
            end: 1,
        },
        CellSpan {
            key: CellKey {
                x: i32::MAX,
                y: i32::MAX,
            },
            start: 1,
            end: 2,
        },
    ];
    let mut overflow_fallback = CellLookup::default();
    overflow_fallback
        .rebuild(&overflow_envelope)
        .expect("overflowing area should use fallback");
    assert_eq!(overflow_fallback.cells(), 0);
    assert_eq!(
        overflow_fallback.find(&overflow_envelope, overflow_envelope[0].key),
        Some(overflow_envelope[0])
    );
}
use crate::engine::state::{BodyRange, SnakeKind, SnakeState};

fn snake(id: u64, body: BodyRange, radius: f64) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: u32::try_from(id).expect("test id should fit"),
        kind: SnakeKind::External,
        alive: true,
        population_slot: None,
        brain: None,
        baseline_slot: None,
        baseline_strategy: None,
        position: WorldPoint { x: 0.0, y: 0.0 },
        previous_position: WorldPoint { x: 0.0, y: 0.0 },
        direction: 0.0,
        radius,
        speed: 0.0,
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
        control_accumulator_seconds: 0.0,
        delivered_observation_points: 0.0,
        observation_delivered: false,
        completed_step_heading_delta: 0.0,
        completed_step_speed_delta: 0.0,
        completed_step_applied_turn: 0.0,
        body,
        skin: 0,
    }
}

fn long_segment_world(reverse_snakes: bool) -> WorldState {
    let mut world = WorldState {
        body_points: vec![
            WorldPoint { x: -150.0, y: 0.0 },
            WorldPoint { x: 150.0, y: 0.0 },
            WorldPoint { x: 0.0, y: -150.0 },
            WorldPoint { x: 0.0, y: 150.0 },
        ],
        ..WorldState::default()
    };
    world.snakes = vec![
        snake(20, BodyRange { start: 0, len: 2 }, 8.0),
        snake(10, BodyRange { start: 2, len: 2 }, 9.0),
    ];
    world.snakes[0].position = world.body_points[0];
    world.snakes[1].position = world.body_points[2];
    if reverse_snakes {
        world.snakes.reverse();
    }
    world
}

#[test]
fn long_segments_cover_every_cell_in_their_bounds() {
    let mut world = long_segment_world(false);
    world.snakes.truncate(1);
    world.body_points.truncate(2);
    let index = BodySpatialIndex::build(&world, 50.0, 100).expect("index should build");
    assert_eq!(index.diagnostics().segments, 1);
    assert_eq!(index.diagnostics().entries, 7);
    assert_eq!(index.diagnostics().occupied_cells, 7);

    let mut scratch = BodyQueryScratch::default();
    let query = index
        .collect_candidates(WorldPoint { x: 0.0, y: 0.0 }, 1.0, &mut scratch)
        .expect("center query should succeed");
    assert_eq!(query.candidates, 1);
    assert_eq!(index.candidates(&scratch).count(), 1);
}

#[test]
fn sparse_body_and_pellet_indexes_query_through_automatic_fallback() {
    let far = 1_000_000.0;
    let mut world = WorldState {
        body_points: vec![
            WorldPoint { x: -far, y: -far },
            WorldPoint {
                x: -far + 1.0,
                y: -far,
            },
            WorldPoint { x: far, y: far },
            WorldPoint {
                x: far + 1.0,
                y: far,
            },
        ],
        pellets: vec![
            PelletState {
                id: 20,
                position: WorldPoint { x: -far, y: -far },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 10,
                position: WorldPoint { x: far, y: far },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
        ],
        ..WorldState::default()
    };
    world.snakes = vec![
        snake(20, BodyRange { start: 0, len: 2 }, 8.0),
        snake(10, BodyRange { start: 2, len: 2 }, 8.0),
    ];

    let body = BodySpatialIndex::build(&world, 10.0, 2).expect("body index should build");
    let pellets = PelletSpatialIndex::build(&world, 10.0, 2).expect("pellet index should build");
    assert_eq!(body.diagnostics().lookup_cells, 0);
    assert_eq!(pellets.diagnostics().lookup_cells, 0);

    let center = WorldPoint { x: far, y: far };
    let mut body_scratch = BodyQueryScratch::default();
    let body_query = body
        .collect_candidates(center, 2.0, &mut body_scratch)
        .expect("sparse body query should succeed");
    assert_eq!(body_query.candidates, 1);
    assert_eq!(
        body.candidates(&body_scratch)
            .map(|segment| segment.owner_id)
            .collect::<Vec<_>>(),
        vec![10]
    );

    let mut pellet_scratch = PelletQueryScratch::default();
    let pellet_query = pellets
        .collect_candidates(center, 2.0, &mut pellet_scratch)
        .expect("sparse pellet query should succeed");
    assert_eq!(pellet_query.candidates, 1);
    assert_eq!(
        pellets
            .candidates(&pellet_scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>(),
        vec![10]
    );
}

#[test]
fn retained_rebuild_matches_fresh_indexes_and_stabilizes_capacities() {
    let world = long_segment_world(false);
    let mut pellet_world = world.clone();
    pellet_world.pellets = vec![
        PelletState {
            id: 30,
            position: WorldPoint { x: 25.0, y: 0.0 },
            value: 1.0,
            kind: 0,
            color: 0,
            owner: None,
        },
        PelletState {
            id: 10,
            position: WorldPoint { x: -25.0, y: 0.0 },
            value: 2.0,
            kind: 1,
            color: 2,
            owner: Some(20),
        },
    ];
    let fresh_body = BodySpatialIndex::build(&pellet_world, 50.0, 100).unwrap();
    let fresh_pellets = PelletSpatialIndex::build(&pellet_world, 25.0, 100).unwrap();
    let mut body = BodySpatialIndex::empty();
    let mut pellets = PelletSpatialIndex::empty();
    body.rebuild(&pellet_world, 50.0, 100).unwrap();
    pellets.rebuild(&pellet_world, 25.0, 100).unwrap();
    assert_eq!(body.diagnostics(), fresh_body.diagnostics());
    assert_eq!(pellets.diagnostics(), fresh_pellets.diagnostics());

    let mut fresh_body_scratch = BodyQueryScratch::default();
    let mut rebuilt_body_scratch = BodyQueryScratch::default();
    fresh_body
        .collect_candidates(
            WorldPoint { x: 0.0, y: 0.0 },
            200.0,
            &mut fresh_body_scratch,
        )
        .unwrap();
    body.collect_candidates(
        WorldPoint { x: 0.0, y: 0.0 },
        200.0,
        &mut rebuilt_body_scratch,
    )
    .unwrap();
    assert_eq!(
        fresh_body
            .candidates(&fresh_body_scratch)
            .copied()
            .collect::<Vec<_>>(),
        body.candidates(&rebuilt_body_scratch)
            .copied()
            .collect::<Vec<_>>()
    );

    let mut fresh_pellet_scratch = PelletQueryScratch::default();
    let mut rebuilt_pellet_scratch = PelletQueryScratch::default();
    fresh_pellets
        .collect_candidates(
            WorldPoint { x: 0.0, y: 0.0 },
            100.0,
            &mut fresh_pellet_scratch,
        )
        .unwrap();
    pellets
        .collect_candidates(
            WorldPoint { x: 0.0, y: 0.0 },
            100.0,
            &mut rebuilt_pellet_scratch,
        )
        .unwrap();
    assert_eq!(
        fresh_pellets
            .candidates(&fresh_pellet_scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>(),
        pellets
            .candidates(&rebuilt_pellet_scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>()
    );

    let first_body = body.diagnostics();
    let first_pellets = pellets.diagnostics();
    for _ in 0..24 {
        body.rebuild(&pellet_world, 50.0, 100).unwrap();
        pellets.rebuild(&pellet_world, 25.0, 100).unwrap();
        assert_eq!(body.diagnostics(), first_body);
        assert_eq!(pellets.diagnostics(), first_pellets);
    }
}

#[test]
fn duplicate_cell_hits_yield_one_deterministically_ordered_segment() {
    let world = long_segment_world(false);
    let index = BodySpatialIndex::build(&world, 50.0, 100).expect("index should build");
    let mut scratch = BodyQueryScratch::default();
    let query = index
        .collect_candidates(WorldPoint { x: 0.0, y: 0.0 }, 120.0, &mut scratch)
        .expect("query should succeed");
    assert!(query.entries_visited > query.candidates);
    let owners = index
        .candidates(&scratch)
        .map(|record| record.owner_id)
        .collect::<Vec<_>>();
    assert_eq!(owners, vec![10, 20]);

    let reversed = long_segment_world(true);
    let reversed_index =
        BodySpatialIndex::build(&reversed, 50.0, 100).expect("reversed index should build");
    let mut reversed_scratch = BodyQueryScratch::default();
    reversed_index
        .collect_candidates(WorldPoint { x: 0.0, y: 0.0 }, 120.0, &mut reversed_scratch)
        .expect("reversed query should succeed");
    let reversed_owners = reversed_index
        .candidates(&reversed_scratch)
        .map(|record| record.owner_id)
        .collect::<Vec<_>>();
    assert_eq!(reversed_owners, owners);
}

#[test]
fn bounded_body_query_keeps_exact_uncapped_order_and_reuses_capacity() {
    let mut world = WorldState::default();
    for (offset, owner_id) in [1_u64, 10, 20, 30].into_iter().enumerate() {
        let x = if owner_id == 1 { 0.0 } else { owner_id as f64 };
        let start = world.body_points.len();
        world
            .body_points
            .extend([WorldPoint { x, y: 0.0 }, WorldPoint { x: x + 1.0, y: 0.0 }]);
        world.snakes.push(snake(
            owner_id,
            BodyRange { start, len: 2 },
            5.0 + offset as f64,
        ));
    }
    let index = BodySpatialIndex::build(&world, 10.0, 100).expect("index should build");
    let mut scratch = BodySensorQueryScratch::default();

    let exact = index
        .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 25.0, 1, 2, &mut scratch)
        .expect("exact-boundary query should succeed");
    assert_eq!(exact.candidates, 2);
    assert!(!exact.candidate_limit_reached);
    assert_eq!(
        index
            .sensor_candidates(&scratch)
            .map(|record| record.owner_id)
            .collect::<Vec<_>>(),
        vec![10, 20]
    );

    let capped = index
        .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 100.0, 1, 2, &mut scratch)
        .expect("capped query should succeed");
    assert_eq!(capped.candidates, 2);
    assert!(capped.candidate_limit_reached);
    assert_eq!(scratch.candidate_count(), 2);
    let warmed_capacity = scratch.candidate_capacity();
    index
        .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 100.0, 1, 2, &mut scratch)
        .expect("repeat query should succeed");
    assert_eq!(scratch.candidate_capacity(), warmed_capacity);
}

#[test]
fn complete_body_admission_fails_instead_of_truncating() {
    let world = long_segment_world(false);
    let error = BodySpatialIndex::build(&world, 50.0, 10)
        .expect_err("complete entry limit should reject the build");
    assert!(matches!(
        error,
        SpatialIndexError::EntryLimitExceeded {
            kind: "body",
            required: 14,
            maximum: 10
        }
    ));
}

#[test]
fn dead_snakes_contribute_no_collision_or_sensor_truth() {
    let mut world = long_segment_world(false);
    world.snakes[0].alive = false;
    let index = BodySpatialIndex::build(&world, 50.0, 100).expect("index should build");
    assert_eq!(index.diagnostics().segments, 1);
    assert_eq!(index.maximum_owner_radius(), 9.0);
}

#[test]
fn pellet_cell_order_retains_full_width_ids_source_ties_and_rebuild_capacity() {
    let ids = [u64::MAX, 0, 1_u64 << 63, 7, u64::MAX, 7, 0];
    let mut index = PelletSpatialIndex::empty();
    let mut warmed = None;
    for rotation in 0..ids.len() * 2 {
        let mut world = WorldState::default();
        for offset in 0..ids.len() {
            let item = (rotation + offset) % ids.len();
            world.pellets.push(PelletState {
                id: ids[item],
                position: WorldPoint {
                    x: if item.is_multiple_of(2) { -0.5 } else { 0.5 },
                    y: if item.is_multiple_of(3) { -10.5 } else { 10.5 },
                },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            });
        }
        if rotation >= ids.len() {
            world.pellets.reverse();
        }
        index.rebuild(&world, 10.0, ids.len()).unwrap();
        // Build the expected public identities directly from the source world,
        // independent of canonical record ordinals and the entry comparator.
        let mut expected: Vec<_> = world
            .pellets
            .iter()
            .enumerate()
            .map(|(source, pellet)| {
                (
                    if pellet.position.x < 0.0 { -1 } else { 0 },
                    if pellet.position.y < 0.0 { -2 } else { 1 },
                    pellet.id,
                    source,
                )
            })
            .collect();
        expected.sort_unstable();
        let actual: Vec<_> = index
            .entries
            .iter()
            .map(|entry| {
                let pellet = &index.pellets[entry.pellet];
                (entry.key.x, entry.key.y, pellet.id, pellet.source_index)
            })
            .collect();
        assert_eq!(actual, expected);
        let capacity = (
            index.pellets.capacity(),
            index.entries.capacity(),
            index.cells.capacity(),
            index.lookup.span_indices.capacity(),
        );
        if let Some(previous) = warmed {
            assert_eq!(capacity, previous);
        }
        warmed = Some(capacity);
    }
}

#[test]
fn warmed_pellet_grouping_matches_cold_sort_through_grid_changes() {
    let mut retained = PelletSpatialIndex::empty();
    for spacing in [1.0, 1.0, 10.0, 10.0, 100_000.0, 2.0, 0.0] {
        let world = WorldState {
            pellets: (0..20)
                .rev()
                .map(|item| PelletState {
                    id: u64::MAX - item % 7,
                    position: WorldPoint {
                        x: ((item % 5) as f64 - 2.0) * spacing,
                        y: ((item / 5) as f64 - 2.0) * spacing,
                    },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                })
                .collect(),
            ..WorldState::default()
        };
        let cold = PelletSpatialIndex::build(&world, 10.0, 20).unwrap();
        retained.rebuild(&world, 10.0, 20).unwrap();
        assert_eq!(retained.pellets, cold.pellets);
        assert_eq!(retained.entries, cold.entries);
        assert_eq!(retained.cells, cold.cells);
        assert_eq!(retained.lookup.minimum, cold.lookup.minimum);
        assert_eq!(retained.lookup.width, cold.lookup.width);
        assert_eq!(retained.lookup.span_indices, cold.lookup.span_indices);
        assert_eq!(
            retained.diagnostics.occupied_cells,
            cold.diagnostics.occupied_cells
        );
    }
    let warmed_capacity = retained.lookup.span_indices.capacity();
    retained.rebuild(&WorldState::default(), 10.0, 20).unwrap();
    assert!(retained.entries.is_empty() && retained.cells.is_empty());
    assert!(retained.lookup.span_indices.is_empty());
    assert_eq!(retained.lookup.span_indices.capacity(), warmed_capacity);
}

#[test]
fn pellets_are_nearest_first_with_stable_id_ties() {
    let world = WorldState {
        pellets: vec![
            PelletState {
                id: 20,
                position: WorldPoint { x: -10.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 10,
                position: WorldPoint { x: 10.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 30,
                position: WorldPoint { x: 80.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
        ],
        ..WorldState::default()
    };
    let index = PelletSpatialIndex::build(&world, 25.0, 3).expect("index should build");
    let mut scratch = PelletQueryScratch::default();
    let query = index
        .collect_candidates(WorldPoint { x: 0.0, y: 0.0 }, 20.0, &mut scratch)
        .expect("query should succeed");
    assert_eq!(query.candidates, 2);
    let ids = index
        .candidates(&scratch)
        .map(|pellet| pellet.id)
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![10, 20]);
}

#[test]
fn bounded_pellet_query_keeps_exact_nearest_prefix_and_reuses_capacity() {
    let world = WorldState {
        pellets: vec![
            PelletState {
                id: 30,
                position: WorldPoint { x: 30.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 20,
                position: WorldPoint { x: 20.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 10,
                position: WorldPoint { x: 10.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
        ],
        ..WorldState::default()
    };
    let index = PelletSpatialIndex::build(&world, 10.0, 3).expect("index should build");
    let mut scratch = PelletQueryScratch::default();

    let exact = index
        .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 25.0, 2, &mut scratch)
        .expect("exact-boundary query should succeed");
    assert_eq!(exact.candidates, 2);
    assert!(!exact.candidate_limit_reached);

    let capped = index
        .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 100.0, 2, &mut scratch)
        .expect("capped query should succeed");
    assert_eq!(capped.candidates, 2);
    assert!(capped.candidate_limit_reached);
    assert_eq!(
        index
            .candidates(&scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>(),
        vec![10, 20]
    );
    let warmed_capacity = scratch.candidate_capacity();
    index
        .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 100.0, 2, &mut scratch)
        .expect("repeat query should succeed");
    assert_eq!(scratch.candidate_capacity(), warmed_capacity);
}

#[test]
fn bounded_pellet_query_matches_complete_prefix_across_rings_and_ties() {
    let positions = [
        (70_u64, -130.0, -15.0),
        (20, -80.0, 60.0),
        (90, -25.0, -100.0),
        (10, 100.0, 0.0),
        (30, 0.0, 100.0),
        (40, 145.0, 145.0),
        (50, -210.0, 35.0),
        (60, 280.0, -90.0),
    ];
    let make_world = |reverse: bool| {
        let iterator: Box<dyn Iterator<Item = &(u64, f64, f64)>> = if reverse {
            Box::new(positions.iter().rev())
        } else {
            Box::new(positions.iter())
        };
        WorldState {
            pellets: iterator
                .map(|(id, x, y)| PelletState {
                    id: *id,
                    position: WorldPoint { x: *x, y: *y },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                })
                .collect(),
            ..WorldState::default()
        }
    };
    let center = WorldPoint { x: 0.0, y: 0.0 };
    let radius = 500.0;
    let world = make_world(false);
    let index =
        PelletSpatialIndex::build(&world, 70.0, positions.len()).expect("index should build");
    let mut complete_scratch = PelletQueryScratch::default();
    index
        .collect_candidates(center, radius, &mut complete_scratch)
        .expect("complete query should succeed");
    let expected = index
        .candidates(&complete_scratch)
        .map(|pellet| pellet.id)
        .collect::<Vec<_>>();

    for limit in 0..=positions.len() + 1 {
        let mut bounded_scratch = PelletQueryScratch::default();
        let diagnostics = index
            .collect_sensor_candidates(center, radius, limit, &mut bounded_scratch)
            .expect("bounded query should succeed");
        let actual = index
            .candidates(&bounded_scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>();
        assert_eq!(
            actual,
            expected.iter().copied().take(limit).collect::<Vec<_>>()
        );
        assert_eq!(diagnostics.candidate_limit_reached, expected.len() > limit);
    }

    let reversed = make_world(true);
    let reversed_index = PelletSpatialIndex::build(&reversed, 70.0, positions.len())
        .expect("reversed index should build");
    let mut reversed_scratch = PelletQueryScratch::default();
    reversed_index
        .collect_sensor_candidates(center, radius, 4, &mut reversed_scratch)
        .expect("reversed bounded query should succeed");
    assert_eq!(
        reversed_index
            .candidates(&reversed_scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>(),
        expected[..4]
    );
}

#[test]
fn pellet_ring_stop_does_not_skip_equal_distance_smaller_id() {
    let center = WorldPoint { x: 5.0, y: 5.0 };
    let world = WorldState {
        pellets: vec![
            PelletState {
                id: 1,
                position: WorldPoint { x: 10.0, y: 5.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 2,
                position: WorldPoint { x: 49.0, y: 5.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 100,
                position: WorldPoint { x: 41.0, y: 32.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 200,
                position: WorldPoint { x: -31.0, y: 32.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
            PelletState {
                id: 50,
                position: WorldPoint { x: 50.0, y: 5.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            },
        ],
        ..WorldState::default()
    };
    let index = PelletSpatialIndex::build(&world, 10.0, 5).expect("index should build");
    let mut scratch = PelletQueryScratch::default();
    let diagnostics = index
        .collect_sensor_candidates(center, 100.0, 3, &mut scratch)
        .expect("bounded query should succeed");
    assert!(diagnostics.candidate_limit_reached);
    assert_eq!(
        index
            .candidates(&scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>(),
        vec![1, 2, 50]
    );
}

#[test]
fn pellet_admission_is_checked_before_index_construction() {
    let world = WorldState {
        pellets: vec![PelletState {
            id: 1,
            position: WorldPoint { x: 0.0, y: 0.0 },
            value: 1.0,
            kind: 0,
            color: 0,
            owner: None,
        }],
        ..WorldState::default()
    };
    assert!(matches!(
        PelletSpatialIndex::build(&world, 120.0, 0),
        Err(SpatialIndexError::EntryLimitExceeded {
            kind: "pellet",
            required: 1,
            maximum: 0
        })
    ));
}
