//! Test-hook-only retained fresh run-start persistence and activation session.
//!
//! The session uses the production [`PendingRunStartTransition`] and owns one
//! real generation-one boundary across N-API calls. It exposes only bounded
//! scalar proof and is absent from production addons.

use super::generation_handoff_fixture::{fixture_checkpoint_limits, fixture_run_start};
use super::graph::GraphBundle;
use super::inference_fixture::{graph_limits, scenario_graph, Stage4InferenceScenarioName};
use super::run_start::{PendingRunStartTransition, RunStartTransitionError};
use super::state::RunStartPublication;
use super::step_config::RunningStepWorkLimits;
use super::{checkpoint::CheckpointDescriptor, checkpoint::CheckpointOperationId};
use std::path::Path;
use std::sync::Arc;

/// Bounded state proving the fresh authority remains staged through durability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunStartHandoffSnapshot {
    /// Rust-owned nonzero handoff correlation token.
    pub transition_epoch: u64,
    /// Current generation.
    pub generation: u64,
    /// Current completed-step count.
    pub completed_step: u64,
    /// Whether one immutable descriptor has published.
    pub checkpoint_published: bool,
    /// Whether the exact SQLite acknowledgement is retained.
    pub persistence_acknowledged: bool,
    /// Whether collision-safe running authority has published.
    pub authority_published: bool,
    /// Current authority snake count.
    pub snake_count: usize,
    /// Current authority pellet count.
    pub pellet_count: usize,
    /// Physical checkpoint publications, excluding exact retries.
    pub checkpoint_publications: u32,
    /// Successful boundary-to-running publications.
    pub authority_publications: u32,
}

/// Retained owner of one real production run-start transition.
#[derive(Debug)]
pub struct RunStartHandoffFixtureSession {
    transition: PendingRunStartTransition,
    checkpoint_publications: u32,
    authority_publications: u32,
}

impl RunStartHandoffFixtureSession {
    /// Construct one admitted generation-one boundary without activating it.
    pub fn new() -> Result<Self, String> {
        if env!("SLITHER_NATIVE_BUILD_CLASS") != "test-hooks" {
            return Err("run-start handoff fixture requires a test-hooks native build".to_owned());
        }
        let graph_limits = graph_limits();
        let graph = Arc::new(
            GraphBundle::compile(
                scenario_graph(Stage4InferenceScenarioName::P0),
                &graph_limits,
            )
            .map_err(|error| format!("run-start handoff graph failed: {error}"))?,
        );
        let (candidate, policy) = fixture_run_start(&graph)?;
        let transition = PendingRunStartTransition::admit(
            candidate,
            graph,
            policy,
            fixture_checkpoint_limits(),
            graph_limits,
            RunningStepWorkLimits::provisional_defaults(),
        )
        .map_err(format_error)?;
        Ok(Self {
            transition,
            checkpoint_publications: 0,
            authority_publications: 0,
        })
    }

    /// Publish or exactly retry the real immutable run-start checkpoint.
    pub fn publish_checkpoint(
        &mut self,
        managed_directory: &Path,
        operation_id: CheckpointOperationId,
    ) -> Result<CheckpointDescriptor, String> {
        let was_published = self.transition.checkpoint_published();
        let descriptor = self
            .transition
            .publish_checkpoint(managed_directory, operation_id)
            .map_err(format_error)?;
        if !was_published {
            self.checkpoint_publications = self
                .checkpoint_publications
                .checked_add(1)
                .ok_or_else(|| "run-start checkpoint publication count overflowed".to_owned())?;
        }
        Ok(descriptor)
    }

    /// Apply one complete worker-echoed descriptor to the Rust barrier.
    pub fn acknowledge_persistence(
        &mut self,
        descriptor: &CheckpointDescriptor,
    ) -> Result<(), String> {
        self.transition
            .acknowledge_persistence(descriptor)
            .map_err(format_error)
    }

    /// Construct and publish the one initial running authority.
    pub fn publish_running_authority(&mut self) -> Result<RunStartPublication, String> {
        let publication = self
            .transition
            .publish_running_authority()
            .map_err(format_error)?;
        self.authority_publications = self
            .authority_publications
            .checked_add(1)
            .ok_or_else(|| "run-start authority publication count overflowed".to_owned())?;
        Ok(publication)
    }

    /// Inspect bounded scalar proof without copying authoritative arrays.
    #[must_use]
    pub fn snapshot(&self) -> RunStartHandoffSnapshot {
        RunStartHandoffSnapshot {
            transition_epoch: self.transition.transition_epoch(),
            generation: self.transition.generation(),
            completed_step: self.transition.completed_step(),
            checkpoint_published: self.transition.checkpoint_published(),
            persistence_acknowledged: self.transition.persistence_acknowledged(),
            authority_published: self.transition.authority_published(),
            snake_count: self.transition.snake_count(),
            pellet_count: self.transition.pellet_count(),
            checkpoint_publications: self.checkpoint_publications,
            authority_publications: self.authority_publications,
        }
    }
}

fn format_error(error: RunStartTransitionError) -> String {
    format!("run-start handoff failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "slither-run-start-{label}-{}-{id}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("unique run-start test directory must create");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).expect("run-start test directory must remove");
        }
    }

    #[test]
    fn retained_run_start_begins_as_an_unpublished_empty_boundary() {
        let fixture = RunStartHandoffFixtureSession::new().expect("fixture must construct");
        let snapshot = fixture.snapshot();
        assert_ne!(snapshot.transition_epoch, 0);
        assert_eq!(snapshot.generation, 1);
        assert_eq!(snapshot.completed_step, 0);
        assert!(!snapshot.checkpoint_published);
        assert!(!snapshot.persistence_acknowledged);
        assert!(!snapshot.authority_published);
        assert_eq!(snapshot.snake_count, 0);
        assert_eq!(snapshot.pellet_count, 0);
        assert_eq!(snapshot.checkpoint_publications, 0);
        assert_eq!(snapshot.authority_publications, 0);
    }

    #[test]
    fn exact_descriptor_acknowledgement_is_the_only_path_to_one_running_authority() {
        let directory = TestDirectory::new("barrier");
        let mut fixture = RunStartHandoffFixtureSession::new().expect("fixture must construct");
        assert!(fixture.publish_running_authority().is_err());

        let operation = CheckpointOperationId::parse("30303030303030303030303030303030")
            .expect("operation must parse");
        let descriptor = fixture
            .publish_checkpoint(&directory.path, operation.clone())
            .expect("checkpoint must publish");
        assert_eq!(
            fixture
                .publish_checkpoint(&directory.path, operation)
                .expect("same operation must retry"),
            descriptor
        );
        assert!(fixture
            .publish_checkpoint(
                &directory.path,
                CheckpointOperationId::parse("31313131313131313131313131313131")
                    .expect("operation must parse"),
            )
            .is_err());

        let mut mismatched = descriptor.clone();
        mismatched.logical_root_sha256 = "0".repeat(64);
        let error = fixture
            .acknowledge_persistence(&mismatched)
            .expect_err("mismatched acknowledgement must fail");
        assert!(error.contains("logical root"));
        assert!(!fixture.snapshot().persistence_acknowledged);
        assert!(!fixture.snapshot().authority_published);
        assert_eq!(fixture.snapshot().snake_count, 0);

        fixture
            .acknowledge_persistence(&descriptor)
            .expect("exact acknowledgement must succeed");
        assert!(fixture.snapshot().persistence_acknowledged);
        assert!(!fixture.snapshot().authority_published);
        let publication = fixture
            .publish_running_authority()
            .expect("durable boundary must activate");
        assert_eq!(publication.generation, 1);
        assert_eq!(publication.completed_step, 0);
        assert!(fixture.snapshot().authority_published);
        assert!(fixture.snapshot().snake_count > 0);
        assert!(fixture.snapshot().pellet_count > 0);
        assert_eq!(fixture.snapshot().checkpoint_publications, 1);
        assert_eq!(fixture.snapshot().authority_publications, 1);
        assert!(fixture.publish_running_authority().is_err());
        assert_eq!(fixture.snapshot().authority_publications, 1);
    }
}
