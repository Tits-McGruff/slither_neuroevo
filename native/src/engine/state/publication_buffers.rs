//! Publication buffers for authoritative state.

use super::*;

pub(super) fn running_step_buffer_identity(
    replacement: &RunningStepReplacement<'_>,
) -> RunningStepBufferIdentity {
    RunningStepBufferIdentity {
        world: std::ptr::from_ref(&*replacement.world).addr(),
        rng: std::ptr::from_ref(&*replacement.rng).addr(),
        allocators: std::ptr::from_ref(&*replacement.allocators).addr(),
        brains: std::ptr::from_ref(&*replacement.brains).addr(),
        evolved_generation: std::ptr::from_ref(&*replacement.evolved_generation).addr(),
        baseline_lifecycle: std::ptr::from_ref(&*replacement.baseline_lifecycle).addr(),
        ambient_pellet_accumulator: replacement.ambient_pellet_accumulator.to_bits(),
        sensor_generation_best: replacement
            .sensor_generation
            .best_points_this_generation()
            .to_bits(),
        generation_elapsed_seconds: replacement.generation_elapsed_seconds.to_bits(),
        wall_accumulator_seconds: replacement.wall_accumulator_seconds.to_bits(),
        mutation_proof: replacement.mutation.proof_identity(),
        baseline_v2_proof: replacement
            .mutation
            .baseline_v2
            .map(|proof| proof.identity()),
        evolved_mutation_proof: replacement
            .mutation
            .evolved
            .map_or(0, |proof| std::ptr::from_ref(proof).addr()),
    }
}

pub(super) fn generation_start_buffer_identity(
    replacement: &GenerationStartReplacement<'_>,
) -> GenerationStartBufferIdentity {
    GenerationStartBufferIdentity {
        world: std::ptr::from_ref(&*replacement.world).addr(),
        rng: std::ptr::from_ref(&*replacement.rng).addr(),
        allocators: std::ptr::from_ref(&*replacement.allocators).addr(),
        brains: std::ptr::from_ref(&*replacement.brains).addr(),
        fixed_step: std::ptr::from_ref(&*replacement.fixed_step).addr(),
        wall_accumulator_seconds: replacement.wall_accumulator_seconds.to_bits(),
        proof: std::ptr::from_ref(replacement.proof).addr(),
    }
}

pub(super) fn swap_generation_start_buffers(
    candidate: &mut StateCandidate,
    replacement: &mut GenerationStartReplacement<'_>,
) {
    std::mem::swap(&mut candidate.world, replacement.world);
    std::mem::swap(&mut candidate.rng, replacement.rng);
    std::mem::swap(&mut candidate.allocators, replacement.allocators);
    std::mem::swap(&mut candidate.brains, replacement.brains);
    std::mem::swap(&mut candidate.fixed_step, replacement.fixed_step);
}

pub(super) fn swap_initial_run_start_buffers(
    candidate: &mut StateCandidate,
    replacement: &mut InitialRunStartReplacement<'_>,
) {
    std::mem::swap(&mut candidate.world, replacement.world);
    std::mem::swap(&mut candidate.rng, replacement.rng);
    std::mem::swap(&mut candidate.allocators, replacement.allocators);
    std::mem::swap(&mut candidate.fixed_step, replacement.fixed_step);
}

/// Exchange only fields a running fixed step is allowed to replace.
pub(super) fn swap_running_step_buffers(
    candidate: &mut StateCandidate,
    replacement: &mut RunningStepReplacement<'_>,
) {
    std::mem::swap(&mut candidate.world, replacement.world);
    std::mem::swap(&mut candidate.rng, replacement.rng);
    std::mem::swap(&mut candidate.allocators, replacement.allocators);
    std::mem::swap(&mut candidate.brains, replacement.brains);
    std::mem::swap(
        &mut candidate.fixed_step.evolved_generation,
        replacement.evolved_generation,
    );
    std::mem::swap(
        &mut candidate.fixed_step.baseline_lifecycle,
        replacement.baseline_lifecycle,
    );
}
