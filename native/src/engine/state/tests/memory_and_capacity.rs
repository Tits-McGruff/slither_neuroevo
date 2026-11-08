use super::*;

#[test]
fn retired_budget_reclamation_preserves_state_and_rejects_invalid_context() {
    let graph = default_graph();
    let candidate = complete_running_candidate(&graph);
    let mut authority = own_complete_running(candidate, graph);
    let before = authority.state().clone();
    let original = authority.memory_ceiling_bytes();
    assert!(authority
        .reclaim_retired_memory_budget(original - 1)
        .is_err());
    assert_eq!(authority.memory_ceiling_bytes(), original);
    authority
        .reclaim_retired_memory_budget(original + 1)
        .unwrap();
    assert_eq!(authority.memory_ceiling_bytes(), original + 1);
    assert_eq!(authority.state(), &before);
    authority.candidate.phase =
        AuthorityPhase::GenerationBoundary(GenerationBoundaryKind::RunStart);
    assert!(authority
        .reclaim_retired_memory_budget(original + 2)
        .is_err());
    assert_eq!(authority.memory_ceiling_bytes(), original + 1);
}

#[test]
fn terminal_memory_rejection_precedes_evolution_and_preserves_authority() {
    use crate::engine::running_step::{
        RunningStepCoordinator, RunningStepError, RunningStepInputs,
    };
    use crate::engine::step_config::RunningStepWorkLimits;
    let graph = default_graph();
    let source = complete_running_candidate(&graph);
    let mut authority = own_complete_running(source, Arc::clone(&graph));
    let before = authority.state().clone();
    let peak = generation_numeric_peak_bytes(
        authority.memory.total_bytes,
        before.population.len(),
        &graph,
    )
    .unwrap();
    authority.memory_ceiling_bytes = peak - 1;
    let mut coordinator =
        RunningStepCoordinator::try_new(&authority, RunningStepWorkLimits::provisional_defaults())
            .unwrap();
    coordinator.request_manual_finish();
    let result = coordinator.advance_nonterminal(
        &mut authority,
        RunningStepInputs {
            wall_now_ms: 100,
            wall_accumulator_seconds: 0.0,
        },
    );
    assert!(matches!(result, Err(RunningStepError::State(error))
        if matches!(*error, StateError::MemoryCeilingExceeded {estimated_bytes,ceiling_bytes}
            if estimated_bytes==peak && ceiling_bytes==peak-1)));
    assert_eq!(coordinator.last_step_cost().generation_evolution, 0);
    assert!(coordinator.pending_generation_transition().is_none());
    assert_eq!(authority.state(), &before);
    assert!(generation_numeric_peak_bytes(usize::MAX, 1, &graph).is_err());
}

#[test]
fn requested_capacity_and_scratch_are_charged_before_authority() {
    let graph = default_graph();
    let compact = candidate(&graph, 1);
    let compact_estimate = estimate_state_memory(&compact, &graph).unwrap();
    assert_eq!(
        compact_estimate.scratch_bytes,
        compact.config.worker_scratch_bytes
            + compact.config.checkpoint_scratch_bytes
            + std::mem::size_of::<f64>()
    );

    let mut expanded = candidate(&graph, 1);
    expanded.config.max_body_points += 500_000;
    expanded.config.max_pellets += 50_000;
    expanded.config.spatial_index_bytes += 8 * 1024 * 1024;
    expanded.config.worker_scratch_bytes += 4 * 1024 * 1024;
    refresh_config_hash(&mut expanded);
    let expanded_estimate = estimate_state_memory(&expanded, &graph).unwrap();
    assert!(expanded_estimate.structural_bytes > compact_estimate.structural_bytes);
    assert!(expanded_estimate.frame_bytes > compact_estimate.frame_bytes);
    assert!(expanded_estimate.spatial_bytes > compact_estimate.spatial_bytes);
    assert!(expanded_estimate.scratch_bytes > compact_estimate.scratch_bytes);
    assert!(matches!(
        own(expanded, graph, compact_estimate.total_bytes),
        Err(StateError::MemoryCeilingExceeded { .. })
    ));
}

#[test]
fn checked_memory_overflow_and_ceiling_fail_before_authority() {
    assert!(matches!(
        checked_allocation_bytes(usize::MAX, 2, "test overflow"),
        Err(StateError::ArithmeticOverflow {
            context: "test overflow"
        })
    ));

    let graph = default_graph();
    let state_candidate = candidate(&graph, 2);
    let estimate = estimate_state_memory(&state_candidate, &graph).unwrap();
    assert!(matches!(
        own(
            state_candidate,
            Arc::clone(&graph),
            estimate.total_bytes - 1
        ),
        Err(StateError::MemoryCeilingExceeded { .. })
    ));

    let compact = candidate(&graph, 1);
    let compact_estimate = estimate_state_memory(&compact, &graph).unwrap();
    let mut over_reserved = candidate(&graph, 1);
    over_reserved.identity.run_id.reserve(4_096);
    let reserved_estimate = estimate_state_memory(&over_reserved, &graph).unwrap();
    assert!(reserved_estimate.text_bytes >= compact_estimate.text_bytes + 4_096);
}
