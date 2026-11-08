use super::*;
use crate::engine::food::{FoodConfig, FoodWorkspace};
use crate::engine::movement::{MovementConfig, MovementWorkspace};
use crate::engine::spatial::IndexedPelletWorld;
use crate::engine::state::{BodyRange, SnakeKind, WorldState};
use std::mem::size_of;

const DT: f64 = 1.0 / 180.0;

#[test]
fn equal_cell_entry_order_keeps_every_large_query_candidate_canonical() {
    const RECORDS: usize = 200_003;
    let point = WorldPoint { x: 0.0, y: 0.0 };
    let segment = SweptSegment {
        owner_id: 2,
        segment_end: 5,
        previous_start: point,
        previous_end: point,
        current_start: point,
        current_end: point,
        movement_radius: 9.0,
        final_radius: 9.0,
        newly_grown: false,
        removed_at_final: false,
    };
    let mut index = SweptBodyIndex {
        cell_size: 1.0,
        maximum_query_cells: 64,
        segments: vec![segment; RECORDS],
        ..SweptBodyIndex::default()
    };
    // Each record occupies the center plus one adjacent cell. Source order
    // deliberately differs from canonical segment order, beyond the old cap.
    for segment in (0..RECORDS).rev() {
        index.entries.push(CellEntry {
            key: CellKey { x: 0, y: 0 },
            segment,
        });
        index.entries.push(CellEntry {
            key: CellKey {
                x: if segment % 2 == 0 { -1 } else { 1 },
                y: 0,
            },
            segment,
        });
    }
    index.entries.sort_unstable_by_key(|entry| entry.key);
    let left_end = RECORDS.div_ceil(2);
    let center_end = left_end + RECORDS;
    index.cells = vec![
        CellSpan {
            key: CellKey { x: -1, y: 0 },
            start: 0,
            end: left_end,
        },
        CellSpan {
            key: CellKey { x: 0, y: 0 },
            start: left_end,
            end: center_end,
        },
        CellSpan {
            key: CellKey { x: 1, y: 0 },
            start: center_end,
            end: RECORDS * 2,
        },
    ];
    let mut scratch = CandidateScratch::default();
    let (_, visited) = index
        .collect_candidates(point, point, 1.0, &mut scratch)
        .unwrap();
    assert_eq!(visited, RECORDS * 2);
    assert_eq!(scratch.candidates, (0..RECORDS).collect::<Vec<_>>());
    let capacity = scratch.candidates.capacity();
    for span in &index.cells {
        index.entries[span.start..span.end].reverse();
    }
    let (_, visited) = index
        .collect_candidates(point, point, 1.0, &mut scratch)
        .unwrap();
    assert_eq!(visited, RECORDS * 2);
    assert_eq!(scratch.candidates, (0..RECORDS).collect::<Vec<_>>());
    assert_eq!(scratch.candidates.capacity(), capacity);
}

#[derive(Clone, Debug, PartialEq)]
struct CollisionResult {
    head_head: Vec<(u64, u64)>,
    head_body: Vec<(u64, u64, usize)>,
    deaths: Vec<(u64, bool, bool, Option<u64>, bool)>,
    awards: Vec<(u64, u64)>,
    diagnostics: CollisionDiagnostics,
}

fn snake(id: u64, position: WorldPoint, direction: f64, length: usize) -> SnakeState {
    SnakeState {
        id,
        frame_v1_id: u32::try_from(id).expect("fixture ID should fit frame v1"),
        kind: SnakeKind::Evolved,
        alive: true,
        population_slot: Some(u32::try_from(id - 1).expect("fixture slot should fit")),
        brain: None,
        baseline_slot: None,
        baseline_strategy: None,
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

#[test]
fn cached_head_bound_preserves_exact_rejections_and_extreme_radius_errors() {
    let radii = [
        f64::from_bits(1),
        f64::MIN_POSITIVE,
        1.0e-150,
        1.0,
        1.0e150,
        f64::MAX / 4.0,
        f64::MAX,
        0.0,
        -1.0,
        f64::INFINITY,
        f64::NAN,
    ];
    let mut first = snake(1, WorldPoint { x: 0.0, y: 0.0 }, 0.0, 5);
    let mut second = snake(
        2,
        WorldPoint {
            x: 1000.0,
            y: 1000.0,
        },
        0.0,
        5,
    );
    for scale in [
        f64::from_bits(1),
        f64::MIN_POSITIVE,
        0.5,
        0.82,
        1.0,
        1.0e150,
    ] {
        for movement_first in radii {
            for final_first in radii {
                first.radius = final_first;
                let left = SweptHeadEnvelope::from_snake(&first, movement_first, scale);
                assert_ne!(
                    left.maximum_radius, 0.0,
                    "alive entries cannot use the dead marker"
                );
                for movement_second in radii {
                    for final_second in radii {
                        second.radius = final_second;
                        let right = SweptHeadEnvelope::from_snake(&second, movement_second, scale);
                        let broad = (left.maximum_radius + right.maximum_radius) * scale
                            + CONTACT_SPACE_TOLERANCE;
                        if swept_head_envelopes_separate(left, right, broad) {
                            // An early miss must preserve both old threshold validation
                            // and the exact old swept-envelope rejection.
                            let movement =
                                combined_threshold(movement_first, movement_second, scale)
                                    .expect("early miss suppressed a movement-threshold error");
                            let final_radius = combined_threshold(final_first, final_second, scale)
                                .expect("early miss suppressed a final-threshold error");
                            assert!(swept_head_envelopes_separate(
                                left,
                                right,
                                movement.max(final_radius) + CONTACT_SPACE_TOLERANCE,
                            ));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(size_of::<SweptHeadEnvelope>(), 6 * size_of::<f64>());
    assert_eq!(SweptHeadEnvelope::default().maximum_radius, 0.0);
}

#[test]
fn swept_head_envelope_keeps_every_exact_contact_candidate() {
    let first = snake(1, WorldPoint { x: 0.0, y: 0.0 }, 0.0, 5);
    for x in (-4..=4).map(|step| f64::from(step) * 10.0) {
        for y in (-4..=4).map(|step| f64::from(step) * 10.0) {
            for direction in [0.0, -15.0, 15.0] {
                let mut second = snake(2, WorldPoint { x, y }, 0.0, 5);
                second.previous_position.x = x + direction;
                let threshold = combined_threshold(first.radius, second.radius, 0.82).unwrap();
                let exact = temporal_point_point_contact_time(
                    first.previous_position,
                    first.position,
                    second.previous_position,
                    second.position,
                    threshold,
                    threshold,
                )
                .unwrap();
                if exact.is_some() {
                    assert!(!swept_head_envelopes_separate(
                        SweptHeadEnvelope::from_snake(&first, first.radius, 0.82),
                        SweptHeadEnvelope::from_snake(&second, second.radius, 0.82),
                        threshold + CONTACT_SPACE_TOLERANCE,
                    ));
                }
            }
        }
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
    let mut snakes = Vec::with_capacity(entries.len());
    let mut body_points = Vec::new();
    for (mut snake, body) in entries {
        snake.body = BodyRange {
            start: body_points.len(),
            len: body.len(),
        };
        snake.position = body[0];
        snake.previous_position = body[0];
        body_points.extend(body);
        snakes.push(snake);
    }
    WorldState {
        ray_cursor: Default::default(),
        snakes,
        body_points,
        pellets: Vec::new(),
        controller_leases: Vec::new(),
    }
}

fn default_index(world: &WorldState) -> IndexedPelletWorld<'_> {
    IndexedPelletWorld::build(world, 120.0, 1_000_000).expect("fixture world should index")
}

fn execute(
    world: &WorldState,
    movement_config: MovementConfig,
    dt: f64,
    collision_config: CollisionConfig,
) -> Result<CollisionResult, CollisionError> {
    let indexed = default_index(world);
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(world, movement_config, dt, 100_000, 100_000)
        .expect("fixture movement should prepare");
    let mut food_workspace = FoodWorkspace::new();
    let food = food_workspace
        .prepare(
            &indexed,
            movement,
            movement_config,
            FoodConfig::typescript_defaults(),
            100_000,
            100_000,
        )
        .expect("fixture food should prepare");
    let mut collision_workspace = CollisionWorkspace::new();
    let prepared = collision_workspace.prepare(food, collision_config)?;
    Ok(CollisionResult {
        head_head: prepared
            .head_head_contacts()
            .iter()
            .map(|contact| (contact.first_id, contact.second_id))
            .collect(),
        head_body: prepared
            .head_body_contacts()
            .iter()
            .map(|contact| (contact.victim_id, contact.owner_id, contact.segment_end))
            .collect(),
        deaths: prepared
            .deaths()
            .iter()
            .map(|death| {
                (
                    death.victim_id,
                    death.wall,
                    death.head_to_head,
                    death.body_owner_id,
                    death.drop_corpse_pellets,
                )
            })
            .collect(),
        awards: prepared
            .awards()
            .iter()
            .map(|award| (award.killer_id, award.victim_id))
            .collect(),
        diagnostics: prepared.diagnostics(),
    })
}

#[test]
fn point_point_sweep_detects_crossing_without_final_overlap() {
    let contact = swept_point_point_contact_time(
        WorldPoint { x: -10.0, y: 0.0 },
        WorldPoint { x: 10.0, y: 0.0 },
        WorldPoint { x: 10.0, y: 0.0 },
        WorldPoint { x: -10.0, y: 0.0 },
        1.0,
    )
    .expect("finite sweep")
    .expect("crossing should collide");
    assert!((contact.0 - 0.475).abs() <= 1.0e-12);
    assert!(!contact.1);
}

#[test]
fn temporal_head_radii_preserve_shrink_contacts_and_delay_growth_to_final() {
    let shrink = temporal_point_point_contact_time(
        ORIGIN,
        ORIGIN,
        WorldPoint { x: 3.0, y: 0.0 },
        WorldPoint { x: 3.0, y: 0.0 },
        4.0,
        2.0,
    )
    .expect("finite shrinking radii")
    .expect("pre-food radii should collide");
    assert_eq!(shrink.0, 0.0);

    let growth = temporal_point_point_contact_time(
        ORIGIN,
        ORIGIN,
        WorldPoint { x: 10.0, y: 0.0 },
        WorldPoint { x: 3.0, y: 0.0 },
        2.0,
        4.0,
    )
    .expect("finite growing radii")
    .expect("final larger radii should collide");
    assert_eq!(growth.0, 1.0);
}

#[test]
fn point_segment_sweep_detects_crossing_and_rejects_clear_near_miss() {
    let crossing = swept_point_segment_contact(
        WorldPoint { x: -10.0, y: 0.0 },
        WorldPoint { x: 10.0, y: 0.0 },
        WorldPoint { x: 0.0, y: -5.0 },
        WorldPoint { x: 0.0, y: -5.0 },
        WorldPoint { x: 0.0, y: 5.0 },
        WorldPoint { x: 0.0, y: 5.0 },
        1.0,
    )
    .expect("finite crossing");
    assert!(crossing.time.is_some());

    let moving_segment = swept_point_segment_contact(
        WorldPoint { x: 0.0, y: 0.0 },
        WorldPoint { x: 0.0, y: 0.0 },
        WorldPoint { x: -10.0, y: -5.0 },
        WorldPoint { x: 10.0, y: -5.0 },
        WorldPoint { x: -10.0, y: 5.0 },
        WorldPoint { x: 10.0, y: 5.0 },
        1.0,
    )
    .expect("finite moving-segment crossing");
    assert!(moving_segment.time.is_some());

    let clear = swept_point_segment_contact(
        WorldPoint { x: -10.0, y: 2.0 },
        WorldPoint { x: 10.0, y: 2.0 },
        WorldPoint { x: 0.0, y: -5.0 },
        WorldPoint { x: 0.0, y: -5.0 },
        WorldPoint { x: 0.0, y: 0.0 },
        WorldPoint { x: 0.0, y: 0.0 },
        1.0,
    )
    .expect("finite near miss");
    assert_eq!(clear.time, None);

    let collinear_far = origin_convex_hull_distance([
        WorldPoint { x: 100.0, y: 0.0 },
        WorldPoint { x: 110.0, y: 0.0 },
        WorldPoint { x: 120.0, y: 0.0 },
        WorldPoint { x: 130.0, y: 0.0 },
    ]);
    assert_eq!(collinear_far, 100.0);
}

#[test]
fn newly_grown_segment_participates_only_at_the_final_boundary() {
    let mut victim = snake(1, WorldPoint { x: 10.0, y: 0.0 }, 0.0, 5);
    victim.previous_position = WorldPoint { x: -10.0, y: 0.0 };
    let mut segment = SweptSegment {
        owner_id: 2,
        segment_end: 5,
        previous_start: WorldPoint { x: 0.0, y: -5.0 },
        previous_end: WorldPoint { x: 0.0, y: 5.0 },
        current_start: WorldPoint { x: 0.0, y: -5.0 },
        current_end: WorldPoint { x: 0.0, y: 5.0 },
        movement_radius: 1.0,
        final_radius: 1.0,
        newly_grown: true,
        removed_at_final: false,
    };
    let absent_during_sweep =
        head_segment_contact(&victim, segment, 1.0, 1.0).expect("finite new segment");
    assert_eq!(absent_during_sweep.time, None);

    segment.newly_grown = false;
    let existing_during_sweep =
        head_segment_contact(&victim, segment, 1.0, 1.0).expect("finite existing segment");
    assert!(existing_during_sweep.time.is_some());

    victim.position = WorldPoint { x: 0.0, y: 0.0 };
    segment.newly_grown = true;
    let present_at_final =
        head_segment_contact(&victim, segment, 1.0, 1.0).expect("finite final contact");
    assert_eq!(present_at_final.time, Some(1.0));
}

#[test]
fn temporal_body_radii_preserve_shrink_contacts_and_delay_growth_to_final() {
    let mut victim = snake(1, WorldPoint { x: 3.0, y: 0.0 }, 0.0, 5);
    victim.previous_position = victim.position;
    victim.radius = 1.0;
    let segment = SweptSegment {
        owner_id: 2,
        segment_end: 1,
        previous_start: WorldPoint { x: 0.0, y: -1.0 },
        previous_end: WorldPoint { x: 0.0, y: 1.0 },
        current_start: WorldPoint { x: 0.0, y: -1.0 },
        current_end: WorldPoint { x: 0.0, y: 1.0 },
        movement_radius: 3.0,
        final_radius: 1.0,
        newly_grown: false,
        removed_at_final: false,
    };
    let shrink =
        head_segment_contact(&victim, segment, 4.0, 2.0).expect("finite shrinking body radii");
    assert_eq!(shrink.time, Some(0.0));

    victim.previous_position = WorldPoint { x: 10.0, y: 0.0 };
    let growth =
        head_segment_contact(&victim, segment, 2.0, 4.0).expect("finite growing body radii");
    assert_eq!(growth.time, Some(1.0));
}

#[test]
fn swept_solver_never_misses_a_dense_deterministic_reference_sample() {
    fn next(state: &mut u64) -> f64 {
        *state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*state >> 11) as f64 / ((1_u64 << 53) as f64)) * 40.0 - 20.0
    }

    let mut state = 0x5eed_c011_1510_u64;
    for case in 0..256 {
        let head_start = WorldPoint {
            x: next(&mut state),
            y: next(&mut state),
        };
        let head_end = WorldPoint {
            x: next(&mut state),
            y: next(&mut state),
        };
        let first_start = WorldPoint {
            x: next(&mut state),
            y: next(&mut state),
        };
        let first_end = WorldPoint {
            x: next(&mut state),
            y: next(&mut state),
        };
        let second_start = WorldPoint {
            x: next(&mut state),
            y: next(&mut state),
        };
        let second_end = WorldPoint {
            x: next(&mut state),
            y: next(&mut state),
        };
        let threshold = next(&mut state).abs() * 0.2 + 0.05;
        let result = swept_point_segment_contact(
            head_start,
            head_end,
            first_start,
            first_end,
            second_start,
            second_end,
            threshold,
        )
        .expect("generated coordinates are finite");
        let sampled_contact = (0..=2_000).any(|sample| {
            let time = sample as f64 / 2_000.0;
            point_segment_distance(
                lerp_point(head_start, head_end, time),
                lerp_point(first_start, first_end, time),
                lerp_point(second_start, second_end, time),
            ) <= threshold
        });
        assert!(
            !sampled_contact || result.time.is_some(),
            "case {case} missed a sampled contact"
        );
    }
}

#[test]
fn contact_search_work_exhaustion_rejects_instead_of_inventing_a_death() {
    let result = swept_point_segment_contact(
        ORIGIN,
        ORIGIN,
        WorldPoint {
            x: 180_072_921.461,
            y: 29_040_963.112,
        },
        WorldPoint {
            x: 528_149_625.679,
            y: -284_165_289.248,
        },
        WorldPoint {
            x: -849_316_543.656,
            y: -83_524_087.658,
        },
        WorldPoint {
            x: 728_775_745.439,
            y: 827_863_633.599,
        },
        1.0,
    );

    assert!(matches!(
        result,
        Err(CollisionError::ContactSearchIntervalLimitExceeded {
            required,
            maximum: MAXIMUM_CONTACT_SEARCH_INTERVALS,
        }) if required == MAXIMUM_CONTACT_SEARCH_INTERVALS + 1
    ));
}

#[test]
fn conservative_contact_respects_the_documented_spatial_gap() {
    let gap = CONTACT_HULL_TOLERANCE * 0.5;
    let result = swept_point_segment_contact(
        ORIGIN,
        ORIGIN,
        WorldPoint {
            x: 1.0 + gap,
            y: -1.0,
        },
        WorldPoint {
            x: 1.0 + gap,
            y: -1.0,
        },
        WorldPoint {
            x: 1.0 + gap,
            y: 1.0,
        },
        WorldPoint {
            x: 1.0 + gap,
            y: 1.0,
        },
        1.0,
    )
    .expect("bounded near-contact should resolve");

    assert_eq!(result.time, Some(0.0));
    assert!(result.conservative);
    assert!(
        point_segment_distance(
            ORIGIN,
            WorldPoint {
                x: 1.0 + gap,
                y: -1.0,
            },
            WorldPoint {
                x: 1.0 + gap,
                y: 1.0,
            },
        ) - 1.0
            <= CONTACT_SPACE_TOLERANCE
    );
}

#[test]
fn swept_bounds_count_every_crossed_cell() {
    let segment = SweptSegment {
        owner_id: 1,
        segment_end: 1,
        previous_start: WorldPoint { x: -25.0, y: -5.0 },
        previous_end: WorldPoint { x: -15.0, y: 5.0 },
        current_start: WorldPoint { x: 15.0, y: -5.0 },
        current_end: WorldPoint { x: 25.0, y: 5.0 },
        movement_radius: 1.0,
        final_radius: 1.0,
        newly_grown: false,
        removed_at_final: false,
    };
    let (minimum, maximum) =
        swept_segment_cell_bounds(segment, 1.0, 10.0).expect("bounds should fit");
    assert_eq!(minimum, CellKey { x: -3, y: -1 });
    assert_eq!(maximum, CellKey { x: 2, y: 0 });
    assert_eq!(cell_rectangle_count(minimum, maximum), Ok(12));
}

#[test]
fn immutable_head_head_snapshot_kills_both_without_neck_credit() {
    let position = WorldPoint { x: 0.0, y: 0.0 };
    let first = snake(1, position, 0.0, 5);
    let second = snake(2, position, std::f64::consts::PI, 5);
    let world = pack_world(vec![
        (first, line_body(position, 0.0, 5)),
        (second, line_body(position, std::f64::consts::PI, 5)),
    ]);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("head overlap should resolve");

    assert_eq!(result.head_head, vec![(1, 2)]);
    assert_eq!(
        result.deaths,
        vec![(1, false, true, None, true), (2, false, true, None, true)]
    );
    assert!(result.head_body.is_empty());
    assert!(result.awards.is_empty());
}

fn unambiguous_body_world(reverse: bool) -> WorldState {
    let victim_position = WorldPoint { x: 0.0, y: 0.0 };
    let owner_position = WorldPoint { x: 100.0, y: 0.0 };
    let victim = (
        snake(1, victim_position, 0.0, 5),
        line_body(victim_position, 0.0, 5),
    );
    let owner = (
        snake(2, owner_position, 0.0, 15),
        line_body(owner_position, 0.0, 15),
    );
    if reverse {
        pack_world(vec![owner, victim])
    } else {
        pack_world(vec![victim, owner])
    }
}

#[test]
fn unambiguous_body_owner_credit_is_array_order_invariant() {
    let forward = execute(
        &unambiguous_body_world(false),
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("forward body collision should resolve");
    let reversed = execute(
        &unambiguous_body_world(true),
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("reversed body collision should resolve");

    assert_eq!(forward.head_head, reversed.head_head);
    assert_eq!(forward.head_body, reversed.head_body);
    assert_eq!(forward.deaths, reversed.deaths);
    assert_eq!(forward.awards, reversed.awards);
    assert_eq!(forward.awards, vec![(2, 1)]);
    assert_eq!(forward.deaths, vec![(1, false, false, Some(2), true)]);
}

#[test]
fn exact_multi_body_contact_uses_lower_stable_owner_id() {
    let victim_position = WorldPoint { x: 0.0, y: 0.0 };
    let upper = WorldPoint { x: 0.0, y: 100.0 };
    let lower = WorldPoint { x: 0.0, y: -100.0 };
    let world = pack_world(vec![
        (
            snake(3, lower, -std::f64::consts::FRAC_PI_2, 15),
            line_body(lower, -std::f64::consts::FRAC_PI_2, 15),
        ),
        (
            snake(1, victim_position, 0.0, 5),
            line_body(victim_position, 0.0, 5),
        ),
        (
            snake(2, upper, std::f64::consts::FRAC_PI_2, 15),
            line_body(upper, std::f64::consts::FRAC_PI_2, 15),
        ),
    ]);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("multi-body collision should resolve");

    assert_eq!(result.awards, vec![(2, 1)]);
    assert_eq!(result.head_body[0].0, 1);
    assert_eq!(result.head_body[0].1, 2);
}

#[test]
fn same_owner_exact_time_uses_lower_stable_segment_offset() {
    let victim_position = WorldPoint { x: 0.0, y: 0.0 };
    let owner_position = WorldPoint { x: 0.0, y: 50.0 };
    let owner_body = (0..10)
        .map(|index| WorldPoint {
            x: 0.0,
            y: 50.0 - index as f64 * 10.0,
        })
        .collect();
    let mut owner = snake(2, owner_position, std::f64::consts::FRAC_PI_2, 10);
    owner.radius = 0.1;
    let mut victim = snake(1, victim_position, 0.0, 5);
    victim.radius = 0.1;
    let world = pack_world(vec![
        (owner, owner_body),
        (victim, line_body(victim_position, 0.0, 5)),
    ]);
    let movement = MovementConfig {
        snake_radius: 0.1,
        snake_radius_max: 0.1,
        snake_thickness_scale: 0.0,
        snake_spacing: 10.0,
        world_radius: 10_000.0,
        ..MovementConfig::typescript_defaults()
    };
    let result = execute(
        &world,
        movement,
        1.0e-6,
        CollisionConfig {
            hit_scale: 1.0,
            ..CollisionConfig::typescript_defaults()
        },
    )
    .expect("same-owner segment tie should resolve");

    assert_eq!(result.head_body, vec![(1, 2, 5)]);
    assert_eq!(result.awards, vec![(2, 1)]);
}

#[test]
fn body_of_simultaneously_killed_owner_remains_an_obstacle() {
    let victim_position = WorldPoint { x: 0.0, y: 0.0 };
    let owner_position = WorldPoint { x: 100.0, y: 0.0 };
    let world = pack_world(vec![
        (
            snake(1, victim_position, 0.0, 5),
            line_body(victim_position, 0.0, 5),
        ),
        (
            snake(2, owner_position, 0.0, 15),
            line_body(owner_position, 0.0, 15),
        ),
        (
            snake(3, owner_position, std::f64::consts::PI, 5),
            line_body(owner_position, std::f64::consts::PI, 5),
        ),
    ]);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("immutable collision snapshot should resolve");

    assert!(result.head_head.contains(&(2, 3)));
    assert!(result.deaths.iter().any(|death| death.0 == 2));
    assert!(result.deaths.iter().any(|death| death.0 == 3));
    assert_eq!(result.awards, vec![(2, 1)]);
}

#[test]
fn wall_killed_body_owner_remains_an_obstacle_for_the_same_snapshot() {
    let victim_position = WorldPoint { x: 3_415.2, y: 0.0 };
    let owner_position = WorldPoint { x: 3_490.2, y: 0.0 };
    let world = pack_world(vec![
        (
            snake(1, victim_position, std::f64::consts::FRAC_PI_2, 5),
            line_body(victim_position, std::f64::consts::FRAC_PI_2, 5),
        ),
        (
            snake(2, owner_position, 0.0, 15),
            line_body(owner_position, 0.0, 15),
        ),
    ]);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("wall and body outcomes should share one snapshot");

    assert_eq!(result.awards, vec![(2, 1)]);
    assert_eq!(
        result.deaths,
        vec![
            (1, false, false, Some(2), true),
            (2, true, false, None, false),
        ]
    );
}

#[test]
fn simultaneous_wall_and_head_contact_kills_both_without_credit() {
    let position = WorldPoint { x: 3_490.2, y: 0.0 };
    let world = pack_world(vec![
        (snake(1, position, 0.0, 5), line_body(position, 0.0, 5)),
        (
            snake(2, position, std::f64::consts::PI, 5),
            line_body(position, std::f64::consts::PI, 5),
        ),
    ]);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("wall and head contact should resolve together");

    assert_eq!(result.head_head, vec![(1, 2)]);
    assert_eq!(
        result.deaths,
        vec![(1, true, true, None, false), (2, false, true, None, true)]
    );
    assert!(result.awards.is_empty());
}

#[test]
fn ordinary_shrink_tail_sweeps_before_disappearing_at_final_boundary() {
    let victim_position = WorldPoint { x: 0.0, y: 0.0 };
    let owner_position = WorldPoint { x: 100.0, y: 0.0 };
    let victim = snake(1, victim_position, std::f64::consts::FRAC_PI_2, 5);
    let mut owner = snake(2, owner_position, 0.0, 15);
    owner.target_length = 5.0;
    let world = pack_world(vec![
        (
            victim,
            line_body(victim_position, std::f64::consts::FRAC_PI_2, 5),
        ),
        (owner, line_body(owner_position, 0.0, 15)),
    ]);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("ordinary-shrink sweep should resolve");

    assert_eq!(result.awards, vec![(2, 1)]);
    assert!(result.head_body[0].2 >= 5);
}

#[test]
fn swept_head_crosses_body_even_when_both_endpoints_are_clear() {
    let victim_position = WorldPoint { x: -30.0, y: 0.0 };
    let owner_position = WorldPoint { x: 0.0, y: 50.0 };
    let mut victim = snake(1, victim_position, 0.0, 5);
    victim.speed = 10_000.0;
    victim.radius = 1.0;
    let mut owner = snake(2, owner_position, std::f64::consts::FRAC_PI_2, 9);
    owner.radius = 1.0;
    let world = pack_world(vec![
        (victim, line_body(victim_position, 0.0, 5)),
        (
            owner,
            line_body(owner_position, std::f64::consts::FRAC_PI_2, 9),
        ),
    ]);
    let movement = MovementConfig {
        snake_radius: 1.0,
        snake_radius_max: 1.0,
        snake_thickness_scale: 0.0,
        world_radius: 10_000.0,
        ..MovementConfig::typescript_defaults()
    };
    let result = execute(
        &world,
        movement,
        0.006,
        CollisionConfig {
            hit_scale: 1.0,
            ..CollisionConfig::typescript_defaults()
        },
    )
    .expect("swept crossing should resolve");

    assert_eq!(result.awards, vec![(2, 1)]);
    assert_eq!(result.deaths, vec![(1, false, false, Some(2), true)]);
}

#[test]
fn earlier_world_contact_beats_a_later_lower_owner_id() {
    let victim_position = WorldPoint { x: -30.0, y: 0.0 };
    let mut victim = snake(1, victim_position, 0.0, 5);
    victim.speed = 10_000.0;
    victim.radius = 1.0;
    let mut later_low_id = snake(
        2,
        WorldPoint { x: 10.0, y: 50.0 },
        std::f64::consts::FRAC_PI_2,
        9,
    );
    later_low_id.radius = 1.0;
    let mut earlier_high_id = snake(
        3,
        WorldPoint { x: -10.0, y: 50.0 },
        std::f64::consts::FRAC_PI_2,
        9,
    );
    earlier_high_id.radius = 1.0;
    let world = pack_world(vec![
        (
            later_low_id,
            line_body(
                WorldPoint { x: 10.0, y: 50.0 },
                std::f64::consts::FRAC_PI_2,
                9,
            ),
        ),
        (victim, line_body(victim_position, 0.0, 5)),
        (
            earlier_high_id,
            line_body(
                WorldPoint { x: -10.0, y: 50.0 },
                std::f64::consts::FRAC_PI_2,
                9,
            ),
        ),
    ]);
    let movement = MovementConfig {
        snake_radius: 1.0,
        snake_radius_max: 1.0,
        snake_thickness_scale: 0.0,
        world_radius: 10_000.0,
        ..MovementConfig::typescript_defaults()
    };
    let result = execute(
        &world,
        movement,
        0.006,
        CollisionConfig {
            hit_scale: 1.0,
            ..CollisionConfig::typescript_defaults()
        },
    )
    .expect("two swept owner contacts should resolve");

    assert_eq!(result.awards, vec![(3, 1)]);
    assert_eq!(result.head_body[0].1, 3);
}

#[test]
fn configured_head_segment_skip_is_honored() {
    let world = unambiguous_body_world(false);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig {
            skip_segments: 15,
            ..CollisionConfig::typescript_defaults()
        },
    )
    .expect("skipped collision index should resolve");

    assert!(result.deaths.is_empty());
    assert!(result.awards.is_empty());
}

#[test]
fn self_body_is_never_a_collision_target() {
    let position = WorldPoint { x: 0.0, y: 0.0 };
    let world = pack_world(vec![(
        snake(1, position, 0.0, 4),
        vec![
            position,
            WorldPoint { x: -7.5, y: 0.0 },
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: 7.5, y: 0.0 },
        ],
    )]);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("self-overlap should not fault");

    assert!(result.deaths.is_empty());
    assert!(result.awards.is_empty());
}

#[test]
fn wall_death_is_carried_without_corpse_drop_or_award() {
    let position = WorldPoint { x: 3_490.2, y: 0.0 };
    let world = pack_world(vec![(
        snake(1, position, 0.0, 5),
        line_body(position, 0.0, 5),
    )]);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig::typescript_defaults(),
    )
    .expect("wall death should resolve");

    assert_eq!(result.deaths, vec![(1, true, false, None, false)]);
    assert!(result.awards.is_empty());
}

#[test]
fn index_capacity_rejects_complete_snapshot_without_authority_write() {
    let world = unambiguous_body_world(false);
    let authority_before = world.clone();
    let indexed = default_index(&world);
    let movement_config = MovementConfig::typescript_defaults();
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, movement_config, DT, 100_000, 100_000)
        .expect("movement should prepare");
    let mut food_workspace = FoodWorkspace::new();
    let food = food_workspace
        .prepare(
            &indexed,
            movement,
            movement_config,
            FoodConfig::typescript_defaults(),
            100_000,
            100_000,
        )
        .expect("food should prepare");
    let mut collision_workspace = CollisionWorkspace::new();
    let result = collision_workspace.prepare(
        food,
        CollisionConfig {
            cell_size: 1.0,
            maximum_index_entries: 1,
            ..CollisionConfig::typescript_defaults()
        },
    );

    assert!(matches!(
        result,
        Err(CollisionError::IndexEntryLimitExceeded { maximum: 1, .. })
    ));
    assert!(!collision_workspace.is_ready());
    assert_eq!(world, authority_before);
}

#[test]
fn oversized_head_query_rejects_the_snapshot_instead_of_truncating_truth() {
    let world = unambiguous_body_world(false);
    let result = execute(
        &world,
        MovementConfig::typescript_defaults(),
        DT,
        CollisionConfig {
            cell_size: 1.0,
            maximum_index_entries: 1_000_000,
            maximum_query_cells: 1,
            ..CollisionConfig::typescript_defaults()
        },
    );

    assert!(matches!(
        result,
        Err(CollisionError::QueryCellLimitExceeded { maximum: 1, .. })
    ));
}

#[test]
fn warmed_detection_reuses_every_owned_capacity() {
    let world = unambiguous_body_world(false);
    let indexed = default_index(&world);
    let movement_config = MovementConfig::typescript_defaults();
    let mut movement_workspace = MovementWorkspace::new();
    let movement = movement_workspace
        .prepare(&world, movement_config, DT, 100_000, 100_000)
        .expect("movement should prepare");
    let mut food_workspace = FoodWorkspace::new();
    let food = food_workspace
        .prepare(
            &indexed,
            movement,
            movement_config,
            FoodConfig::typescript_defaults(),
            100_000,
            100_000,
        )
        .expect("food should prepare");
    let mut workspace = CollisionWorkspace::new();
    let first = workspace
        .prepare(food, CollisionConfig::typescript_defaults())
        .expect("warm collision should prepare")
        .diagnostics();

    for _ in 0..24 {
        let next = workspace
            .prepare(food, CollisionConfig::typescript_defaults())
            .expect("reused collision should prepare")
            .diagnostics();
        assert_eq!(next.segment_capacity, first.segment_capacity);
        assert_eq!(next.order_capacity, first.order_capacity);
        assert_eq!(next.head_envelope_capacity, first.head_envelope_capacity);
        assert_eq!(next.entry_capacity, first.entry_capacity);
        assert_eq!(next.cell_capacity, first.cell_capacity);
        assert_eq!(next.candidate_capacity, first.candidate_capacity);
        assert_eq!(
            next.seen_generation_capacity,
            first.seen_generation_capacity
        );
        assert_eq!(next.head_head_capacity, first.head_head_capacity);
        assert_eq!(next.head_body_capacity, first.head_body_capacity);
        assert_eq!(next.death_capacity, first.death_capacity);
        assert_eq!(next.death_flag_capacity, first.death_flag_capacity);
        assert_eq!(next.award_capacity, first.award_capacity);
    }
}

#[test]
fn earlier_contact_precedes_owner_id_and_exact_time_uses_stable_owner() {
    let later_low_id = HeadBodyContact {
        victim_id: 1,
        owner_id: 2,
        segment_end: 1,
        time: 0.75,
        conservative: false,
    };
    let earlier_high_id = HeadBodyContact {
        victim_id: 1,
        owner_id: 3,
        segment_end: 1,
        time: 0.25,
        conservative: false,
    };
    assert!(body_contact_precedes(earlier_high_id, later_low_id));

    let tied_low_id = HeadBodyContact {
        time: 0.25,
        ..later_low_id
    };
    assert!(body_contact_precedes(tied_low_id, earlier_high_id));
}

#[test]
fn diagnostic_struct_remains_plain_bounded_metadata() {
    assert!(size_of::<CollisionDiagnostics>() <= 24 * size_of::<usize>());
}
