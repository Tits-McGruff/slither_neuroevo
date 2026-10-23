//! Bounded test-addon accounting for ordinary, already-published observations.
//! No world sampling, delivery-marker consumption, RNG or retained observation rows.

use super::calculation::CalculationBatchKey;
use super::sensors::SensorSampleDiagnostics;

/// Fixed counters for one source class; all quality denominators are explicitly v4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObservationCounts {
    pub observations: u64,
    pub v4_observations: u64,
    pub ray_incomplete_observations: u64,
    pub body_capped_observations: u64,
    pub pellet_capped_observations: u64,
    pub query_saturated_observations: u64,
    pub ray_charged_units: u64,
}

impl ObservationCounts {
    /// Count one constructed source observation without inspecting its vector again.
    pub(crate) fn record(&mut self, sample: &SensorSampleDiagnostics, v4: bool) {
        self.observations = self.observations.saturating_add(1);
        if !v4 {
            return;
        }
        self.v4_observations = self.v4_observations.saturating_add(1);
        let ray = sample.incomplete_rays != 0;
        let body = sample.segment_cap_hits != 0;
        let pellet = sample.pellet_cap_hits != 0;
        self.ray_incomplete_observations = self
            .ray_incomplete_observations
            .saturating_add(u64::from(ray));
        self.body_capped_observations = self
            .body_capped_observations
            .saturating_add(u64::from(body));
        self.pellet_capped_observations = self
            .pellet_capped_observations
            .saturating_add(u64::from(pellet));
        self.query_saturated_observations = self
            .query_saturated_observations
            .saturating_add(u64::from(ray || body || pellet));
        self.ray_charged_units = self
            .ray_charged_units
            .saturating_add(sample.ray_work.charged());
    }

    /// Keep cumulative diagnostics monotonic even if a lifetime counter saturates.
    fn merge(&mut self, other: Self) {
        self.observations = self.observations.saturating_add(other.observations);
        self.v4_observations = self.v4_observations.saturating_add(other.v4_observations);
        self.ray_incomplete_observations = self
            .ray_incomplete_observations
            .saturating_add(other.ray_incomplete_observations);
        self.body_capped_observations = self
            .body_capped_observations
            .saturating_add(other.body_capped_observations);
        self.pellet_capped_observations = self
            .pellet_capped_observations
            .saturating_add(other.pellet_capped_observations);
        self.query_saturated_observations = self
            .query_saturated_observations
            .saturating_add(other.query_saturated_observations);
        self.ray_charged_units = self
            .ray_charged_units
            .saturating_add(other.ray_charged_units);
    }
}

/// One coherent cached prefix, independent of separately read operational health.
#[derive(Clone, Debug, Default)]
pub(crate) struct ObservationQuality {
    run_id: String,
    last_key: Option<CalculationBatchKey>,
    world_epoch: u64,
    generation: u64,
    completed_step: u64,
    ordinary_published_steps: u64,
    consumers: [ObservationCounts; 3],
}

impl ObservationQuality {
    /// Exact run/batch equality excludes repeated health reads and live config changes.
    pub(crate) fn needs_record(&self, run_id: &str, key: CalculationBatchKey) -> bool {
        self.run_id != run_id || self.last_key != Some(key)
    }

    /// Called only after the loop verifies a Ready, currently published batch key.
    pub(crate) fn record(
        &mut self,
        run_id: &str,
        key: CalculationBatchKey,
        world_epoch: u64,
        generation: u64,
        completed_step: u64,
        consumers: [ObservationCounts; 3],
    ) {
        if !self.needs_record(run_id, key) {
            return;
        }
        if self.run_id != run_id {
            self.run_id.clear();
            self.run_id.push_str(run_id);
        }
        self.last_key = Some(key);
        self.world_epoch = world_epoch;
        self.generation = generation;
        self.completed_step = completed_step;
        self.ordinary_published_steps = self.ordinary_published_steps.saturating_add(1);
        for (total, count) in self.consumers.iter_mut().zip(consumers) {
            total.merge(count);
        }
    }

    /// Serialize exact hexadecimal counts only when the coarse test bridge requests them.
    pub(crate) fn to_json(&self) -> Result<String, serde_json::Error> {
        let names = ["neural", "baseline", "external"];
        let consumers: Vec<_> = self
            .consumers
            .iter()
            .zip(names)
            .map(|(c, name)| {
                serde_json::json!({
                    "consumerClass": name,
                    "observations": format!("{:016x}", c.observations),
                    "v4Observations": format!("{:016x}", c.v4_observations),
                    "rayIncompleteObservations": format!("{:016x}", c.ray_incomplete_observations),
                    "bodyCappedObservations": format!("{:016x}", c.body_capped_observations),
                    "pelletCappedObservations": format!("{:016x}", c.pellet_capped_observations),
                    "querySaturatedObservations": format!("{:016x}", c.query_saturated_observations),
                    "rayChargedUnits": format!("{:016x}", c.ray_charged_units)
                })
            })
            .collect();
        serde_json::to_string(&serde_json::json!({
            "schemaVersion": 1,
            "runId": self.last_key.map(|_| self.run_id.as_str()),
            "ordinaryPublishedSteps": format!("{:016x}", self.ordinary_published_steps),
            "worldEpoch": format!("{:016x}", self.world_epoch),
            "generation": format!("{:016x}", self.generation),
            "completedStep": format!("{:016x}", self.completed_step),
            "consumers": consumers
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unions_count_each_v4_observation_once_and_exclude_legacy_quality_denominators() {
        let mut counts = ObservationCounts::default();
        let mut sample = SensorSampleDiagnostics {
            incomplete_rays: 3,
            segment_cap_hits: 1,
            pellet_cap_hits: 2,
            ..SensorSampleDiagnostics::default()
        };
        counts.record(&sample, false);
        assert_eq!(counts.observations, 1);
        assert_eq!(counts.v4_observations, 0);
        counts.record(&sample, true);
        assert_eq!(counts.query_saturated_observations, 1);
        assert_eq!(counts.ray_incomplete_observations, 1);
        assert_eq!(counts.body_capped_observations, 1);
        assert_eq!(counts.pellet_capped_observations, 1);
        sample.incomplete_rays = 0;
        sample.segment_cap_hits = 0;
        sample.pellet_cap_hits = 0;
        counts.record(&sample, true);
        assert_eq!(counts.observations, 3);
        assert_eq!(counts.v4_observations, 2);
        assert_eq!(counts.query_saturated_observations, 1);
    }

    #[test]
    fn repeated_reads_do_not_count_twice_but_new_runs_and_batches_do() {
        let mut observer = ObservationQuality::default();
        let key = CalculationBatchKey::new(1, 1, 1);
        let row = ObservationCounts {
            observations: 2,
            v4_observations: 2,
            ..ObservationCounts::default()
        };
        observer.record("run-a", key, 1, 1, 1, [row; 3]);
        observer.record("run-a", key, 2, 1, 1, [row; 3]);
        assert_eq!(observer.ordinary_published_steps, 1);
        observer.record("run-b", key, 3, 1, 1, [row; 3]);
        observer.record(
            "run-b",
            CalculationBatchKey::new(1, 2, 1),
            3,
            1,
            2,
            [row; 3],
        );
        assert_eq!(observer.ordinary_published_steps, 3);
        assert!(observer.consumers.iter().all(|c| c.observations == 6));
        let parsed: serde_json::Value = serde_json::from_str(&observer.to_json().unwrap()).unwrap();
        assert_eq!(parsed["ordinaryPublishedSteps"], "0000000000000003");
        assert_eq!(parsed["consumers"][2]["consumerClass"], "external");
        assert_eq!(parsed["consumers"][2]["observations"], "0000000000000006");
    }

    #[test]
    fn diagnostic_overflow_saturates_instead_of_wrapping_or_panicking() {
        let mut counts = ObservationCounts {
            observations: u64::MAX,
            ..ObservationCounts::default()
        };
        counts.merge(ObservationCounts {
            observations: 1,
            ..ObservationCounts::default()
        });
        assert_eq!(counts.observations, u64::MAX);
    }
}
