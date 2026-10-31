//! Bounded operational generation diagnostics. Never checkpoint/replay state.
//! Timers observe existing work; no diagnostic value controls simulation decisions.
use super::state::{SnakeKind, WorldState};
use std::time::Instant;

pub(crate) fn nanos(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}
fn hex(value: u64) -> String {
    format!("{value:016x}")
}

/// Sum/sample-count preserves exact means without floating-point hot-path arithmetic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Distribution {
    pub samples: u64,
    pub total: u64,
    pub maximum: u64,
}
impl Distribution {
    pub fn observe(&mut self, value: u64) {
        self.samples = self.samples.saturating_add(1);
        self.total = self.total.saturating_add(value);
        self.maximum = self.maximum.max(value);
    }
    pub fn merge(&mut self, other: Self) {
        self.samples = self.samples.saturating_add(other.samples);
        self.total = self.total.saturating_add(other.total);
        self.maximum = self.maximum.max(other.maximum);
    }
    fn json(self) -> serde_json::Value {
        if self.samples == 0 {
            return serde_json::Value::Null;
        }
        serde_json::json!({"samples":hex(self.samples),"total":hex(self.total),"maximum":hex(self.maximum)})
    }
}

/// Wall time for serial phases/batches. Nested collision fields overlap collision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkTimings {
    pub publication: u64,
    pub world_total: u64,
    pub control_commit: u64,
    pub control_total: u64,
    pub prefix: u64,
    pub authority_begin: u64,
    pub control_body_index: u64,
    pub control_pellet_index: u64,
    pub sensor_head_prepare: u64,
    pub movement: u64,
    pub food: u64,
    pub collision: u64,
    pub collision_index: u64,
    pub collision_head_head: u64,
    pub collision_head_body: u64,
    pub physics_pellet_index: u64,
    pub effects: u64,
    pub control_spatial_index: u64,
    pub body_ray_prepare: u64,
    pub baseline_sensing: u64,
    pub baseline_policy: u64,
    pub neural_sensing: u64,
    pub neural_inference: u64,
    pub neural_batch_preparation: u64,
}
impl WorkTimings {
    pub fn merge(&mut self, other: Self) {
        self.control_body_index = self
            .control_body_index
            .saturating_add(other.control_body_index);
        self.control_pellet_index = self
            .control_pellet_index
            .saturating_add(other.control_pellet_index);
        self.sensor_head_prepare = self
            .sensor_head_prepare
            .saturating_add(other.sensor_head_prepare);
        self.authority_begin = self.authority_begin.saturating_add(other.authority_begin);
        self.prefix = self.prefix.saturating_add(other.prefix);
        self.control_total = self.control_total.saturating_add(other.control_total);
        self.control_commit = self.control_commit.saturating_add(other.control_commit);
        self.world_total = self.world_total.saturating_add(other.world_total);
        self.publication = self.publication.saturating_add(other.publication);
        self.movement = self.movement.saturating_add(other.movement);
        self.food = self.food.saturating_add(other.food);
        self.collision = self.collision.saturating_add(other.collision);
        self.collision_index = self.collision_index.saturating_add(other.collision_index);
        self.collision_head_head = self
            .collision_head_head
            .saturating_add(other.collision_head_head);
        self.collision_head_body = self
            .collision_head_body
            .saturating_add(other.collision_head_body);
        self.physics_pellet_index = self
            .physics_pellet_index
            .saturating_add(other.physics_pellet_index);
        self.effects = self.effects.saturating_add(other.effects);
        self.control_spatial_index = self
            .control_spatial_index
            .saturating_add(other.control_spatial_index);
        self.body_ray_prepare = self.body_ray_prepare.saturating_add(other.body_ray_prepare);
        self.baseline_sensing = self.baseline_sensing.saturating_add(other.baseline_sensing);
        self.baseline_policy = self.baseline_policy.saturating_add(other.baseline_policy);
        self.neural_sensing = self.neural_sensing.saturating_add(other.neural_sensing);
        self.neural_inference = self.neural_inference.saturating_add(other.neural_inference);
        self.neural_batch_preparation = self
            .neural_batch_preparation
            .saturating_add(other.neural_batch_preparation);
    }
}

/// Summed worker-local query time is CPU-like elapsed work, not parallel batch wall time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SensorWork {
    pub body_candidates: Distribution,
    pub ray_candidates: Distribution,
    pub body_query_worker_ns: u64,
    pub body_ray_query_worker_ns: u64,
}
impl SensorWork {
    pub fn merge(&mut self, other: Self) {
        self.body_candidates.merge(other.body_candidates);
        self.ray_candidates.merge(other.ray_candidates);
        self.body_query_worker_ns = self
            .body_query_worker_ns
            .saturating_add(other.body_query_worker_ns);
        self.body_ray_query_worker_ns = self
            .body_ray_query_worker_ns
            .saturating_add(other.body_ray_query_worker_ns);
    }
    pub fn sample(
        &mut self,
        diagnostics: super::sensors::SensorSampleDiagnostics,
        body_ns: u64,
        ray_ns: Option<u64>,
    ) {
        self.body_candidates
            .observe(diagnostics.body_query.candidates as u64);
        self.body_query_worker_ns = self.body_query_worker_ns.saturating_add(body_ns);
        if let Some(ns) = ray_ns {
            self.ray_candidates
                .observe(diagnostics.ray_work.candidates as u64);
            self.body_ray_query_worker_ns = self.body_ray_query_worker_ns.saturating_add(ns);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StepWork {
    pub timings: WorkTimings,
    pub sensors: SensorWork,
    pub neural_deliveries: u64,
    pub baseline_deliveries: u64,
    pub ray_preparations: u64,
    pub alive_evolved: u64,
    pub alive_total: u64,
    pub body_points: u64,
    pub body_segments: u64,
    pub evolved_lengths: Distribution,
    pub pellets: u64,
}
impl StepWork {
    /// Sample the completed physical boundary, including only currently alive bodies.
    pub fn observe_world(&mut self, world: &WorldState) {
        for snake in &world.snakes {
            if !snake.alive {
                continue;
            }
            self.alive_total += 1;
            self.body_points += snake.body.len as u64;
            self.body_segments += snake.body.len.saturating_sub(1) as u64;
            if snake.kind == SnakeKind::Evolved {
                self.alive_evolved += 1;
                self.evolved_lengths.observe(snake.body.len as u64);
            }
        }
        self.pellets = world.pellets.len() as u64;
    }
}

/// Process-local coverage only: resume is marked partial, never reconstructed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GenerationWork {
    world_epoch: u64,
    generation: u64,
    first_step: u64,
    last_step: u64,
    steps: u64,
    partial: bool,
    timings: WorkTimings,
    sensors: SensorWork,
    neural_deliveries: u64,
    baseline_deliveries: u64,
    ray_preparations: u64,
    alive_evolved: Distribution,
    alive_total: Distribution,
    body_points: Distribution,
    body_segments: Distribution,
    evolved_lengths: Distribution,
    pellets: Distribution,
}
impl GenerationWork {
    pub(crate) fn record(
        &mut self,
        epoch: u64,
        generation: u64,
        step: u64,
        elapsed_before: f64,
        work: StepWork,
    ) {
        if self.world_epoch != epoch || self.generation != generation {
            *self = Self {
                world_epoch: epoch,
                generation,
                first_step: step,
                partial: elapsed_before > 1.0e-9,
                ..Self::default()
            };
        }
        // A held publication retry must never charge the same committed step twice.
        if step <= self.last_step {
            return;
        }
        self.last_step = step;
        self.steps = self.steps.saturating_add(1);
        self.timings.merge(work.timings);
        self.sensors.merge(work.sensors);
        self.neural_deliveries = self
            .neural_deliveries
            .saturating_add(work.neural_deliveries);
        self.baseline_deliveries = self
            .baseline_deliveries
            .saturating_add(work.baseline_deliveries);
        self.ray_preparations = self.ray_preparations.saturating_add(work.ray_preparations);
        self.alive_evolved.observe(work.alive_evolved);
        self.alive_total.observe(work.alive_total);
        self.body_points.observe(work.body_points);
        self.body_segments.observe(work.body_segments);
        self.evolved_lengths.merge(work.evolved_lengths);
        self.pellets.observe(work.pellets);
    }

    /// Allocates only once at the durable generation handoff. Integers cross JS as hex.
    pub fn to_json(self) -> String {
        let mut timings = serde_json::json!({
            "controlBodyIndex":hex(self.timings.control_body_index),
            "controlPelletIndex":hex(self.timings.control_pellet_index),
            "sensorHeadPrepare":hex(self.timings.sensor_head_prepare),
            "authorityBegin":hex(self.timings.authority_begin),
            "prefix":hex(self.timings.prefix),
            "controlTotal":hex(self.timings.control_total),
            "controlCommit":hex(self.timings.control_commit),
            "worldTotal":hex(self.timings.world_total),
            "publication":hex(self.timings.publication),
            "movement":hex(self.timings.movement),
            "food":hex(self.timings.food),
            "collision":hex(self.timings.collision),
            "collisionIndex":hex(self.timings.collision_index),
            "collisionHeadHead":hex(self.timings.collision_head_head),
            "collisionHeadBody":hex(self.timings.collision_head_body),
            "physicsPelletIndex":hex(self.timings.physics_pellet_index),
            "effects":hex(self.timings.effects),
            "controlSpatialIndex":hex(self.timings.control_spatial_index),
            "bodyRayPrepare":hex(self.timings.body_ray_prepare),
            "baselineSensing":hex(self.timings.baseline_sensing),
            "baselinePolicy":hex(self.timings.baseline_policy),
            "neuralSensing":hex(self.timings.neural_sensing),
            "neuralInference":hex(self.timings.neural_inference),
            "neuralBatchPreparation":hex(self.timings.neural_batch_preparation),
            "bodyQueryWorker":if self.sensors.body_candidates.samples == 0 { None } else { Some(hex(self.sensors.body_query_worker_ns)) },
            "bodyRayQueryWorker":if self.sensors.ray_candidates.samples == 0 { None } else { Some(hex(self.sensors.body_ray_query_worker_ns)) }
        });
        if self.ray_preparations == 0 {
            timings["bodyRayPrepare"] = serde_json::Value::Null;
            timings["sensorHeadPrepare"] = serde_json::Value::Null;
        }
        if self.baseline_deliveries == 0 {
            timings["baselineSensing"] = serde_json::Value::Null;
            timings["baselinePolicy"] = serde_json::Value::Null;
        }
        serde_json::json!({
            "version":1, "generation":hex(self.generation), "firstStep":hex(self.first_step),
            "lastStep":hex(self.last_step), "steps":hex(self.steps), "partial":self.partial,
            "timingsNs":timings,
            "neuralDeliveries":hex(self.neural_deliveries), "baselineDeliveries":hex(self.baseline_deliveries),
            "rayPreparations":hex(self.ray_preparations),
            "workload":{
                "aliveEvolved":self.alive_evolved.json(), "aliveTotal":self.alive_total.json(),
                "bodyPoints":self.body_points.json(), "bodySegments":self.body_segments.json(),
                "evolvedBodyLength":self.evolved_lengths.json(), "pellets":self.pellets.json(),
                "bodyQueryCandidates":self.sensors.body_candidates.json(),
                "rayQueryCandidates":self.sensors.ray_candidates.json()
            }
        }).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resets_at_epoch_and_generation_boundaries_and_ignores_retry() {
        let mut record = GenerationWork::default();
        let work = StepWork {
            alive_total: 3,
            body_points: 30,
            ..StepWork::default()
        };
        record.record(1, 1, 1, 0.0, work);
        record.record(1, 1, 1, 0.0, work);
        record.record(1, 1, 2, 1.0 / 60.0, work);
        assert_eq!(record.steps, 2);
        assert_eq!(record.body_points.total, 60);
        record.record(1, 2, 3, 0.0, work);
        assert_eq!(record.steps, 1);
        assert_eq!(record.body_points.total, 30);
        assert!(!record.partial);
        record.record(2, 2, 40, 10.0, work);
        assert_eq!(record.steps, 1);
        assert!(record.partial);
    }
    #[test]
    fn unavailable_is_null_and_exact_means_keep_denominators() {
        let mut record = GenerationWork::default();
        record.record(1, 1, 1, 0.0, StepWork::default());
        let value: serde_json::Value = serde_json::from_str(&record.to_json()).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["steps"], "0000000000000001");
        assert!(value["timingsNs"]["bodyRayPrepare"].is_null());
        assert!(value["workload"]["rayQueryCandidates"].is_null());
        assert!(value["workload"]["evolvedBodyLength"].is_null());
        assert_eq!(
            value["workload"]["aliveEvolved"]["total"],
            "0000000000000000"
        );
        let mut a = Distribution::default();
        a.observe(3);
        a.observe(9);
        assert_eq!((a.samples, a.total, a.maximum), (2, 12, 9));
    }
}
