//! V4 sensor-only pressure on existing native dense fixtures, with explicit geometry audit.
//! Immutable synthetic boundaries do not establish evolved-policy or complete-step performance.
use super::calculation::CalculationBatchKey;
use super::evolved_lives::{EvolvedExperimentConfig, EvolvedGenerationState};
use super::movement::MovementConfig;
use super::ray_work::{
    prepare_ray_quotas, RayObservationOwner, RayQuotaCursor, RayWorkLimits, RayWorkMeter,
};
use super::sensing_fixture::{build_world, digest_world, Stage4SensingScenarioName};
use super::sensor_contract::SensorProfile;
use super::sensor_rays::BodyRayIndexWorkspace;
use super::sensors::SensorConfig;
use super::sensors_v4::{
    PreparedV4Sensing, SensorV4Evaluator, V4HeadWorkspace, V4ObservationContext, V4SensorScratch,
};
use super::spatial::{IndexedSensorWorld, SensorIndexConfig};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Instant;

/// Conservative audit excludes the first thirty head-adjacent body points.
const AUDIT_HEAD_SKIP: usize = 30;

/// No fixture dimensions or production defaults are rewritten by the standalone runner.
pub struct CrowdedSensorOptions {
    pub scenario: Stage4SensingScenarioName,
    pub profile: SensorProfile,
    pub bins: usize,
    pub workers: usize,
    pub warmup_passes: usize,
    pub measured_passes: usize,
    pub ray_limits: RayWorkLimits,
}

/// Scalar work from one complete sensing pass, independent of worker partitioning.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Work {
    observations: u64,
    charged: u64,
    incomplete_observations: u64,
    incomplete_rays: u64,
    body_candidates: u64,
    body_capped: u64,
    pellet_candidates: u64,
    pellet_capped: u64,
    head_checks: u64,
}
impl Work {
    fn merge(&mut self, other: Self) {
        self.observations += other.observations;
        self.charged += other.charged;
        self.incomplete_observations += other.incomplete_observations;
        self.incomplete_rays += other.incomplete_rays;
        self.body_candidates += other.body_candidates;
        self.body_capped += other.body_capped;
        self.pellet_candidates += other.pellet_candidates;
        self.pellet_capped += other.pellet_capped;
        self.head_checks += other.head_checks;
    }
}

/// One retained row block and scratch allocation per bounded calculation worker.
struct Worker {
    first: usize,
    rows: Vec<f32>,
    scratch: V4SensorScratch,
    work: Work,
}

/// Reuse the native stable-owner grants and actual evaluator over disjoint row blocks.
fn pass(
    pool: &rayon::ThreadPool,
    workers: &mut [Worker],
    evaluator: &SensorV4Evaluator,
    indexed: &IndexedSensorWorld<'_>,
    prepared: PreparedV4Sensing<'_>,
    owners: &[(RayObservationOwner, usize)],
) -> Result<Work, String> {
    pool.install(|| {
        workers.par_iter_mut().try_for_each(|worker| {
            worker.work = Work::default();
            for (offset, row) in worker
                .rows
                .chunks_exact_mut(evaluator.layout().input_size())
                .enumerate()
            {
                let ordinal = worker.first + offset;
                let index = owners[ordinal].1;
                let grant = prepared.grants_by_snake[index];
                let mut meter = RayWorkMeter::new(grant);
                let receipt = evaluator
                    .sample_with_trace(
                        indexed,
                        prepared.body_rays,
                        prepared.context,
                        index,
                        row,
                        &mut meter,
                        &mut worker.scratch,
                        None,
                        prepared.heads,
                    )
                    .map_err(|e| e.to_string())?;
                let work = receipt.diagnostics;
                if work.ray.charged() > u64::from(grant) {
                    return Err("crowded per-owner ray grant exceeded".to_owned());
                }
                worker.work.merge(Work {
                    observations: 1,
                    charged: work.ray.charged(),
                    incomplete_observations: u64::from(work.incomplete_rays > 0),
                    incomplete_rays: work.incomplete_rays as u64,
                    body_candidates: work.body.candidates as u64,
                    body_capped: u64::from(work.body.candidate_limit_reached),
                    pellet_candidates: work.pellets.candidates as u64,
                    pellet_capped: u64::from(work.pellets.candidate_limit_reached),
                    head_checks: work.head_checks as u64,
                });
            }
            Ok::<_, String>(())
        })
    })?;
    let mut total = Work::default();
    for worker in workers {
        total.merge(worker.work);
    }
    Ok(total)
}

/// Audit actual endpoints inside the arena; long off-arena tails cannot qualify density.
fn geometry(world: &super::state::WorldState, radius: f64) -> Result<Value, String> {
    let inside = |point: &super::state::WorldPoint| -> Result<bool, String> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err("nonfinite crowded geometry".into());
        }
        Ok(point.x * point.x + point.y * point.y <= radius * radius)
    };
    let mut points = 0usize;
    let mut collision_segments = 0usize;
    for snake in &world.snakes {
        let body = world
            .body_points
            .get(
                snake.body.start
                    ..snake
                        .body
                        .start
                        .checked_add(snake.body.len)
                        .ok_or("crowded body range overflow")?,
            )
            .ok_or("crowded body bounds")?;
        for point in body {
            points += usize::from(inside(point)?);
        }
        for (index, segment) in body.windows(2).enumerate() {
            if index + 1 >= AUDIT_HEAD_SKIP && inside(&segment[0])? && inside(&segment[1])? {
                collision_segments += 1;
            }
        }
    }
    let mut pellets = 0usize;
    for pellet in &world.pellets {
        pellets += usize::from(inside(&pellet.position)?);
    }
    if points != world.body_points.len() || pellets != world.pellets.len() {
        return Err("crowded fixture contains off-arena geometry".into());
    }
    Ok(
        json!({"arenaRadius":radius,"headSkipPoints":AUDIT_HEAD_SKIP,"bodyPoints":points,"inArenaCollisionSegments":collision_segments,
        "pellets":pellets,"outsideArenaPoints":0,"outsideArenaPellets":0}),
    )
}

/// Bound all work before allocation; timing includes dispatch/evaluation, while proof hashing is cold.
pub fn run_crowded_sensor_qualification(
    options: &CrowdedSensorOptions,
    allocations: impl Fn() -> u64,
) -> Result<Value, String> {
    if !matches!(
        options.scenario,
        Stage4SensingScenarioName::DenseBody | Stage4SensingScenarioName::DensePellet
    ) || options.profile == SensorProfile::V3
        || !(1..=32).contains(&options.workers)
        || !(1..=100).contains(&options.warmup_passes)
        || !(1..=1000).contains(&options.measured_passes)
        || options.ray_limits.maximum_due() != 512
    {
        return Err("requires existing dense fixture, v4, 1..32 workers, 1..100 warmup and 1..1000 measured passes, 512 due admission".into());
    }
    let config = SensorConfig {
        bins: options.bins,
        ..SensorConfig::default()
    };
    let radius = config.world_radius;
    let evaluator = SensorV4Evaluator::new(
        options.profile,
        config,
        MovementConfig::typescript_defaults(),
        1.0 / 60.0,
    )
    .map_err(|e| e.to_string())?;
    let world = build_world(options.scenario.spec())?;
    let world_sha = digest_world(&world);
    let audit = geometry(&world, radius)?;
    if options.scenario == Stage4SensingScenarioName::DenseBody
        && audit["inArenaCollisionSegments"].as_u64().unwrap_or(0) <= 200_000
    {
        return Err("dense-body requires over 200000 actual in-arena collision segments".into());
    }
    let index_started = Instant::now();
    let indexed = IndexedSensorWorld::build(
        &world,
        SensorIndexConfig {
            body_cell_size: 70.0,
            pellet_cell_size: 120.0,
            maximum_body_entries: 2_000_000,
            maximum_pellet_entries: 25_000,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut rays = BodyRayIndexWorkspace::default();
    let ray_view = rays
        .prepare(indexed.body_index())
        .map_err(|e| e.to_string())?;
    let mut heads = V4HeadWorkspace::try_new(world.snakes.len()).map_err(|e| e.to_string())?;
    let head_view = heads
        .prepare(&world, options.profile)
        .map_err(|e| e.to_string())?;
    let cold_index_ms = index_started.elapsed().as_secs_f64() * 1000.0;
    let mut owners: Vec<_> = world
        .snakes
        .iter()
        .enumerate()
        .map(|(index, snake)| {
            RayObservationOwner::of(snake)
                .map(|owner| (owner, index))
                .map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    owners.sort_unstable_by_key(|entry| entry.0);
    let keys: Vec<_> = owners.iter().map(|entry| entry.0).collect();
    let mut grants = vec![0; keys.len()];
    let proposal = prepare_ray_quotas(
        CalculationBatchKey::new(1, 1, 1),
        options.ray_limits,
        RayQuotaCursor::default(),
        &keys,
        &mut grants,
    )
    .map_err(|e| e.to_string())?;
    let mut grants_by_snake = vec![0; world.snakes.len()];
    for ((_, index), grant) in owners.iter().zip(&grants) {
        grants_by_snake[*index] = *grant;
    }
    let lives = EvolvedGenerationState::default();
    let prepared = PreparedV4Sensing {
        body_rays: ray_view,
        heads: head_view,
        grants_by_snake: &grants_by_snake,
        context: V4ObservationContext {
            generation_elapsed_seconds: 120.0,
            evolved: &lives,
            experiment: EvolvedExperimentConfig::legacy(240.0),
        },
    };
    let block = owners.len().div_ceil(options.workers);
    let mut workers: Vec<_> = (0..owners.len())
        .step_by(block)
        .map(|first| Worker {
            first,
            rows: vec![0.0; block.min(owners.len() - first) * evaluator.layout().input_size()],
            scratch: V4SensorScratch::default(),
            work: Work::default(),
        })
        .collect();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.workers)
        .build()
        .map_err(|e| e.to_string())?;
    for _ in 0..options.warmup_passes {
        pass(&pool, &mut workers, &evaluator, &indexed, prepared, &owners)?;
    }
    let before: Vec<_> = workers
        .iter()
        .map(|worker| worker.scratch.diagnostics())
        .collect();
    let mut times = Vec::with_capacity(options.measured_passes);
    let mut allocation_counts = Vec::with_capacity(options.measured_passes);
    let mut proof = None;
    for _ in 0..options.measured_passes {
        let allocated = allocations();
        let started = Instant::now();
        let work = pass(&pool, &mut workers, &evaluator, &indexed, prepared, &owners)?;
        times.push(started.elapsed().as_secs_f64() * 1000.0);
        allocation_counts.push(
            allocations()
                .checked_sub(allocated)
                .ok_or("allocation counter regressed")?,
        );
        if work.charged > u64::from(proposal.granted()) || work.observations != owners.len() as u64
        {
            return Err("crowded complete-pass work mismatch".into());
        }
        if proof.is_some_and(|previous| previous != work) {
            return Err("immutable crowded pass changed work".into());
        }
        proof = Some(work);
    }
    let after: Vec<_> = workers
        .iter()
        .map(|worker| worker.scratch.diagnostics())
        .collect();
    if before != after {
        return Err("crowded sensor scratch grew after warmup".into());
    }
    let mut hash = Sha256::new();
    hash.update(b"slither.crowded-v4-observations.v1\0");
    for worker in &workers {
        for value in &worker.rows {
            if !value.is_finite() || !(-1.0..=1.0).contains(value) {
                return Err("invalid crowded normalized observation".into());
            }
            hash.update(value.to_bits().to_le_bytes());
        }
    }
    if digest_world(&world) != world_sha {
        return Err("crowded observer changed world".into());
    }
    let scratch: Vec<_> = after.iter().map(|value| json!({"fixedBytes":value.fixed_bytes,
        "bodyDuplicateMarkerCapacity":value.body_duplicate_marker_capacity,"bodyCandidateCapacity":value.body_candidate_capacity,
        "pelletCandidateCapacity":value.pellet_candidate_capacity,"rayPacketCapacity":value.ray_packet_capacity,"rayPacketBytes":value.ray_packet_bytes})).collect();
    let ray_capacity = rays.diagnostics().map_err(|e| e.to_string())?;
    Ok(
        json!({"schemaVersion":1,"gate":"crowded-v4-sensing-v1","sourceSha256":crate::native_addon_source_sha256(),
        "target":crate::native_addon_build_target(),"buildProfile":crate::native_addon_build_profile(),"buildClass":crate::native_addon_build_class(),
        "rustc":crate::native_addon_rustc_version(),"scenario":options.scenario.label(),"sensor":evaluator.layout(),
        "sensorSha256":evaluator.layout().sha256_hex().map_err(|e| e.to_string())?,"workers":options.workers,
        "warmupPasses":options.warmup_passes,"measuredPasses":options.measured_passes,"worldSha256":world_sha,
        "worldUnchanged":true,"geometry":audit,"snakes":world.snakes.len(),"coldIndexesMs":cold_index_ms,
        "rayMinimum":options.ray_limits.minimum(),"rayCap":options.ray_limits.cap(),"rayBatch":options.ray_limits.batch(),
        "grantedWork":proposal.granted(),"proofPassWork":proof,"observationsSha256":format!("{:x}",hash.finalize()),
        "sensingMs":super::sensor_qualification::distribution(&mut times),"allocationOperationsPerPass":allocation_counts,
        "scratchStable":true,"scratch":scratch,"rayIndexOwnedBytes":ray_capacity.owned_bytes,
        "peakRssBytes":super::sensor_qualification::process_peak_rss_bytes(),
        "scope":"Repeated immutable native dense-body/dense-pellet sensing boundaries, stable-owner fair grants, cached native head facts and bounded worker scratch. Index construction is one cold sample; dispatch and work counters are inside sensing timing. No movement, inference, capture, teacher/trainer, evolved-policy distribution, wall-debt scheduler, sustained gameplay or production-default qualification."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(
        scenario: Stage4SensingScenarioName,
        profile: SensorProfile,
    ) -> CrowdedSensorOptions {
        CrowdedSensorOptions {
            scenario,
            profile,
            bins: 8,
            workers: 1,
            warmup_passes: 1,
            measured_passes: 2,
            ray_limits: RayWorkLimits::new(1, 1, 512, 512).unwrap(),
        }
    }
    #[test]
    fn in_arena_dense_geometry_and_saturation_are_identical_across_workers() {
        for (scenario, profile) in [
            (Stage4SensingScenarioName::DenseBody, SensorProfile::Motion),
            (Stage4SensingScenarioName::DensePellet, SensorProfile::Basic),
        ] {
            let mut options = options(scenario, profile);
            let serial = run_crowded_sensor_qualification(&options, || 0).unwrap();
            options.workers = 3;
            let parallel = run_crowded_sensor_qualification(&options, || 0).unwrap();
            for key in [
                "worldSha256",
                "geometry",
                "sensorSha256",
                "grantedWork",
                "proofPassWork",
                "observationsSha256",
            ] {
                assert_eq!(serial[key], parallel[key], "{key}");
            }
            assert_eq!(serial["geometry"]["outsideArenaPoints"], 0);
            assert!(
                serial["proofPassWork"]["incompleteObservations"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
            assert_eq!(parallel["scratchStable"], true);
        }
    }
    #[test]
    fn geometry_audit_rejects_off_arena_and_invalid_body_ranges() {
        let mut world = build_world(Stage4SensingScenarioName::P0.spec()).unwrap();
        world.body_points[0].x = 3501.0;
        assert!(geometry(&world, 3500.0).is_err());
        world.body_points[0].x = f64::NAN;
        assert!(geometry(&world, 3500.0).is_err());
        world.snakes[0].body.start = usize::MAX;
        assert!(geometry(&world, 3500.0).is_err());
    }
    #[test]
    fn ordinary_short_bodies_and_unbounded_passes_cannot_claim_crowded_evidence() {
        let mut value = options(Stage4SensingScenarioName::P0, SensorProfile::Basic);
        assert!(run_crowded_sensor_qualification(&value, || 0).is_err());
        value.scenario = Stage4SensingScenarioName::DenseBody;
        value.measured_passes = 1001;
        assert!(run_crowded_sensor_qualification(&value, || 0).is_err());
        value.measured_passes = 1;
        value.profile = SensorProfile::V3;
        assert!(run_crowded_sensor_qualification(&value, || 0).is_err());
    }
}
