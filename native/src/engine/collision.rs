//! Immutable swept collision detection and deterministic outcome proposals.
//!
//! The temporary TypeScript resolver mutates deaths while iterating one
//! midpoint-only grid, so later heads observe a different world. This module
//! instead indexes every cell touched by each live body's swept bounds, reads
//! one post-food snapshot, and emits stable death/kill proposals for a later
//! all-or-nothing physics commit.

use super::food::PreparedFood;
use super::state::{SnakeState, WorldPoint, WorldState};
use std::cmp::Ordering;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::Instant;

const CONTACT_SPACE_TOLERANCE: f64 = 1.0e-9;
const CONTACT_HULL_TOLERANCE: f64 = CONTACT_SPACE_TOLERANCE * 0.5;
const CONTACT_INTERVAL_MOTION_TOLERANCE: f64 = CONTACT_SPACE_TOLERANCE * 0.5;
const MAXIMUM_CONTACT_SEARCH_DEPTH: u32 = 56;
const MAXIMUM_CONTACT_SEARCH_INTERVALS: usize = 4_096;
const ORIGIN: WorldPoint = WorldPoint { x: 0.0, y: 0.0 };

/// Collision settings projected from one admitted settings revision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionConfig {
    /// Broad-phase cell width in world units.
    pub cell_size: f64,
    /// Multiplier applied to the sum of both collision radii.
    pub hit_scale: f64,
    /// Number of body segments near each head omitted from head/body collision.
    pub skip_segments: usize,
    /// Maximum swept segment-to-cell entries admitted for one substep.
    pub maximum_index_entries: usize,
    /// Maximum broad-phase cells one swept head query may visit.
    pub maximum_query_cells: usize,
}

impl CollisionConfig {
    /// Current TypeScript defaults plus explicit safe Rust capacities.
    #[must_use]
    pub const fn typescript_defaults() -> Self {
        Self {
            cell_size: 70.0,
            hit_scale: 0.82,
            skip_segments: 0,
            maximum_index_entries: 2_000_000,
            maximum_query_cells: 262_144,
        }
    }

    pub(crate) fn validate(self) -> Result<(), CollisionError> {
        if !self.cell_size.is_finite() || self.cell_size <= 0.0 {
            return Err(CollisionError::InvalidConfig { field: "cell_size" });
        }
        if !self.hit_scale.is_finite() || self.hit_scale <= 0.0 {
            return Err(CollisionError::InvalidConfig { field: "hit_scale" });
        }
        if self.maximum_index_entries == 0 {
            return Err(CollisionError::InvalidConfig {
                field: "maximum_index_entries",
            });
        }
        if self.maximum_query_cells == 0 {
            return Err(CollisionError::InvalidConfig {
                field: "maximum_query_cells",
            });
        }
        Ok(())
    }
}

/// One unordered simultaneous head-to-head contact.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadHeadContact {
    /// Lower stable snake identity.
    pub first_id: u64,
    /// Higher stable snake identity.
    pub second_id: u64,
    /// Earliest normalized substep contact time in `[0, 1]`.
    pub time: f64,
    /// Whether a roundoff-scale closest approach was conservatively treated as contact.
    pub conservative: bool,
}

/// Deterministically selected head-to-body contact for one victim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeadBodyContact {
    /// Stable victim identity.
    pub victim_id: u64,
    /// Stable body-owner identity receiving credit after commit.
    pub owner_id: u64,
    /// One-based body segment end offset.
    pub segment_end: usize,
    /// Earliest normalized substep contact time in `[0, 1]`.
    pub time: f64,
    /// Whether the continuous search conservatively resolved a sub-nanounit
    /// spatial ambiguity rather than sampling a point already inside.
    pub conservative: bool,
}

/// One stable death to apply only after every collision has been detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeathProposal {
    /// Stable victim identity.
    pub victim_id: u64,
    /// Source/staged snake-array index; never a public identity.
    pub victim_index: usize,
    /// Movement detected arena-boundary death.
    pub wall: bool,
    /// At least one simultaneous head-to-head contact killed this snake.
    pub head_to_head: bool,
    /// Selected body owner for a simultaneous head-to-body death, if any.
    pub body_owner_id: Option<u64>,
    /// Normal collision deaths drop corpse pellets; wall deaths do not.
    pub drop_corpse_pellets: bool,
}

/// One kill award paired with the victim that caused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KillAward {
    /// Stable body owner receiving one kill and configured points.
    pub killer_id: u64,
    /// Stable victim identity; one victim can award at most one kill.
    pub victim_id: u64,
}

/// Current collision work and retained scratch capacities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollisionDiagnostics {
    /// Live swept body segments represented by the index.
    pub indexed_segments: usize,
    /// Complete segment-to-cell entries represented by the index.
    pub index_entries: usize,
    /// Occupied broad-phase cells.
    pub occupied_cells: usize,
    /// Broad-phase cells visited across all head/body queries.
    pub query_cells_visited: usize,
    /// Deduplicated segment candidates examined across all heads.
    pub body_candidates: usize,
    /// Exact simultaneous head/head contacts.
    pub head_head_contacts: usize,
    /// Selected head/body contacts.
    pub head_body_contacts: usize,
    /// Staged deaths.
    pub deaths: usize,
    /// Staged kill awards.
    pub awards: usize,
    /// Contacts accepted at the documented numeric ambiguity tolerance.
    pub conservative_contacts: usize,
    /// Largest interval-search work count for one head/segment pair.
    pub maximum_contact_intervals: usize,
    /// Retained swept-segment capacity.
    pub segment_capacity: usize,
    /// Retained stable snake-order capacity.
    pub order_capacity: usize,
    /// Retained per-snake swept head-envelope capacity.
    pub head_envelope_capacity: usize,
    /// Retained cell-entry capacity.
    pub entry_capacity: usize,
    /// Retained occupied-cell capacity.
    pub cell_capacity: usize,
    /// Retained candidate capacity.
    pub candidate_capacity: usize,
    /// Retained candidate-generation marker capacity.
    pub seen_generation_capacity: usize,
    /// Retained head/head-contact capacity.
    pub head_head_capacity: usize,
    /// Retained selected body-contact capacity.
    pub head_body_capacity: usize,
    /// Retained death capacity.
    pub death_capacity: usize,
    /// Retained per-snake death-flag capacity.
    pub death_flag_capacity: usize,
    /// Retained award capacity.
    pub award_capacity: usize,
}

/// Immutable view of one completely detected collision snapshot.
#[derive(Clone, Copy, Debug)]
pub struct PreparedCollision<'collision, 'food, 'world> {
    food: PreparedFood<'food, 'world>,
    head_head_contacts: &'collision [HeadHeadContact],
    head_body_contacts: &'collision [HeadBodyContact],
    deaths: &'collision [DeathProposal],
    awards: &'collision [KillAward],
    diagnostics: CollisionDiagnostics,
    work_timings: super::generation_work::WorkTimings,
    #[cfg(feature = "engine-test-hooks")]
    phase_timings: CollisionPhaseTimings,
}

/// Benchmark-only attribution within collision preparation.
#[cfg(feature = "engine-test-hooks")]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CollisionPhaseTimings {
    pub index_ms: f64,
    pub head_head_ms: f64,
    pub head_body_ms: f64,
}

impl<'collision, 'food, 'world> PreparedCollision<'collision, 'food, 'world> {
    /// Exact post-food state and source-world binding used for detection.
    #[must_use]
    pub const fn food(self) -> PreparedFood<'food, 'world> {
        self.food
    }

    /// Simultaneous head/head contacts in stable pair order.
    #[must_use]
    pub const fn head_head_contacts(self) -> &'collision [HeadHeadContact] {
        self.head_head_contacts
    }

    /// At most one selected body owner per victim, in stable victim order.
    #[must_use]
    pub const fn head_body_contacts(self) -> &'collision [HeadBodyContact] {
        self.head_body_contacts
    }

    /// Complete stable death proposals.
    #[must_use]
    pub const fn deaths(self) -> &'collision [DeathProposal] {
        self.deaths
    }

    /// Complete stable kill awards.
    #[must_use]
    pub const fn awards(self) -> &'collision [KillAward] {
        self.awards
    }

    /// Current work and retained capacities.
    #[must_use]
    pub const fn diagnostics(self) -> CollisionDiagnostics {
        self.diagnostics
    }

    pub(crate) const fn work_timings(self) -> super::generation_work::WorkTimings {
        self.work_timings
    }

    #[cfg(feature = "engine-test-hooks")]
    pub(crate) const fn phase_timings(self) -> CollisionPhaseTimings {
        self.phase_timings
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct CellKey {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellEntry {
    key: CellKey,
    segment: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellSpan {
    key: CellKey,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SweptSegment {
    owner_id: u64,
    segment_end: usize,
    previous_start: WorldPoint,
    previous_end: WorldPoint,
    current_start: WorldPoint,
    current_end: WorldPoint,
    movement_radius: f64,
    final_radius: f64,
    newly_grown: bool,
    removed_at_final: bool,
}

#[derive(Clone, Debug, Default)]
struct CandidateScratch {
    seen_generation: Vec<u32>,
    generation: u32,
    candidates: Vec<usize>,
}

impl CandidateScratch {
    fn begin(&mut self, records: usize) -> Result<(), CollisionError> {
        reserve_for(&mut self.seen_generation, records, "candidate generations")?;
        if self.seen_generation.len() < records {
            self.seen_generation.resize(records, 0);
        }
        reserve_for(&mut self.candidates, records, "collision candidates")?;
        self.candidates.clear();
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.seen_generation.fill(0);
            self.generation = 1;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct SweptBodyIndex {
    cell_size: f64,
    maximum_query_cells: usize,
    segments: Vec<SweptSegment>,
    entries: Vec<CellEntry>,
    cells: Vec<CellSpan>,
}

impl SweptBodyIndex {
    fn clear(&mut self) {
        self.segments.clear();
        self.entries.clear();
        self.cells.clear();
    }

    fn rebuild(
        &mut self,
        food: PreparedFood<'_, '_>,
        order: &[usize],
        config: CollisionConfig,
    ) -> Result<(), CollisionError> {
        self.clear();
        self.cell_size = config.cell_size;
        self.maximum_query_cells = config.maximum_query_cells;
        let source = food.source_world();
        if source.snakes.len() != food.snakes().len() {
            return Err(CollisionError::FoodShapeMismatch);
        }

        let first_segment = config.skip_segments.max(1);
        let mut required_segments = 0usize;
        let mut required_entries = 0usize;
        for &snake_index in order {
            let source_snake = source
                .snakes
                .get(snake_index)
                .ok_or(CollisionError::FoodShapeMismatch)?;
            let staged = food
                .snakes()
                .get(snake_index)
                .ok_or(CollisionError::FoodShapeMismatch)?;
            if source_snake.id != staged.id {
                return Err(CollisionError::FoodShapeMismatch);
            }
            let source_body = body_slice(source, source_snake)?;
            let final_body = food
                .body_for(staged)
                .ok_or(CollisionError::InvalidBodyRange {
                    snake_id: staged.id,
                })?;
            let movement_body = food.movement_body_for_index(snake_index).ok_or(
                CollisionError::InvalidBodyRange {
                    snake_id: staged.id,
                },
            )?;
            let collision_body_length = movement_body.len().max(final_body.len());
            if !source_snake.alive || first_segment >= collision_body_length {
                continue;
            }
            validate_radius(staged)?;
            let movement_radius = food
                .movement_radius_for_index(snake_index)
                .ok_or(CollisionError::FoodShapeMismatch)?;
            validate_radius_value(movement_radius)?;
            for segment_end in first_segment..collision_body_length {
                let segment = swept_segment(
                    source_snake,
                    source_body,
                    staged,
                    movement_body,
                    final_body,
                    movement_radius,
                    segment_end,
                )?;
                let expansion = movement_radius.max(staged.radius) * config.hit_scale;
                let (minimum, maximum) =
                    swept_segment_cell_bounds(segment, expansion, config.cell_size)?;
                required_segments =
                    required_segments
                        .checked_add(1)
                        .ok_or(CollisionError::ArithmeticOverflow {
                            context: "swept segment count",
                        })?;
                required_entries = required_entries
                    .checked_add(cell_rectangle_count(minimum, maximum)?)
                    .ok_or(CollisionError::ArithmeticOverflow {
                        context: "swept index entry count",
                    })?;
                if required_entries > config.maximum_index_entries {
                    return Err(CollisionError::IndexEntryLimitExceeded {
                        required: required_entries,
                        maximum: config.maximum_index_entries,
                    });
                }
            }
        }

        reserve_for(&mut self.segments, required_segments, "swept segments")?;
        reserve_for(&mut self.entries, required_entries, "swept cell entries")?;
        for &snake_index in order {
            let source_snake = &source.snakes[snake_index];
            let staged = &food.snakes()[snake_index];
            let source_body = body_slice(source, source_snake)?;
            let final_body = food
                .body_for(staged)
                .ok_or(CollisionError::InvalidBodyRange {
                    snake_id: staged.id,
                })?;
            let movement_body = food.movement_body_for_index(snake_index).ok_or(
                CollisionError::InvalidBodyRange {
                    snake_id: staged.id,
                },
            )?;
            let collision_body_length = movement_body.len().max(final_body.len());
            if !source_snake.alive || first_segment >= collision_body_length {
                continue;
            }
            let movement_radius = food
                .movement_radius_for_index(snake_index)
                .ok_or(CollisionError::FoodShapeMismatch)?;
            for segment_end in first_segment..collision_body_length {
                let segment = swept_segment(
                    source_snake,
                    source_body,
                    staged,
                    movement_body,
                    final_body,
                    movement_radius,
                    segment_end,
                )?;
                let expansion = movement_radius.max(staged.radius) * config.hit_scale;
                let (minimum, maximum) =
                    swept_segment_cell_bounds(segment, expansion, config.cell_size)?;
                let segment_index = self.segments.len();
                self.segments.push(segment);
                for y in minimum.y..=maximum.y {
                    for x in minimum.x..=maximum.x {
                        self.entries.push(CellEntry {
                            key: CellKey { x, y },
                            segment: segment_index,
                        });
                    }
                }
            }
        }
        debug_assert_eq!(self.segments.len(), required_segments);
        debug_assert_eq!(self.entries.len(), required_entries);
        // Only cell grouping belongs here. A complete query deduplicates every
        // visited entry and sorts segment indices afterward into canonical
        // owner/segment order. Ordering equal-cell entries by segment would do
        // that work twice and prevents the sort from exploiting repeated keys.
        self.entries.sort_unstable_by_key(|entry| entry.key);
        reserve_for(&mut self.cells, self.entries.len(), "swept occupied cells")?;
        let mut start = 0usize;
        while start < self.entries.len() {
            let key = self.entries[start].key;
            let mut end = start + 1;
            while end < self.entries.len() && self.entries[end].key == key {
                end += 1;
            }
            self.cells.push(CellSpan { key, start, end });
            start = end;
        }
        Ok(())
    }

    fn collect_candidates(
        &self,
        previous_head: WorldPoint,
        current_head: WorldPoint,
        expansion: f64,
        scratch: &mut CandidateScratch,
    ) -> Result<(usize, usize), CollisionError> {
        validate_point(previous_head, "previous head")?;
        validate_point(current_head, "current head")?;
        if !expansion.is_finite() || expansion < 0.0 {
            return Err(CollisionError::NonFiniteGeometry {
                context: "head query expansion",
            });
        }
        scratch.begin(self.segments.len())?;
        if self.segments.is_empty() {
            return Ok((0, 0));
        }
        let minimum = CellKey {
            x: cell_coordinate(
                previous_head.x.min(current_head.x) - expansion,
                self.cell_size,
            )?,
            y: cell_coordinate(
                previous_head.y.min(current_head.y) - expansion,
                self.cell_size,
            )?,
        };
        let maximum = CellKey {
            x: cell_coordinate(
                previous_head.x.max(current_head.x) + expansion,
                self.cell_size,
            )?,
            y: cell_coordinate(
                previous_head.y.max(current_head.y) + expansion,
                self.cell_size,
            )?,
        };
        let cells_visited = cell_rectangle_count(minimum, maximum)?;
        if cells_visited > self.maximum_query_cells {
            return Err(CollisionError::QueryCellLimitExceeded {
                required: cells_visited,
                maximum: self.maximum_query_cells,
            });
        }
        let mut entries_visited = 0usize;
        for y in minimum.y..=maximum.y {
            for x in minimum.x..=maximum.x {
                let Some(span) = find_cell_span(&self.cells, CellKey { x, y }) else {
                    continue;
                };
                for entry in &self.entries[span.start..span.end] {
                    entries_visited = entries_visited.saturating_add(1);
                    if scratch.seen_generation[entry.segment] == scratch.generation {
                        continue;
                    }
                    scratch.seen_generation[entry.segment] = scratch.generation;
                    scratch.candidates.push(entry.segment);
                }
            }
        }
        // Segment indices already have the canonical owner/segment order.
        scratch.candidates.sort_unstable();
        Ok((cells_visited, entries_visited))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DeathFlags {
    wall: bool,
    head_to_head: bool,
    body_owner_id: Option<u64>,
}

/// One immutable swept head bound and its two collision radii for pair checks.
#[derive(Clone, Copy, Debug, Default)]
struct SweptHeadEnvelope {
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
    movement_radius: f64,
    /// Conservative radius for early misses; zero marks dead entries, NaN keeps exact checks.
    maximum_radius: f64,
}

impl SweptHeadEnvelope {
    /// Cache pair-invariant geometry after the staged motion has been validated.
    fn from_snake(snake: &SnakeState, movement_radius: f64, hit_scale: f64) -> Self {
        // Checking each scaled radius once guarantees a rejected pair's two
        // exact sums cannot underflow to zero. Overflow/invalid input retains
        // the original pair validation through the NaN fallback.
        let scaled_movement = movement_radius * hit_scale;
        let scaled_final = snake.radius * hit_scale;
        let maximum_radius = if movement_radius.is_finite()
            && movement_radius > 0.0
            && snake.radius.is_finite()
            && snake.radius > 0.0
            && scaled_movement.is_finite()
            && scaled_movement > 0.0
            && scaled_final.is_finite()
            && scaled_final > 0.0
        {
            movement_radius.max(snake.radius)
        } else {
            f64::NAN
        };
        Self {
            min_x: snake.previous_position.x.min(snake.position.x),
            max_x: snake.previous_position.x.max(snake.position.x),
            min_y: snake.previous_position.y.min(snake.position.y),
            max_y: snake.previous_position.y.max(snake.position.y),
            movement_radius,
            maximum_radius,
        }
    }
}

/// Reusable collision index, query, and deterministic outcome scratch.
#[derive(Clone, Debug, Default)]
pub struct CollisionWorkspace {
    work_timings: super::generation_work::WorkTimings,
    order: Vec<usize>,
    head_envelopes: Vec<SweptHeadEnvelope>,
    index: SweptBodyIndex,
    query_scratch: CandidateScratch,
    head_head_contacts: Vec<HeadHeadContact>,
    head_body_contacts: Vec<HeadBodyContact>,
    death_flags: Vec<DeathFlags>,
    deaths: Vec<DeathProposal>,
    awards: Vec<KillAward>,
    query_cells_visited: usize,
    body_candidates: usize,
    conservative_contacts: usize,
    maximum_contact_intervals: usize,
    ready: bool,
    #[cfg(feature = "engine-test-hooks")]
    phase_timings: CollisionPhaseTimings,
}

impl CollisionWorkspace {
    /// Construct empty reusable collision scratch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Detect one immutable post-food collision snapshot without mutating authority.
    pub fn prepare<'collision, 'food, 'world>(
        &'collision mut self,
        food: PreparedFood<'food, 'world>,
        config: CollisionConfig,
    ) -> Result<PreparedCollision<'collision, 'food, 'world>, CollisionError> {
        self.clear();
        config.validate()?;
        let source = food.source_world();
        if source.snakes.len() != food.snakes().len()
            || food.movement_proposals().len() != food.snakes().len()
        {
            return Err(CollisionError::FoodShapeMismatch);
        }
        reserve_for(&mut self.order, food.snakes().len(), "collision order")?;
        reserve_for(
            &mut self.head_envelopes,
            food.snakes().len(),
            "collision head envelopes",
        )?;
        reserve_for(
            &mut self.death_flags,
            food.snakes().len(),
            "collision death flags",
        )?;
        reserve_for(&mut self.deaths, food.snakes().len(), "collision deaths")?;
        reserve_for(&mut self.awards, food.snakes().len(), "collision awards")?;
        reserve_for(
            &mut self.head_body_contacts,
            food.snakes().len(),
            "head/body contacts",
        )?;
        let maximum_pairs = food
            .snakes()
            .len()
            .checked_mul(food.snakes().len().saturating_sub(1))
            .and_then(|value| value.checked_div(2))
            .ok_or(CollisionError::ArithmeticOverflow {
                context: "head/head pair capacity",
            })?;
        reserve_for(
            &mut self.head_head_contacts,
            maximum_pairs,
            "head/head contacts",
        )?;
        self.order.extend(0..food.snakes().len());
        self.order
            .sort_unstable_by_key(|index| food.snakes()[*index].id);
        for pair in self.order.windows(2) {
            if food.snakes()[pair[0]].id == food.snakes()[pair[1]].id {
                return Err(CollisionError::DuplicateSnakeId(food.snakes()[pair[0]].id));
            }
        }
        self.death_flags
            .resize(food.snakes().len(), DeathFlags::default());
        for proposal in food.movement_proposals() {
            let staged = food
                .snakes()
                .get(proposal.snake_index)
                .ok_or(CollisionError::FoodShapeMismatch)?;
            if staged.id != proposal.snake_id {
                return Err(CollisionError::FoodShapeMismatch);
            }
            if proposal.wall_death {
                self.death_flags[proposal.snake_index].wall = true;
            }
        }

        let phase_started = Instant::now();
        self.index.rebuild(food, &self.order, config)?;
        self.work_timings.collision_index = super::generation_work::nanos(phase_started);
        #[cfg(feature = "engine-test-hooks")]
        {
            self.phase_timings.index_ms = phase_started.elapsed().as_secs_f64() * 1_000.0;
        }
        let phase_started = Instant::now();
        self.detect_head_head(food, config)?;
        self.work_timings.collision_head_head = super::generation_work::nanos(phase_started);
        #[cfg(feature = "engine-test-hooks")]
        {
            self.phase_timings.head_head_ms = phase_started.elapsed().as_secs_f64() * 1_000.0;
        }
        let phase_started = Instant::now();
        self.detect_head_body(food, config)?;
        self.work_timings.collision_head_body = super::generation_work::nanos(phase_started);
        #[cfg(feature = "engine-test-hooks")]
        {
            self.phase_timings.head_body_ms = phase_started.elapsed().as_secs_f64() * 1_000.0;
        }

        for &snake_index in &self.order {
            let snake = &food.snakes()[snake_index];
            let flags = self.death_flags[snake_index];
            if !flags.wall && !flags.head_to_head && flags.body_owner_id.is_none() {
                continue;
            }
            self.deaths.push(DeathProposal {
                victim_id: snake.id,
                victim_index: snake_index,
                wall: flags.wall,
                head_to_head: flags.head_to_head,
                body_owner_id: flags.body_owner_id,
                drop_corpse_pellets: !flags.wall,
            });
            if let Some(killer_id) = flags.body_owner_id {
                self.awards.push(KillAward {
                    killer_id,
                    victim_id: snake.id,
                });
            }
        }
        self.ready = true;
        let diagnostics = self.diagnostics();
        Ok(PreparedCollision {
            food,
            head_head_contacts: &self.head_head_contacts,
            head_body_contacts: &self.head_body_contacts,
            deaths: &self.deaths,
            awards: &self.awards,
            diagnostics,
            work_timings: self.work_timings,
            #[cfg(feature = "engine-test-hooks")]
            phase_timings: self.phase_timings,
        })
    }

    fn detect_head_head(
        &mut self,
        food: PreparedFood<'_, '_>,
        config: CollisionConfig,
    ) -> Result<(), CollisionError> {
        for &snake_index in &self.order {
            if food.source_world().snakes[snake_index].alive {
                validate_motion(&food.snakes()[snake_index])?;
            }
        }
        self.head_envelopes
            .resize(food.snakes().len(), SweptHeadEnvelope::default());
        for &snake_index in &self.order {
            if food.source_world().snakes[snake_index].alive {
                let movement_radius = food
                    .movement_radius_for_index(snake_index)
                    .ok_or(CollisionError::FoodShapeMismatch)?;
                self.head_envelopes[snake_index] = SweptHeadEnvelope::from_snake(
                    &food.snakes()[snake_index],
                    movement_radius,
                    config.hit_scale,
                );
            }
        }
        for left_order in 0..self.order.len() {
            let left_index = self.order[left_order];
            let left = &food.snakes()[left_index];
            if !food.source_world().snakes[left_index].alive {
                continue;
            }
            let left_envelope = self.head_envelopes[left_index];
            for &right_index in &self.order[left_order + 1..] {
                let right_envelope = self.head_envelopes[right_index];
                if right_envelope.maximum_radius == 0.0 {
                    continue; // Default entries belong only to dead source snakes.
                }
                // Monotone positive arithmetic bounds both exact thresholds.
                // An infinite/NaN bound cannot reject, preserving their errors.
                if swept_head_envelopes_separate(
                    left_envelope,
                    right_envelope,
                    (left_envelope.maximum_radius + right_envelope.maximum_radius)
                        * config.hit_scale
                        + CONTACT_SPACE_TOLERANCE,
                ) {
                    continue;
                }
                let right = &food.snakes()[right_index];
                let movement_threshold = combined_threshold(
                    left_envelope.movement_radius,
                    right_envelope.movement_radius,
                    config.hit_scale,
                )?;
                let final_threshold =
                    combined_threshold(left.radius, right.radius, config.hit_scale)?;
                // A pair cannot meet if even its entire swept axis-aligned
                // envelopes stay farther apart than the larger legal radius.
                // Keep the contact tolerance so this only rejects clear misses.
                if swept_head_envelopes_separate(
                    left_envelope,
                    right_envelope,
                    movement_threshold.max(final_threshold) + CONTACT_SPACE_TOLERANCE,
                ) {
                    continue;
                }
                let Some((time, conservative)) = temporal_point_point_contact_time(
                    left.previous_position,
                    left.position,
                    right.previous_position,
                    right.position,
                    movement_threshold,
                    final_threshold,
                )?
                else {
                    continue;
                };
                self.head_head_contacts.push(HeadHeadContact {
                    first_id: left.id,
                    second_id: right.id,
                    time,
                    conservative,
                });
                if conservative {
                    self.conservative_contacts = self.conservative_contacts.saturating_add(1);
                }
                self.death_flags[left_index].head_to_head = true;
                self.death_flags[right_index].head_to_head = true;
            }
        }
        Ok(())
    }

    fn detect_head_body(
        &mut self,
        food: PreparedFood<'_, '_>,
        config: CollisionConfig,
    ) -> Result<(), CollisionError> {
        for &victim_index in &self.order {
            let victim = &food.snakes()[victim_index];
            if !food.source_world().snakes[victim_index].alive {
                continue;
            }
            validate_motion(victim)?;
            let victim_movement_radius = food
                .movement_radius_for_index(victim_index)
                .ok_or(CollisionError::FoodShapeMismatch)?;
            let head_expansion = victim_movement_radius.max(victim.radius) * config.hit_scale;
            let (cells, _) = self.index.collect_candidates(
                victim.previous_position,
                victim.position,
                head_expansion,
                &mut self.query_scratch,
            )?;
            self.query_cells_visited = self.query_cells_visited.saturating_add(cells);
            self.body_candidates = self
                .body_candidates
                .saturating_add(self.query_scratch.candidates.len());
            let mut selected: Option<HeadBodyContact> = None;
            for &segment_index in &self.query_scratch.candidates {
                let segment = self.index.segments[segment_index];
                if segment.owner_id == victim.id
                    || head_head_pair_exists(&self.head_head_contacts, victim.id, segment.owner_id)
                {
                    continue;
                }
                let movement_threshold = combined_threshold(
                    victim_movement_radius,
                    segment.movement_radius,
                    config.hit_scale,
                )?;
                let final_threshold =
                    combined_threshold(victim.radius, segment.final_radius, config.hit_scale)?;
                let hit =
                    head_segment_contact(victim, segment, movement_threshold, final_threshold)?;
                self.maximum_contact_intervals =
                    self.maximum_contact_intervals.max(hit.intervals_examined);
                let Some(time) = hit.time else {
                    continue;
                };
                let candidate = HeadBodyContact {
                    victim_id: victim.id,
                    owner_id: segment.owner_id,
                    segment_end: segment.segment_end,
                    time,
                    conservative: hit.conservative,
                };
                if selected.is_none_or(|current| body_contact_precedes(candidate, current)) {
                    selected = Some(candidate);
                }
            }
            if let Some(contact) = selected {
                if contact.conservative {
                    self.conservative_contacts = self.conservative_contacts.saturating_add(1);
                }
                self.death_flags[victim_index].body_owner_id = Some(contact.owner_id);
                self.head_body_contacts.push(contact);
            }
        }
        Ok(())
    }

    /// Current work and retained capacities, including after rejection.
    #[must_use]
    pub fn diagnostics(&self) -> CollisionDiagnostics {
        CollisionDiagnostics {
            indexed_segments: self.index.segments.len(),
            index_entries: self.index.entries.len(),
            occupied_cells: self.index.cells.len(),
            query_cells_visited: self.query_cells_visited,
            body_candidates: self.body_candidates,
            head_head_contacts: self.head_head_contacts.len(),
            head_body_contacts: self.head_body_contacts.len(),
            deaths: self.deaths.len(),
            awards: self.awards.len(),
            conservative_contacts: self.conservative_contacts,
            maximum_contact_intervals: self.maximum_contact_intervals,
            segment_capacity: self.index.segments.capacity(),
            order_capacity: self.order.capacity(),
            head_envelope_capacity: self.head_envelopes.capacity(),
            entry_capacity: self.index.entries.capacity(),
            cell_capacity: self.index.cells.capacity(),
            candidate_capacity: self.query_scratch.candidates.capacity(),
            seen_generation_capacity: self.query_scratch.seen_generation.capacity(),
            head_head_capacity: self.head_head_contacts.capacity(),
            head_body_capacity: self.head_body_contacts.capacity(),
            death_capacity: self.deaths.capacity(),
            death_flag_capacity: self.death_flags.capacity(),
            award_capacity: self.awards.capacity(),
        }
    }

    /// Whether the latest preparation reached a complete immutable result.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    fn clear(&mut self) {
        self.ready = false;
        #[cfg(feature = "engine-test-hooks")]
        {
            self.phase_timings = CollisionPhaseTimings::default();
        }
        self.order.clear();
        self.head_envelopes.clear();
        self.head_head_contacts.clear();
        self.head_body_contacts.clear();
        self.death_flags.clear();
        self.deaths.clear();
        self.awards.clear();
        self.query_cells_visited = 0;
        self.body_candidates = 0;
        self.conservative_contacts = 0;
        self.maximum_contact_intervals = 0;
        self.index.clear();
        self.query_scratch.candidates.clear();
    }
}

fn swept_segment(
    source_snake: &SnakeState,
    source_body: &[WorldPoint],
    staged: &SnakeState,
    movement_body: &[WorldPoint],
    final_body: &[WorldPoint],
    movement_radius: f64,
    segment_end: usize,
) -> Result<SweptSegment, CollisionError> {
    let existed_during_movement = segment_end < movement_body.len();
    let exists_at_final = segment_end < final_body.len();
    if !existed_during_movement && !exists_at_final {
        return Err(CollisionError::InvalidBodyRange {
            snake_id: staged.id,
        });
    }
    let (current_start, current_end) = if existed_during_movement {
        (movement_body[segment_end - 1], movement_body[segment_end])
    } else {
        (final_body[segment_end - 1], final_body[segment_end])
    };
    if existed_during_movement && exists_at_final {
        let final_start = final_body[segment_end - 1];
        let final_end = final_body[segment_end];
        if final_start != current_start || final_end != current_end {
            return Err(CollisionError::FoodShapeMismatch);
        }
    }
    let newly_grown = !existed_during_movement;
    let removed_at_final = existed_during_movement && !exists_at_final;
    let (previous_start, previous_end) = if newly_grown {
        // A newly grown segment appears only at the final collision boundary;
        // it has no earlier authoritative geometry to sweep from.
        (current_start, current_end)
    } else {
        let previous_start =
            *source_body
                .get(segment_end - 1)
                .ok_or(CollisionError::InvalidBodyRange {
                    snake_id: staged.id,
                })?;
        let previous_end =
            *source_body
                .get(segment_end)
                .ok_or(CollisionError::InvalidBodyRange {
                    snake_id: staged.id,
                })?;
        (previous_start, previous_end)
    };
    for point in [previous_start, previous_end, current_start, current_end] {
        validate_point(point, "swept body point")?;
    }
    if source_snake.id != staged.id {
        return Err(CollisionError::FoodShapeMismatch);
    }
    Ok(SweptSegment {
        owner_id: staged.id,
        segment_end,
        previous_start,
        previous_end,
        current_start,
        current_end,
        movement_radius,
        final_radius: staged.radius,
        newly_grown,
        removed_at_final,
    })
}

fn swept_segment_cell_bounds(
    segment: SweptSegment,
    expansion: f64,
    cell_size: f64,
) -> Result<(CellKey, CellKey), CollisionError> {
    if !expansion.is_finite() || expansion < 0.0 {
        return Err(CollisionError::NonFiniteGeometry {
            context: "segment expansion",
        });
    }
    let minimum_x = segment
        .previous_start
        .x
        .min(segment.previous_end.x)
        .min(segment.current_start.x)
        .min(segment.current_end.x)
        - expansion;
    let maximum_x = segment
        .previous_start
        .x
        .max(segment.previous_end.x)
        .max(segment.current_start.x)
        .max(segment.current_end.x)
        + expansion;
    let minimum_y = segment
        .previous_start
        .y
        .min(segment.previous_end.y)
        .min(segment.current_start.y)
        .min(segment.current_end.y)
        - expansion;
    let maximum_y = segment
        .previous_start
        .y
        .max(segment.previous_end.y)
        .max(segment.current_start.y)
        .max(segment.current_end.y)
        + expansion;
    if !minimum_x.is_finite()
        || !maximum_x.is_finite()
        || !minimum_y.is_finite()
        || !maximum_y.is_finite()
    {
        return Err(CollisionError::NonFiniteGeometry {
            context: "expanded segment bounds",
        });
    }
    Ok((
        CellKey {
            x: cell_coordinate(minimum_x, cell_size)?,
            y: cell_coordinate(minimum_y, cell_size)?,
        },
        CellKey {
            x: cell_coordinate(maximum_x, cell_size)?,
            y: cell_coordinate(maximum_y, cell_size)?,
        },
    ))
}

fn cell_coordinate(value: f64, cell_size: f64) -> Result<i32, CollisionError> {
    let coordinate = (value / cell_size).floor();
    if !coordinate.is_finite()
        || coordinate < f64::from(i32::MIN)
        || coordinate > f64::from(i32::MAX)
    {
        return Err(CollisionError::CellCoordinateOutOfRange);
    }
    Ok(coordinate as i32)
}

fn cell_rectangle_count(minimum: CellKey, maximum: CellKey) -> Result<usize, CollisionError> {
    let width = i64::from(maximum.x)
        .checked_sub(i64::from(minimum.x))
        .and_then(|value| value.checked_add(1))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(CollisionError::ArithmeticOverflow {
            context: "collision cell width",
        })?;
    let height = i64::from(maximum.y)
        .checked_sub(i64::from(minimum.y))
        .and_then(|value| value.checked_add(1))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(CollisionError::ArithmeticOverflow {
            context: "collision cell height",
        })?;
    width
        .checked_mul(height)
        .ok_or(CollisionError::ArithmeticOverflow {
            context: "collision cell rectangle",
        })
}

fn find_cell_span(cells: &[CellSpan], key: CellKey) -> Option<CellSpan> {
    cells
        .binary_search_by_key(&key, |span| span.key)
        .ok()
        .map(|index| cells[index])
}

fn head_head_pair_exists(contacts: &[HeadHeadContact], first: u64, second: u64) -> bool {
    let key = (first.min(second), first.max(second));
    contacts
        .binary_search_by(|contact| (contact.first_id, contact.second_id).cmp(&key))
        .is_ok()
}

fn body_contact_precedes(candidate: HeadBodyContact, current: HeadBodyContact) -> bool {
    candidate
        .time
        .total_cmp(&current.time)
        .then_with(|| candidate.owner_id.cmp(&current.owner_id))
        .then_with(|| candidate.segment_end.cmp(&current.segment_end))
        == Ordering::Less
}

fn combined_threshold(first: f64, second: f64, scale: f64) -> Result<f64, CollisionError> {
    let threshold = (first + second) * scale;
    if !threshold.is_finite() || threshold <= 0.0 {
        return Err(CollisionError::NonFiniteGeometry {
            context: "combined collision threshold",
        });
    }
    Ok(threshold)
}

/// Reject only head trajectories whose full swept envelopes cannot touch.
fn swept_head_envelopes_separate(
    first: SweptHeadEnvelope,
    second: SweptHeadEnvelope,
    radius: f64,
) -> bool {
    if first.min_x > second.max_x + radius || second.min_x > first.max_x + radius {
        return true;
    }
    first.min_y > second.max_y + radius || second.min_y > first.max_y + radius
}

fn swept_point_point_contact_time(
    first_start: WorldPoint,
    first_end: WorldPoint,
    second_start: WorldPoint,
    second_end: WorldPoint,
    threshold: f64,
) -> Result<Option<(f64, bool)>, CollisionError> {
    for point in [first_start, first_end, second_start, second_end] {
        validate_point(point, "head/head trajectory")?;
    }
    let relative_start = subtract(first_start, second_start);
    let relative_velocity = subtract(
        subtract(first_end, first_start),
        subtract(second_end, second_start),
    );
    let c = dot(relative_start, relative_start) - threshold * threshold;
    if c <= 0.0 {
        return Ok(Some((0.0, false)));
    }
    let a = dot(relative_velocity, relative_velocity);
    if a == 0.0 {
        return Ok(None);
    }
    let b = 2.0 * dot(relative_start, relative_velocity);
    let closest_time = (-dot(relative_start, relative_velocity) / a).clamp(0.0, 1.0);
    let closest = WorldPoint {
        x: relative_start.x + relative_velocity.x * closest_time,
        y: relative_start.y + relative_velocity.y * closest_time,
    };
    let threshold_squared = threshold * threshold;
    let conservative_threshold = threshold + CONTACT_SPACE_TOLERANCE;
    let conservative_threshold_squared = conservative_threshold * conservative_threshold;
    if !threshold_squared.is_finite() || !conservative_threshold_squared.is_finite() {
        return Err(CollisionError::NonFiniteGeometry {
            context: "head/head threshold square",
        });
    }
    let closest_squared = dot(closest, closest);
    if closest_squared > conservative_threshold_squared {
        return Ok(None);
    }
    let raw_discriminant = b * b - 4.0 * a * c;
    let discriminant = raw_discriminant.max(0.0);
    if !discriminant.is_finite() {
        return Err(CollisionError::NonFiniteGeometry {
            context: "head/head discriminant",
        });
    }
    let time = (-b - discriminant.sqrt()) / (2.0 * a);
    if (0.0..=1.0).contains(&time) {
        Ok(Some((
            time,
            closest_squared > threshold_squared || raw_discriminant < 0.0,
        )))
    } else if closest_squared <= conservative_threshold_squared {
        Ok(Some((closest_time, true)))
    } else {
        Ok(None)
    }
}

fn temporal_point_point_contact_time(
    first_start: WorldPoint,
    first_end: WorldPoint,
    second_start: WorldPoint,
    second_end: WorldPoint,
    movement_threshold: f64,
    final_threshold: f64,
) -> Result<Option<(f64, bool)>, CollisionError> {
    if let Some(contact) = swept_point_point_contact_time(
        first_start,
        first_end,
        second_start,
        second_end,
        movement_threshold,
    )? {
        if contact.0 < 1.0 {
            return Ok(Some(contact));
        }
    }
    let final_distance = length(subtract(first_end, second_end));
    final_boundary_contact(final_distance, final_threshold)
}

fn final_boundary_contact(
    distance: f64,
    threshold: f64,
) -> Result<Option<(f64, bool)>, CollisionError> {
    if !distance.is_finite() || !threshold.is_finite() || threshold <= 0.0 {
        return Err(CollisionError::NonFiniteGeometry {
            context: "final collision boundary",
        });
    }
    if distance <= threshold {
        return Ok(Some((1.0, false)));
    }
    Ok((distance <= threshold + CONTACT_SPACE_TOLERANCE).then_some((1.0, true)))
}

#[derive(Clone, Copy, Debug, Default)]
struct SweptContactResult {
    time: Option<f64>,
    conservative: bool,
    intervals_examined: usize,
}

fn head_segment_contact(
    victim: &SnakeState,
    segment: SweptSegment,
    movement_threshold: f64,
    final_threshold: f64,
) -> Result<SweptContactResult, CollisionError> {
    if segment.newly_grown {
        let distance =
            point_segment_distance(victim.position, segment.current_start, segment.current_end);
        let contact = final_boundary_contact(distance, final_threshold)?;
        return Ok(SweptContactResult {
            time: contact.map(|entry| entry.0),
            conservative: contact.is_some_and(|entry| entry.1),
            intervals_examined: 1,
        });
    }
    let mut result = swept_point_segment_contact(
        victim.previous_position,
        victim.position,
        segment.previous_start,
        segment.current_start,
        segment.previous_end,
        segment.current_end,
        movement_threshold,
    )?;
    if result.time.is_some_and(|time| time < 1.0) {
        return Ok(result);
    }
    if segment.removed_at_final {
        result.time = None;
        result.conservative = false;
        return Ok(result);
    }
    let final_distance =
        point_segment_distance(victim.position, segment.current_start, segment.current_end);
    let final_contact = final_boundary_contact(final_distance, final_threshold)?;
    result.time = final_contact.map(|entry| entry.0);
    result.conservative = final_contact.is_some_and(|entry| entry.1);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn swept_point_segment_contact(
    head_start: WorldPoint,
    head_end: WorldPoint,
    segment_start_start: WorldPoint,
    segment_start_end: WorldPoint,
    segment_end_start: WorldPoint,
    segment_end_end: WorldPoint,
    threshold: f64,
) -> Result<SweptContactResult, CollisionError> {
    for point in [
        head_start,
        head_end,
        segment_start_start,
        segment_start_end,
        segment_end_start,
        segment_end_end,
    ] {
        validate_point(point, "head/body trajectory")?;
    }
    let first_start = subtract(segment_start_start, head_start);
    let first_end = subtract(segment_start_end, head_end);
    let second_start = subtract(segment_end_start, head_start);
    let second_end = subtract(segment_end_end, head_end);
    let maximum_motion =
        length(subtract(first_end, first_start)).max(length(subtract(second_end, second_start)));
    if !maximum_motion.is_finite() {
        return Err(CollisionError::NonFiniteGeometry {
            context: "relative segment motion",
        });
    }
    let mut intervals_examined = 0usize;
    let found = search_swept_contact(
        first_start,
        first_end,
        second_start,
        second_end,
        threshold,
        maximum_motion,
        0.0,
        1.0,
        0,
        &mut intervals_examined,
    )?;
    Ok(SweptContactResult {
        time: found.map(|entry| entry.0),
        conservative: found.is_some_and(|entry| entry.1),
        intervals_examined,
    })
}

#[allow(clippy::too_many_arguments)]
fn search_swept_contact(
    first_start: WorldPoint,
    first_end: WorldPoint,
    second_start: WorldPoint,
    second_end: WorldPoint,
    threshold: f64,
    maximum_motion: f64,
    interval_start: f64,
    interval_end: f64,
    depth: u32,
    intervals_examined: &mut usize,
) -> Result<Option<(f64, bool)>, CollisionError> {
    *intervals_examined =
        intervals_examined
            .checked_add(1)
            .ok_or(CollisionError::ArithmeticOverflow {
                context: "contact-search interval count",
            })?;
    if *intervals_examined > MAXIMUM_CONTACT_SEARCH_INTERVALS {
        return Err(CollisionError::ContactSearchIntervalLimitExceeded {
            required: *intervals_examined,
            maximum: MAXIMUM_CONTACT_SEARCH_INTERVALS,
        });
    }
    let first_at_start = lerp_point(first_start, first_end, interval_start);
    let second_at_start = lerp_point(second_start, second_end, interval_start);
    if point_segment_distance(ORIGIN, first_at_start, second_at_start) <= threshold {
        return Ok(Some((interval_start, false)));
    }
    let first_at_end = lerp_point(first_start, first_end, interval_end);
    let second_at_end = lerp_point(second_start, second_end, interval_end);
    let lower_bound =
        origin_convex_hull_distance([first_at_start, second_at_start, second_at_end, first_at_end]);
    if lower_bound > threshold + CONTACT_HULL_TOLERANCE {
        return Ok(None);
    }
    let interval_motion = maximum_motion * (interval_end - interval_start);
    if interval_motion <= CONTACT_INTERVAL_MOTION_TOLERANCE {
        // Every endpoint of the interval's relative segment lies within
        // `interval_motion` of its start geometry. Combined with the convex
        // hull slack above, the returned start geometry is at most
        // `CONTACT_SPACE_TOLERANCE` beyond the exact threshold.
        return Ok(Some((interval_start, true)));
    }
    if depth >= MAXIMUM_CONTACT_SEARCH_DEPTH {
        return Err(CollisionError::ContactSearchDepthLimitExceeded {
            depth,
            remaining_motion: interval_motion,
        });
    }
    let midpoint = (interval_start + interval_end) * 0.5;
    let first_half = search_swept_contact(
        first_start,
        first_end,
        second_start,
        second_end,
        threshold,
        maximum_motion,
        interval_start,
        midpoint,
        depth + 1,
        intervals_examined,
    )?;
    if first_half.is_some() {
        return Ok(first_half);
    }
    search_swept_contact(
        first_start,
        first_end,
        second_start,
        second_end,
        threshold,
        maximum_motion,
        midpoint,
        interval_end,
        depth + 1,
        intervals_examined,
    )
}

fn origin_convex_hull_distance(points: [WorldPoint; 4]) -> f64 {
    for first in 0..2 {
        for second in first + 1..3 {
            for third in second + 1..4 {
                if origin_in_triangle(points[first], points[second], points[third]) {
                    return 0.0;
                }
            }
        }
    }
    let mut distance = f64::INFINITY;
    for first in 0..4 {
        distance = distance.min(length(points[first]));
        for second in first + 1..4 {
            distance = distance.min(point_segment_distance(
                ORIGIN,
                points[first],
                points[second],
            ));
        }
    }
    distance
}

fn origin_in_triangle(first: WorldPoint, second: WorldPoint, third: WorldPoint) -> bool {
    if cross(subtract(second, first), subtract(third, first)) == 0.0 {
        return false;
    }
    let first_sign = cross(subtract(second, first), negate(first));
    let second_sign = cross(subtract(third, second), negate(second));
    let third_sign = cross(subtract(first, third), negate(third));
    let has_negative = first_sign < 0.0 || second_sign < 0.0 || third_sign < 0.0;
    let has_positive = first_sign > 0.0 || second_sign > 0.0 || third_sign > 0.0;
    !(has_negative && has_positive)
}

fn point_segment_distance(point: WorldPoint, start: WorldPoint, end: WorldPoint) -> f64 {
    let segment = subtract(end, start);
    let denominator = dot(segment, segment);
    if denominator == 0.0 {
        return length(subtract(point, start));
    }
    let projection = (dot(subtract(point, start), segment) / denominator).clamp(0.0, 1.0);
    let closest = WorldPoint {
        x: start.x + segment.x * projection,
        y: start.y + segment.y * projection,
    };
    length(subtract(point, closest))
}

fn lerp_point(start: WorldPoint, end: WorldPoint, amount: f64) -> WorldPoint {
    WorldPoint {
        x: start.x + (end.x - start.x) * amount,
        y: start.y + (end.y - start.y) * amount,
    }
}

fn subtract(first: WorldPoint, second: WorldPoint) -> WorldPoint {
    WorldPoint {
        x: first.x - second.x,
        y: first.y - second.y,
    }
}

fn negate(point: WorldPoint) -> WorldPoint {
    WorldPoint {
        x: -point.x,
        y: -point.y,
    }
}

fn dot(first: WorldPoint, second: WorldPoint) -> f64 {
    first.x * second.x + first.y * second.y
}

fn cross(first: WorldPoint, second: WorldPoint) -> f64 {
    first.x * second.y - first.y * second.x
}

fn length(point: WorldPoint) -> f64 {
    point.x.hypot(point.y)
}

fn body_slice<'a>(
    world: &'a WorldState,
    snake: &SnakeState,
) -> Result<&'a [WorldPoint], CollisionError> {
    let end = snake
        .body
        .start
        .checked_add(snake.body.len)
        .ok_or(CollisionError::InvalidBodyRange { snake_id: snake.id })?;
    world
        .body_points
        .get(snake.body.start..end)
        .ok_or(CollisionError::InvalidBodyRange { snake_id: snake.id })
}

fn validate_radius(snake: &SnakeState) -> Result<(), CollisionError> {
    validate_radius_value(snake.radius)
}

fn validate_radius_value(radius: f64) -> Result<(), CollisionError> {
    if !radius.is_finite() || radius <= 0.0 {
        return Err(CollisionError::NonFiniteGeometry {
            context: "snake collision radius",
        });
    }
    Ok(())
}

fn validate_motion(snake: &SnakeState) -> Result<(), CollisionError> {
    validate_radius(snake)?;
    validate_point(snake.previous_position, "previous snake position")?;
    validate_point(snake.position, "current snake position")
}

fn validate_point(point: WorldPoint, context: &'static str) -> Result<(), CollisionError> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(CollisionError::NonFiniteGeometry { context });
    }
    Ok(())
}

fn reserve_for<T>(
    values: &mut Vec<T>,
    required: usize,
    context: &'static str,
) -> Result<(), CollisionError> {
    if values.capacity() < required {
        values
            .try_reserve_exact(required.saturating_sub(values.len()))
            .map_err(|_| CollisionError::AllocationFailed { context, required })?;
    }
    Ok(())
}

/// Rejected collision settings, geometry, capacity, or source binding.
#[derive(Clone, Debug, PartialEq)]
pub enum CollisionError {
    /// One collision setting is invalid.
    InvalidConfig { field: &'static str },
    /// Post-food records no longer match their exact source world.
    FoodShapeMismatch,
    /// Stable snake identity is duplicated.
    DuplicateSnakeId(u64),
    /// One body range is empty, incoherent, or out of bounds.
    InvalidBodyRange { snake_id: u64 },
    /// Derived geometry was non-finite.
    NonFiniteGeometry { context: &'static str },
    /// A derived cell coordinate does not fit the admitted integer grid.
    CellCoordinateOutOfRange,
    /// Checked arithmetic failed.
    ArithmeticOverflow { context: &'static str },
    /// Complete swept index storage exceeds the admitted capacity.
    IndexEntryLimitExceeded { required: usize, maximum: usize },
    /// One complete swept-head query exceeds admitted work.
    QueryCellLimitExceeded { required: usize, maximum: usize },
    /// Continuous contact subdivision exceeded its admitted interval work.
    ContactSearchIntervalLimitExceeded { required: usize, maximum: usize },
    /// Continuous contact subdivision reached its depth limit before the
    /// documented spatial ambiguity bound was established.
    ContactSearchDepthLimitExceeded { depth: u32, remaining_motion: f64 },
    /// Reusable scratch could not be reserved.
    AllocationFailed {
        context: &'static str,
        required: usize,
    },
}

impl Display for CollisionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig { field } => write!(formatter, "invalid collision config: {field}"),
            Self::FoodShapeMismatch => {
                write!(formatter, "collision food state does not match source")
            }
            Self::DuplicateSnakeId(id) => write!(formatter, "duplicate collision snake ID {id}"),
            Self::InvalidBodyRange { snake_id } => {
                write!(
                    formatter,
                    "invalid collision body range for snake {snake_id}"
                )
            }
            Self::NonFiniteGeometry { context } => {
                write!(formatter, "collision derived non-finite {context}")
            }
            Self::CellCoordinateOutOfRange => {
                write!(formatter, "collision cell coordinate is outside i32 range")
            }
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "collision arithmetic overflow while calculating {context}"
                )
            }
            Self::IndexEntryLimitExceeded { required, maximum } => write!(
                formatter,
                "collision index needs {required} entries, exceeding maximum {maximum}"
            ),
            Self::QueryCellLimitExceeded { required, maximum } => write!(
                formatter,
                "collision query needs {required} cells, exceeding maximum {maximum}"
            ),
            Self::ContactSearchIntervalLimitExceeded { required, maximum } => write!(
                formatter,
                "collision contact search needs at least {required} intervals, exceeding maximum {maximum}"
            ),
            Self::ContactSearchDepthLimitExceeded {
                depth,
                remaining_motion,
            } => write!(
                formatter,
                "collision contact search reached depth {depth} with {remaining_motion} world units of unresolved relative motion"
            ),
            Self::AllocationFailed { context, required } => write!(
                formatter,
                "collision could not reserve {required} entries for {context}"
            ),
        }
    }
}

impl Error for CollisionError {}

#[cfg(test)]
mod tests;
