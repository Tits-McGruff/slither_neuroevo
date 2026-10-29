//! Immutable population selection through the ordinary strict checkpoint decoder.
use super::{checkpoint, fresh_run, graph::GraphBundle, state::NormalizedSetting};
use serde_json::{json, Value};
use std::{error::Error, path::Path};

/// Canonical copied weights/state and provenance; never modifies its checkpoint source.
pub struct EvolvedCohort {
    pub graph: GraphBundle,
    pub weights: Vec<Box<[f32]>>,
    pub recurrent: Vec<Box<[f32]>>,
    pub settings: Vec<NormalizedSetting>,
    pub provenance: Value,
}

impl EvolvedCohort {
    /// Select unique evenly spaced original population slots, without fitness selection.
    /// This is explicitly population transfer, not exact cross-build continuation.
    pub fn load(path: &Path, count: usize) -> Result<Self, Box<dyn Error>> {
        Self::load_for_class(path, count, &crate::native_addon_build_class())
    }

    /// Read-only production-population transfer into a qualification build.
    /// The producer must still be production/release on the supported target;
    /// this never changes ordinary restore policy or publishes restored authority.
    pub fn load_production(path: &Path, count: usize) -> Result<Self, Box<dyn Error>> {
        Self::load_for_class(path, count, "production")
    }

    fn load_for_class(
        path: &Path,
        count: usize,
        producer_class: &str,
    ) -> Result<Self, Box<dyn Error>> {
        if !(1..=300).contains(&count) {
            return Err("cohort size requires 1..300".into());
        }
        let (limits, graph_limits, mut policy) =
            fresh_run::stage6a_p0_archive_validation_contract(4 * 1024 * 1024 * 1024, false)?;
        policy.expected_build_class = producer_class.into();
        let restored = checkpoint::restore_checkpoint(path, &limits, &graph_limits, &policy)?;
        let state = restored.state.state();
        if state.population.len() < count {
            return Err("source population smaller than unique cohort".into());
        }
        let mut weights = Vec::with_capacity(count);
        let mut recurrent = Vec::with_capacity(count);
        let mut selection = Vec::with_capacity(count);
        for index in 0..count {
            let slot = index * state.population.len() / count;
            let genome = &state.population[slot];
            let brain = state
                .brains
                .iter()
                .find(|brain| brain.handle == genome.brain)
                .ok_or("validated population brain missing")?;
            weights.push(genome.weights.clone());
            recurrent.push(brain.recurrent.clone());
            selection.push(json!({"cohortSlot":index,"sourceSlot":genome.slot,
                "genomeId":genome.lineage.genome_id.to_string(),
                "birthGeneration":genome.lineage.birth_generation,
                "fitness":genome.fitness}));
        }
        Ok(Self {
            graph: GraphBundle::compile(restored.state.graph_spec().clone(), &graph_limits)?,
            weights,
            recurrent,
            settings: state.config.settings.clone(),
            provenance: json!({"sourceRun":state.identity.run_id,
                "sourceGeneration":state.generation.generation,
                "sourcePopulation":state.population.len(),
                "sourceBackend":state.identity.math_backend,
                "sourceBuildClass":state.identity.build_class,
                "sourceTarget":state.identity.target_triple,
                "sourceProfile":state.identity.build_profile,
                "sourceBuild":state.identity.engine_build_id,
                "logicalRootSha256":restored.content.logical_root_sha256,
                "selectionRule":"floor(cohortSlot * sourcePopulation / cohortSize)",
                "selection":selection,
                "transfer":"canonical F32 population transfer; not exact continuation"}),
        })
    }
}
