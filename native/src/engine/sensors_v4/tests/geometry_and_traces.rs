use super::*;

#[test]
fn sector_bin_matches_original_v4_formula_for_dense_boundary_and_extreme_vectors() {
    let mut interior = 0;
    let mut ordinary = 0;
    for bins in 8..=MAX_SENSOR_BINS {
        for direction in [
            -1.0e300, -1.0e20, -TAU, -PI, -2.1, -0.37, -0.0, 0.0, 0.37, 2.1, PI, TAU, 1.0e20,
            1.0e300,
        ] {
            let classifier = CenteredAngularBins::new(direction, bins);
            for scale in [
                1.0e-320,
                f64::MIN_POSITIVE / 2.0,
                f64::MIN_POSITIVE,
                1.0e-300,
                1.0e-12,
                1.0,
                1.0e12,
                1.0e300,
            ] {
                for sample in 0..2000_u64 {
                    let angle = (sample.wrapping_mul(65_537) % 2000) as f64 / 2000.0 * TAU;
                    let (y, x) = angle.sin_cos();
                    let (x, y) = (x * scale, y * scale);
                    let distance = x.hypot(y);
                    let expected = angle_to_centered_bin(wrap(y.atan2(x) - direction), bins);
                    assert_eq!(
                        sector_bin(Some(&classifier), x, y, distance, direction, bins),
                        expected,
                        "B={bins} heading={direction} scale={scale} angle={angle}"
                    );
                    // Body proximity supplies sqrt(dx*dx+dy*dy), which can underflow.
                    let squared_distance = (x * x + y * y).sqrt();
                    assert_eq!(
                        sector_bin(Some(&classifier), x, y, squared_distance, direction, bins),
                        expected,
                        "squared norm: B={bins} heading={direction} scale={scale} angle={angle}"
                    );
                    if scale == 1.0 && direction.abs() <= TAU {
                        ordinary += 1;
                        interior +=
                            usize::from(classifier.classify_interior(x, y, distance).is_some());
                    }
                    if sample % 101 == 0 {
                        assert_eq!(sector_bin(None, x, y, distance, direction, bins), expected);
                    }
                }
                for boundary in 0..bins {
                    let angle = direction - PI + (boundary as f64 + 0.5) * TAU / bins as f64;
                    for delta in [
                        -1.0e-9,
                        -1.0e-10,
                        -1.0e-12,
                        -f64::EPSILON,
                        0.0,
                        f64::EPSILON,
                        1.0e-12,
                        1.0e-10,
                        1.0e-9,
                    ] {
                        let (y, x) = (angle + delta).sin_cos();
                        let (x, y) = (x * scale, y * scale);
                        let expected = angle_to_centered_bin(wrap(y.atan2(x) - direction), bins);
                        assert_eq!(sector_bin(Some(&classifier), x,y,x.hypot(y),direction,bins),
                                   expected, "B={bins} heading={direction} scale={scale} boundary={boundary} delta={delta}");
                    }
                }
            }
            for (x, y) in [
                (0.0, 0.0),
                (-0.0, 0.0),
                (0.0, -0.0),
                (-0.0, -0.0),
                (f64::INFINITY, 1.0),
                (1.0, f64::INFINITY),
                (f64::NAN, 1.0),
            ] {
                let expected = angle_to_centered_bin(wrap(y.atan2(x) - direction), bins);
                assert_eq!(
                    sector_bin(Some(&classifier), x, y, x.hypot(y), direction, bins),
                    expected
                );
            }
        }
    }
    // Ensure dense ordinary inputs exercise the optimization rather than always falling back.
    assert!(interior > ordinary * 9 / 10);
}

#[test]
fn food_channels_and_raw_traces_match_reference_accumulation_for_sparse_and_dense_queries() {
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for bins in 8..=MAX_SENSOR_BINS {
            let evaluator = evaluator(profile, bins);
            for direction in [-TAU, -PI, -0.37, 0.0, PI, TAU, 1.0e20] {
                let mut source = WorldState::default();
                add_snake(
                    &mut source,
                    1,
                    SnakeKind::External,
                    point(0.0, 0.0),
                    direction,
                    5,
                );
                let mut positions = Vec::new();
                for index in 0..256 {
                    let angle = index as f64 * 2.399963229728653;
                    let distance = [
                        0.0, 50.0, 519.999999, 520.0, 520.000001, 1199.0, 1200.0, 1200.1,
                    ][index % 8];
                    positions.push(point(angle.cos() * distance, angle.sin() * distance));
                }
                for boundary in 0..bins {
                    let angle = direction - PI + (boundary as f64 + 0.5) * TAU / bins as f64;
                    for delta in [-1.0e-9, -1.0e-12, 0.0, 1.0e-12, 1.0e-9] {
                        for distance in [520.0, 521.0] {
                            positions.push(point(
                                (angle + delta).cos() * distance,
                                (angle + delta).sin() * distance,
                            ));
                        }
                    }
                }
                positions.extend([
                    point(0.0, 0.0),
                    point(-0.0, 0.0),
                    point(0.0, -0.0),
                    point(-0.0, -0.0),
                ]);
                for (index, position) in positions.into_iter().enumerate() {
                    source.pellets.push(PelletState {
                        id: index as u64 + 1000,
                        position,
                        value: [0.1, 1.0, 6.0, 8.0][index % 4],
                        kind: 0,
                        color: 0,
                        owner: None,
                    });
                }
                for sparse in [true, false] {
                    let mut world = source.clone();
                    if sparse {
                        world.pellets.truncate(bins - 1);
                    }
                    let before = world.clone();
                    let indexed = IndexedSensorWorld::build(
                        &world,
                        SensorIndexConfig {
                            body_cell_size: 70.0,
                            pellet_cell_size: 120.0,
                            maximum_body_entries: 1_000_000,
                            maximum_pellet_entries: 100_000,
                        },
                    )
                    .unwrap();
                    let mut rays = BodyRayIndexWorkspace::default();
                    let view = rays.prepare(indexed.body_index()).unwrap();
                    let lives = EvolvedGenerationState::default();
                    let context = V4ObservationContext {
                        generation_elapsed_seconds: 120.0,
                        evolved: &lives,
                        experiment: EvolvedExperimentConfig::legacy(240.0),
                    };
                    let mut output = vec![0.0; evaluator.layout().input_size()];
                    let mut scratch = V4SensorScratch::default();
                    let mut trace = crate::engine::sensor_trace::SensorTrace::default();
                    let result = evaluator
                        .sample_with_trace(
                            &indexed,
                            view,
                            context,
                            0,
                            &mut output,
                            &mut RayWorkMeter::new(4096),
                            &mut scratch,
                            Some(&mut trace),
                            None,
                        )
                        .unwrap();
                    let mut near = [0.0; MAX_SENSOR_BINS];
                    let mut far = [0.0; MAX_SENSOR_BINS];
                    let cfg = evaluator.common.config();
                    for pellet in indexed.pellet_index().candidates(&scratch.pellets) {
                        let (dx, dy) = (pellet.position.x, pellet.position.y);
                        let distance = dx.hypot(dy);
                        // Independent old v4 bin formula and original ordered accumulation.
                        let bin = angle_to_centered_bin(wrap(dy.atan2(dx) - direction), bins);
                        let strength = (1.0 - distance / result.radii.far).clamp(0.0, 1.0)
                            * (pellet.value / cfg.food_value).clamp(0.0, 6.0);
                        if distance <= result.radii.near {
                            near[bin] += strength;
                        } else {
                            far[bin] += strength;
                        }
                    }
                    for bin in 0..bins {
                        assert_eq!(trace.food_strength[bin].to_bits(), near[bin].to_bits());
                        assert_eq!(trace.far_food_strength[bin].to_bits(), far[bin].to_bits());
                        assert_eq!(
                            output[24 + bin].to_bits(),
                            saturate(near[bin], cfg.food_saturation).to_bits()
                        );
                        assert_eq!(
                            output[24 + bins + bin].to_bits(),
                            saturate(far[bin], cfg.food_saturation).to_bits()
                        );
                    }
                    assert_eq!(world, before);
                }
            }
        }
    }
}

#[test]
fn body_head_and_projected_sectors_match_original_scan_across_profiles_and_bins() {
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for bins in 8..=MAX_SENSOR_BINS {
            let evaluator = evaluator(profile, bins);
            for direction in [-TAU, -PI, -0.37, 0.0, PI, TAU, 1.0e20] {
                let mut world = WorldState::default();
                add_snake(
                    &mut world,
                    1,
                    SnakeKind::External,
                    point(0.0, 0.0),
                    direction,
                    5,
                );
                for index in 0..bins * 2 {
                    let angle = direction - PI
                        + (index as f64 + 0.5) * TAU / bins as f64
                        + [-1.0e-12, 0.0, 1.0e-12][index % 3];
                    let distance = [0.0, 20.0, 100.0, 519.0, 530.0, 600.0, 1200.0][index % 7];
                    let heading = angle + [0.0, PI, PI / 2.0][index % 3];
                    add_snake(
                        &mut world,
                        900 - index as u64 * 13,
                        SnakeKind::External,
                        point(angle.cos() * distance, angle.sin() * distance),
                        heading,
                        10 + index % 3,
                    );
                    let other = world.snakes.last_mut().unwrap();
                    other.speed = [0.0, 165.0, 247.5, 1.0e5][index % 4];
                    other.alive = index % 11 != 0;
                }
                let before = world.clone();
                let indexed = IndexedSensorWorld::build(
                    &world,
                    SensorIndexConfig {
                        body_cell_size: 70.0,
                        pellet_cell_size: 120.0,
                        maximum_body_entries: 1_000_000,
                        maximum_pellet_entries: 100_000,
                    },
                )
                .unwrap();
                let mut rays = BodyRayIndexWorkspace::default();
                let view = rays.prepare(indexed.body_index()).unwrap();
                let lives = EvolvedGenerationState::default();
                let context = V4ObservationContext {
                    generation_elapsed_seconds: 120.0,
                    evolved: &lives,
                    experiment: EvolvedExperimentConfig::legacy(240.0),
                };
                let mut output = vec![0.0; evaluator.layout().input_size()];
                let mut scratch = V4SensorScratch::default();
                let mut trace = crate::engine::sensor_trace::SensorTrace::default();
                let result = evaluator
                    .sample_with_trace(
                        &indexed,
                        view,
                        context,
                        0,
                        &mut output,
                        &mut RayWorkMeter::new(4096),
                        &mut scratch,
                        Some(&mut trace),
                        None,
                    )
                    .unwrap();
                let cfg = evaluator.common.config();
                let snake = &world.snakes[0];
                let mut body = [result.radii.near; MAX_SENSOR_BINS];
                if result.diagnostics.body.candidate_limit_reached {
                    body.fill(0.0);
                } else {
                    for segment in indexed.body_index().sensor_candidates(&scratch.body) {
                        let closest =
                            closest_point_on_segment(snake.position, segment.start, segment.end);
                        let clearance = (closest.distance_squared.sqrt()
                            - (snake.radius + segment.owner_radius) * cfg.collision_hit_scale)
                            .max(0.0);
                        if clearance <= result.radii.near {
                            let bin = angle_to_centered_bin(
                                wrap(
                                    (closest.point.y - snake.position.y)
                                        .atan2(closest.point.x - snake.position.x)
                                        - direction,
                                ),
                                bins,
                            );
                            body[bin] = body[bin].min(clearance);
                        }
                    }
                }
                let mut counts = [0_usize; MAX_SENSOR_BINS];
                let mut clearance = [result.radii.near; MAX_SENSOR_BINS];
                let mut nearest = [usize::MAX; MAX_SENSOR_BINS];
                let mut urgency = [0.0_f64; MAX_SENSOR_BINS];
                let mut seconds = [None::<f64>; MAX_SENSOR_BINS];
                for (index, other) in world.snakes.iter().enumerate().skip(1) {
                    if !other.alive {
                        continue;
                    }
                    let (dx, dy) = (
                        other.position.x - snake.position.x,
                        other.position.y - snake.position.y,
                    );
                    let radius = (snake.radius + other.radius) * cfg.collision_hit_scale;
                    let gap = (dx.hypot(dy) - radius).max(0.0);
                    if gap <= result.radii.near {
                        let bin = angle_to_centered_bin(wrap(dy.atan2(dx) - direction), bins);
                        counts[bin] += 1;
                        if nearest[bin] == usize::MAX
                            || gap < clearance[bin]
                            || (gap == clearance[bin] && other.id < world.snakes[nearest[bin]].id)
                        {
                            nearest[bin] = index;
                            clearance[bin] = gap;
                        }
                    }
                    if profile == SensorProfile::Motion {
                        let encounter = projected_head_encounter(
                            point(dx, dy),
                            point(
                                other.direction.cos() * other.speed - direction.cos() * snake.speed,
                                other.direction.sin() * other.speed - direction.sin() * snake.speed,
                            ),
                            radius,
                        )
                        .unwrap();
                        if let Some(encounter) = encounter {
                            let bin = angle_to_centered_bin(
                                wrap(
                                    encounter
                                        .relative_at_contact
                                        .y
                                        .atan2(encounter.relative_at_contact.x)
                                        - direction,
                                ),
                                bins,
                            );
                            urgency[bin] =
                                urgency[bin].max(1.0 - encounter.seconds / MOTION_HORIZON_SECONDS);
                            seconds[bin] =
                                Some(seconds[bin].map_or(encounter.seconds, |previous| {
                                    previous.min(encounter.seconds)
                                }));
                        }
                    }
                }
                for bin in 0..bins {
                    assert_eq!(trace.body_clearance[bin].to_bits(), body[bin].to_bits());
                    assert_eq!(
                        trace.head_clearance[bin].to_bits(),
                        clearance[bin].to_bits()
                    );
                    assert_eq!(trace.head_count[bin], counts[bin]);
                    let id = if nearest[bin] == usize::MAX {
                        0
                    } else {
                        world.snakes[nearest[bin]].id
                    };
                    assert_eq!(trace.head_ids[bin], id);
                    assert_eq!(trace.urgency[bin].to_bits(), urgency[bin].to_bits());
                    assert_eq!(trace.encounter_seconds[bin], seconds[bin]);
                    assert_eq!(
                        output[24 + 2 * bins + bin].to_bits(),
                        bipolar(body[bin] / result.radii.near).to_bits()
                    );
                    assert_eq!(
                        output[24 + 5 * bins + bin].to_bits(),
                        bipolar(clearance[bin] / result.radii.near).to_bits()
                    );
                    if profile == SensorProfile::Motion {
                        assert_eq!(result.representative_head_ids[bin], id);
                        assert_eq!(
                            output[24 + 13 * bins + bin].to_bits(),
                            (urgency[bin] as f32).to_bits()
                        );
                        assert_eq!(
                            output[24 + 14 * bins + bin].to_bits(),
                            ((counts[bin] as f64 / (counts[bin] as f64 + 4.0)) as f32).to_bits()
                        );
                    }
                }
                assert_eq!(world, before);
            }
        }
    }
}

#[test]
fn shared_head_facts_preserve_every_vector_receipt_and_trace_across_profiles_bins_and_grants() {
    let mut world = WorldState::default();
    for (index, heading) in [0.0, PI, PI / 2.0, -PI / 2.0, TAU * 1.0e8, -TAU * 1.0e8]
        .into_iter()
        .enumerate()
    {
        add_snake(
            &mut world,
            900 - index as u64 * 13,
            SnakeKind::External,
            point(index as f64 * 37.0 - 100.0, (index % 3) as f64 * 41.0),
            heading,
            10 + index,
        );
        world.snakes[index].speed = [0.0, 165.0, 247.5, 42.0, 1.0e5, 5.0][index];
    }
    let original = world.clone();
    let indexed = IndexedSensorWorld::build(
        &world,
        SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 1_000_000,
            maximum_pellet_entries: 100_000,
        },
    )
    .unwrap();
    let mut rays = BodyRayIndexWorkspace::default();
    let body_rays = rays.prepare(indexed.body_index()).unwrap();
    let lives = EvolvedGenerationState::default();
    let context = V4ObservationContext {
        generation_elapsed_seconds: 120.0,
        evolved: &lives,
        experiment: EvolvedExperimentConfig::legacy(240.0),
    };
    let mut heads = V4HeadWorkspace::try_new(world.snakes.len()).unwrap();
    let capacity = heads.capacity();
    let mut reference_scratch = V4SensorScratch::default();
    let mut cached_scratch = V4SensorScratch::default();
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        let prepared = heads.prepare(&world, profile).unwrap().unwrap();
        for bins in 8..=32 {
            let evaluator = evaluator(profile, bins);
            for grant in [0, 7, 128, 4096] {
                for observer in 0..world.snakes.len() {
                    let mut reference = vec![0.0; evaluator.layout().input_size()];
                    let mut cached = reference.clone();
                    let mut reference_trace = crate::engine::sensor_trace::SensorTrace::default();
                    let mut cached_trace = crate::engine::sensor_trace::SensorTrace::default();
                    let mut reference_meter = RayWorkMeter::new(grant);
                    let mut cached_meter = RayWorkMeter::new(grant);
                    let expected = evaluator
                        .sample_with_trace(
                            &indexed,
                            body_rays,
                            context,
                            observer,
                            &mut reference,
                            &mut reference_meter,
                            &mut reference_scratch,
                            Some(&mut reference_trace),
                            None,
                        )
                        .unwrap();
                    let actual = evaluator
                        .sample_with_trace(
                            &indexed,
                            body_rays,
                            context,
                            observer,
                            &mut cached,
                            &mut cached_meter,
                            &mut cached_scratch,
                            Some(&mut cached_trace),
                            Some(prepared),
                        )
                        .unwrap();
                    assert_eq!(
                        reference
                            .iter()
                            .map(|value| value.to_bits())
                            .collect::<Vec<_>>(),
                        cached
                            .iter()
                            .map(|value| value.to_bits())
                            .collect::<Vec<_>>()
                    );
                    assert_eq!(actual, expected);
                    assert_eq!(cached_meter, reference_meter);
                    assert_eq!(cached_trace, reference_trace);
                }
            }
        }
        assert_eq!(heads.capacity(), capacity);
    }
    assert_eq!(world, original);
}

#[test]
fn shared_head_facts_reject_wrong_world_profile_and_invalid_alive_heads_without_growth() {
    let mut world = WorldState::default();
    add_snake(&mut world, 7, SnakeKind::External, point(0.0, 0.0), 0.0, 10);
    add_snake(
        &mut world,
        99,
        SnakeKind::Baseline,
        point(80.0, 0.0),
        PI,
        20,
    );
    let other_world = world.clone();
    let mut heads = V4HeadWorkspace::try_new(2).unwrap();
    let prepared = heads
        .prepare(&world, SensorProfile::Motion)
        .unwrap()
        .unwrap();
    assert!(prepared.is_from(&world, SensorProfile::Motion));
    assert!(!prepared.is_from(&other_world, SensorProfile::Motion));
    assert!(!prepared.is_from(&world, SensorProfile::Basic));
    let indexed = IndexedSensorWorld::build(
        &other_world,
        SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 1_000_000,
            maximum_pellet_entries: 100_000,
        },
    )
    .unwrap();
    let mut rays = BodyRayIndexWorkspace::default();
    let view = rays.prepare(indexed.body_index()).unwrap();
    let lives = EvolvedGenerationState::default();
    let evaluator = evaluator(SensorProfile::Motion, 8);
    let mut output = vec![0.25; evaluator.layout().input_size()];
    assert_eq!(
        evaluator.sample_with_trace(
            &indexed,
            view,
            V4ObservationContext {
                generation_elapsed_seconds: 0.0,
                evolved: &lives,
                experiment: EvolvedExperimentConfig::legacy(240.0),
            },
            0,
            &mut output,
            &mut RayWorkMeter::new(128),
            &mut V4SensorScratch::default(),
            None,
            Some(prepared)
        ),
        Err(V4SensorError::Shape)
    );
    assert!(output.iter().all(|value| *value == 0.25));
    let capacity = heads.capacity();
    world.snakes[1].speed = f64::NAN;
    assert!(matches!(
        heads.prepare(&world, SensorProfile::Motion),
        Err(V4SensorError::Shape)
    ));
    world.snakes[1].alive = false;
    assert!(heads
        .prepare(&world, SensorProfile::Motion)
        .unwrap()
        .is_some());
    assert_eq!(heads.capacity(), capacity);
    let mut short = V4HeadWorkspace::try_new(1).unwrap();
    assert!(short
        .prepare(&world, SensorProfile::Motion)
        .unwrap()
        .is_none());
    assert_eq!(short.capacity(), 1);
    assert!(matches!(
        short.prepare(&world, SensorProfile::V3),
        Err(V4SensorError::Config)
    ));
    assert!(V4HeadWorkspace::try_new(usize::MAX).is_err());
}

#[test]
fn raw_trace_preserves_quota_results_and_distinguishes_incomplete_rays() {
    use crate::engine::sensor_trace::{SensorTrace, SensorTraceRay};
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 10);
    add_snake(&mut world, 2, SnakeKind::Baseline, point(80.0, 0.0), PI, 20);
    let indexed = IndexedSensorWorld::build(
        &world,
        SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 1_000_000,
            maximum_pellet_entries: 100_000,
        },
    )
    .unwrap();
    let mut rays = BodyRayIndexWorkspace::default();
    let view = rays.prepare(indexed.body_index()).unwrap();
    let lives = EvolvedGenerationState::default();
    for profile in [SensorProfile::Basic, SensorProfile::Motion] {
        for grant in [0, 4096] {
            let evaluator = evaluator(profile, 16);
            let context = V4ObservationContext {
                generation_elapsed_seconds: 120.0,
                evolved: &lives,
                experiment: EvolvedExperimentConfig::legacy(240.0),
            };
            let mut expected = vec![0.0; evaluator.layout().input_size()];
            let mut actual = expected.clone();
            let mut plain_meter = RayWorkMeter::new(grant);
            let plain = evaluator
                .sample(
                    &indexed,
                    view,
                    context,
                    0,
                    &mut expected,
                    &mut plain_meter,
                    &mut V4SensorScratch::default(),
                )
                .unwrap();
            let mut trace = SensorTrace::default();
            let mut traced_meter = RayWorkMeter::new(grant);
            let traced = evaluator
                .sample_with_trace(
                    &indexed,
                    view,
                    context,
                    0,
                    &mut actual,
                    &mut traced_meter,
                    &mut V4SensorScratch::default(),
                    Some(&mut trace),
                    None,
                )
                .unwrap();
            assert_eq!(actual, expected);
            assert_eq!(traced.diagnostics, plain.diagnostics);
            assert_eq!(traced.delivery, plain.delivery);
            assert_eq!(traced_meter, plain_meter);
            assert!(trace.valid);
            assert_eq!(trace.physical_id, 1);
            assert_eq!(trace.head_ids[8], 2);
            assert_eq!(trace.head_count[8], 1);
            assert_eq!(
                trace.head_clearance[8],
                80.0 - 18.0 * evaluator.common.config().collision_hit_scale
            );
            if grant == 0 {
                assert!(trace.body_rays[..16].contains(&SensorTraceRay::Incomplete));
            } else {
                assert!(trace.body_rays[..16]
                    .iter()
                    .all(|ray| matches!(ray, SensorTraceRay::Hit(_) | SensorTraceRay::Clear)));
                assert!(trace.body_rays[..16]
                    .iter()
                    .any(|ray| matches!(ray, SensorTraceRay::Hit(_))));
            }
            assert!(trace.body_rays[16..]
                .iter()
                .all(|ray| *ray == SensorTraceRay::NotSampled));
            if profile == SensorProfile::Motion {
                assert_eq!(trace.relative_forward_speed[8], -330.0);
                assert_eq!(trace.head_length_ratio[8], 2.0);
                assert!(trace.encounter_seconds.iter().any(Option::is_some));
            }
            let mut invalid = vec![];
            assert!(evaluator
                .sample_with_trace(
                    &indexed,
                    view,
                    context,
                    0,
                    &mut invalid,
                    &mut traced_meter,
                    &mut V4SensorScratch::default(),
                    Some(&mut trace),
                    None,
                )
                .is_err());
            assert!(!trace.valid);
        }
    }
}

#[test]
fn food_rings_have_exact_near_boundary_and_coincident_food_has_finite_declared_sector() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
    for (id, distance) in [
        (10, 0.0),
        (11, 520.0),
        (12, 521.0),
        (13, 1200.0),
        (14, 1200.1),
    ] {
        world.pellets.push(PelletState {
            id,
            position: point(distance, 0.0),
            value: 1.0,
            kind: 0,
            color: 0,
            owner: None,
        });
    }
    let (values, result) = simple(SensorProfile::Basic, &world);
    let near = 1.0 + 1.0 - 520.0 / 1200.0;
    let far = 1.0 - 521.0 / 1200.0;
    close(values[24 + 8], 2.0 * near / (near + 4.0) - 1.0);
    close(values[24 + 16 + 8], 2.0 * far / (far + 4.0) - 1.0);
    assert_eq!(result.diagnostics.pellets.candidates, 4);
    for bin in 0..16 {
        if bin != 8 {
            assert_eq!(values[24 + bin], -1.0);
            assert_eq!(values[40 + bin], -1.0);
        }
    }
}

#[test]
fn approaching_receding_and_tied_nearest_heads_share_exact_ordered_motion_channels() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
    add_snake(
        &mut world,
        30,
        SnakeKind::Baseline,
        point(100.0, 0.0),
        PI,
        20,
    );
    add_snake(
        &mut world,
        20,
        SnakeKind::Baseline,
        point(100.0, 0.0),
        PI,
        20,
    );
    let (approach, result) = simple(SensorProfile::Motion, &world);
    let bin = 8;
    assert_eq!(result.representative_head_ids[bin], 20);
    assert_eq!(approach[24 + 6 * 16 + bin], 1.0);
    close(approach[24 + 7 * 16 + bin], 0.0);
    close(approach[24 + 8 * 16 + bin], 0.5);
    close(approach[24 + 9 * 16 + bin], 0.0);
    close(approach[24 + 10 * 16 + bin], -1.0);
    close(approach[24 + 11 * 16 + bin], -0.33);
    close(approach[24 + 12 * 16 + bin], 0.0);
    close(
        approach[24 + 13 * 16 + bin],
        1.0 - (100.0 - 18.0 * 0.82) / 330.0 / 2.0,
    );
    close(approach[24 + 14 * 16 + bin], 2.0 / 6.0);
    world.snakes.swap(1, 2);
    let (reordered, ids) = simple(SensorProfile::Motion, &world);
    assert_eq!(approach, reordered);
    assert_eq!(result.representative_head_ids, ids.representative_head_ids);
    for other in &mut world.snakes[1..] {
        other.direction = 0.0;
        other.speed = 500.0;
    }
    let (recede, _) = simple(SensorProfile::Motion, &world);
    close(recede[24 + 11 * 16 + bin], 0.335);
    assert_eq!(recede[24 + 13 * 16 + bin], 0.0);
}

#[test]
fn crossing_urgency_includes_nonrepresentative_heads_and_predicted_contact_sector() {
    let mut world = WorldState::default();
    add_snake(&mut world, 1, SnakeKind::External, point(0.0, 0.0), 0.0, 5);
    world.snakes[0].speed = 100.0;
    add_snake(&mut world, 2, SnakeKind::Baseline, point(25.0, 0.0), 0.0, 5);
    world.snakes[1].speed = 100.0;
    add_snake(
        &mut world,
        3,
        SnakeKind::Baseline,
        point(100.0, 100.0),
        -PI / 2.0,
        5,
    );
    world.snakes[2].speed = 100.0;
    let (values, ids) = simple(SensorProfile::Motion, &world);
    assert_eq!(ids.representative_head_ids[8], 2);
    let crossing_bin = 10;
    assert!(values[24 + 13 * 16 + crossing_bin] > 0.5);
    assert_eq!(values[24 + 13 * 16 + 8], 0.0);
}
