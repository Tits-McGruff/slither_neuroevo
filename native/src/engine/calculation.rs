//! Reusable deterministic calculation work, scratch, and proposal contracts.
//!
//! This module intentionally contains no sensing, neural, movement, or
//! collision math. It resolves due work from immutable authoritative-state
//! records, owns fixed-capacity workspace allocations, exposes disjoint slices
//! for scalar workers, and withholds proposal values until the coordinator can
//! seal a complete batch.

use super::state::{BrainHandle, BrainOwner, BrainRuntimeState, PopulationGenome, SnakeState};
use std::error::Error;
use std::fmt;
use std::mem::size_of;

/// Exact identity of one calculation phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalculationBatchKey {
    generation: u64,
    step: u64,
    population_epoch: u64,
}

impl CalculationBatchKey {
    /// Construct one exact phase identity without narrowing any field.
    pub const fn new(generation: u64, step: u64, population_epoch: u64) -> Self {
        Self {
            generation,
            step,
            population_epoch,
        }
    }

    /// Current generation identity.
    pub const fn generation(self) -> u64 {
        self.generation
    }

    /// Fixed-step identity selected by the coordinator.
    pub const fn step(self) -> u64 {
        self.step
    }

    /// Population/brain epoch required by every work item.
    pub const fn population_epoch(self) -> u64 {
        self.population_epoch
    }
}

/// Due indexes collected into the workspace before immutable-state resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalculationCandidateIndex {
    snake_index: usize,
    brain_index: usize,
}

impl CalculationCandidateIndex {
    /// Construct indexes that must later resolve against immutable state views.
    pub const fn new(snake_index: usize, brain_index: usize) -> Self {
        Self {
            snake_index,
            brain_index,
        }
    }

    /// Snake-array index to resolve.
    pub const fn snake_index(self) -> usize {
        self.snake_index
    }

    /// Brain-array index to resolve.
    pub const fn brain_index(self) -> usize {
        self.brain_index
    }
}

/// Immutable, state-resolved work passed to a scalar executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalculationWorkUnit {
    ordinal: usize,
    snake_index: usize,
    brain_index: usize,
    snake_id: u64,
    brain: BrainHandle,
    population_slot: Option<u32>,
}

impl CalculationWorkUnit {
    /// Dense position in deterministic batch order.
    pub const fn ordinal(self) -> usize {
        self.ordinal
    }

    /// Resolved snake index in the immutable state view.
    pub const fn snake_index(self) -> usize {
        self.snake_index
    }

    /// Resolved brain index in the immutable state view.
    pub const fn brain_index(self) -> usize {
        self.brain_index
    }

    /// Exact stable snake identity.
    pub const fn snake_id(self) -> u64 {
        self.snake_id
    }

    /// Exact stable brain handle and epoch.
    pub const fn brain(self) -> BrainHandle {
        self.brain
    }

    /// Dense evolved-population slot, if applicable.
    pub const fn population_slot(self) -> Option<u32> {
        self.population_slot
    }

    /// Bind this unit to one exact calculation phase.
    pub const fn identity(self, batch: CalculationBatchKey) -> CalculationWorkIdentity {
        CalculationWorkIdentity {
            batch,
            ordinal: self.ordinal,
            snake_id: self.snake_id,
            brain: self.brain,
        }
    }
}

/// Exact identity retained with one staged calculation value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalculationWorkIdentity {
    batch: CalculationBatchKey,
    ordinal: usize,
    snake_id: u64,
    brain: BrainHandle,
}

impl CalculationWorkIdentity {
    /// Batch that produced the proposal.
    pub const fn batch(self) -> CalculationBatchKey {
        self.batch
    }

    /// Dense deterministic result position.
    pub const fn ordinal(self) -> usize {
        self.ordinal
    }

    /// Exact stable snake identity.
    pub const fn snake_id(self) -> u64 {
        self.snake_id
    }

    /// Exact stable brain identity and epoch.
    pub const fn brain(self) -> BrainHandle {
        self.brain
    }
}

/// One contiguous half-open range in deterministic work order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkRange {
    start: usize,
    end: usize,
}

impl WorkRange {
    /// Inclusive start position.
    pub const fn start(self) -> usize {
        self.start
    }

    /// Exclusive end position.
    pub const fn end(self) -> usize {
        self.end
    }

    /// Number of work items in this range.
    pub const fn len(self) -> usize {
        self.end - self.start
    }

    /// Whether the range contains no work.
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Calculate one balanced contiguous partition without allocating.
pub fn work_range(
    work_count: usize,
    partition_index: usize,
    partition_count: usize,
) -> Result<WorkRange, CalculationError> {
    if partition_count == 0 {
        return Err(CalculationError::ZeroPartitionCount);
    }
    if partition_index >= partition_count {
        return Err(CalculationError::InvalidPartitionIndex {
            partition_index,
            partition_count,
        });
    }
    let base = work_count / partition_count;
    let remainder = work_count % partition_count;
    let base_start =
        partition_index
            .checked_mul(base)
            .ok_or(CalculationError::ArithmeticOverflow {
                context: "work partition base start",
            })?;
    let start = base_start
        .checked_add(partition_index.min(remainder))
        .ok_or(CalculationError::ArithmeticOverflow {
            context: "work partition start",
        })?;
    let length = base + usize::from(partition_index < remainder);
    let end = start
        .checked_add(length)
        .ok_or(CalculationError::ArithmeticOverflow {
            context: "work partition end",
        })?;
    Ok(WorkRange { start, end })
}

/// Fixed counts for one worker's reusable typed scratch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalculationScratchLayout {
    /// Complete-graph activation floats for one work item.
    pub activation_floats: usize,
    /// Gathered-input floats required by graph nodes.
    pub gather_floats: usize,
    /// Backend temporary floats for the complete scalar operation.
    pub temporary_floats: usize,
}

impl CalculationScratchLayout {
    /// Checked logical bytes required by all typed slices.
    pub fn required_bytes(self) -> Result<usize, CalculationError> {
        let floats = self
            .activation_floats
            .checked_add(self.gather_floats)
            .and_then(|count| count.checked_add(self.temporary_floats))
            .ok_or(CalculationError::ArithmeticOverflow {
                context: "calculation scratch float count",
            })?;
        floats
            .checked_mul(size_of::<f32>())
            .ok_or(CalculationError::ArithmeticOverflow {
                context: "calculation scratch bytes",
            })
    }
}

/// One worker's fixed scratch allocation.
#[derive(Debug)]
pub struct CalculationScratch {
    layout: CalculationScratchLayout,
    activation: Vec<f32>,
    gather: Vec<f32>,
    temporary: Vec<f32>,
    allocated_bytes: usize,
}

impl CalculationScratch {
    /// Allocate fixed typed scratch after checked byte admission.
    pub fn try_new(
        layout: CalculationScratchLayout,
        budget_bytes: usize,
    ) -> Result<Self, CalculationError> {
        let required_bytes = layout.required_bytes()?;
        if required_bytes > budget_bytes {
            return Err(CalculationError::WorkspaceBudgetExceeded {
                required_bytes,
                budget_bytes,
            });
        }
        let activation = try_zeroed_floats(layout.activation_floats, "activation scratch")?;
        let gather = try_zeroed_floats(layout.gather_floats, "gather scratch")?;
        let temporary = try_zeroed_floats(layout.temporary_floats, "temporary scratch")?;
        let allocated_bytes = checked_float_capacity_bytes([&activation, &gather, &temporary])?;
        if allocated_bytes > budget_bytes {
            return Err(CalculationError::WorkspaceBudgetExceeded {
                required_bytes: allocated_bytes,
                budget_bytes,
            });
        }
        Ok(Self {
            layout,
            activation,
            gather,
            temporary,
            allocated_bytes,
        })
    }

    /// Fixed logical layout selected at allocation time.
    pub const fn layout(&self) -> CalculationScratchLayout {
        self.layout
    }

    /// Bytes charged from actual backing capacities.
    pub const fn allocated_bytes(&self) -> usize {
        self.allocated_bytes
    }

    /// Borrow fixed slices without exposing a growable vector.
    pub fn view(&mut self) -> CalculationScratchView<'_> {
        CalculationScratchView {
            activation: &mut self.activation,
            gather: &mut self.gather,
            temporary: &mut self.temporary,
        }
    }
}

/// Fixed mutable scratch slices visible to one scalar work callback.
pub struct CalculationScratchView<'a> {
    /// Complete-graph activation storage.
    pub activation: &'a mut [f32],
    /// Input-gather storage.
    pub gather: &'a mut [f32],
    /// Operation-specific temporary storage.
    pub temporary: &'a mut [f32],
}

/// One staged value paired with its exact batch/work identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalculationResult<T> {
    identity: CalculationWorkIdentity,
    value: T,
}

impl<T> CalculationResult<T> {
    /// Exact identity that produced this value.
    pub const fn identity(&self) -> CalculationWorkIdentity {
        self.identity
    }

    /// Proposed value.
    pub const fn value(&self) -> &T {
        &self.value
    }
}

/// Private-content proposal slot that workers may receive only as disjoint slices.
pub struct CalculationProposalSlot<T> {
    result: Option<CalculationResult<T>>,
}

impl<T> CalculationProposalSlot<T> {
    fn empty() -> Self {
        Self { result: None }
    }
}

impl<T> fmt::Debug for CalculationProposalSlot<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CalculationProposalSlot")
            .field("occupied", &self.result.is_some())
            .finish()
    }
}

/// Borrowed complete view; creation proves every active slot is present.
#[derive(Debug)]
pub struct CompletedCalculationView<'a, T> {
    key: CalculationBatchKey,
    slots: &'a [CalculationProposalSlot<T>],
}

/// Disjoint active buffers borrowed for one coordinator-controlled execution.
pub struct CalculationExecutionBuffers<'a, T> {
    /// Exact phase identity required by every proposal.
    pub key: CalculationBatchKey,
    /// Deterministically ordered immutable work.
    pub work: &'a [CalculationWorkUnit],
    /// Matching proposal slots, ready to split with `work` into disjoint ranges.
    pub proposals: &'a mut [CalculationProposalSlot<T>],
    /// One independently owned scratch allocation per configured worker.
    pub scratches: &'a mut [CalculationScratch],
}

impl<T> CompletedCalculationView<'_, T> {
    /// Exact phase identity shared by every result.
    pub const fn key(&self) -> CalculationBatchKey {
        self.key
    }

    /// Number of complete results.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether this complete batch contains no work.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Read one complete result by deterministic ordinal.
    pub fn result(&self, ordinal: usize) -> Option<&CalculationResult<T>> {
        self.slots
            .get(ordinal)
            .and_then(|slot| slot.result.as_ref())
    }

    /// Iterate complete results in deterministic ordinal order.
    pub fn iter(&self) -> impl Iterator<Item = &CalculationResult<T>> {
        self.slots.iter().map(|slot| {
            slot.result
                .as_ref()
                .expect("sealed calculation workspace invariant")
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MappingIdentity {
    SnakeId(u64),
    SnakeIndex(usize),
    BrainHandle(BrainHandle),
    BrainIndex(usize),
    PopulationSlot(u32),
}

/// One admitted, reusable allocation set for scalar calculation phases.
///
/// The workspace itself does not allocate during a successful phase after
/// construction. Callers must likewise use fixed-size proposal values and an
/// allocation-free calculation callback when applying this contract to a hot
/// authoritative path; allocations owned by generic `T` or callback code are
/// outside the workspace's memory accounting.
pub struct CalculationWorkspace<T> {
    max_work: usize,
    budget_bytes: usize,
    allocated_bytes: usize,
    key: Option<CalculationBatchKey>,
    prepared: bool,
    candidates: Vec<CalculationCandidateIndex>,
    work: Vec<CalculationWorkUnit>,
    validation: Vec<MappingIdentity>,
    proposals: Vec<CalculationProposalSlot<T>>,
    scratches: Vec<CalculationScratch>,
}

impl<T> fmt::Debug for CalculationWorkspace<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CalculationWorkspace")
            .field("max_work", &self.max_work)
            .field("worker_count", &self.scratches.len())
            .field("active_candidates", &self.candidates.len())
            .field("active_work", &self.work.len())
            .field("prepared", &self.prepared)
            .field("budget_bytes", &self.budget_bytes)
            .field("allocated_bytes", &self.allocated_bytes)
            .finish_non_exhaustive()
    }
}

impl<T> CalculationWorkspace<T> {
    /// Checked requested bytes for fixed workspace buffers and all worker scratch.
    pub fn required_bytes(
        max_work: usize,
        worker_count: usize,
        scratch_layout: CalculationScratchLayout,
    ) -> Result<usize, CalculationError> {
        if worker_count == 0 {
            return Err(CalculationError::ZeroWorkerCount);
        }
        let validation_count =
            max_work
                .checked_mul(5)
                .ok_or(CalculationError::ArithmeticOverflow {
                    context: "workspace validation identity count",
                })?;
        let scratch_bytes = scratch_layout
            .required_bytes()?
            .checked_mul(worker_count)
            .ok_or(CalculationError::ArithmeticOverflow {
                context: "combined worker scratch bytes",
            })?;
        checked_sum(&[
            size_of::<Self>(),
            checked_element_bytes::<CalculationCandidateIndex>(max_work, "candidate indexes")?,
            checked_element_bytes::<CalculationWorkUnit>(max_work, "work units")?,
            checked_element_bytes::<MappingIdentity>(validation_count, "mapping validation")?,
            checked_element_bytes::<CalculationProposalSlot<T>>(max_work, "proposal slots")?,
            checked_element_bytes::<CalculationScratch>(worker_count, "worker scratch records")?,
            scratch_bytes,
        ])
    }

    /// Allocate all buffers once and reject actual backing capacity over budget.
    pub fn try_new(
        max_work: usize,
        worker_count: usize,
        scratch_layout: CalculationScratchLayout,
        budget_bytes: usize,
    ) -> Result<Self, CalculationError> {
        let required_bytes = Self::required_bytes(max_work, worker_count, scratch_layout)?;
        if required_bytes > budget_bytes {
            return Err(CalculationError::WorkspaceBudgetExceeded {
                required_bytes,
                budget_bytes,
            });
        }

        let candidates = try_vec_with_capacity(max_work, "candidate indexes")?;
        let work = try_vec_with_capacity(max_work, "work units")?;
        let validation_capacity =
            max_work
                .checked_mul(5)
                .ok_or(CalculationError::ArithmeticOverflow {
                    context: "workspace validation identity capacity",
                })?;
        let validation = try_vec_with_capacity(validation_capacity, "mapping validation")?;
        let mut proposals = try_vec_with_capacity(max_work, "proposal slots")?;
        proposals.resize_with(max_work, CalculationProposalSlot::empty);
        let mut scratches = try_vec_with_capacity(worker_count, "worker scratch records")?;
        for _ in 0..worker_count {
            scratches.push(CalculationScratch::try_new(
                scratch_layout,
                scratch_layout.required_bytes()?,
            )?);
        }

        let mut workspace = Self {
            max_work,
            budget_bytes,
            allocated_bytes: 0,
            key: None,
            prepared: false,
            candidates,
            work,
            validation,
            proposals,
            scratches,
        };
        workspace.allocated_bytes = workspace.actual_allocated_bytes()?;
        if workspace.allocated_bytes > budget_bytes {
            return Err(CalculationError::WorkspaceBudgetExceeded {
                required_bytes: workspace.allocated_bytes,
                budget_bytes,
            });
        }
        Ok(workspace)
    }

    /// Maximum due work admitted at construction.
    pub const fn max_work(&self) -> usize {
        self.max_work
    }

    /// Number of independent worker scratch allocations.
    pub fn worker_count(&self) -> usize {
        self.scratches.len()
    }

    /// Bytes charged from actual vector capacities and nested scratch buffers.
    pub const fn allocated_bytes(&self) -> usize {
        self.allocated_bytes
    }

    /// Begin a new phase while retaining every allocation.
    pub fn begin(&mut self, key: CalculationBatchKey) {
        for slot in &mut self.proposals[..self.work.len()] {
            slot.result = None;
        }
        self.key = Some(key);
        self.prepared = false;
        self.candidates.clear();
        self.work.clear();
        self.validation.clear();
    }

    /// Append one due index pair without growing the candidate buffer.
    pub fn try_push_candidate(
        &mut self,
        snake_index: usize,
        brain_index: usize,
    ) -> Result<(), CalculationError> {
        if self.key.is_none() {
            return Err(CalculationError::WorkspaceNotBegun);
        }
        if self.prepared {
            return Err(CalculationError::WorkspaceAlreadyPrepared);
        }
        if self.candidates.len() >= self.max_work {
            return Err(CalculationError::WorkCapacityExceeded {
                requested: self.candidates.len().saturating_add(1),
                capacity: self.max_work,
            });
        }
        self.candidates
            .push(CalculationCandidateIndex::new(snake_index, brain_index));
        Ok(())
    }

    /// Resolve due indexes from actual immutable state and prepare stable work.
    pub fn prepare(
        &mut self,
        snakes: &[SnakeState],
        brains: &[BrainRuntimeState],
        population: &[PopulationGenome],
    ) -> Result<(), CalculationError> {
        let result = self.prepare_inner(snakes, brains, population);
        if result.is_err() {
            self.prepared = false;
            self.work.clear();
            self.validation.clear();
        }
        result
    }

    fn prepare_inner(
        &mut self,
        snakes: &[SnakeState],
        brains: &[BrainRuntimeState],
        population: &[PopulationGenome],
    ) -> Result<(), CalculationError> {
        let key = self.key.ok_or(CalculationError::WorkspaceNotBegun)?;
        if self.prepared {
            return Err(CalculationError::WorkspaceAlreadyPrepared);
        }
        debug_assert!(self.work.capacity() >= self.max_work);
        debug_assert!(self.validation.capacity() >= self.max_work.saturating_mul(5));

        for candidate in &self.candidates {
            let snake = snakes.get(candidate.snake_index).ok_or(
                CalculationError::SnakeIndexOutOfBounds {
                    index: candidate.snake_index,
                    snake_count: snakes.len(),
                },
            )?;
            let brain = brains.get(candidate.brain_index).ok_or(
                CalculationError::BrainIndexOutOfBounds {
                    index: candidate.brain_index,
                    brain_count: brains.len(),
                },
            )?;
            let requested = snake.brain.ok_or(CalculationError::SnakeHasNoBrain {
                snake_id: snake.id,
                snake_index: candidate.snake_index,
            })?;
            if requested != brain.handle {
                return Err(CalculationError::BrainMappingMismatch {
                    snake_id: snake.id,
                    requested,
                    mapped: brain.handle,
                });
            }
            if brain.handle.epoch != key.population_epoch {
                return Err(CalculationError::StaleBrainEpoch {
                    brain: brain.handle,
                    expected_epoch: key.population_epoch,
                });
            }

            match snake.population_slot {
                Some(slot) => {
                    if brain.owner != BrainOwner::PopulationSlot(slot) {
                        return Err(CalculationError::BrainOwnerMismatch {
                            brain: brain.handle,
                            expected: BrainOwner::PopulationSlot(slot),
                            mapped: brain.owner,
                        });
                    }
                    let slot_index = usize::try_from(slot).map_err(|_| {
                        CalculationError::PopulationSlotOutOfBounds {
                            slot,
                            population_count: population.len(),
                        }
                    })?;
                    let genome = population.get(slot_index).ok_or(
                        CalculationError::PopulationSlotOutOfBounds {
                            slot,
                            population_count: population.len(),
                        },
                    )?;
                    if genome.slot != slot {
                        return Err(CalculationError::PopulationSlotRecordMismatch {
                            expected_slot: slot,
                            recorded_slot: genome.slot,
                        });
                    }
                    if genome.brain != brain.handle {
                        return Err(CalculationError::PopulationBrainMismatch {
                            slot,
                            expected: genome.brain,
                            mapped: brain.handle,
                        });
                    }
                }
                None => {
                    let expected = BrainOwner::Entity(snake.id);
                    if brain.owner != expected {
                        return Err(CalculationError::BrainOwnerMismatch {
                            brain: brain.handle,
                            expected,
                            mapped: brain.owner,
                        });
                    }
                }
            }

            self.work.push(CalculationWorkUnit {
                ordinal: 0,
                snake_index: candidate.snake_index,
                brain_index: candidate.brain_index,
                snake_id: snake.id,
                brain: brain.handle,
                population_slot: snake.population_slot,
            });
        }

        self.work.sort_unstable_by_key(|unit| {
            (
                unit.brain.epoch,
                unit.brain.id,
                unit.snake_id,
                unit.snake_index,
                unit.brain_index,
                unit.population_slot,
            )
        });
        for (ordinal, unit) in self.work.iter_mut().enumerate() {
            unit.ordinal = ordinal;
            self.validation
                .push(MappingIdentity::SnakeId(unit.snake_id));
            self.validation
                .push(MappingIdentity::SnakeIndex(unit.snake_index));
            self.validation
                .push(MappingIdentity::BrainHandle(unit.brain));
            self.validation
                .push(MappingIdentity::BrainIndex(unit.brain_index));
            if let Some(slot) = unit.population_slot {
                self.validation.push(MappingIdentity::PopulationSlot(slot));
            }
        }
        self.validation.sort_unstable();
        for pair in self.validation.windows(2) {
            if pair[0] == pair[1] {
                return Err(duplicate_mapping_error(pair[0]));
            }
        }
        for slot in &mut self.proposals[..self.work.len()] {
            slot.result = None;
        }
        self.prepared = true;
        Ok(())
    }

    /// Borrow disjoint top-level buffers for coordinator-controlled partitioning.
    ///
    /// The caller may split `work` and `proposals` into matching disjoint slices
    /// and move each pair plus one distinct scratch value into scoped workers.
    pub fn execution_buffers(
        &mut self,
    ) -> Result<CalculationExecutionBuffers<'_, T>, CalculationError> {
        if !self.prepared {
            return Err(CalculationError::WorkspaceNotPrepared);
        }
        let key = self.key.ok_or(CalculationError::WorkspaceNotBegun)?;
        let active = self.work.len();
        Ok(CalculationExecutionBuffers {
            key,
            work: &self.work,
            proposals: &mut self.proposals[..active],
            scratches: &mut self.scratches,
        })
    }

    /// Borrow the deterministic work order after immutable-state resolution.
    ///
    /// This read-only view is used by coordinator-owned staging and commit
    /// code after worker scratch has been returned. It never exposes the
    /// private candidate or proposal storage.
    pub fn prepared_work(&self) -> Result<&[CalculationWorkUnit], CalculationError> {
        if !self.prepared {
            return Err(CalculationError::WorkspaceNotPrepared);
        }
        Ok(&self.work)
    }

    /// Seal a complete borrowed view without consuming any workspace allocation.
    pub fn seal(
        &self,
        expected_key: CalculationBatchKey,
    ) -> Result<CompletedCalculationView<'_, T>, CalculationError> {
        if !self.prepared {
            return Err(CalculationError::WorkspaceNotPrepared);
        }
        let actual = self.key.ok_or(CalculationError::WorkspaceNotBegun)?;
        if actual != expected_key {
            return Err(CalculationError::BatchKeyMismatch {
                expected: expected_key,
                actual,
            });
        }
        let active = &self.proposals[..self.work.len()];
        for (ordinal, (unit, slot)) in self.work.iter().zip(active).enumerate() {
            let result = slot
                .result
                .as_ref()
                .ok_or(CalculationError::IncompleteBatch {
                    completed: active.iter().filter(|slot| slot.result.is_some()).count(),
                    expected: active.len(),
                })?;
            let expected_identity = unit.identity(actual);
            if result.identity != expected_identity || result.identity.ordinal != ordinal {
                return Err(CalculationError::ProposalIdentityMismatch {
                    ordinal,
                    expected: expected_identity,
                    actual: result.identity,
                });
            }
        }
        Ok(CompletedCalculationView {
            key: actual,
            slots: active,
        })
    }

    fn actual_allocated_bytes(&self) -> Result<usize, CalculationError> {
        let scratch_backing = self.scratches.iter().try_fold(0usize, |total, scratch| {
            total.checked_add(scratch.allocated_bytes()).ok_or(
                CalculationError::ArithmeticOverflow {
                    context: "actual combined scratch backing bytes",
                },
            )
        })?;
        checked_sum(&[
            size_of::<Self>(),
            checked_capacity_bytes::<CalculationCandidateIndex>(
                self.candidates.capacity(),
                "candidate index capacity",
            )?,
            checked_capacity_bytes::<CalculationWorkUnit>(self.work.capacity(), "work capacity")?,
            checked_capacity_bytes::<MappingIdentity>(
                self.validation.capacity(),
                "validation capacity",
            )?,
            checked_capacity_bytes::<CalculationProposalSlot<T>>(
                self.proposals.capacity(),
                "proposal capacity",
            )?,
            checked_capacity_bytes::<CalculationScratch>(
                self.scratches.capacity(),
                "scratch record capacity",
            )?,
            scratch_backing,
        ])
    }
}

/// Execute one already-disjoint partition into its matching proposal slice.
pub fn execute_partition<T, E, F>(
    key: CalculationBatchKey,
    work: &[CalculationWorkUnit],
    proposals: &mut [CalculationProposalSlot<T>],
    scratch: &mut CalculationScratch,
    mut calculate: F,
) -> Result<(), ExecutePartitionError<E>>
where
    F: FnMut(&CalculationWorkUnit, &mut CalculationScratchView<'_>) -> Result<T, E>,
{
    if work.len() != proposals.len() {
        return Err(ExecutePartitionError::Contract(Box::new(
            CalculationError::PartitionLengthMismatch {
                work: work.len(),
                proposals: proposals.len(),
            },
        )));
    }
    if let Some((offset, _)) = proposals
        .iter()
        .enumerate()
        .find(|(_, slot)| slot.result.is_some())
    {
        return Err(ExecutePartitionError::Contract(Box::new(
            CalculationError::ProposalAlreadyPresent {
                ordinal: work.get(offset).map_or(offset, |unit| unit.ordinal),
            },
        )));
    }
    if let Some(first) = work.first() {
        for (offset, unit) in work.iter().enumerate() {
            let expected = first.ordinal.checked_add(offset).ok_or_else(|| {
                ExecutePartitionError::Contract(Box::new(CalculationError::ArithmeticOverflow {
                    context: "partition ordinal",
                }))
            })?;
            if unit.ordinal != expected {
                return Err(ExecutePartitionError::Contract(Box::new(
                    CalculationError::NonContiguousWorkSlice {
                        expected_ordinal: expected,
                        actual_ordinal: unit.ordinal,
                    },
                )));
            }
        }
    }

    for (offset, (unit, slot)) in work.iter().zip(proposals.iter_mut()).enumerate() {
        let value = {
            let mut view = scratch.view();
            match calculate(unit, &mut view) {
                Ok(value) => value,
                Err(source) => {
                    for completed in &mut proposals[..offset] {
                        completed.result = None;
                    }
                    return Err(ExecutePartitionError::Work {
                        identity: unit.identity(key),
                        source,
                    });
                }
            }
        };
        slot.result = Some(CalculationResult {
            identity: unit.identity(key),
            value,
        });
    }
    Ok(())
}

fn duplicate_mapping_error(identity: MappingIdentity) -> CalculationError {
    match identity {
        MappingIdentity::SnakeId(id) => CalculationError::DuplicateSnakeId(id),
        MappingIdentity::SnakeIndex(index) => CalculationError::DuplicateSnakeIndex(index),
        MappingIdentity::BrainHandle(brain) => CalculationError::DuplicateBrainHandle(brain),
        MappingIdentity::BrainIndex(index) => CalculationError::DuplicateBrainIndex(index),
        MappingIdentity::PopulationSlot(slot) => CalculationError::DuplicatePopulationSlot(slot),
    }
}

fn try_vec_with_capacity<T>(
    capacity: usize,
    buffer: &'static str,
) -> Result<Vec<T>, CalculationError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| CalculationError::AllocationFailed {
            buffer,
            elements: capacity,
        })?;
    Ok(values)
}

fn try_zeroed_floats(count: usize, buffer: &'static str) -> Result<Vec<f32>, CalculationError> {
    let mut values = try_vec_with_capacity(count, buffer)?;
    values.resize(count, 0.0);
    Ok(values)
}

fn checked_element_bytes<T>(
    count: usize,
    context: &'static str,
) -> Result<usize, CalculationError> {
    count
        .checked_mul(size_of::<T>())
        .ok_or(CalculationError::ArithmeticOverflow { context })
}

fn checked_capacity_bytes<T>(
    capacity: usize,
    context: &'static str,
) -> Result<usize, CalculationError> {
    checked_element_bytes::<T>(capacity, context)
}

fn checked_float_capacity_bytes(buffers: [&Vec<f32>; 3]) -> Result<usize, CalculationError> {
    buffers.into_iter().try_fold(0usize, |total, buffer| {
        total
            .checked_add(checked_capacity_bytes::<f32>(
                buffer.capacity(),
                "scratch backing capacity",
            )?)
            .ok_or(CalculationError::ArithmeticOverflow {
                context: "scratch backing bytes",
            })
    })
}

fn checked_sum(values: &[usize]) -> Result<usize, CalculationError> {
    values.iter().try_fold(0usize, |total, value| {
        total
            .checked_add(*value)
            .ok_or(CalculationError::ArithmeticOverflow {
                context: "calculation workspace total bytes",
            })
    })
}

/// Deterministic contract, mapping, and resource-admission failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalculationError {
    /// Workspace requires at least one independent scratch owner.
    ZeroWorkerCount,
    /// Work partitioning requires at least one partition.
    ZeroPartitionCount,
    /// Requested partition index is outside its partition count.
    InvalidPartitionIndex {
        /// Requested zero-based partition.
        partition_index: usize,
        /// Configured partition count.
        partition_count: usize,
    },
    /// Checked count or byte arithmetic overflowed.
    ArithmeticOverflow {
        /// Static calculation context.
        context: &'static str,
    },
    /// A bounded allocation failed.
    AllocationFailed {
        /// Static storage role.
        buffer: &'static str,
        /// Requested element count.
        elements: usize,
    },
    /// Fixed workspace storage exceeds its admitted budget.
    WorkspaceBudgetExceeded {
        /// Required logical or actual backing bytes.
        required_bytes: usize,
        /// Caller-supplied total budget.
        budget_bytes: usize,
    },
    /// A phase must be begun before candidates are accepted.
    WorkspaceNotBegun,
    /// Candidate preparation was requested twice without beginning a new phase.
    WorkspaceAlreadyPrepared,
    /// Execution or sealing was requested before successful preparation.
    WorkspaceNotPrepared,
    /// Due work exceeded the fixed capacity.
    WorkCapacityExceeded {
        /// Requested active count.
        requested: usize,
        /// Fixed admitted capacity.
        capacity: usize,
    },
    /// Candidate snake index is outside the immutable state view.
    SnakeIndexOutOfBounds {
        /// Requested index.
        index: usize,
        /// Available snake records.
        snake_count: usize,
    },
    /// Candidate brain index is outside the immutable state view.
    BrainIndexOutOfBounds {
        /// Requested index.
        index: usize,
        /// Available brain records.
        brain_count: usize,
    },
    /// Due snake does not own a neural brain.
    SnakeHasNoBrain {
        /// Exact snake identity.
        snake_id: u64,
        /// Resolved snake index.
        snake_index: usize,
    },
    /// Snake-side handle did not resolve to the indexed brain record.
    BrainMappingMismatch {
        /// Exact snake identity.
        snake_id: u64,
        /// Handle stored by the snake.
        requested: BrainHandle,
        /// Handle found at the brain index.
        mapped: BrainHandle,
    },
    /// Candidate brain belongs to another population epoch.
    StaleBrainEpoch {
        /// Resolved handle.
        brain: BrainHandle,
        /// Required epoch.
        expected_epoch: u64,
    },
    /// Resolved brain owner does not match the snake record.
    BrainOwnerMismatch {
        /// Resolved handle.
        brain: BrainHandle,
        /// Owner implied by the snake record.
        expected: BrainOwner,
        /// Owner on the brain record.
        mapped: BrainOwner,
    },
    /// Dense slot is outside the population array.
    PopulationSlotOutOfBounds {
        /// Requested slot.
        slot: u32,
        /// Available population records.
        population_count: usize,
    },
    /// Dense array position does not contain its required slot identity.
    PopulationSlotRecordMismatch {
        /// Dense slot required by the snake.
        expected_slot: u32,
        /// Slot recorded at that array position.
        recorded_slot: u32,
    },
    /// Population genome and brain slab disagree on the stable handle.
    PopulationBrainMismatch {
        /// Dense population slot.
        slot: u32,
        /// Handle recorded by the genome.
        expected: BrainHandle,
        /// Handle resolved from the brain slab.
        mapped: BrainHandle,
    },
    /// Two candidates name one stable snake identity.
    DuplicateSnakeId(u64),
    /// Two candidates name one snake index.
    DuplicateSnakeIndex(usize),
    /// Two candidates name one stable brain handle.
    DuplicateBrainHandle(BrainHandle),
    /// Two candidates name one brain index.
    DuplicateBrainIndex(usize),
    /// Two evolved candidates name one population slot.
    DuplicatePopulationSlot(u32),
    /// Work and proposal slices must be exactly aligned.
    PartitionLengthMismatch {
        /// Work slice length.
        work: usize,
        /// Proposal slice length.
        proposals: usize,
    },
    /// Work slice ordinals must be contiguous.
    NonContiguousWorkSlice {
        /// Required ordinal.
        expected_ordinal: usize,
        /// Actual ordinal.
        actual_ordinal: usize,
    },
    /// A partition attempted to overwrite a staged proposal.
    ProposalAlreadyPresent {
        /// Global deterministic ordinal.
        ordinal: usize,
    },
    /// Proposal storage belongs to another batch.
    BatchKeyMismatch {
        /// Expected phase.
        expected: CalculationBatchKey,
        /// Workspace phase.
        actual: CalculationBatchKey,
    },
    /// Staged identity does not match its resolved work record.
    ProposalIdentityMismatch {
        /// Global deterministic ordinal.
        ordinal: usize,
        /// Required identity.
        expected: CalculationWorkIdentity,
        /// Staged identity.
        actual: CalculationWorkIdentity,
    },
    /// Coordinator attempted to seal before every active result existed.
    IncompleteBatch {
        /// Present result count.
        completed: usize,
        /// Required result count.
        expected: usize,
    },
}

impl fmt::Display for CalculationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl Error for CalculationError {}

/// Partition execution failure retaining the exact failed work identity.
#[derive(Debug, PartialEq, Eq)]
pub enum ExecutePartitionError<E> {
    /// Work/proposal contract failed before or during calculation.
    Contract(Box<CalculationError>),
    /// Scalar callback failed for one exact work item.
    Work {
        /// Exact failed batch and work identity.
        identity: CalculationWorkIdentity,
        /// Callback-specific failure.
        source: E,
    },
}

impl<E: fmt::Display> fmt::Display for ExecutePartitionError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => error.fmt(formatter),
            Self::Work { identity, source } => {
                write!(formatter, "calculation work {identity:?} failed: {source}")
            }
        }
    }
}

impl<E: Error + 'static> Error for ExecutePartitionError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error.as_ref()),
            Self::Work { source, .. } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::{BodyRange, GenomeLineage, SnakeKind, WorldPoint};
    use super::*;
    use std::thread;

    const EPOCH: u64 = 7;

    fn key() -> CalculationBatchKey {
        CalculationBatchKey::new(3, 41, EPOCH)
    }

    fn snake(id: u64, brain: BrainHandle, population_slot: Option<u32>) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id.min(u64::from(u32::MAX))).unwrap(),
            kind: if population_slot.is_some() {
                SnakeKind::Evolved
            } else {
                SnakeKind::Resurrected
            },
            alive: true,
            population_slot,
            brain: Some(brain),
            baseline_slot: None,
            baseline_strategy: None,
            position: WorldPoint { x: 0.0, y: 0.0 },
            previous_position: WorldPoint { x: 0.0, y: 0.0 },
            direction: 0.0,
            radius: 1.0,
            speed: 1.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: 1.0,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange { start: 0, len: 0 },
            skin: 0,
        }
    }

    fn entity_fixture(count: usize) -> (Vec<SnakeState>, Vec<BrainRuntimeState>) {
        let mut snakes = Vec::with_capacity(count);
        let mut brains = Vec::with_capacity(count);
        for index in 0..count {
            let id = 1_000 + index as u64;
            let handle = BrainHandle {
                id: 2_000 + index as u64,
                epoch: EPOCH,
            };
            snakes.push(snake(id, handle, None));
            brains.push(BrainRuntimeState {
                handle,
                owner: BrainOwner::Entity(id),
                non_population_weights: Some(vec![index as f32].into_boxed_slice()),
                recurrent: Vec::new().into_boxed_slice(),
            });
        }
        (snakes, brains)
    }

    fn population_fixture() -> (
        Vec<SnakeState>,
        Vec<BrainRuntimeState>,
        Vec<PopulationGenome>,
    ) {
        let handle = BrainHandle {
            id: 9,
            epoch: EPOCH,
        };
        let snakes = vec![snake(100, handle, Some(0))];
        let brains = vec![BrainRuntimeState {
            handle,
            owner: BrainOwner::PopulationSlot(0),
            non_population_weights: None,
            recurrent: Vec::new().into_boxed_slice(),
        }];
        let population = vec![PopulationGenome {
            slot: 0,
            brain: handle,
            lineage: GenomeLineage {
                genome_id: 1,
                birth_generation: 1,
                parent_a: None,
                parent_b: None,
            },
            fitness: 0.0,
            weights: vec![1.0].into_boxed_slice(),
        }];
        (snakes, brains, population)
    }

    fn workspace<T>(max_work: usize, workers: usize) -> CalculationWorkspace<T> {
        CalculationWorkspace::try_new(
            max_work,
            workers,
            CalculationScratchLayout {
                activation_floats: 4,
                gather_floats: 2,
                temporary_floats: 2,
            },
            usize::MAX,
        )
        .unwrap()
    }

    fn prepare_entities<T>(
        workspace: &mut CalculationWorkspace<T>,
        order: &[usize],
        snakes: &[SnakeState],
        brains: &[BrainRuntimeState],
    ) {
        workspace.begin(key());
        for index in order {
            workspace.try_push_candidate(*index, *index).unwrap();
        }
        workspace.prepare(snakes, brains, &[]).unwrap();
    }

    fn execute_concurrently(worker_count: usize) -> Vec<(CalculationWorkIdentity, u64)> {
        let (snakes, brains) = entity_fixture(17);
        let mut workspace = workspace::<u64>(17, worker_count);
        let order: Vec<_> = (0..17).rev().collect();
        prepare_entities(&mut workspace, &order, &snakes, &brains);
        let CalculationExecutionBuffers {
            key: batch,
            work,
            proposals,
            scratches,
        } = workspace.execution_buffers().unwrap();
        let total = work.len();
        thread::scope(|scope| {
            let mut remaining_work = work;
            let mut remaining_proposals = proposals;
            let mut handles = Vec::with_capacity(worker_count);
            for (partition, scratch) in scratches.iter_mut().enumerate() {
                let range = work_range(total, partition, worker_count).unwrap();
                let (partition_work, work_tail) = remaining_work.split_at(range.len());
                let (partition_proposals, proposal_tail) =
                    remaining_proposals.split_at_mut(range.len());
                remaining_work = work_tail;
                remaining_proposals = proposal_tail;
                handles.push(scope.spawn(move || {
                    execute_partition(
                        batch,
                        partition_work,
                        partition_proposals,
                        scratch,
                        |unit, scratch| {
                            scratch.activation[0] = 1.0;
                            Ok::<u64, &'static str>(unit.snake_id() ^ unit.brain().id)
                        },
                    )
                }));
            }
            for handle in handles {
                handle.join().unwrap().unwrap();
            }
        });
        workspace
            .seal(key())
            .unwrap()
            .iter()
            .map(|result| (result.identity(), *result.value()))
            .collect()
    }

    #[test]
    fn shuffled_candidates_resolve_to_one_exact_order_and_u64_identity() {
        let (mut snakes, mut brains) = entity_fixture(3);
        snakes[0].id = u64::MAX;
        brains[0].owner = BrainOwner::Entity(u64::MAX);
        brains[0].handle.id = u64::MAX;
        snakes[0].brain = Some(brains[0].handle);
        let mut left = workspace::<u64>(3, 1);
        let mut right = workspace::<u64>(3, 1);
        prepare_entities(&mut left, &[2, 0, 1], &snakes, &brains);
        prepare_entities(&mut right, &[1, 2, 0], &snakes, &brains);
        assert_eq!(left.work, right.work);
        assert_eq!(left.work.last().unwrap().snake_id(), u64::MAX);
        assert_eq!(left.work.last().unwrap().brain().id, u64::MAX);
    }

    #[test]
    fn preparation_rejects_oob_and_incoherent_state_resolution() {
        let (snakes, brains) = entity_fixture(1);
        let mut ws = workspace::<u64>(2, 1);

        ws.begin(key());
        ws.try_push_candidate(1, 0).unwrap();
        assert!(matches!(
            ws.prepare(&snakes, &brains, &[]),
            Err(CalculationError::SnakeIndexOutOfBounds { .. })
        ));
        assert!(matches!(
            ws.seal(key()),
            Err(CalculationError::WorkspaceNotPrepared)
        ));

        ws.begin(key());
        ws.try_push_candidate(0, 1).unwrap();
        assert!(matches!(
            ws.prepare(&snakes, &brains, &[]),
            Err(CalculationError::BrainIndexOutOfBounds { .. })
        ));

        let mut wrong_brains = brains.clone();
        wrong_brains[0].handle.id += 1;
        ws.begin(key());
        ws.try_push_candidate(0, 0).unwrap();
        assert!(matches!(
            ws.prepare(&snakes, &wrong_brains, &[]),
            Err(CalculationError::BrainMappingMismatch { .. })
        ));

        let mut stale = brains.clone();
        stale[0].handle.epoch -= 1;
        let mut stale_snakes = snakes.clone();
        stale_snakes[0].brain = Some(stale[0].handle);
        ws.begin(key());
        ws.try_push_candidate(0, 0).unwrap();
        assert!(matches!(
            ws.prepare(&stale_snakes, &stale, &[]),
            Err(CalculationError::StaleBrainEpoch { .. })
        ));

        let mut wrong_owner = brains.clone();
        wrong_owner[0].owner = BrainOwner::Entity(snakes[0].id + 1);
        ws.begin(key());
        ws.try_push_candidate(0, 0).unwrap();
        assert!(matches!(
            ws.prepare(&snakes, &wrong_owner, &[]),
            Err(CalculationError::BrainOwnerMismatch { .. })
        ));
    }

    #[test]
    fn population_slot_record_and_handle_are_resolved_not_trusted() {
        let (snakes, brains, population) = population_fixture();
        let mut ws = workspace::<u64>(1, 1);

        let mut out_of_bounds = snakes.clone();
        out_of_bounds[0].population_slot = Some(1);
        let mut out_of_bounds_brains = brains.clone();
        out_of_bounds_brains[0].owner = BrainOwner::PopulationSlot(1);
        ws.begin(key());
        ws.try_push_candidate(0, 0).unwrap();
        assert!(matches!(
            ws.prepare(&out_of_bounds, &out_of_bounds_brains, &population),
            Err(CalculationError::PopulationSlotOutOfBounds { .. })
        ));

        let mut wrong_record = population.clone();
        wrong_record[0].slot = 1;
        ws.begin(key());
        ws.try_push_candidate(0, 0).unwrap();
        assert!(matches!(
            ws.prepare(&snakes, &brains, &wrong_record),
            Err(CalculationError::PopulationSlotRecordMismatch { .. })
        ));

        let mut wrong_handle = population.clone();
        wrong_handle[0].brain.id += 1;
        ws.begin(key());
        ws.try_push_candidate(0, 0).unwrap();
        assert!(matches!(
            ws.prepare(&snakes, &brains, &wrong_handle),
            Err(CalculationError::PopulationBrainMismatch { .. })
        ));
    }

    #[test]
    fn duplicate_due_records_are_rejected_before_execution() {
        let (snakes, brains) = entity_fixture(2);
        let mut ws = workspace::<u64>(2, 1);
        ws.begin(key());
        ws.try_push_candidate(0, 0).unwrap();
        ws.try_push_candidate(0, 0).unwrap();
        assert!(matches!(
            ws.prepare(&snakes, &brains, &[]),
            Err(CalculationError::DuplicateSnakeId(_))
                | Err(CalculationError::DuplicateSnakeIndex(_))
                | Err(CalculationError::DuplicateBrainHandle(_))
                | Err(CalculationError::DuplicateBrainIndex(_))
        ));
    }

    #[test]
    fn allocation_free_ranges_cover_zero_one_and_many() {
        assert_eq!(work_range(0, 0, 3).unwrap(), WorkRange { start: 0, end: 0 });
        assert_eq!(work_range(1, 0, 4).unwrap(), WorkRange { start: 0, end: 1 });
        assert_eq!(work_range(1, 3, 4).unwrap(), WorkRange { start: 1, end: 1 });
        assert_eq!(
            work_range(10, 0, 3).unwrap(),
            WorkRange { start: 0, end: 4 }
        );
        assert_eq!(
            work_range(10, 1, 3).unwrap(),
            WorkRange { start: 4, end: 7 }
        );
        assert_eq!(
            work_range(10, 2, 3).unwrap(),
            WorkRange { start: 7, end: 10 }
        );
        assert!(matches!(
            work_range(1, 0, 0),
            Err(CalculationError::ZeroPartitionCount)
        ));
        assert!(matches!(
            work_range(1, 2, 2),
            Err(CalculationError::InvalidPartitionIndex { .. })
        ));
    }

    #[test]
    fn scoped_one_two_four_seven_worker_results_are_bit_identical() {
        let scalar = execute_concurrently(1);
        assert_eq!(execute_concurrently(2), scalar);
        assert_eq!(execute_concurrently(4), scalar);
        assert_eq!(execute_concurrently(7), scalar);
    }

    #[test]
    fn failed_partition_cannot_seal_or_expose_partial_values() {
        let (snakes, brains) = entity_fixture(3);
        let mut ws = workspace::<u64>(3, 1);
        prepare_entities(&mut ws, &[0, 1, 2], &snakes, &brains);
        let CalculationExecutionBuffers {
            key: batch,
            work,
            proposals,
            scratches,
        } = ws.execution_buffers().unwrap();
        let error = execute_partition(batch, work, proposals, &mut scratches[0], |unit, _| {
            if unit.ordinal() == 1 {
                Err("injected")
            } else {
                Ok(unit.snake_id())
            }
        })
        .unwrap_err();
        assert!(matches!(
            error,
            ExecutePartitionError::Work {
                source: "injected",
                ..
            }
        ));
        assert_eq!(
            ws.seal(key()).unwrap_err(),
            CalculationError::IncompleteBatch {
                completed: 0,
                expected: 3,
            }
        );
    }

    #[test]
    fn unsealed_proposal_debug_output_redacts_its_value() {
        let mut slot = CalculationProposalSlot::empty();
        slot.result = Some(CalculationResult {
            identity: CalculationWorkIdentity {
                batch: key(),
                ordinal: 0,
                snake_id: 1,
                brain: BrainHandle {
                    id: 2,
                    epoch: EPOCH,
                },
            },
            value: "uncommitted-secret",
        });

        let debug = format!("{slot:?}");
        assert_eq!(debug, "CalculationProposalSlot { occupied: true }");
        assert!(!debug.contains("uncommitted-secret"));
    }

    #[test]
    fn workspace_budget_and_arithmetic_fail_before_use() {
        let layout = CalculationScratchLayout {
            activation_floats: 2,
            gather_floats: 1,
            temporary_floats: 0,
        };
        let required = CalculationWorkspace::<u64>::required_bytes(4, 2, layout).unwrap();
        assert!(matches!(
            CalculationWorkspace::<u64>::try_new(4, 2, layout, required - 1),
            Err(CalculationError::WorkspaceBudgetExceeded { .. })
        ));
        assert!(matches!(
            CalculationWorkspace::<u64>::required_bytes(usize::MAX, 2, layout),
            Err(CalculationError::ArithmeticOverflow { .. })
        ));
        assert!(matches!(
            CalculationWorkspace::<u64>::try_new(1, 0, layout, usize::MAX),
            Err(CalculationError::ZeroWorkerCount)
        ));
    }

    #[test]
    fn complete_cycles_reuse_all_capacities_across_active_count_changes() {
        let (snakes, brains) = entity_fixture(5);
        let mut ws = workspace::<u64>(5, 2);
        let pointers = (
            ws.candidates.as_ptr(),
            ws.work.as_ptr(),
            ws.validation.as_ptr(),
            ws.proposals.as_ptr(),
            ws.scratches.as_ptr(),
            ws.scratches
                .iter()
                .map(|scratch| {
                    (
                        scratch.activation.as_ptr(),
                        scratch.gather.as_ptr(),
                        scratch.temporary.as_ptr(),
                    )
                })
                .collect::<Vec<_>>(),
        );
        let capacities = (
            ws.candidates.capacity(),
            ws.work.capacity(),
            ws.validation.capacity(),
            ws.proposals.capacity(),
            ws.scratches.capacity(),
        );

        for active in [5usize, 2, 4, 0, 3] {
            let order: Vec<_> = (0..active).rev().collect();
            prepare_entities(&mut ws, &order, &snakes, &brains);
            let CalculationExecutionBuffers {
                key: batch,
                work,
                proposals,
                scratches,
            } = ws.execution_buffers().unwrap();
            execute_partition(batch, work, proposals, &mut scratches[0], |unit, _| {
                Ok::<u64, &'static str>(unit.snake_id())
            })
            .unwrap();
            {
                let view = ws.seal(key()).unwrap();
                assert_eq!(view.len(), active);
                assert_eq!(view.iter().count(), active);
            }
            assert_eq!(
                (
                    ws.candidates.as_ptr(),
                    ws.work.as_ptr(),
                    ws.validation.as_ptr(),
                    ws.proposals.as_ptr(),
                    ws.scratches.as_ptr(),
                ),
                (pointers.0, pointers.1, pointers.2, pointers.3, pointers.4)
            );
            assert_eq!(
                (
                    ws.candidates.capacity(),
                    ws.work.capacity(),
                    ws.validation.capacity(),
                    ws.proposals.capacity(),
                    ws.scratches.capacity(),
                ),
                capacities
            );
            assert_eq!(
                ws.scratches
                    .iter()
                    .map(|scratch| {
                        (
                            scratch.activation.as_ptr(),
                            scratch.gather.as_ptr(),
                            scratch.temporary.as_ptr(),
                        )
                    })
                    .collect::<Vec<_>>(),
                pointers.5
            );
        }
    }
}
