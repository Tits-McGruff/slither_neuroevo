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

    fn validate(self) -> Result<(), CollisionError> {
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
        self.entries.sort_unstable_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| {
                    self.segments[left.segment]
                        .owner_id
                        .cmp(&self.segments[right.segment].owner_id)
                })
                .then_with(|| {
                    self.segments[left.segment]
                        .segment_end
                        .cmp(&self.segments[right.segment].segment_end)
                })
        });
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
        scratch.candidates.sort_unstable_by(|left, right| {
            self.segments[*left]
                .owner_id
                .cmp(&self.segments[*right].owner_id)
                .then_with(|| {
                    self.segments[*left]
                        .segment_end
                        .cmp(&self.segments[*right].segment_end)
                })
        });
        Ok((cells_visited, entries_visited))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct DeathFlags {
    wall: bool,
    head_to_head: bool,
    body_owner_id: Option<u64>,
}

/// Reusable collision index, query, and deterministic outcome scratch.
#[derive(Clone, Debug, Default)]
pub struct CollisionWorkspace {
    order: Vec<usize>,
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

        self.index.rebuild(food, &self.order, config)?;
        self.detect_head_head(food, config)?;
        self.detect_head_body(food, config)?;

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
        })
    }

    fn detect_head_head(
        &mut self,
        food: PreparedFood<'_, '_>,
        config: CollisionConfig,
    ) -> Result<(), CollisionError> {
        for left_order in 0..self.order.len() {
            let left_index = self.order[left_order];
            let left = &food.snakes()[left_index];
            if !food.source_world().snakes[left_index].alive {
                continue;
            }
            validate_motion(left)?;
            for &right_index in &self.order[left_order + 1..] {
                let right = &food.snakes()[right_index];
                if !food.source_world().snakes[right_index].alive {
                    continue;
                }
                validate_motion(right)?;
                let left_movement_radius = food
                    .movement_radius_for_index(left_index)
                    .ok_or(CollisionError::FoodShapeMismatch)?;
                let right_movement_radius = food
                    .movement_radius_for_index(right_index)
                    .ok_or(CollisionError::FoodShapeMismatch)?;
                let movement_threshold = combined_threshold(
                    left_movement_radius,
                    right_movement_radius,
                    config.hit_scale,
                )?;
                let final_threshold =
                    combined_threshold(left.radius, right.radius, config.hit_scale)?;
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
        self.order.clear();
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
mod tests {
    use super::*;
    use crate::engine::food::{FoodConfig, FoodWorkspace};
    use crate::engine::movement::{MovementConfig, MovementWorkspace};
    use crate::engine::spatial::{IndexedSensorWorld, SensorIndexConfig};
    use crate::engine::state::{BodyRange, SnakeKind, WorldState};
    use std::mem::size_of;

    const DT: f64 = 1.0 / 180.0;

    #[derive(Clone, Debug, PartialEq)]
    struct CollisionResult {
        head_head: Vec<(u64, u64)>,
        head_body: Vec<(u64, u64, usize)>,
        deaths: Vec<(u64, bool, bool, Option<u64>, bool)>,
        awards: Vec<(u64, u64)>,
        diagnostics: CollisionDiagnostics,
    }

    fn snake(id: u64, position: WorldPoint, direction: f64, length: usize) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id).expect("fixture ID should fit frame v1"),
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(u32::try_from(id - 1).expect("fixture slot should fit")),
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
            position,
            previous_position: position,
            direction,
            radius: 9.0,
            speed: 165.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 10.0,
            kills: 0,
            target_length: length as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start: 0,
                len: length,
            },
            skin: 0,
        }
    }

    fn line_body(position: WorldPoint, direction: f64, length: usize) -> Vec<WorldPoint> {
        (0..length)
            .map(|index| WorldPoint {
                x: position.x - direction.cos() * index as f64 * 7.5,
                y: position.y - direction.sin() * index as f64 * 7.5,
            })
            .collect()
    }

    fn pack_world(entries: Vec<(SnakeState, Vec<WorldPoint>)>) -> WorldState {
        let mut snakes = Vec::with_capacity(entries.len());
        let mut body_points = Vec::new();
        for (mut snake, body) in entries {
            snake.body = BodyRange {
                start: body_points.len(),
                len: body.len(),
            };
            snake.position = body[0];
            snake.previous_position = body[0];
            body_points.extend(body);
            snakes.push(snake);
        }
        WorldState {
            snakes,
            body_points,
            pellets: Vec::new(),
            controller_leases: Vec::new(),
        }
    }

    fn default_index(world: &WorldState) -> IndexedSensorWorld<'_> {
        IndexedSensorWorld::build(
            world,
            SensorIndexConfig {
                body_cell_size: 70.0,
                pellet_cell_size: 120.0,
                maximum_body_entries: 1_000_000,
                maximum_pellet_entries: 1_000_000,
            },
        )
        .expect("fixture world should index")
    }

    fn execute(
        world: &WorldState,
        movement_config: MovementConfig,
        dt: f64,
        collision_config: CollisionConfig,
    ) -> Result<CollisionResult, CollisionError> {
        let indexed = default_index(world);
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(world, movement_config, dt, 100_000, 100_000)
            .expect("fixture movement should prepare");
        let mut food_workspace = FoodWorkspace::new();
        let food = food_workspace
            .prepare(
                &indexed,
                movement,
                movement_config,
                FoodConfig::typescript_defaults(),
                100_000,
                100_000,
            )
            .expect("fixture food should prepare");
        let mut collision_workspace = CollisionWorkspace::new();
        let prepared = collision_workspace.prepare(food, collision_config)?;
        Ok(CollisionResult {
            head_head: prepared
                .head_head_contacts()
                .iter()
                .map(|contact| (contact.first_id, contact.second_id))
                .collect(),
            head_body: prepared
                .head_body_contacts()
                .iter()
                .map(|contact| (contact.victim_id, contact.owner_id, contact.segment_end))
                .collect(),
            deaths: prepared
                .deaths()
                .iter()
                .map(|death| {
                    (
                        death.victim_id,
                        death.wall,
                        death.head_to_head,
                        death.body_owner_id,
                        death.drop_corpse_pellets,
                    )
                })
                .collect(),
            awards: prepared
                .awards()
                .iter()
                .map(|award| (award.killer_id, award.victim_id))
                .collect(),
            diagnostics: prepared.diagnostics(),
        })
    }

    #[test]
    fn point_point_sweep_detects_crossing_without_final_overlap() {
        let contact = swept_point_point_contact_time(
            WorldPoint { x: -10.0, y: 0.0 },
            WorldPoint { x: 10.0, y: 0.0 },
            WorldPoint { x: 10.0, y: 0.0 },
            WorldPoint { x: -10.0, y: 0.0 },
            1.0,
        )
        .expect("finite sweep")
        .expect("crossing should collide");
        assert!((contact.0 - 0.475).abs() <= 1.0e-12);
        assert!(!contact.1);
    }

    #[test]
    fn temporal_head_radii_preserve_shrink_contacts_and_delay_growth_to_final() {
        let shrink = temporal_point_point_contact_time(
            ORIGIN,
            ORIGIN,
            WorldPoint { x: 3.0, y: 0.0 },
            WorldPoint { x: 3.0, y: 0.0 },
            4.0,
            2.0,
        )
        .expect("finite shrinking radii")
        .expect("pre-food radii should collide");
        assert_eq!(shrink.0, 0.0);

        let growth = temporal_point_point_contact_time(
            ORIGIN,
            ORIGIN,
            WorldPoint { x: 10.0, y: 0.0 },
            WorldPoint { x: 3.0, y: 0.0 },
            2.0,
            4.0,
        )
        .expect("finite growing radii")
        .expect("final larger radii should collide");
        assert_eq!(growth.0, 1.0);
    }

    #[test]
    fn point_segment_sweep_detects_crossing_and_rejects_clear_near_miss() {
        let crossing = swept_point_segment_contact(
            WorldPoint { x: -10.0, y: 0.0 },
            WorldPoint { x: 10.0, y: 0.0 },
            WorldPoint { x: 0.0, y: -5.0 },
            WorldPoint { x: 0.0, y: -5.0 },
            WorldPoint { x: 0.0, y: 5.0 },
            WorldPoint { x: 0.0, y: 5.0 },
            1.0,
        )
        .expect("finite crossing");
        assert!(crossing.time.is_some());

        let moving_segment = swept_point_segment_contact(
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: -10.0, y: -5.0 },
            WorldPoint { x: 10.0, y: -5.0 },
            WorldPoint { x: -10.0, y: 5.0 },
            WorldPoint { x: 10.0, y: 5.0 },
            1.0,
        )
        .expect("finite moving-segment crossing");
        assert!(moving_segment.time.is_some());

        let clear = swept_point_segment_contact(
            WorldPoint { x: -10.0, y: 2.0 },
            WorldPoint { x: 10.0, y: 2.0 },
            WorldPoint { x: 0.0, y: -5.0 },
            WorldPoint { x: 0.0, y: -5.0 },
            WorldPoint { x: 0.0, y: 0.0 },
            WorldPoint { x: 0.0, y: 0.0 },
            1.0,
        )
        .expect("finite near miss");
        assert_eq!(clear.time, None);

        let collinear_far = origin_convex_hull_distance([
            WorldPoint { x: 100.0, y: 0.0 },
            WorldPoint { x: 110.0, y: 0.0 },
            WorldPoint { x: 120.0, y: 0.0 },
            WorldPoint { x: 130.0, y: 0.0 },
        ]);
        assert_eq!(collinear_far, 100.0);
    }

    #[test]
    fn newly_grown_segment_participates_only_at_the_final_boundary() {
        let mut victim = snake(1, WorldPoint { x: 10.0, y: 0.0 }, 0.0, 5);
        victim.previous_position = WorldPoint { x: -10.0, y: 0.0 };
        let mut segment = SweptSegment {
            owner_id: 2,
            segment_end: 5,
            previous_start: WorldPoint { x: 0.0, y: -5.0 },
            previous_end: WorldPoint { x: 0.0, y: 5.0 },
            current_start: WorldPoint { x: 0.0, y: -5.0 },
            current_end: WorldPoint { x: 0.0, y: 5.0 },
            movement_radius: 1.0,
            final_radius: 1.0,
            newly_grown: true,
            removed_at_final: false,
        };
        let absent_during_sweep =
            head_segment_contact(&victim, segment, 1.0, 1.0).expect("finite new segment");
        assert_eq!(absent_during_sweep.time, None);

        segment.newly_grown = false;
        let existing_during_sweep =
            head_segment_contact(&victim, segment, 1.0, 1.0).expect("finite existing segment");
        assert!(existing_during_sweep.time.is_some());

        victim.position = WorldPoint { x: 0.0, y: 0.0 };
        segment.newly_grown = true;
        let present_at_final =
            head_segment_contact(&victim, segment, 1.0, 1.0).expect("finite final contact");
        assert_eq!(present_at_final.time, Some(1.0));
    }

    #[test]
    fn temporal_body_radii_preserve_shrink_contacts_and_delay_growth_to_final() {
        let mut victim = snake(1, WorldPoint { x: 3.0, y: 0.0 }, 0.0, 5);
        victim.previous_position = victim.position;
        victim.radius = 1.0;
        let segment = SweptSegment {
            owner_id: 2,
            segment_end: 1,
            previous_start: WorldPoint { x: 0.0, y: -1.0 },
            previous_end: WorldPoint { x: 0.0, y: 1.0 },
            current_start: WorldPoint { x: 0.0, y: -1.0 },
            current_end: WorldPoint { x: 0.0, y: 1.0 },
            movement_radius: 3.0,
            final_radius: 1.0,
            newly_grown: false,
            removed_at_final: false,
        };
        let shrink =
            head_segment_contact(&victim, segment, 4.0, 2.0).expect("finite shrinking body radii");
        assert_eq!(shrink.time, Some(0.0));

        victim.previous_position = WorldPoint { x: 10.0, y: 0.0 };
        let growth =
            head_segment_contact(&victim, segment, 2.0, 4.0).expect("finite growing body radii");
        assert_eq!(growth.time, Some(1.0));
    }

    #[test]
    fn swept_solver_never_misses_a_dense_deterministic_reference_sample() {
        fn next(state: &mut u64) -> f64 {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((*state >> 11) as f64 / ((1_u64 << 53) as f64)) * 40.0 - 20.0
        }

        let mut state = 0x5eed_c011_1510_u64;
        for case in 0..256 {
            let head_start = WorldPoint {
                x: next(&mut state),
                y: next(&mut state),
            };
            let head_end = WorldPoint {
                x: next(&mut state),
                y: next(&mut state),
            };
            let first_start = WorldPoint {
                x: next(&mut state),
                y: next(&mut state),
            };
            let first_end = WorldPoint {
                x: next(&mut state),
                y: next(&mut state),
            };
            let second_start = WorldPoint {
                x: next(&mut state),
                y: next(&mut state),
            };
            let second_end = WorldPoint {
                x: next(&mut state),
                y: next(&mut state),
            };
            let threshold = next(&mut state).abs() * 0.2 + 0.05;
            let result = swept_point_segment_contact(
                head_start,
                head_end,
                first_start,
                first_end,
                second_start,
                second_end,
                threshold,
            )
            .expect("generated coordinates are finite");
            let sampled_contact = (0..=2_000).any(|sample| {
                let time = sample as f64 / 2_000.0;
                point_segment_distance(
                    lerp_point(head_start, head_end, time),
                    lerp_point(first_start, first_end, time),
                    lerp_point(second_start, second_end, time),
                ) <= threshold
            });
            assert!(
                !sampled_contact || result.time.is_some(),
                "case {case} missed a sampled contact"
            );
        }
    }

    #[test]
    fn contact_search_work_exhaustion_rejects_instead_of_inventing_a_death() {
        let result = swept_point_segment_contact(
            ORIGIN,
            ORIGIN,
            WorldPoint {
                x: 180_072_921.461,
                y: 29_040_963.112,
            },
            WorldPoint {
                x: 528_149_625.679,
                y: -284_165_289.248,
            },
            WorldPoint {
                x: -849_316_543.656,
                y: -83_524_087.658,
            },
            WorldPoint {
                x: 728_775_745.439,
                y: 827_863_633.599,
            },
            1.0,
        );

        assert!(matches!(
            result,
            Err(CollisionError::ContactSearchIntervalLimitExceeded {
                required,
                maximum: MAXIMUM_CONTACT_SEARCH_INTERVALS,
            }) if required == MAXIMUM_CONTACT_SEARCH_INTERVALS + 1
        ));
    }

    #[test]
    fn conservative_contact_respects_the_documented_spatial_gap() {
        let gap = CONTACT_HULL_TOLERANCE * 0.5;
        let result = swept_point_segment_contact(
            ORIGIN,
            ORIGIN,
            WorldPoint {
                x: 1.0 + gap,
                y: -1.0,
            },
            WorldPoint {
                x: 1.0 + gap,
                y: -1.0,
            },
            WorldPoint {
                x: 1.0 + gap,
                y: 1.0,
            },
            WorldPoint {
                x: 1.0 + gap,
                y: 1.0,
            },
            1.0,
        )
        .expect("bounded near-contact should resolve");

        assert_eq!(result.time, Some(0.0));
        assert!(result.conservative);
        assert!(
            point_segment_distance(
                ORIGIN,
                WorldPoint {
                    x: 1.0 + gap,
                    y: -1.0,
                },
                WorldPoint {
                    x: 1.0 + gap,
                    y: 1.0,
                },
            ) - 1.0
                <= CONTACT_SPACE_TOLERANCE
        );
    }

    #[test]
    fn swept_bounds_count_every_crossed_cell() {
        let segment = SweptSegment {
            owner_id: 1,
            segment_end: 1,
            previous_start: WorldPoint { x: -25.0, y: -5.0 },
            previous_end: WorldPoint { x: -15.0, y: 5.0 },
            current_start: WorldPoint { x: 15.0, y: -5.0 },
            current_end: WorldPoint { x: 25.0, y: 5.0 },
            movement_radius: 1.0,
            final_radius: 1.0,
            newly_grown: false,
            removed_at_final: false,
        };
        let (minimum, maximum) =
            swept_segment_cell_bounds(segment, 1.0, 10.0).expect("bounds should fit");
        assert_eq!(minimum, CellKey { x: -3, y: -1 });
        assert_eq!(maximum, CellKey { x: 2, y: 0 });
        assert_eq!(cell_rectangle_count(minimum, maximum), Ok(12));
    }

    #[test]
    fn immutable_head_head_snapshot_kills_both_without_neck_credit() {
        let position = WorldPoint { x: 0.0, y: 0.0 };
        let first = snake(1, position, 0.0, 5);
        let second = snake(2, position, std::f64::consts::PI, 5);
        let world = pack_world(vec![
            (first, line_body(position, 0.0, 5)),
            (second, line_body(position, std::f64::consts::PI, 5)),
        ]);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("head overlap should resolve");

        assert_eq!(result.head_head, vec![(1, 2)]);
        assert_eq!(
            result.deaths,
            vec![(1, false, true, None, true), (2, false, true, None, true)]
        );
        assert!(result.head_body.is_empty());
        assert!(result.awards.is_empty());
    }

    fn unambiguous_body_world(reverse: bool) -> WorldState {
        let victim_position = WorldPoint { x: 0.0, y: 0.0 };
        let owner_position = WorldPoint { x: 100.0, y: 0.0 };
        let victim = (
            snake(1, victim_position, 0.0, 5),
            line_body(victim_position, 0.0, 5),
        );
        let owner = (
            snake(2, owner_position, 0.0, 15),
            line_body(owner_position, 0.0, 15),
        );
        if reverse {
            pack_world(vec![owner, victim])
        } else {
            pack_world(vec![victim, owner])
        }
    }

    #[test]
    fn unambiguous_body_owner_credit_is_array_order_invariant() {
        let forward = execute(
            &unambiguous_body_world(false),
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("forward body collision should resolve");
        let reversed = execute(
            &unambiguous_body_world(true),
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("reversed body collision should resolve");

        assert_eq!(forward.head_head, reversed.head_head);
        assert_eq!(forward.head_body, reversed.head_body);
        assert_eq!(forward.deaths, reversed.deaths);
        assert_eq!(forward.awards, reversed.awards);
        assert_eq!(forward.awards, vec![(2, 1)]);
        assert_eq!(forward.deaths, vec![(1, false, false, Some(2), true)]);
    }

    #[test]
    fn exact_multi_body_contact_uses_lower_stable_owner_id() {
        let victim_position = WorldPoint { x: 0.0, y: 0.0 };
        let upper = WorldPoint { x: 0.0, y: 100.0 };
        let lower = WorldPoint { x: 0.0, y: -100.0 };
        let world = pack_world(vec![
            (
                snake(3, lower, -std::f64::consts::FRAC_PI_2, 15),
                line_body(lower, -std::f64::consts::FRAC_PI_2, 15),
            ),
            (
                snake(1, victim_position, 0.0, 5),
                line_body(victim_position, 0.0, 5),
            ),
            (
                snake(2, upper, std::f64::consts::FRAC_PI_2, 15),
                line_body(upper, std::f64::consts::FRAC_PI_2, 15),
            ),
        ]);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("multi-body collision should resolve");

        assert_eq!(result.awards, vec![(2, 1)]);
        assert_eq!(result.head_body[0].0, 1);
        assert_eq!(result.head_body[0].1, 2);
    }

    #[test]
    fn same_owner_exact_time_uses_lower_stable_segment_offset() {
        let victim_position = WorldPoint { x: 0.0, y: 0.0 };
        let owner_position = WorldPoint { x: 0.0, y: 50.0 };
        let owner_body = (0..10)
            .map(|index| WorldPoint {
                x: 0.0,
                y: 50.0 - index as f64 * 10.0,
            })
            .collect();
        let mut owner = snake(2, owner_position, std::f64::consts::FRAC_PI_2, 10);
        owner.radius = 0.1;
        let mut victim = snake(1, victim_position, 0.0, 5);
        victim.radius = 0.1;
        let world = pack_world(vec![
            (owner, owner_body),
            (victim, line_body(victim_position, 0.0, 5)),
        ]);
        let movement = MovementConfig {
            snake_radius: 0.1,
            snake_radius_max: 0.1,
            snake_thickness_scale: 0.0,
            snake_spacing: 10.0,
            world_radius: 10_000.0,
            ..MovementConfig::typescript_defaults()
        };
        let result = execute(
            &world,
            movement,
            1.0e-6,
            CollisionConfig {
                hit_scale: 1.0,
                ..CollisionConfig::typescript_defaults()
            },
        )
        .expect("same-owner segment tie should resolve");

        assert_eq!(result.head_body, vec![(1, 2, 5)]);
        assert_eq!(result.awards, vec![(2, 1)]);
    }

    #[test]
    fn body_of_simultaneously_killed_owner_remains_an_obstacle() {
        let victim_position = WorldPoint { x: 0.0, y: 0.0 };
        let owner_position = WorldPoint { x: 100.0, y: 0.0 };
        let world = pack_world(vec![
            (
                snake(1, victim_position, 0.0, 5),
                line_body(victim_position, 0.0, 5),
            ),
            (
                snake(2, owner_position, 0.0, 15),
                line_body(owner_position, 0.0, 15),
            ),
            (
                snake(3, owner_position, std::f64::consts::PI, 5),
                line_body(owner_position, std::f64::consts::PI, 5),
            ),
        ]);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("immutable collision snapshot should resolve");

        assert!(result.head_head.contains(&(2, 3)));
        assert!(result.deaths.iter().any(|death| death.0 == 2));
        assert!(result.deaths.iter().any(|death| death.0 == 3));
        assert_eq!(result.awards, vec![(2, 1)]);
    }

    #[test]
    fn wall_killed_body_owner_remains_an_obstacle_for_the_same_snapshot() {
        let victim_position = WorldPoint { x: 3_415.2, y: 0.0 };
        let owner_position = WorldPoint { x: 3_490.2, y: 0.0 };
        let world = pack_world(vec![
            (
                snake(1, victim_position, std::f64::consts::FRAC_PI_2, 5),
                line_body(victim_position, std::f64::consts::FRAC_PI_2, 5),
            ),
            (
                snake(2, owner_position, 0.0, 15),
                line_body(owner_position, 0.0, 15),
            ),
        ]);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("wall and body outcomes should share one snapshot");

        assert_eq!(result.awards, vec![(2, 1)]);
        assert_eq!(
            result.deaths,
            vec![
                (1, false, false, Some(2), true),
                (2, true, false, None, false),
            ]
        );
    }

    #[test]
    fn simultaneous_wall_and_head_contact_kills_both_without_credit() {
        let position = WorldPoint { x: 3_490.2, y: 0.0 };
        let world = pack_world(vec![
            (snake(1, position, 0.0, 5), line_body(position, 0.0, 5)),
            (
                snake(2, position, std::f64::consts::PI, 5),
                line_body(position, std::f64::consts::PI, 5),
            ),
        ]);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("wall and head contact should resolve together");

        assert_eq!(result.head_head, vec![(1, 2)]);
        assert_eq!(
            result.deaths,
            vec![(1, true, true, None, false), (2, false, true, None, true)]
        );
        assert!(result.awards.is_empty());
    }

    #[test]
    fn ordinary_shrink_tail_sweeps_before_disappearing_at_final_boundary() {
        let victim_position = WorldPoint { x: 0.0, y: 0.0 };
        let owner_position = WorldPoint { x: 100.0, y: 0.0 };
        let victim = snake(1, victim_position, std::f64::consts::FRAC_PI_2, 5);
        let mut owner = snake(2, owner_position, 0.0, 15);
        owner.target_length = 5.0;
        let world = pack_world(vec![
            (
                victim,
                line_body(victim_position, std::f64::consts::FRAC_PI_2, 5),
            ),
            (owner, line_body(owner_position, 0.0, 15)),
        ]);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("ordinary-shrink sweep should resolve");

        assert_eq!(result.awards, vec![(2, 1)]);
        assert!(result.head_body[0].2 >= 5);
    }

    #[test]
    fn swept_head_crosses_body_even_when_both_endpoints_are_clear() {
        let victim_position = WorldPoint { x: -30.0, y: 0.0 };
        let owner_position = WorldPoint { x: 0.0, y: 50.0 };
        let mut victim = snake(1, victim_position, 0.0, 5);
        victim.speed = 10_000.0;
        victim.radius = 1.0;
        let mut owner = snake(2, owner_position, std::f64::consts::FRAC_PI_2, 9);
        owner.radius = 1.0;
        let world = pack_world(vec![
            (victim, line_body(victim_position, 0.0, 5)),
            (
                owner,
                line_body(owner_position, std::f64::consts::FRAC_PI_2, 9),
            ),
        ]);
        let movement = MovementConfig {
            snake_radius: 1.0,
            snake_radius_max: 1.0,
            snake_thickness_scale: 0.0,
            world_radius: 10_000.0,
            ..MovementConfig::typescript_defaults()
        };
        let result = execute(
            &world,
            movement,
            0.006,
            CollisionConfig {
                hit_scale: 1.0,
                ..CollisionConfig::typescript_defaults()
            },
        )
        .expect("swept crossing should resolve");

        assert_eq!(result.awards, vec![(2, 1)]);
        assert_eq!(result.deaths, vec![(1, false, false, Some(2), true)]);
    }

    #[test]
    fn earlier_world_contact_beats_a_later_lower_owner_id() {
        let victim_position = WorldPoint { x: -30.0, y: 0.0 };
        let mut victim = snake(1, victim_position, 0.0, 5);
        victim.speed = 10_000.0;
        victim.radius = 1.0;
        let mut later_low_id = snake(
            2,
            WorldPoint { x: 10.0, y: 50.0 },
            std::f64::consts::FRAC_PI_2,
            9,
        );
        later_low_id.radius = 1.0;
        let mut earlier_high_id = snake(
            3,
            WorldPoint { x: -10.0, y: 50.0 },
            std::f64::consts::FRAC_PI_2,
            9,
        );
        earlier_high_id.radius = 1.0;
        let world = pack_world(vec![
            (
                later_low_id,
                line_body(
                    WorldPoint { x: 10.0, y: 50.0 },
                    std::f64::consts::FRAC_PI_2,
                    9,
                ),
            ),
            (victim, line_body(victim_position, 0.0, 5)),
            (
                earlier_high_id,
                line_body(
                    WorldPoint { x: -10.0, y: 50.0 },
                    std::f64::consts::FRAC_PI_2,
                    9,
                ),
            ),
        ]);
        let movement = MovementConfig {
            snake_radius: 1.0,
            snake_radius_max: 1.0,
            snake_thickness_scale: 0.0,
            world_radius: 10_000.0,
            ..MovementConfig::typescript_defaults()
        };
        let result = execute(
            &world,
            movement,
            0.006,
            CollisionConfig {
                hit_scale: 1.0,
                ..CollisionConfig::typescript_defaults()
            },
        )
        .expect("two swept owner contacts should resolve");

        assert_eq!(result.awards, vec![(3, 1)]);
        assert_eq!(result.head_body[0].1, 3);
    }

    #[test]
    fn configured_head_segment_skip_is_honored() {
        let world = unambiguous_body_world(false);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig {
                skip_segments: 15,
                ..CollisionConfig::typescript_defaults()
            },
        )
        .expect("skipped collision index should resolve");

        assert!(result.deaths.is_empty());
        assert!(result.awards.is_empty());
    }

    #[test]
    fn self_body_is_never_a_collision_target() {
        let position = WorldPoint { x: 0.0, y: 0.0 };
        let world = pack_world(vec![(
            snake(1, position, 0.0, 4),
            vec![
                position,
                WorldPoint { x: -7.5, y: 0.0 },
                WorldPoint { x: 0.0, y: 0.0 },
                WorldPoint { x: 7.5, y: 0.0 },
            ],
        )]);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("self-overlap should not fault");

        assert!(result.deaths.is_empty());
        assert!(result.awards.is_empty());
    }

    #[test]
    fn wall_death_is_carried_without_corpse_drop_or_award() {
        let position = WorldPoint { x: 3_490.2, y: 0.0 };
        let world = pack_world(vec![(
            snake(1, position, 0.0, 5),
            line_body(position, 0.0, 5),
        )]);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig::typescript_defaults(),
        )
        .expect("wall death should resolve");

        assert_eq!(result.deaths, vec![(1, true, false, None, false)]);
        assert!(result.awards.is_empty());
    }

    #[test]
    fn index_capacity_rejects_complete_snapshot_without_authority_write() {
        let world = unambiguous_body_world(false);
        let authority_before = world.clone();
        let indexed = default_index(&world);
        let movement_config = MovementConfig::typescript_defaults();
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, movement_config, DT, 100_000, 100_000)
            .expect("movement should prepare");
        let mut food_workspace = FoodWorkspace::new();
        let food = food_workspace
            .prepare(
                &indexed,
                movement,
                movement_config,
                FoodConfig::typescript_defaults(),
                100_000,
                100_000,
            )
            .expect("food should prepare");
        let mut collision_workspace = CollisionWorkspace::new();
        let result = collision_workspace.prepare(
            food,
            CollisionConfig {
                cell_size: 1.0,
                maximum_index_entries: 1,
                ..CollisionConfig::typescript_defaults()
            },
        );

        assert!(matches!(
            result,
            Err(CollisionError::IndexEntryLimitExceeded { maximum: 1, .. })
        ));
        assert!(!collision_workspace.is_ready());
        assert_eq!(world, authority_before);
    }

    #[test]
    fn oversized_head_query_rejects_the_snapshot_instead_of_truncating_truth() {
        let world = unambiguous_body_world(false);
        let result = execute(
            &world,
            MovementConfig::typescript_defaults(),
            DT,
            CollisionConfig {
                cell_size: 1.0,
                maximum_index_entries: 1_000_000,
                maximum_query_cells: 1,
                ..CollisionConfig::typescript_defaults()
            },
        );

        assert!(matches!(
            result,
            Err(CollisionError::QueryCellLimitExceeded { maximum: 1, .. })
        ));
    }

    #[test]
    fn warmed_detection_reuses_every_owned_capacity() {
        let world = unambiguous_body_world(false);
        let indexed = default_index(&world);
        let movement_config = MovementConfig::typescript_defaults();
        let mut movement_workspace = MovementWorkspace::new();
        let movement = movement_workspace
            .prepare(&world, movement_config, DT, 100_000, 100_000)
            .expect("movement should prepare");
        let mut food_workspace = FoodWorkspace::new();
        let food = food_workspace
            .prepare(
                &indexed,
                movement,
                movement_config,
                FoodConfig::typescript_defaults(),
                100_000,
                100_000,
            )
            .expect("food should prepare");
        let mut workspace = CollisionWorkspace::new();
        let first = workspace
            .prepare(food, CollisionConfig::typescript_defaults())
            .expect("warm collision should prepare")
            .diagnostics();

        for _ in 0..24 {
            let next = workspace
                .prepare(food, CollisionConfig::typescript_defaults())
                .expect("reused collision should prepare")
                .diagnostics();
            assert_eq!(next.segment_capacity, first.segment_capacity);
            assert_eq!(next.order_capacity, first.order_capacity);
            assert_eq!(next.entry_capacity, first.entry_capacity);
            assert_eq!(next.cell_capacity, first.cell_capacity);
            assert_eq!(next.candidate_capacity, first.candidate_capacity);
            assert_eq!(
                next.seen_generation_capacity,
                first.seen_generation_capacity
            );
            assert_eq!(next.head_head_capacity, first.head_head_capacity);
            assert_eq!(next.head_body_capacity, first.head_body_capacity);
            assert_eq!(next.death_capacity, first.death_capacity);
            assert_eq!(next.death_flag_capacity, first.death_flag_capacity);
            assert_eq!(next.award_capacity, first.award_capacity);
        }
    }

    #[test]
    fn earlier_contact_precedes_owner_id_and_exact_time_uses_stable_owner() {
        let later_low_id = HeadBodyContact {
            victim_id: 1,
            owner_id: 2,
            segment_end: 1,
            time: 0.75,
            conservative: false,
        };
        let earlier_high_id = HeadBodyContact {
            victim_id: 1,
            owner_id: 3,
            segment_end: 1,
            time: 0.25,
            conservative: false,
        };
        assert!(body_contact_precedes(earlier_high_id, later_low_id));

        let tied_low_id = HeadBodyContact {
            time: 0.25,
            ..later_low_id
        };
        assert!(body_contact_precedes(tied_low_id, earlier_high_id));
    }

    #[test]
    fn diagnostic_struct_remains_plain_bounded_metadata() {
        assert!(size_of::<CollisionDiagnostics>() <= 24 * size_of::<usize>());
    }
}
