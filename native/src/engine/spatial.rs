//! Complete deterministic spatial indexes derived from one stable world view.
//!
//! The current TypeScript collision grid stores one midpoint per segment. That
//! can miss long segments and its historical capacity ceiling could silently
//! omit collision truth. This module stores every segment in every cell touched
//! by its axis-aligned bounds, checks the complete entry count before the main
//! allocation, and keeps collision truth separate from later sensor work caps.

use super::state::{PelletState, WorldPoint, WorldState};
use std::cmp::Ordering;
use std::error::Error;
use std::fmt;
use std::mem::size_of;

/// Dense lookup is allowed up to this many cell coordinates per index.
const MAXIMUM_DENSE_LOOKUP_CELLS: usize = 262_144;
/// Sparse grids stay on sorted binary lookup beyond this coordinate/span ratio.
const MAXIMUM_DENSE_LOOKUP_RATIO: usize = 16;
/// Small bounded world grids use direct lookup even when sparsely occupied.
const MINIMUM_DENSE_LOOKUP_ALLOWANCE: usize = 4_096;

/// Integer spatial-cell identity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct CellKey {
    x: i32,
    y: i32,
}

/// One sorted contiguous cell range in a flat entry array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellSpan {
    key: CellKey,
    start: usize,
    end: usize,
}

/// Optional bounded direct mapping from cell coordinates to sorted spans.
#[derive(Clone, Debug, Default)]
struct CellLookup {
    minimum: CellKey,
    width: usize,
    span_indices: Vec<usize>,
}

impl CellLookup {
    fn build(cells: &[CellSpan]) -> Result<Self, SpatialIndexError> {
        let Some(first) = cells.first() else {
            return Ok(Self::default());
        };
        let mut minimum = first.key;
        let mut maximum = first.key;
        for span in &cells[1..] {
            minimum.x = minimum.x.min(span.key.x);
            minimum.y = minimum.y.min(span.key.y);
            maximum.x = maximum.x.max(span.key.x);
            maximum.y = maximum.y.max(span.key.y);
        }
        let width = coordinate_span(minimum.x, maximum.x, "cell lookup width")?;
        let height = coordinate_span(minimum.y, maximum.y, "cell lookup height")?;
        let Some(area) = width.checked_mul(height) else {
            // A coordinate envelope too large to describe densely is exactly
            // the case this optional lookup is meant to decline. The sorted
            // cell spans remain the complete index and provide the fallback.
            return Ok(Self::default());
        };
        let density_allowance = cells
            .len()
            .saturating_mul(MAXIMUM_DENSE_LOOKUP_RATIO)
            .clamp(MINIMUM_DENSE_LOOKUP_ALLOWANCE, MAXIMUM_DENSE_LOOKUP_CELLS);
        if area > density_allowance {
            return Ok(Self::default());
        }
        let mut span_indices = Vec::new();
        span_indices
            .try_reserve_exact(area)
            .map_err(|_| SpatialIndexError::AllocationFailed {
                context: "dense cell lookup",
                requested: area,
            })?;
        span_indices.resize(area, usize::MAX);
        for (span_index, span) in cells.iter().enumerate() {
            let offset = cell_lookup_offset(minimum, width, span.key).ok_or(
                SpatialIndexError::ArithmeticOverflow {
                    context: "dense cell lookup offset",
                },
            )?;
            span_indices[offset] = span_index;
        }
        Ok(Self {
            minimum,
            width,
            span_indices,
        })
    }

    fn find(&self, cells: &[CellSpan], key: CellKey) -> Option<CellSpan> {
        if self.span_indices.is_empty() {
            return find_cell_span(cells, key);
        }
        let offset = cell_lookup_offset(self.minimum, self.width, key)?;
        let span_index = *self.span_indices.get(offset)?;
        (span_index != usize::MAX).then(|| cells[span_index])
    }

    const fn cells(&self) -> usize {
        self.span_indices.len()
    }

    fn estimated_bytes(&self) -> Result<usize, SpatialIndexError> {
        self.span_indices
            .capacity()
            .checked_mul(size_of::<usize>())
            .ok_or(SpatialIndexError::ArithmeticOverflow {
                context: "dense cell lookup bytes",
            })
    }
}

/// Stable derived reference to one snake-body segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodySegmentRecord {
    /// Stable internal owner identity.
    pub owner_id: u64,
    /// Dense snake index in the stable source view used to build this index.
    pub snake_index: usize,
    /// One-based segment-end offset inside the owner's head-to-tail body.
    pub segment_end: usize,
    /// Segment start point.
    pub start: WorldPoint,
    /// Segment end point.
    pub end: WorldPoint,
    /// Owner collision radius at index-build time.
    pub owner_radius: f64,
}

/// One cell-to-segment entry; a long segment deliberately has several entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BodyCellEntry {
    key: CellKey,
    segment: usize,
}

/// A body candidate ordered by spatial lower bound and stable identity.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BodyCandidate {
    segment: usize,
    lower_bound_squared: f64,
    owner_id: u64,
    segment_end: usize,
}

/// Reusable per-worker duplicate-suppression and candidate storage.
#[derive(Clone, Debug, Default)]
pub struct BodyQueryScratch {
    seen_generation: Vec<u32>,
    generation: u32,
    candidates: Vec<BodyCandidate>,
}

/// Reusable storage for a bounded sensor-only body query.
///
/// This distinct type cannot be passed to [`BodySpatialIndex::candidates`],
/// which is the complete collision-query iterator. A cap-hit prefix is useful
/// only for sensor diagnostics because the sensor output is then saturated.
#[derive(Clone, Debug, Default)]
pub struct BodySensorQueryScratch {
    inner: BodyQueryScratch,
}

impl BodySensorQueryScratch {
    /// Number of candidates retained by the most recent sensor query.
    #[must_use]
    pub fn candidate_count(&self) -> usize {
        self.inner.candidate_count()
    }

    /// Allocated bounded candidate slots retained for reuse.
    #[must_use]
    pub fn candidate_capacity(&self) -> usize {
        self.inner.candidate_capacity()
    }

    /// Allocated duplicate-marker slots retained for reuse.
    #[must_use]
    pub fn duplicate_marker_capacity(&self) -> usize {
        self.inner.duplicate_marker_capacity()
    }
}

impl BodyQueryScratch {
    /// Number of unique candidates retained by the most recent query.
    #[must_use]
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// Allocated candidate slots retained for reuse by later queries.
    #[must_use]
    pub fn candidate_capacity(&self) -> usize {
        self.candidates.capacity()
    }

    /// Allocated duplicate-marker slots retained for later queries.
    #[must_use]
    pub fn duplicate_marker_capacity(&self) -> usize {
        self.seen_generation.capacity()
    }

    /// Reuse checked allocations while beginning a query.
    fn begin(
        &mut self,
        record_count: usize,
        candidate_capacity: usize,
    ) -> Result<(), SpatialIndexError> {
        if self.seen_generation.len() < record_count {
            self.seen_generation
                .try_reserve_exact(record_count - self.seen_generation.len())
                .map_err(|_| SpatialIndexError::AllocationFailed {
                    context: "body-query duplicate markers",
                    requested: record_count,
                })?;
            self.seen_generation.resize(record_count, 0);
        }
        if self.candidates.capacity() < candidate_capacity {
            self.candidates
                .try_reserve_exact(candidate_capacity.saturating_sub(self.candidates.len()))
                .map_err(|_| SpatialIndexError::AllocationFailed {
                    context: "body-query candidates",
                    requested: candidate_capacity,
                })?;
        }
        self.candidates.clear();
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.seen_generation.fill(0);
            self.generation = 1;
        }
        Ok(())
    }
}

/// Exact operational counts from one body-index build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BodyIndexDiagnostics {
    /// Unique alive body segments.
    pub segments: usize,
    /// Total cell entries after complete AABB coverage.
    pub entries: usize,
    /// Number of occupied cells.
    pub occupied_cells: usize,
    /// Coordinate slots in the bounded direct lookup, or zero for fallback.
    pub lookup_cells: usize,
    /// Configured admission ceiling for cell entries.
    pub maximum_entries: usize,
    /// Estimated owned bytes for records, entries, cell spans, and lookup.
    pub estimated_bytes: usize,
}

/// Query work counts that do not affect simulation decisions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialQueryDiagnostics {
    /// Cell ranges visited, including empty coordinates.
    pub cells_visited: usize,
    /// Raw cell entries visited before duplicate suppression.
    pub entries_visited: usize,
    /// Unique candidates retained after bounds filtering.
    pub candidates: usize,
    /// More qualifying candidates existed than the caller's bounded limit.
    pub candidate_limit_reached: bool,
}

/// Complete immutable body-segment index for one stable pre-movement state.
#[derive(Clone, Debug)]
pub struct BodySpatialIndex {
    cell_size: f64,
    records: Vec<BodySegmentRecord>,
    entries: Vec<BodyCellEntry>,
    cells: Vec<CellSpan>,
    lookup: CellLookup,
    maximum_owner_radius: f64,
    diagnostics: BodyIndexDiagnostics,
}

impl BodySpatialIndex {
    /// Build a complete body index or fail before a partial index can be used.
    pub fn build(
        world: &WorldState,
        cell_size: f64,
        maximum_entries: usize,
    ) -> Result<Self, SpatialIndexError> {
        validate_cell_size(cell_size)?;
        let mut segment_count = 0usize;
        let mut maximum_owner_radius = 0.0_f64;
        for snake in &world.snakes {
            if !snake.alive || snake.body.len < 2 {
                continue;
            }
            let body_end = snake.body.start.checked_add(snake.body.len).ok_or(
                SpatialIndexError::ArithmeticOverflow {
                    context: "snake body range",
                },
            )?;
            if body_end > world.body_points.len() {
                return Err(SpatialIndexError::InvalidBodyRange { snake_id: snake.id });
            }
            maximum_owner_radius = maximum_owner_radius.max(snake.radius);
            segment_count = segment_count.checked_add(snake.body.len - 1).ok_or(
                SpatialIndexError::ArithmeticOverflow {
                    context: "body-index segment count",
                },
            )?;
        }
        if segment_count > maximum_entries {
            return Err(SpatialIndexError::EntryLimitExceeded {
                kind: "body",
                required: segment_count,
                maximum: maximum_entries,
            });
        }

        let mut records = Vec::new();
        records.try_reserve_exact(segment_count).map_err(|_| {
            SpatialIndexError::AllocationFailed {
                context: "body-index records",
                requested: segment_count,
            }
        })?;
        let mut required_entries = 0usize;
        for (snake_index, snake) in world.snakes.iter().enumerate() {
            if !snake.alive || snake.body.len < 2 {
                continue;
            }
            for segment_end in 1..snake.body.len {
                let start = world.body_points[snake.body.start + segment_end - 1];
                let end = world.body_points[snake.body.start + segment_end];
                validate_point(start)?;
                validate_point(end)?;
                let (minimum, maximum) = segment_cell_bounds(start, end, cell_size)?;
                required_entries = required_entries
                    .checked_add(cell_rectangle_count(minimum, maximum)?)
                    .ok_or(SpatialIndexError::ArithmeticOverflow {
                        context: "body-index entry count",
                    })?;
                if required_entries > maximum_entries {
                    return Err(SpatialIndexError::EntryLimitExceeded {
                        kind: "body",
                        required: required_entries,
                        maximum: maximum_entries,
                    });
                }
                records.push(BodySegmentRecord {
                    owner_id: snake.id,
                    snake_index,
                    segment_end,
                    start,
                    end,
                    owner_radius: snake.radius,
                });
            }
        }

        let mut entries = Vec::new();
        entries.try_reserve_exact(required_entries).map_err(|_| {
            SpatialIndexError::AllocationFailed {
                context: "body-index entries",
                requested: required_entries,
            }
        })?;
        for (segment, record) in records.iter().enumerate() {
            let (minimum, maximum) = segment_cell_bounds(record.start, record.end, cell_size)?;
            for y in minimum.y..=maximum.y {
                for x in minimum.x..=maximum.x {
                    entries.push(BodyCellEntry {
                        key: CellKey { x, y },
                        segment,
                    });
                }
            }
        }
        debug_assert_eq!(entries.len(), required_entries);
        entries.sort_unstable_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| {
                    records[left.segment]
                        .owner_id
                        .cmp(&records[right.segment].owner_id)
                })
                .then_with(|| {
                    records[left.segment]
                        .segment_end
                        .cmp(&records[right.segment].segment_end)
                })
        });
        let cells = build_cell_spans(&entries, |entry| entry.key)?;
        let lookup = CellLookup::build(&cells)?;
        let estimated_bytes = checked_owned_bytes(&[
            (records.capacity(), size_of::<BodySegmentRecord>()),
            (entries.capacity(), size_of::<BodyCellEntry>()),
            (cells.capacity(), size_of::<CellSpan>()),
        ])?
        .checked_add(lookup.estimated_bytes()?)
        .ok_or(SpatialIndexError::ArithmeticOverflow {
            context: "body-index estimated bytes",
        })?;
        let diagnostics = BodyIndexDiagnostics {
            segments: records.len(),
            entries: entries.len(),
            occupied_cells: cells.len(),
            lookup_cells: lookup.cells(),
            maximum_entries,
            estimated_bytes,
        };
        Ok(Self {
            cell_size,
            records,
            entries,
            cells,
            lookup,
            maximum_owner_radius,
            diagnostics,
        })
    }

    /// Return immutable build diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> BodyIndexDiagnostics {
        self.diagnostics
    }

    /// Largest alive owner radius represented by this derived index.
    #[must_use]
    pub fn maximum_owner_radius(&self) -> f64 {
        self.maximum_owner_radius
    }

    /// Retrieve one stable derived segment record.
    #[must_use]
    pub fn segment(&self, index: usize) -> Option<&BodySegmentRecord> {
        self.records.get(index)
    }

    /// Collect every unique candidate whose segment AABB intersects the query.
    ///
    /// This complete path is for collision truth. It preflights reusable
    /// storage for every record and never applies a sensor work cap.
    pub fn collect_candidates(
        &self,
        center: WorldPoint,
        radius: f64,
        scratch: &mut BodyQueryScratch,
    ) -> Result<SpatialQueryDiagnostics, SpatialIndexError> {
        validate_query(center, radius)?;
        scratch.begin(self.records.len(), self.records.len())?;
        let mut diagnostics = self.visit_query_cells(center, radius, |entry, diagnostics| {
            diagnostics.entries_visited = diagnostics.entries_visited.saturating_add(1);
            if scratch.seen_generation[entry.segment] == scratch.generation {
                return;
            }
            scratch.seen_generation[entry.segment] = scratch.generation;
            let record = &self.records[entry.segment];
            let lower_bound_squared =
                point_to_aabb_distance_squared(center, record.start, record.end);
            if lower_bound_squared > radius * radius {
                return;
            }
            scratch.candidates.push(BodyCandidate {
                segment: entry.segment,
                lower_bound_squared,
                owner_id: record.owner_id,
                segment_end: record.segment_end,
            });
        })?;
        scratch.candidates.sort_unstable_by(compare_body_candidates);
        diagnostics.candidates = scratch.candidates.len();
        Ok(diagnostics)
    }

    /// Retain at most `maximum_candidates` nearest non-owner candidates.
    ///
    /// Uncapped results match complete exact-distance ordering. Once an
    /// additional qualifying candidate proves the cap was exceeded, traversal
    /// stops because the sensor contract conservatively saturates every body
    /// hazard; the retained capped prefix must not be used as collision truth.
    pub fn collect_sensor_candidates(
        &self,
        center: WorldPoint,
        radius: f64,
        excluded_owner_id: u64,
        maximum_candidates: usize,
        scratch: &mut BodySensorQueryScratch,
    ) -> Result<SpatialQueryDiagnostics, SpatialIndexError> {
        let scratch = &mut scratch.inner;
        validate_query(center, radius)?;
        scratch.begin(self.records.len(), maximum_candidates)?;
        let radius_squared = radius * radius;
        let mut qualifying = 0usize;
        let mut diagnostics = SpatialQueryDiagnostics::default();
        visit_query_cells_nearest_first(
            center,
            radius,
            self.cell_size,
            |key| {
                diagnostics.cells_visited = diagnostics.cells_visited.saturating_add(1);
                let Some(span) = self.lookup.find(&self.cells, key) else {
                    return true;
                };
                for entry in &self.entries[span.start..span.end] {
                    diagnostics.entries_visited = diagnostics.entries_visited.saturating_add(1);
                    if scratch.seen_generation[entry.segment] == scratch.generation {
                        continue;
                    }
                    scratch.seen_generation[entry.segment] = scratch.generation;
                    let record = &self.records[entry.segment];
                    if record.owner_id == excluded_owner_id {
                        continue;
                    }
                    let lower_bound_squared =
                        point_to_segment_distance_squared(center, record.start, record.end);
                    if lower_bound_squared > radius_squared {
                        continue;
                    }
                    qualifying = qualifying.saturating_add(1);
                    retain_bounded(
                        &mut scratch.candidates,
                        BodyCandidate {
                            segment: entry.segment,
                            lower_bound_squared,
                            owner_id: record.owner_id,
                            segment_end: record.segment_end,
                        },
                        maximum_candidates,
                        compare_body_candidates,
                    );
                    if qualifying > maximum_candidates {
                        return false;
                    }
                }
                true
            },
            |_| false,
        )?;
        scratch.candidates.sort_unstable_by(compare_body_candidates);
        diagnostics.candidates = scratch.candidates.len();
        diagnostics.candidate_limit_reached = qualifying > maximum_candidates;
        Ok(diagnostics)
    }

    fn visit_query_cells(
        &self,
        center: WorldPoint,
        radius: f64,
        mut visit: impl FnMut(&BodyCellEntry, &mut SpatialQueryDiagnostics),
    ) -> Result<SpatialQueryDiagnostics, SpatialIndexError> {
        let minimum = CellKey {
            x: cell_coordinate(center.x - radius, self.cell_size)?,
            y: cell_coordinate(center.y - radius, self.cell_size)?,
        };
        let maximum = CellKey {
            x: cell_coordinate(center.x + radius, self.cell_size)?,
            y: cell_coordinate(center.y + radius, self.cell_size)?,
        };
        let mut diagnostics = SpatialQueryDiagnostics::default();
        for y in minimum.y..=maximum.y {
            for x in minimum.x..=maximum.x {
                diagnostics.cells_visited = diagnostics.cells_visited.saturating_add(1);
                let Some(span) = self.lookup.find(&self.cells, CellKey { x, y }) else {
                    continue;
                };
                for entry in &self.entries[span.start..span.end] {
                    visit(entry, &mut diagnostics);
                }
            }
        }
        Ok(diagnostics)
    }

    /// Iterate candidate records from the most recent query without allocation.
    pub fn candidates<'a>(
        &'a self,
        scratch: &'a BodyQueryScratch,
    ) -> impl ExactSizeIterator<Item = &'a BodySegmentRecord> + 'a {
        scratch
            .candidates
            .iter()
            .map(|candidate| &self.records[candidate.segment])
    }

    /// Iterate the bounded prefix from one sensor-only query.
    pub(crate) fn sensor_candidates<'a>(
        &'a self,
        scratch: &'a BodySensorQueryScratch,
    ) -> impl ExactSizeIterator<Item = &'a BodySegmentRecord> + 'a {
        scratch
            .inner
            .candidates
            .iter()
            .map(|candidate| &self.records[candidate.segment])
    }
}

/// Stable copied pellet record used by a derived index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IndexedPellet {
    /// Stable internal pellet identity.
    pub id: u64,
    /// Dense source index used only against the stable build view.
    pub source_index: usize,
    /// World position.
    pub position: WorldPoint,
    /// Food value.
    pub value: f64,
    /// Stable kind identifier.
    pub kind: u32,
    /// Stable color identifier.
    pub color: u32,
    /// Optional source-snake owner.
    pub owner: Option<u64>,
}

impl From<(usize, &PelletState)> for IndexedPellet {
    fn from((source_index, pellet): (usize, &PelletState)) -> Self {
        Self {
            id: pellet.id,
            source_index,
            position: pellet.position,
            value: pellet.value,
            kind: pellet.kind,
            color: pellet.color,
            owner: pellet.owner,
        }
    }
}

/// One pellet entry in the flat cell index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PelletCellEntry {
    key: CellKey,
    pellet: usize,
}

/// Pellet candidate ordered by exact distance and stable identity.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PelletCandidate {
    pellet: usize,
    distance_squared: f64,
}

/// Reusable per-worker pellet-query storage.
#[derive(Clone, Debug, Default)]
pub struct PelletQueryScratch {
    candidates: Vec<PelletCandidate>,
}

impl PelletQueryScratch {
    /// Number of candidates retained by the most recent query.
    #[must_use]
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// Allocated candidate slots retained for reuse by later queries.
    #[must_use]
    pub fn candidate_capacity(&self) -> usize {
        self.candidates.capacity()
    }

    fn begin(&mut self, candidate_capacity: usize) -> Result<(), SpatialIndexError> {
        if self.candidates.capacity() < candidate_capacity {
            self.candidates
                .try_reserve_exact(candidate_capacity.saturating_sub(self.candidates.len()))
                .map_err(|_| SpatialIndexError::AllocationFailed {
                    context: "pellet-query candidates",
                    requested: candidate_capacity,
                })?;
        }
        self.candidates.clear();
        Ok(())
    }
}

/// Exact operational counts from one pellet-index build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PelletIndexDiagnostics {
    /// Number of indexed pellets.
    pub pellets: usize,
    /// Number of occupied cells.
    pub occupied_cells: usize,
    /// Coordinate slots in the bounded direct lookup, or zero for fallback.
    pub lookup_cells: usize,
    /// Configured pellet-entry ceiling.
    pub maximum_entries: usize,
    /// Estimated owned bytes for copied records, entries, spans, and lookup.
    pub estimated_bytes: usize,
}

/// Complete immutable pellet index for one stable pre-movement state.
#[derive(Clone, Debug)]
pub struct PelletSpatialIndex {
    cell_size: f64,
    pellets: Vec<IndexedPellet>,
    entries: Vec<PelletCellEntry>,
    cells: Vec<CellSpan>,
    lookup: CellLookup,
    diagnostics: PelletIndexDiagnostics,
}

impl PelletSpatialIndex {
    /// Build a complete point index or reject the declared limit.
    pub fn build(
        world: &WorldState,
        cell_size: f64,
        maximum_entries: usize,
    ) -> Result<Self, SpatialIndexError> {
        validate_cell_size(cell_size)?;
        if world.pellets.len() > maximum_entries {
            return Err(SpatialIndexError::EntryLimitExceeded {
                kind: "pellet",
                required: world.pellets.len(),
                maximum: maximum_entries,
            });
        }
        let mut pellets = Vec::new();
        pellets
            .try_reserve_exact(world.pellets.len())
            .map_err(|_| SpatialIndexError::AllocationFailed {
                context: "pellet-index records",
                requested: world.pellets.len(),
            })?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(world.pellets.len())
            .map_err(|_| SpatialIndexError::AllocationFailed {
                context: "pellet-index entries",
                requested: world.pellets.len(),
            })?;
        for (source_index, pellet) in world.pellets.iter().enumerate() {
            validate_point(pellet.position)?;
            pellets.push(IndexedPellet::from((source_index, pellet)));
        }
        pellets.sort_unstable_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.source_index.cmp(&right.source_index))
        });
        for (pellet_index, pellet) in pellets.iter().enumerate() {
            entries.push(PelletCellEntry {
                key: CellKey {
                    x: cell_coordinate(pellet.position.x, cell_size)?,
                    y: cell_coordinate(pellet.position.y, cell_size)?,
                },
                pellet: pellet_index,
            });
        }
        entries.sort_unstable_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| pellets[left.pellet].id.cmp(&pellets[right.pellet].id))
                .then_with(|| {
                    pellets[left.pellet]
                        .source_index
                        .cmp(&pellets[right.pellet].source_index)
                })
        });
        let cells = build_cell_spans(&entries, |entry| entry.key)?;
        let lookup = CellLookup::build(&cells)?;
        let estimated_bytes = checked_owned_bytes(&[
            (pellets.capacity(), size_of::<IndexedPellet>()),
            (entries.capacity(), size_of::<PelletCellEntry>()),
            (cells.capacity(), size_of::<CellSpan>()),
        ])?
        .checked_add(lookup.estimated_bytes()?)
        .ok_or(SpatialIndexError::ArithmeticOverflow {
            context: "pellet-index estimated bytes",
        })?;
        let diagnostics = PelletIndexDiagnostics {
            pellets: pellets.len(),
            occupied_cells: cells.len(),
            lookup_cells: lookup.cells(),
            maximum_entries,
            estimated_bytes,
        };
        Ok(Self {
            cell_size,
            pellets,
            entries,
            cells,
            lookup,
            diagnostics,
        })
    }

    /// Return immutable build diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> PelletIndexDiagnostics {
        self.diagnostics
    }

    /// Collect every exact in-radius pellet for complete non-sensor consumers.
    pub fn collect_candidates(
        &self,
        center: WorldPoint,
        radius: f64,
        scratch: &mut PelletQueryScratch,
    ) -> Result<SpatialQueryDiagnostics, SpatialIndexError> {
        validate_query(center, radius)?;
        scratch.begin(self.pellets.len())?;
        let mut diagnostics = self.visit_query_cells(center, radius, |entry, diagnostics| {
            diagnostics.entries_visited = diagnostics.entries_visited.saturating_add(1);
            let pellet = &self.pellets[entry.pellet];
            let dx = pellet.position.x - center.x;
            let dy = pellet.position.y - center.y;
            let distance_squared = dx * dx + dy * dy;
            if distance_squared <= radius * radius {
                scratch.candidates.push(PelletCandidate {
                    pellet: entry.pellet,
                    distance_squared,
                });
            }
        })?;
        scratch
            .candidates
            .sort_unstable_by(compare_pellet_candidates);
        diagnostics.candidates = scratch.candidates.len();
        Ok(diagnostics)
    }

    /// Retain at most `maximum_candidates` exact nearest pellets.
    ///
    /// This preserves the full nearest-first result prefix without allocating
    /// or sorting a population-sized result vector.
    pub fn collect_sensor_candidates(
        &self,
        center: WorldPoint,
        radius: f64,
        maximum_candidates: usize,
        scratch: &mut PelletQueryScratch,
    ) -> Result<SpatialQueryDiagnostics, SpatialIndexError> {
        validate_query(center, radius)?;
        scratch.begin(maximum_candidates)?;
        let radius_squared = radius * radius;
        let qualifying = std::cell::Cell::new(0usize);
        let worst_retained_distance_squared = std::cell::Cell::new(f64::INFINITY);
        let mut diagnostics = SpatialQueryDiagnostics::default();
        visit_query_cells_nearest_first(
            center,
            radius,
            self.cell_size,
            |key| {
                diagnostics.cells_visited = diagnostics.cells_visited.saturating_add(1);
                let Some(span) = self.lookup.find(&self.cells, key) else {
                    return true;
                };
                for entry in &self.entries[span.start..span.end] {
                    diagnostics.entries_visited = diagnostics.entries_visited.saturating_add(1);
                    let pellet = &self.pellets[entry.pellet];
                    let dx = pellet.position.x - center.x;
                    let dy = pellet.position.y - center.y;
                    let distance_squared = dx * dx + dy * dy;
                    if distance_squared > radius_squared {
                        continue;
                    }
                    qualifying.set(qualifying.get().saturating_add(1));
                    retain_bounded(
                        &mut scratch.candidates,
                        PelletCandidate {
                            pellet: entry.pellet,
                            distance_squared,
                        },
                        maximum_candidates,
                        compare_pellet_candidates,
                    );
                    if scratch.candidates.len() == maximum_candidates && maximum_candidates != 0 {
                        worst_retained_distance_squared.set(scratch.candidates[0].distance_squared);
                    }
                }
                true
            },
            |unvisited_lower_bound_squared| {
                qualifying.get() > maximum_candidates
                    && (maximum_candidates == 0
                        || worst_retained_distance_squared.get() < unvisited_lower_bound_squared)
            },
        )?;
        scratch
            .candidates
            .sort_unstable_by(compare_pellet_candidates);
        diagnostics.candidates = scratch.candidates.len();
        diagnostics.candidate_limit_reached = qualifying.get() > maximum_candidates;
        Ok(diagnostics)
    }

    fn visit_query_cells(
        &self,
        center: WorldPoint,
        radius: f64,
        mut visit: impl FnMut(&PelletCellEntry, &mut SpatialQueryDiagnostics),
    ) -> Result<SpatialQueryDiagnostics, SpatialIndexError> {
        let minimum = CellKey {
            x: cell_coordinate(center.x - radius, self.cell_size)?,
            y: cell_coordinate(center.y - radius, self.cell_size)?,
        };
        let maximum = CellKey {
            x: cell_coordinate(center.x + radius, self.cell_size)?,
            y: cell_coordinate(center.y + radius, self.cell_size)?,
        };
        let mut diagnostics = SpatialQueryDiagnostics::default();
        for y in minimum.y..=maximum.y {
            for x in minimum.x..=maximum.x {
                diagnostics.cells_visited = diagnostics.cells_visited.saturating_add(1);
                let Some(span) = self.lookup.find(&self.cells, CellKey { x, y }) else {
                    continue;
                };
                for entry in &self.entries[span.start..span.end] {
                    visit(entry, &mut diagnostics);
                }
            }
        }
        Ok(diagnostics)
    }

    /// Iterate pellets from the most recent query without allocation.
    pub fn candidates<'a>(
        &'a self,
        scratch: &'a PelletQueryScratch,
    ) -> impl ExactSizeIterator<Item = &'a IndexedPellet> + 'a {
        scratch
            .candidates
            .iter()
            .map(|candidate| &self.pellets[candidate.pellet])
    }
}

/// Build limits for the derived indexes owned by one stable sensor view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorIndexConfig {
    /// Body-index cell width in world units.
    pub body_cell_size: f64,
    /// Pellet-index cell width in world units.
    pub pellet_cell_size: f64,
    /// Complete body-cell-entry admission ceiling.
    pub maximum_body_entries: usize,
    /// Complete pellet-entry admission ceiling.
    pub maximum_pellet_entries: usize,
}

/// One world borrow and both indexes derived from that exact immutable state.
///
/// Keeping the world borrow inside this owner prevents safe Rust from mutating
/// the source world while its sensor indexes are live. Sensor evaluation only
/// accepts this type, so it cannot pair a world with stale independent indexes.
///
/// ```compile_fail
/// use slither_native::engine::spatial::{IndexedSensorWorld, SensorIndexConfig};
/// use slither_native::engine::state::WorldState;
///
/// let mut world = WorldState::default();
/// let indexed = IndexedSensorWorld::build(
///     &world,
///     SensorIndexConfig {
///         body_cell_size: 70.0,
///         pellet_cell_size: 120.0,
///         maximum_body_entries: 1_000,
///         maximum_pellet_entries: 1_000,
///     },
/// ).unwrap();
/// world.pellets.clear();
/// let _still_borrowed = indexed.world();
/// ```
#[derive(Debug)]
pub struct IndexedSensorWorld<'a> {
    world: &'a WorldState,
    body: BodySpatialIndex,
    pellets: PelletSpatialIndex,
}

impl<'a> IndexedSensorWorld<'a> {
    /// Build both complete indexes from one immutable world boundary.
    pub fn build(
        world: &'a WorldState,
        config: SensorIndexConfig,
    ) -> Result<Self, SpatialIndexError> {
        let body =
            BodySpatialIndex::build(world, config.body_cell_size, config.maximum_body_entries)?;
        let pellets = PelletSpatialIndex::build(
            world,
            config.pellet_cell_size,
            config.maximum_pellet_entries,
        )?;
        Ok(Self {
            world,
            body,
            pellets,
        })
    }

    /// Exact immutable world from which both indexes were built.
    #[must_use]
    pub fn world(&self) -> &WorldState {
        self.world
    }

    /// Complete body index bound to this world view.
    #[must_use]
    pub fn body_index(&self) -> &BodySpatialIndex {
        &self.body
    }

    /// Complete pellet index bound to this world view.
    #[must_use]
    pub fn pellet_index(&self) -> &PelletSpatialIndex {
        &self.pellets
    }
}

/// Spatial-index validation, admission, or checked-allocation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpatialIndexError {
    /// A cell or query scalar was not finite and positive where required.
    InvalidScalar { field: &'static str },
    /// A world coordinate could not be represented by the checked cell key.
    CoordinateOutOfRange,
    /// A snake body range did not identify complete source points.
    InvalidBodyRange { snake_id: u64 },
    /// Checked entry/byte arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// Complete indexing would exceed the configured entry ceiling.
    EntryLimitExceeded {
        /// `body` or `pellet`.
        kind: &'static str,
        /// Exact count known when the limit was crossed.
        required: usize,
        /// Configured ceiling.
        maximum: usize,
    },
    /// A fallible vector reservation failed.
    AllocationFailed {
        /// Allocation being attempted.
        context: &'static str,
        /// Requested element count.
        requested: usize,
    },
}

impl fmt::Display for SpatialIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScalar { field } => write!(formatter, "invalid spatial scalar {field}"),
            Self::CoordinateOutOfRange => write!(formatter, "spatial coordinate exceeds i32 cell range"),
            Self::InvalidBodyRange { snake_id } => {
                write!(formatter, "snake {snake_id} has an invalid body range")
            }
            Self::ArithmeticOverflow { context } => {
                write!(formatter, "spatial arithmetic overflowed while computing {context}")
            }
            Self::EntryLimitExceeded {
                kind,
                required,
                maximum,
            } => write!(
                formatter,
                "complete {kind} index requires at least {required} entries, exceeding configured maximum {maximum}"
            ),
            Self::AllocationFailed { context, requested } => write!(
                formatter,
                "could not reserve {requested} elements for {context}"
            ),
        }
    }
}

impl Error for SpatialIndexError {}

fn compare_body_candidates(left: &BodyCandidate, right: &BodyCandidate) -> Ordering {
    left.lower_bound_squared
        .total_cmp(&right.lower_bound_squared)
        .then_with(|| left.owner_id.cmp(&right.owner_id))
        .then_with(|| left.segment_end.cmp(&right.segment_end))
}

fn compare_pellet_candidates(left: &PelletCandidate, right: &PelletCandidate) -> Ordering {
    left.distance_squared
        .total_cmp(&right.distance_squared)
        .then_with(|| left.pellet.cmp(&right.pellet))
}

/// Keep the smallest `limit` values, promoting to a max-heap only at capacity.
///
/// Most sensor queries return fewer candidates than their safety cap. Those
/// queries only append and perform one final deterministic sort instead of
/// paying heap-maintenance cost for every retained candidate.
fn retain_bounded<T>(values: &mut Vec<T>, value: T, limit: usize, compare: fn(&T, &T) -> Ordering) {
    if limit == 0 {
        return;
    }
    if values.len() < limit {
        values.push(value);
        if values.len() == limit {
            build_max_heap(values, compare);
        }
        return;
    }
    if compare(&value, &values[0]) != Ordering::Less {
        return;
    }
    values[0] = value;
    sift_max_heap_down(values, 0, compare);
}

fn build_max_heap<T>(values: &mut [T], compare: fn(&T, &T) -> Ordering) {
    if values.len() < 2 {
        return;
    }
    for parent in (0..=(values.len() - 2) / 2).rev() {
        sift_max_heap_down(values, parent, compare);
    }
}

fn sift_max_heap_down<T>(values: &mut [T], mut parent: usize, compare: fn(&T, &T) -> Ordering) {
    loop {
        let left = parent * 2 + 1;
        if left >= values.len() {
            return;
        }
        let right = left + 1;
        let largest =
            if right < values.len() && compare(&values[left], &values[right]) == Ordering::Less {
                right
            } else {
                left
            };
        if compare(&values[parent], &values[largest]) != Ordering::Less {
            return;
        }
        values.swap(parent, largest);
        parent = largest;
    }
}

fn validate_cell_size(cell_size: f64) -> Result<(), SpatialIndexError> {
    if !cell_size.is_finite() || cell_size <= 0.0 {
        return Err(SpatialIndexError::InvalidScalar { field: "cell_size" });
    }
    Ok(())
}

fn validate_query(center: WorldPoint, radius: f64) -> Result<(), SpatialIndexError> {
    validate_point(center)?;
    if !radius.is_finite() || radius < 0.0 {
        return Err(SpatialIndexError::InvalidScalar {
            field: "query_radius",
        });
    }
    Ok(())
}

fn validate_point(point: WorldPoint) -> Result<(), SpatialIndexError> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(SpatialIndexError::InvalidScalar {
            field: "world_point",
        });
    }
    Ok(())
}

fn cell_coordinate(value: f64, cell_size: f64) -> Result<i32, SpatialIndexError> {
    let coordinate = (value / cell_size).floor();
    if coordinate < f64::from(i32::MIN) || coordinate > f64::from(i32::MAX) {
        return Err(SpatialIndexError::CoordinateOutOfRange);
    }
    Ok(coordinate as i32)
}

fn coordinate_span(
    minimum: i32,
    maximum: i32,
    context: &'static str,
) -> Result<usize, SpatialIndexError> {
    let span = i64::from(maximum) - i64::from(minimum) + 1;
    usize::try_from(span).map_err(|_| SpatialIndexError::ArithmeticOverflow { context })
}

fn cell_lookup_offset(minimum: CellKey, width: usize, key: CellKey) -> Option<usize> {
    let x = usize::try_from(i64::from(key.x) - i64::from(minimum.x)).ok()?;
    let y = usize::try_from(i64::from(key.y) - i64::from(minimum.y)).ok()?;
    if x >= width {
        return None;
    }
    y.checked_mul(width)?.checked_add(x)
}

/// Visit a rectangular query in deterministic concentric cell rings.
///
/// The post-ring callback receives a conservative squared distance to every
/// still-unvisited cell. Returning `true` from it, or `false` from the cell
/// visitor, stops traversal without allocating a cell list.
fn visit_query_cells_nearest_first(
    center: WorldPoint,
    radius: f64,
    cell_size: f64,
    mut visit: impl FnMut(CellKey) -> bool,
    mut stop_after_ring: impl FnMut(f64) -> bool,
) -> Result<(), SpatialIndexError> {
    let minimum = CellKey {
        x: cell_coordinate(center.x - radius, cell_size)?,
        y: cell_coordinate(center.y - radius, cell_size)?,
    };
    let maximum = CellKey {
        x: cell_coordinate(center.x + radius, cell_size)?,
        y: cell_coordinate(center.y + radius, cell_size)?,
    };
    let origin = CellKey {
        x: cell_coordinate(center.x, cell_size)?,
        y: cell_coordinate(center.y, cell_size)?,
    };
    let maximum_ring = [
        i64::from(origin.x) - i64::from(minimum.x),
        i64::from(maximum.x) - i64::from(origin.x),
        i64::from(origin.y) - i64::from(minimum.y),
        i64::from(maximum.y) - i64::from(origin.y),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);

    for ring in 0..=maximum_ring {
        let mut visit_if_in_bounds = |x: i64, y: i64| -> bool {
            if x < i64::from(minimum.x)
                || x > i64::from(maximum.x)
                || y < i64::from(minimum.y)
                || y > i64::from(maximum.y)
            {
                return true;
            }
            let key = CellKey {
                x: i32::try_from(x).expect("bounded cell x must fit i32"),
                y: i32::try_from(y).expect("bounded cell y must fit i32"),
            };
            visit(key)
        };
        let origin_x = i64::from(origin.x);
        let origin_y = i64::from(origin.y);
        if ring == 0 {
            if !visit_if_in_bounds(origin_x, origin_y) {
                return Ok(());
            }
        } else {
            let top = origin_y - ring;
            let bottom = origin_y + ring;
            for x in (origin_x - ring)..=(origin_x + ring) {
                if !visit_if_in_bounds(x, top) || !visit_if_in_bounds(x, bottom) {
                    return Ok(());
                }
            }
            let left = origin_x - ring;
            let right = origin_x + ring;
            for y in (origin_y - ring + 1)..=(origin_y + ring - 1) {
                if !visit_if_in_bounds(left, y) || !visit_if_in_bounds(right, y) {
                    return Ok(());
                }
            }
        }

        let left = origin_x - ring;
        let right = origin_x + ring;
        let top = origin_y - ring;
        let bottom = origin_y + ring;
        let mut unvisited_distance = f64::INFINITY;
        if left > i64::from(minimum.x) {
            unvisited_distance = unvisited_distance.min(center.x - left as f64 * cell_size);
        }
        if right < i64::from(maximum.x) {
            unvisited_distance = unvisited_distance.min((right + 1) as f64 * cell_size - center.x);
        }
        if top > i64::from(minimum.y) {
            unvisited_distance = unvisited_distance.min(center.y - top as f64 * cell_size);
        }
        if bottom < i64::from(maximum.y) {
            unvisited_distance = unvisited_distance.min((bottom + 1) as f64 * cell_size - center.y);
        }
        let unvisited_lower_bound_squared = unvisited_distance * unvisited_distance;
        if stop_after_ring(unvisited_lower_bound_squared) {
            return Ok(());
        }
    }
    Ok(())
}

fn segment_cell_bounds(
    start: WorldPoint,
    end: WorldPoint,
    cell_size: f64,
) -> Result<(CellKey, CellKey), SpatialIndexError> {
    Ok((
        CellKey {
            x: cell_coordinate(start.x.min(end.x), cell_size)?,
            y: cell_coordinate(start.y.min(end.y), cell_size)?,
        },
        CellKey {
            x: cell_coordinate(start.x.max(end.x), cell_size)?,
            y: cell_coordinate(start.y.max(end.y), cell_size)?,
        },
    ))
}

fn cell_rectangle_count(minimum: CellKey, maximum: CellKey) -> Result<usize, SpatialIndexError> {
    let width = i64::from(maximum.x) - i64::from(minimum.x) + 1;
    let height = i64::from(maximum.y) - i64::from(minimum.y) + 1;
    let width = usize::try_from(width).map_err(|_| SpatialIndexError::ArithmeticOverflow {
        context: "cell rectangle width",
    })?;
    let height = usize::try_from(height).map_err(|_| SpatialIndexError::ArithmeticOverflow {
        context: "cell rectangle height",
    })?;
    width
        .checked_mul(height)
        .ok_or(SpatialIndexError::ArithmeticOverflow {
            context: "cell rectangle area",
        })
}

fn build_cell_spans<T>(
    entries: &[T],
    key: impl Fn(&T) -> CellKey,
) -> Result<Vec<CellSpan>, SpatialIndexError> {
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(entries.len())
        .map_err(|_| SpatialIndexError::AllocationFailed {
            context: "spatial cell spans",
            requested: entries.len(),
        })?;
    let mut start = 0usize;
    while start < entries.len() {
        let cell_key = key(&entries[start]);
        let mut end = start + 1;
        while end < entries.len() && key(&entries[end]) == cell_key {
            end += 1;
        }
        cells.push(CellSpan {
            key: cell_key,
            start,
            end,
        });
        start = end;
    }
    Ok(cells)
}

fn find_cell_span(cells: &[CellSpan], key: CellKey) -> Option<CellSpan> {
    cells
        .binary_search_by_key(&key, |span| span.key)
        .ok()
        .map(|index| cells[index])
}

fn point_to_aabb_distance_squared(point: WorldPoint, start: WorldPoint, end: WorldPoint) -> f64 {
    let minimum_x = start.x.min(end.x);
    let maximum_x = start.x.max(end.x);
    let minimum_y = start.y.min(end.y);
    let maximum_y = start.y.max(end.y);
    let dx = if point.x < minimum_x {
        minimum_x - point.x
    } else if point.x > maximum_x {
        point.x - maximum_x
    } else {
        0.0
    };
    let dy = if point.y < minimum_y {
        minimum_y - point.y
    } else if point.y > maximum_y {
        point.y - maximum_y
    } else {
        0.0
    };
    dx * dx + dy * dy
}

fn point_to_segment_distance_squared(point: WorldPoint, start: WorldPoint, end: WorldPoint) -> f64 {
    let segment_x = end.x - start.x;
    let segment_y = end.y - start.y;
    let point_x = point.x - start.x;
    let point_y = point.y - start.y;
    let squared_length = segment_x * segment_x + segment_y * segment_y;
    let along = if squared_length > 1.0e-12 {
        ((point_x * segment_x + point_y * segment_y) / squared_length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest_x = start.x + segment_x * along;
    let closest_y = start.y + segment_y * along;
    let dx = point.x - closest_x;
    let dy = point.y - closest_y;
    dx * dx + dy * dy
}

fn checked_owned_bytes(parts: &[(usize, usize)]) -> Result<usize, SpatialIndexError> {
    parts.iter().try_fold(0usize, |total, (count, width)| {
        let bytes = count
            .checked_mul(*width)
            .ok_or(SpatialIndexError::ArithmeticOverflow {
                context: "spatial owned bytes",
            })?;
        total
            .checked_add(bytes)
            .ok_or(SpatialIndexError::ArithmeticOverflow {
                context: "spatial owned bytes",
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_dense_cell_lookup_matches_sorted_fallback() {
        let cells = vec![
            CellSpan {
                key: CellKey { x: -2, y: 1 },
                start: 0,
                end: 2,
            },
            CellSpan {
                key: CellKey { x: 3, y: 4 },
                start: 2,
                end: 5,
            },
        ];
        let lookup = CellLookup::build(&cells).expect("bounded lookup should build");
        assert!(lookup.cells() > 0);
        assert_eq!(lookup.find(&cells, cells[0].key), Some(cells[0]));
        assert_eq!(lookup.find(&cells, CellKey { x: 0, y: 0 }), None);

        let sparse = vec![
            CellSpan {
                key: CellKey { x: -100_000, y: 0 },
                start: 0,
                end: 1,
            },
            CellSpan {
                key: CellKey { x: 100_000, y: 0 },
                start: 1,
                end: 2,
            },
        ];
        let fallback = CellLookup::build(&sparse).expect("sparse fallback should build");
        assert_eq!(fallback.cells(), 0);
        assert_eq!(fallback.find(&sparse, sparse[1].key), Some(sparse[1]));

        let overflow_envelope = vec![
            CellSpan {
                key: CellKey {
                    x: i32::MIN,
                    y: i32::MIN,
                },
                start: 0,
                end: 1,
            },
            CellSpan {
                key: CellKey {
                    x: i32::MAX,
                    y: i32::MAX,
                },
                start: 1,
                end: 2,
            },
        ];
        let overflow_fallback =
            CellLookup::build(&overflow_envelope).expect("overflowing area should use fallback");
        assert_eq!(overflow_fallback.cells(), 0);
        assert_eq!(
            overflow_fallback.find(&overflow_envelope, overflow_envelope[0].key),
            Some(overflow_envelope[0])
        );
    }
    use crate::engine::state::{BodyRange, SnakeKind, SnakeState};

    fn snake(id: u64, body: BodyRange, radius: f64) -> SnakeState {
        SnakeState {
            id,
            frame_v1_id: u32::try_from(id).expect("test id should fit"),
            kind: SnakeKind::External,
            alive: true,
            population_slot: None,
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
            position: WorldPoint { x: 0.0, y: 0.0 },
            previous_position: WorldPoint { x: 0.0, y: 0.0 },
            direction: 0.0,
            radius,
            speed: 0.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: body.len as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body,
            skin: 0,
        }
    }

    fn long_segment_world(reverse_snakes: bool) -> WorldState {
        let mut world = WorldState {
            body_points: vec![
                WorldPoint { x: -150.0, y: 0.0 },
                WorldPoint { x: 150.0, y: 0.0 },
                WorldPoint { x: 0.0, y: -150.0 },
                WorldPoint { x: 0.0, y: 150.0 },
            ],
            ..WorldState::default()
        };
        world.snakes = vec![
            snake(20, BodyRange { start: 0, len: 2 }, 8.0),
            snake(10, BodyRange { start: 2, len: 2 }, 9.0),
        ];
        world.snakes[0].position = world.body_points[0];
        world.snakes[1].position = world.body_points[2];
        if reverse_snakes {
            world.snakes.reverse();
        }
        world
    }

    #[test]
    fn long_segments_cover_every_cell_in_their_bounds() {
        let mut world = long_segment_world(false);
        world.snakes.truncate(1);
        world.body_points.truncate(2);
        let index = BodySpatialIndex::build(&world, 50.0, 100).expect("index should build");
        assert_eq!(index.diagnostics().segments, 1);
        assert_eq!(index.diagnostics().entries, 7);
        assert_eq!(index.diagnostics().occupied_cells, 7);

        let mut scratch = BodyQueryScratch::default();
        let query = index
            .collect_candidates(WorldPoint { x: 0.0, y: 0.0 }, 1.0, &mut scratch)
            .expect("center query should succeed");
        assert_eq!(query.candidates, 1);
        assert_eq!(index.candidates(&scratch).count(), 1);
    }

    #[test]
    fn sparse_body_and_pellet_indexes_query_through_automatic_fallback() {
        let far = 1_000_000.0;
        let mut world = WorldState {
            body_points: vec![
                WorldPoint { x: -far, y: -far },
                WorldPoint {
                    x: -far + 1.0,
                    y: -far,
                },
                WorldPoint { x: far, y: far },
                WorldPoint {
                    x: far + 1.0,
                    y: far,
                },
            ],
            pellets: vec![
                PelletState {
                    id: 20,
                    position: WorldPoint { x: -far, y: -far },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 10,
                    position: WorldPoint { x: far, y: far },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
            ],
            ..WorldState::default()
        };
        world.snakes = vec![
            snake(20, BodyRange { start: 0, len: 2 }, 8.0),
            snake(10, BodyRange { start: 2, len: 2 }, 8.0),
        ];

        let body = BodySpatialIndex::build(&world, 10.0, 2).expect("body index should build");
        let pellets =
            PelletSpatialIndex::build(&world, 10.0, 2).expect("pellet index should build");
        assert_eq!(body.diagnostics().lookup_cells, 0);
        assert_eq!(pellets.diagnostics().lookup_cells, 0);

        let center = WorldPoint { x: far, y: far };
        let mut body_scratch = BodyQueryScratch::default();
        let body_query = body
            .collect_candidates(center, 2.0, &mut body_scratch)
            .expect("sparse body query should succeed");
        assert_eq!(body_query.candidates, 1);
        assert_eq!(
            body.candidates(&body_scratch)
                .map(|segment| segment.owner_id)
                .collect::<Vec<_>>(),
            vec![10]
        );

        let mut pellet_scratch = PelletQueryScratch::default();
        let pellet_query = pellets
            .collect_candidates(center, 2.0, &mut pellet_scratch)
            .expect("sparse pellet query should succeed");
        assert_eq!(pellet_query.candidates, 1);
        assert_eq!(
            pellets
                .candidates(&pellet_scratch)
                .map(|pellet| pellet.id)
                .collect::<Vec<_>>(),
            vec![10]
        );
    }

    #[test]
    fn duplicate_cell_hits_yield_one_deterministically_ordered_segment() {
        let world = long_segment_world(false);
        let index = BodySpatialIndex::build(&world, 50.0, 100).expect("index should build");
        let mut scratch = BodyQueryScratch::default();
        let query = index
            .collect_candidates(WorldPoint { x: 0.0, y: 0.0 }, 120.0, &mut scratch)
            .expect("query should succeed");
        assert!(query.entries_visited > query.candidates);
        let owners = index
            .candidates(&scratch)
            .map(|record| record.owner_id)
            .collect::<Vec<_>>();
        assert_eq!(owners, vec![10, 20]);

        let reversed = long_segment_world(true);
        let reversed_index =
            BodySpatialIndex::build(&reversed, 50.0, 100).expect("reversed index should build");
        let mut reversed_scratch = BodyQueryScratch::default();
        reversed_index
            .collect_candidates(WorldPoint { x: 0.0, y: 0.0 }, 120.0, &mut reversed_scratch)
            .expect("reversed query should succeed");
        let reversed_owners = reversed_index
            .candidates(&reversed_scratch)
            .map(|record| record.owner_id)
            .collect::<Vec<_>>();
        assert_eq!(reversed_owners, owners);
    }

    #[test]
    fn bounded_body_query_keeps_exact_uncapped_order_and_reuses_capacity() {
        let mut world = WorldState::default();
        for (offset, owner_id) in [1_u64, 10, 20, 30].into_iter().enumerate() {
            let x = if owner_id == 1 { 0.0 } else { owner_id as f64 };
            let start = world.body_points.len();
            world
                .body_points
                .extend([WorldPoint { x, y: 0.0 }, WorldPoint { x: x + 1.0, y: 0.0 }]);
            world.snakes.push(snake(
                owner_id,
                BodyRange { start, len: 2 },
                5.0 + offset as f64,
            ));
        }
        let index = BodySpatialIndex::build(&world, 10.0, 100).expect("index should build");
        let mut scratch = BodySensorQueryScratch::default();

        let exact = index
            .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 25.0, 1, 2, &mut scratch)
            .expect("exact-boundary query should succeed");
        assert_eq!(exact.candidates, 2);
        assert!(!exact.candidate_limit_reached);
        assert_eq!(
            index
                .sensor_candidates(&scratch)
                .map(|record| record.owner_id)
                .collect::<Vec<_>>(),
            vec![10, 20]
        );

        let capped = index
            .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 100.0, 1, 2, &mut scratch)
            .expect("capped query should succeed");
        assert_eq!(capped.candidates, 2);
        assert!(capped.candidate_limit_reached);
        assert_eq!(scratch.candidate_count(), 2);
        let warmed_capacity = scratch.candidate_capacity();
        index
            .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 100.0, 1, 2, &mut scratch)
            .expect("repeat query should succeed");
        assert_eq!(scratch.candidate_capacity(), warmed_capacity);
    }

    #[test]
    fn complete_body_admission_fails_instead_of_truncating() {
        let world = long_segment_world(false);
        let error = BodySpatialIndex::build(&world, 50.0, 10)
            .expect_err("complete entry limit should reject the build");
        assert!(matches!(
            error,
            SpatialIndexError::EntryLimitExceeded {
                kind: "body",
                required: 14,
                maximum: 10
            }
        ));
    }

    #[test]
    fn dead_snakes_contribute_no_collision_or_sensor_truth() {
        let mut world = long_segment_world(false);
        world.snakes[0].alive = false;
        let index = BodySpatialIndex::build(&world, 50.0, 100).expect("index should build");
        assert_eq!(index.diagnostics().segments, 1);
        assert_eq!(index.maximum_owner_radius(), 9.0);
    }

    #[test]
    fn pellets_are_nearest_first_with_stable_id_ties() {
        let world = WorldState {
            pellets: vec![
                PelletState {
                    id: 20,
                    position: WorldPoint { x: -10.0, y: 0.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 10,
                    position: WorldPoint { x: 10.0, y: 0.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 30,
                    position: WorldPoint { x: 80.0, y: 0.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
            ],
            ..WorldState::default()
        };
        let index = PelletSpatialIndex::build(&world, 25.0, 3).expect("index should build");
        let mut scratch = PelletQueryScratch::default();
        let query = index
            .collect_candidates(WorldPoint { x: 0.0, y: 0.0 }, 20.0, &mut scratch)
            .expect("query should succeed");
        assert_eq!(query.candidates, 2);
        let ids = index
            .candidates(&scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![10, 20]);
    }

    #[test]
    fn bounded_pellet_query_keeps_exact_nearest_prefix_and_reuses_capacity() {
        let world = WorldState {
            pellets: vec![
                PelletState {
                    id: 30,
                    position: WorldPoint { x: 30.0, y: 0.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 20,
                    position: WorldPoint { x: 20.0, y: 0.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 10,
                    position: WorldPoint { x: 10.0, y: 0.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
            ],
            ..WorldState::default()
        };
        let index = PelletSpatialIndex::build(&world, 10.0, 3).expect("index should build");
        let mut scratch = PelletQueryScratch::default();

        let exact = index
            .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 25.0, 2, &mut scratch)
            .expect("exact-boundary query should succeed");
        assert_eq!(exact.candidates, 2);
        assert!(!exact.candidate_limit_reached);

        let capped = index
            .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 100.0, 2, &mut scratch)
            .expect("capped query should succeed");
        assert_eq!(capped.candidates, 2);
        assert!(capped.candidate_limit_reached);
        assert_eq!(
            index
                .candidates(&scratch)
                .map(|pellet| pellet.id)
                .collect::<Vec<_>>(),
            vec![10, 20]
        );
        let warmed_capacity = scratch.candidate_capacity();
        index
            .collect_sensor_candidates(WorldPoint { x: 0.0, y: 0.0 }, 100.0, 2, &mut scratch)
            .expect("repeat query should succeed");
        assert_eq!(scratch.candidate_capacity(), warmed_capacity);
    }

    #[test]
    fn bounded_pellet_query_matches_complete_prefix_across_rings_and_ties() {
        let positions = [
            (70_u64, -130.0, -15.0),
            (20, -80.0, 60.0),
            (90, -25.0, -100.0),
            (10, 100.0, 0.0),
            (30, 0.0, 100.0),
            (40, 145.0, 145.0),
            (50, -210.0, 35.0),
            (60, 280.0, -90.0),
        ];
        let make_world = |reverse: bool| {
            let iterator: Box<dyn Iterator<Item = &(u64, f64, f64)>> = if reverse {
                Box::new(positions.iter().rev())
            } else {
                Box::new(positions.iter())
            };
            WorldState {
                pellets: iterator
                    .map(|(id, x, y)| PelletState {
                        id: *id,
                        position: WorldPoint { x: *x, y: *y },
                        value: 1.0,
                        kind: 0,
                        color: 0,
                        owner: None,
                    })
                    .collect(),
                ..WorldState::default()
            }
        };
        let center = WorldPoint { x: 0.0, y: 0.0 };
        let radius = 500.0;
        let world = make_world(false);
        let index =
            PelletSpatialIndex::build(&world, 70.0, positions.len()).expect("index should build");
        let mut complete_scratch = PelletQueryScratch::default();
        index
            .collect_candidates(center, radius, &mut complete_scratch)
            .expect("complete query should succeed");
        let expected = index
            .candidates(&complete_scratch)
            .map(|pellet| pellet.id)
            .collect::<Vec<_>>();

        for limit in 0..=positions.len() + 1 {
            let mut bounded_scratch = PelletQueryScratch::default();
            let diagnostics = index
                .collect_sensor_candidates(center, radius, limit, &mut bounded_scratch)
                .expect("bounded query should succeed");
            let actual = index
                .candidates(&bounded_scratch)
                .map(|pellet| pellet.id)
                .collect::<Vec<_>>();
            assert_eq!(
                actual,
                expected.iter().copied().take(limit).collect::<Vec<_>>()
            );
            assert_eq!(diagnostics.candidate_limit_reached, expected.len() > limit);
        }

        let reversed = make_world(true);
        let reversed_index = PelletSpatialIndex::build(&reversed, 70.0, positions.len())
            .expect("reversed index should build");
        let mut reversed_scratch = PelletQueryScratch::default();
        reversed_index
            .collect_sensor_candidates(center, radius, 4, &mut reversed_scratch)
            .expect("reversed bounded query should succeed");
        assert_eq!(
            reversed_index
                .candidates(&reversed_scratch)
                .map(|pellet| pellet.id)
                .collect::<Vec<_>>(),
            expected[..4]
        );
    }

    #[test]
    fn pellet_ring_stop_does_not_skip_equal_distance_smaller_id() {
        let center = WorldPoint { x: 5.0, y: 5.0 };
        let world = WorldState {
            pellets: vec![
                PelletState {
                    id: 1,
                    position: WorldPoint { x: 10.0, y: 5.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 2,
                    position: WorldPoint { x: 49.0, y: 5.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 100,
                    position: WorldPoint { x: 41.0, y: 32.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 200,
                    position: WorldPoint { x: -31.0, y: 32.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 50,
                    position: WorldPoint { x: 50.0, y: 5.0 },
                    value: 1.0,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
            ],
            ..WorldState::default()
        };
        let index = PelletSpatialIndex::build(&world, 10.0, 5).expect("index should build");
        let mut scratch = PelletQueryScratch::default();
        let diagnostics = index
            .collect_sensor_candidates(center, 100.0, 3, &mut scratch)
            .expect("bounded query should succeed");
        assert!(diagnostics.candidate_limit_reached);
        assert_eq!(
            index
                .candidates(&scratch)
                .map(|pellet| pellet.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 50]
        );
    }

    #[test]
    fn pellet_admission_is_checked_before_index_construction() {
        let world = WorldState {
            pellets: vec![PelletState {
                id: 1,
                position: WorldPoint { x: 0.0, y: 0.0 },
                value: 1.0,
                kind: 0,
                color: 0,
                owner: None,
            }],
            ..WorldState::default()
        };
        assert!(matches!(
            PelletSpatialIndex::build(&world, 120.0, 0),
            Err(SpatialIndexError::EntryLimitExceeded {
                kind: "pellet",
                required: 1,
                maximum: 0
            })
        ));
    }
}
