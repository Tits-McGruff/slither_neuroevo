//! Reusable bounded sensor-only traversal over complete native body records.
//! A balanced hierarchy stores each segment once, avoiding grid duplication
//! and empty-cell scans. Collision truth continues to use the complete grid.

use super::ray_work::{RayWorkMeter, RayWorkUnit};
use super::sensor_geometry::{RayIntersection, SensorGeometryError, SensorRay};
use super::spatial::{BodySegmentRecord, BodySpatialIndex};
use super::state::WorldPoint;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fmt::{Display, Formatter};
use std::mem::size_of;

/// Small deterministic leaves bound local broad-phase work.
const LEAF_SEGMENTS: usize = 4;
#[derive(Clone, Copy, Debug)]
struct RayNode {
    minimum: WorldPoint,
    maximum: WorldPoint,
    maximum_radius: f64,
    /// Zero denotes mixed owners; admitted physical IDs are nonzero.
    sole_owner: u64,
    first: u32,
    count: u32,
    left: u32,
    right: u32,
}

/// Coordinator-owned index storage. No per-worker copy of body geometry.
#[derive(Debug, Default)]
pub struct BodyRayIndexWorkspace {
    order: Vec<u32>,
    nodes: Vec<RayNode>,
}

/// Borrowed view binds the private hierarchy to exactly the immutable body
/// records used to build it. Neither side can rebuild while a query borrows it.
#[derive(Clone, Copy)]
pub struct BodyRayIndex<'workspace, 'source> {
    body: &'source BodySpatialIndex,
    order: &'workspace [u32],
    nodes: &'workspace [RayNode],
}

/// Retained byte/capacity diagnostics for admission and qualification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RayIndexCapacity {
    pub segments: usize,
    pub nodes: usize,
    pub order_capacity: usize,
    pub node_capacity: usize,
    pub owned_bytes: usize,
}

impl BodyRayIndexWorkspace {
    /// Build only for a v4 observation boundary. Existing v3 never pays for it.
    pub fn prepare<'workspace, 'source>(
        &'workspace mut self,
        body: &'source BodySpatialIndex,
    ) -> Result<BodyRayIndex<'workspace, 'source>, RayIndexError> {
        self.order.clear();
        self.nodes.clear();
        let count = body.diagnostics().segments;
        if count > u32::MAX as usize {
            return Err(RayIndexError::Shape);
        }
        let maximum_nodes = if count == 0 {
            0
        } else if count <= LEAF_SEGMENTS {
            1
        } else {
            // Every non-root leaf has at least half a leaf after median splits.
            (count / (LEAF_SEGMENTS / 2))
                .checked_mul(2)
                .and_then(|n| n.checked_sub(1))
                .ok_or(RayIndexError::Arithmetic)?
        };
        if maximum_nodes > u32::MAX as usize {
            return Err(RayIndexError::Shape);
        }
        reserve(&mut self.order, count)?;
        reserve(&mut self.nodes, maximum_nodes)?;
        for index in 0..count {
            let record = body.segment(index).ok_or(RayIndexError::Shape)?;
            if record.owner_id == 0 {
                return Err(RayIndexError::Shape);
            }
            // Enforce supported sensor geometry before exposing a view.
            let ray = SensorRay::new(record.start, 0.0, 0.0)?;
            ray.aabb_clearance(record.end, record.end, record.owner_radius)?;
            self.order.push(index as u32);
        }
        if count != 0 {
            build_node(body, &mut self.order, 0, &mut self.nodes)?;
        }
        debug_assert!(self.nodes.len() <= maximum_nodes);
        Ok(BodyRayIndex {
            body,
            order: &self.order,
            nodes: &self.nodes,
        })
    }

    pub fn diagnostics(&self) -> Result<RayIndexCapacity, RayIndexError> {
        let owned_bytes = self
            .order
            .capacity()
            .checked_mul(size_of::<u32>())
            .and_then(|a| {
                self.nodes
                    .capacity()
                    .checked_mul(size_of::<RayNode>())
                    .and_then(|b| a.checked_add(b))
            })
            .ok_or(RayIndexError::Arithmetic)?;
        Ok(RayIndexCapacity {
            segments: self.order.len(),
            nodes: self.nodes.len(),
            order_capacity: self.order.capacity(),
            node_capacity: self.nodes.capacity(),
            owned_bytes,
        })
    }
}

/// A bounded ray packet shares hierarchy/candidate visits. Exact capsule
/// tests are still charged individually. Stored lower bounds let exhaustion
/// identify unresolved rays without doing uncharged spatial work.
#[derive(Clone, Copy, Debug)]
struct PendingPacket {
    node: u32,
    mask: u32,
    nearest: f64,
    entries: [f64; 32],
}
impl PartialEq for PendingPacket {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node && self.nearest.to_bits() == other.nearest.to_bits()
    }
}
impl Eq for PendingPacket {}
impl PartialOrd for PendingPacket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PendingPacket {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .nearest
            .total_cmp(&self.nearest)
            .then_with(|| other.node.cmp(&self.node))
    }
}
impl PendingPacket {
    fn active(self, best: &[f64; 32], zero: u32) -> u32 {
        let mut remaining = self.mask & !zero;
        let mut active = 0;
        while remaining != 0 {
            let bin = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            if self.entries[bin] <= best[bin] {
                active |= 1u32 << bin;
            }
        }
        active
    }
}
/// Per-worker frontier, bounded by that observation's admitted node work.
#[derive(Clone, Debug, Default)]
pub struct RayQueryScratch {
    packets: BinaryHeap<PendingPacket>,
}
impl RayQueryScratch {
    pub fn frontier_capacity(&self) -> usize {
        self.packets.capacity()
    }
    pub fn packet_capacity(&self) -> usize {
        self.packets.capacity()
    }
    /// Retained packet heap bytes, including all per-bin lower bounds.
    pub fn packet_owned_bytes(&self) -> usize {
        self.packets.capacity() * size_of::<PendingPacket>()
    }
}

impl BodyRayIndex<'_, '_> {
    /// Prevent an evaluator from combining this hierarchy with another boundary.
    pub fn is_from(&self, body: &BodySpatialIndex) -> bool {
        std::ptr::eq(self.body, body)
    }

    /// Traverse a bounded direction packet. One spatial visit handles at most
    /// 32 ray bounds; one candidate visit handles that segment's bounds. Each
    /// actual capsule/ray intersection retains its own charged work unit.
    pub fn trace_packet(
        &self,
        rays: &[SensorRay],
        excluded_owner: u64,
        observer_radius: f64,
        hit_scale: f64,
        meter: &mut RayWorkMeter,
        scratch: &mut RayQueryScratch,
    ) -> Result<[RayIntersection; 32], RayIndexError> {
        if rays.is_empty()
            || rays.len() > 32
            || excluded_owner == 0
            || !observer_radius.is_finite()
            || observer_radius <= 0.0
            || !hit_scale.is_finite()
            || hit_scale <= 0.0
        {
            return Err(RayIndexError::Shape);
        }
        scratch.packets.clear();
        let all = u32::MAX >> (32 - rays.len());
        let mut best = [f64::INFINITY; 32];
        for (bin, ray) in rays.iter().enumerate() {
            best[bin] = ray.maximum();
        }
        let mut found = 0;
        let mut zero = 0;
        let mut unknown = 0;
        if self.nodes.is_empty() {
            return Ok([RayIntersection::Clear; 32]);
        }
        let capacity = self.nodes.len().min(
            (meter.remaining() as usize)
                .checked_add(1)
                .ok_or(RayIndexError::Arithmetic)?,
        );
        if scratch.packets.capacity() < capacity {
            scratch
                .packets
                .try_reserve_exact(capacity)
                .map_err(|_| RayIndexError::Allocation)?;
        }
        if !self.enqueue_packet(
            0,
            all,
            rays,
            excluded_owner,
            observer_radius,
            hit_scale,
            &best,
            meter,
            scratch,
        )? {
            unknown = all;
        }
        'traversal: while unknown == 0 {
            let Some(packet) = scratch.packets.pop() else {
                break;
            };
            let active = packet.active(&best, zero);
            if active == 0 {
                continue;
            }
            let node = &self.nodes[packet.node as usize];
            if node.count == 0 {
                for child in [node.left, node.right] {
                    if !self.enqueue_packet(
                        child,
                        active,
                        rays,
                        excluded_owner,
                        observer_radius,
                        hit_scale,
                        &best,
                        meter,
                        scratch,
                    )? {
                        unknown |= active;
                        break 'traversal;
                    }
                }
            } else {
                let start = node.first as usize;
                for &index in &self.order[start..start + node.count as usize] {
                    let active = packet.active(&best, zero);
                    if active == 0 {
                        break;
                    }
                    if !meter.charge(RayWorkUnit::Candidate) {
                        unknown |= active;
                        break 'traversal;
                    }
                    let record = self
                        .body
                        .segment(index as usize)
                        .ok_or(RayIndexError::Shape)?;
                    if record.owner_id == excluded_owner {
                        continue;
                    }
                    let (minimum, maximum) = record_bounds(record);
                    let inflation = (observer_radius + record.owner_radius) * hit_scale;
                    let mut remaining = active;
                    while remaining != 0 {
                        let bin = remaining.trailing_zeros() as usize;
                        remaining &= remaining - 1;
                        let ray = rays[bin];
                        if !ray
                            .aabb_clearance(minimum, maximum, inflation)?
                            .is_some_and(|entry| entry <= best[bin])
                        {
                            continue;
                        }
                        match ray.charged_capsule_clearance(
                            record.start,
                            record.end,
                            inflation,
                            meter,
                        )? {
                            RayIntersection::Hit(distance) => {
                                found |= 1u32 << bin;
                                best[bin] = best[bin].min(distance);
                                if best[bin] == 0.0 {
                                    zero |= 1u32 << bin;
                                }
                            }
                            RayIntersection::Clear => {}
                            RayIntersection::Incomplete => {
                                unknown |= packet.active(&best, zero);
                                break 'traversal;
                            }
                        }
                    }
                }
            }
        }
        if unknown != 0 {
            for packet in &scratch.packets {
                unknown |= packet.active(&best, zero);
            }
        }
        let mut result = [RayIntersection::Clear; 32];
        for bin in 0..rays.len() {
            let bit = 1u32 << bin;
            result[bin] = if unknown & bit != 0 {
                RayIntersection::Incomplete
            } else if found & bit != 0 {
                RayIntersection::Hit(best[bin])
            } else {
                RayIntersection::Clear
            };
        }
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    fn enqueue_packet(
        &self,
        index: u32,
        mask: u32,
        rays: &[SensorRay],
        excluded_owner: u64,
        observer_radius: f64,
        hit_scale: f64,
        best: &[f64; 32],
        meter: &mut RayWorkMeter,
        scratch: &mut RayQueryScratch,
    ) -> Result<bool, RayIndexError> {
        if !meter.charge(RayWorkUnit::Cell) {
            return Ok(false);
        }
        let node = self.nodes.get(index as usize).ok_or(RayIndexError::Shape)?;
        if node.sole_owner == excluded_owner {
            return Ok(true);
        }
        let inflation = (observer_radius + node.maximum_radius) * hit_scale;
        let mut packet = PendingPacket {
            node: index,
            mask: 0,
            nearest: f64::INFINITY,
            entries: [f64::INFINITY; 32],
        };
        let mut remaining = mask;
        while remaining != 0 {
            let bin = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            if let Some(entry) = rays[bin].aabb_clearance(node.minimum, node.maximum, inflation)? {
                if entry <= best[bin] {
                    packet.mask |= 1u32 << bin;
                    packet.entries[bin] = entry;
                    packet.nearest = packet.nearest.min(entry);
                }
            }
        }
        if packet.mask != 0 {
            scratch.packets.push(packet);
        }
        Ok(true)
    }

    /// Convenience single-ray query shares the same bounded packet traversal.
    pub fn trace(
        &self,
        ray: SensorRay,
        excluded_owner: u64,
        observer_radius: f64,
        hit_scale: f64,
        meter: &mut RayWorkMeter,
        scratch: &mut RayQueryScratch,
    ) -> Result<RayIntersection, RayIndexError> {
        Ok(self.trace_packet(
            &[ray],
            excluded_owner,
            observer_radius,
            hit_scale,
            meter,
            scratch,
        )?[0])
    }
}

fn reserve<T>(values: &mut Vec<T>, count: usize) -> Result<(), RayIndexError> {
    if values.capacity() < count {
        values
            .try_reserve_exact(count.saturating_sub(values.len()))
            .map_err(|_| RayIndexError::Allocation)?;
    }
    Ok(())
}

fn record_bounds(record: &BodySegmentRecord) -> (WorldPoint, WorldPoint) {
    (
        WorldPoint {
            x: record.start.x.min(record.end.x),
            y: record.start.y.min(record.end.y),
        },
        WorldPoint {
            x: record.start.x.max(record.end.x),
            y: record.start.y.max(record.end.y),
        },
    )
}

fn build_node(
    body: &BodySpatialIndex,
    order: &mut [u32],
    first: usize,
    nodes: &mut Vec<RayNode>,
) -> Result<u32, RayIndexError> {
    let initial = body
        .segment(order[0] as usize)
        .ok_or(RayIndexError::Shape)?;
    let (mut minimum, mut maximum) = record_bounds(initial);
    let mut maximum_radius = initial.owner_radius;
    let mut sole_owner = initial.owner_id;
    for &index in &order[1..] {
        let record = body.segment(index as usize).ok_or(RayIndexError::Shape)?;
        let (low, high) = record_bounds(record);
        minimum.x = minimum.x.min(low.x);
        minimum.y = minimum.y.min(low.y);
        maximum.x = maximum.x.max(high.x);
        maximum.y = maximum.y.max(high.y);
        maximum_radius = maximum_radius.max(record.owner_radius);
        if record.owner_id != sole_owner {
            sole_owner = 0;
        }
    }
    let index = u32::try_from(nodes.len()).map_err(|_| RayIndexError::Arithmetic)?;
    nodes.push(RayNode {
        minimum,
        maximum,
        maximum_radius,
        sole_owner,
        first: first as u32,
        count: order.len() as u32,
        left: 0,
        right: 0,
    });
    if order.len() > LEAF_SEGMENTS {
        let x_axis = maximum.x - minimum.x >= maximum.y - minimum.y;
        let middle = order.len() / 2;
        order.select_nth_unstable_by(middle, |a, b| {
            // The generated permutation is in-range and the source stays borrowed.
            let a = body.segment(*a as usize).expect("validated ray record");
            let b = body.segment(*b as usize).expect("validated ray record");
            let center = |r: &BodySegmentRecord| {
                if x_axis {
                    r.start.x * 0.5 + r.end.x * 0.5
                } else {
                    r.start.y * 0.5 + r.end.y * 0.5
                }
            };
            center(a)
                .total_cmp(&center(b))
                .then_with(|| a.owner_id.cmp(&b.owner_id))
                .then_with(|| a.segment_end.cmp(&b.segment_end))
        });
        let (left, right) = order.split_at_mut(middle);
        let left = build_node(body, left, first, nodes)?;
        let right = build_node(body, right, first + middle, nodes)?;
        nodes[index as usize].count = 0;
        nodes[index as usize].left = left;
        nodes[index as usize].right = right;
    }
    Ok(index)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RayIndexError {
    Shape,
    Arithmetic,
    Allocation,
    Geometry(SensorGeometryError),
}
impl From<SensorGeometryError> for RayIndexError {
    fn from(value: SensorGeometryError) -> Self {
        Self::Geometry(value)
    }
}
impl Display for RayIndexError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid sensor ray index: {self:?}")
    }
}
impl std::error::Error for RayIndexError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::{BodyRange, SnakeKind, SnakeState, WorldState};
    use std::f64::consts::TAU;

    fn point(x: f64, y: f64) -> WorldPoint {
        WorldPoint { x, y }
    }
    fn index(bodies: &[(u64, Vec<WorldPoint>, f64)], reverse: bool) -> BodySpatialIndex {
        let mut world = WorldState::default();
        for (id, body, radius) in bodies {
            let start = world.body_points.len();
            world.body_points.extend_from_slice(body);
            world.snakes.push(SnakeState {
                id: *id,
                frame_v1_id: *id as u32,
                kind: SnakeKind::External,
                alive: true,
                population_slot: None,
                brain: None,
                baseline_slot: None,
                baseline_strategy: None,
                position: body[0],
                previous_position: body[0],
                direction: 0.0,
                radius: *radius,
                speed: 0.0,
                boost: false,
                age_seconds: 0.0,
                food: 0.0,
                points: 0.0,
                kills: 0,
                target_length: body.len() as f64,
                fitness: 0.0,
                turn: 0.0,
                previous_turn: 0.0,
                input_boost: false,
                previous_input_boost: false,
                control_accumulator_seconds: 0.0,
                delivered_observation_points: 0.0,
                observation_delivered: false,
                completed_step_heading_delta: 0.0,
                completed_step_speed_delta: 0.0,
                completed_step_applied_turn: 0.0,
                body: BodyRange {
                    start,
                    len: body.len(),
                },
                skin: 0,
            });
        }
        if reverse {
            world.snakes.reverse();
        }
        BodySpatialIndex::build(&world, 70.0, 1_000_000).unwrap()
    }
    fn oracle(
        body: &BodySpatialIndex,
        ray: SensorRay,
        excluded: u64,
        radius: f64,
        scale: f64,
    ) -> RayIntersection {
        let mut nearest: Option<f64> = None;
        for i in 0..body.diagnostics().segments {
            let r = body.segment(i).unwrap();
            if r.owner_id == excluded {
                continue;
            }
            if let Some(hit) = ray
                .capsule_clearance(r.start, r.end, (radius + r.owner_radius) * scale)
                .unwrap()
            {
                nearest = Some(nearest.map_or(hit, |previous| previous.min(hit)));
            }
        }
        nearest.map_or(RayIntersection::Clear, RayIntersection::Hit)
    }
    fn same(actual: RayIntersection, expected: RayIntersection) {
        match (actual, expected) {
            (RayIntersection::Hit(a), RayIntersection::Hit(b)) => {
                assert!((a - b).abs() < 1e-9, "{a} != {b}")
            }
            (a, b) => assert_eq!(a, b),
        }
    }

    #[test]
    fn indexed_rays_match_complete_capsule_oracle_and_world_reordering() {
        let bodies: Vec<_> = (0..64)
            .map(|i| {
                let x = ((i * 73) % 233) as f64 - 116.0;
                let y = ((i * 97) % 211) as f64 - 105.0;
                (
                    i as u64 + 1,
                    vec![
                        point(x, y),
                        point(x + (i % 13) as f64 * 12.0, y - (i % 7) as f64 * 19.0),
                    ],
                    3.0 + (i % 5) as f64,
                )
            })
            .collect();
        let forward = index(&bodies, false);
        let reversed = index(&bodies, true);
        for body in [&forward, &reversed] {
            let mut workspace = BodyRayIndexWorkspace::default();
            let view = workspace.prepare(body).unwrap();
            let mut scratch = RayQueryScratch::default();
            for origin in [point(0.0, 0.0), point(-220.0, 75.0)] {
                for sector in 0..96 {
                    let ray = SensorRay::new(origin, sector as f64 * TAU / 96.0, 500.0).unwrap();
                    let mut meter = RayWorkMeter::new(1024);
                    let actual = view
                        .trace(ray, 1, 9.0, 0.82, &mut meter, &mut scratch)
                        .unwrap();
                    same(actual, oracle(&forward, ray, 1, 9.0, 0.82));
                    assert!(!meter.diagnostics().saturated);
                }
            }
        }
    }

    #[test]
    fn every_small_grant_is_bounded_and_incomplete_results_are_conservative() {
        let bodies: Vec<_> = (1..=48)
            .map(|i| {
                (
                    i,
                    vec![point(i as f64 * 8.0, -80.0), point(i as f64 * 8.0, 80.0)],
                    3.0,
                )
            })
            .collect();
        let body = index(&bodies, false);
        let mut workspace = BodyRayIndexWorkspace::default();
        let view = workspace.prepare(&body).unwrap();
        let ray = SensorRay::new(point(0.0, 0.0), 0.0, 500.0).unwrap();
        let expected = oracle(&body, ray, 99, 2.0, 1.0);
        let mut scratch = RayQueryScratch::default();
        let mut incomplete = 0;
        for grant in 0..=128 {
            let mut meter = RayWorkMeter::new(grant);
            let actual = view
                .trace(ray, 99, 2.0, 1.0, &mut meter, &mut scratch)
                .unwrap();
            let work = meter.diagnostics();
            assert_eq!(
                work.cells + work.candidates + work.intersections + meter.remaining(),
                grant
            );
            if actual == RayIntersection::Incomplete {
                incomplete += 1;
                assert!(work.saturated);
            } else {
                same(actual, expected);
            }
        }
        assert!(incomplete > 0 && incomplete < 128);
    }

    #[test]
    fn long_segments_are_unique_and_own_body_subtrees_do_not_consume_candidate_work() {
        let body = index(
            &[(
                1,
                (0..65).map(|i| point(-(i as f64) * 40.0, 0.0)).collect(),
                8.0,
            )],
            false,
        );
        assert!(body.diagnostics().entries > body.diagnostics().segments);
        let mut workspace = BodyRayIndexWorkspace::default();
        let view = workspace.prepare(&body).unwrap();
        let mut sorted = view.order.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..64).collect::<Vec<u32>>());
        let ray = SensorRay::new(point(0.0, 0.0), std::f64::consts::PI, 1000.0).unwrap();
        let mut scratch = RayQueryScratch::default();
        let mut meter = RayWorkMeter::new(1);
        assert_eq!(
            view.trace(ray, 1, 9.0, 1.0, &mut meter, &mut scratch)
                .unwrap(),
            RayIntersection::Clear
        );
        assert_eq!(meter.diagnostics().cells, 1);
        assert_eq!(meter.diagnostics().candidates, 0);
        assert!(!meter.diagnostics().saturated);
    }

    #[test]
    fn sparse_miss_and_initial_overlap_terminate_without_full_world_scans() {
        let body = index(
            &[(1, vec![point(1000.0, 1000.0), point(1200.0, 1000.0)], 3.0)],
            false,
        );
        let mut workspace = BodyRayIndexWorkspace::default();
        let view = workspace.prepare(&body).unwrap();
        let mut scratch = RayQueryScratch::default();
        let mut meter = RayWorkMeter::new(1);
        assert_eq!(
            view.trace(
                SensorRay::new(point(0.0, 0.0), 0.0, 500.0).unwrap(),
                99,
                9.0,
                1.0,
                &mut meter,
                &mut scratch
            )
            .unwrap(),
            RayIntersection::Clear
        );
        let mut meter = RayWorkMeter::new(3);
        assert_eq!(
            view.trace(
                SensorRay::new(point(1050.0, 1000.0), 0.0, 500.0).unwrap(),
                99,
                9.0,
                1.0,
                &mut meter,
                &mut scratch
            )
            .unwrap(),
            RayIntersection::Hit(0.0)
        );
        assert_eq!(meter.remaining(), 0);
        assert!(!meter.diagnostics().saturated);
    }

    #[test]
    fn warm_queries_and_rebuilds_reuse_capacity_and_empty_world_is_clear() {
        let body = index(
            &[(
                1,
                (0..129).map(|i| point(i as f64 * 7.0, 80.0)).collect(),
                4.0,
            )],
            false,
        );
        let mut workspace = BodyRayIndexWorkspace::default();
        let mut scratch = RayQueryScratch::default();
        let mut warmed = None;
        for _ in 0..3 {
            let view = workspace.prepare(&body).unwrap();
            for sector in 0..32 {
                view.trace(
                    SensorRay::new(point(0.0, 0.0), sector as f64 * TAU / 32.0, 1000.0).unwrap(),
                    99,
                    9.0,
                    0.82,
                    &mut RayWorkMeter::new(512),
                    &mut scratch,
                )
                .unwrap();
            }
            let capacity = (
                workspace.diagnostics().unwrap(),
                scratch.frontier_capacity(),
            );
            if let Some(previous) = warmed {
                assert_eq!(capacity, previous);
            }
            warmed = Some(capacity);
        }
        let empty = BodySpatialIndex::build(&WorldState::default(), 70.0, 1000).unwrap();
        let view = workspace.prepare(&empty).unwrap();
        assert_eq!(
            view.trace(
                SensorRay::new(point(0.0, 0.0), 0.0, 500.0).unwrap(),
                99,
                9.0,
                1.0,
                &mut RayWorkMeter::new(0),
                &mut scratch
            )
            .unwrap(),
            RayIntersection::Clear
        );
    }

    #[test]
    fn packet_results_match_complete_oracles_under_every_exhaustion_boundary() {
        let bodies: Vec<_> = (1..=40)
            .map(|i| {
                let x = ((i * 73) % 233) as f64 - 116.0;
                let y = ((i * 97) % 211) as f64 - 105.0;
                (i, vec![point(x, y), point(x + 50.0, y - 35.0)], 3.0)
            })
            .collect();
        let body = index(&bodies, false);
        let mut workspace = BodyRayIndexWorkspace::default();
        let view = workspace.prepare(&body).unwrap();
        let mut scratch = RayQueryScratch::default();
        for count in [1, 8, 16, 32] {
            let rays: Vec<_> = (0..count)
                .map(|bin| {
                    SensorRay::new(point(-150.0, 35.0), bin as f64 * TAU / count as f64, 500.0)
                        .unwrap()
                })
                .collect();
            let expected: Vec<_> = rays
                .iter()
                .map(|ray| oracle(&body, *ray, 99, 9.0, 0.82))
                .collect();
            for grant in 0..=512 {
                let mut meter = RayWorkMeter::new(grant);
                let result = view
                    .trace_packet(&rays, 99, 9.0, 0.82, &mut meter, &mut scratch)
                    .unwrap();
                let work = meter.diagnostics();
                assert_eq!(
                    work.cells + work.candidates + work.intersections + meter.remaining(),
                    grant
                );
                for (actual, expected) in result.iter().zip(&expected) {
                    if *actual == RayIntersection::Incomplete {
                        assert!(work.saturated);
                    } else {
                        same(*actual, *expected);
                    }
                }
            }
        }
    }

    #[test]
    fn sparse_direction_packet_shares_one_bound_visit_and_reuses_frontier() {
        let body = index(
            &[(1, vec![point(1000.0, 1000.0), point(1200.0, 1000.0)], 3.0)],
            false,
        );
        let mut workspace = BodyRayIndexWorkspace::default();
        let view = workspace.prepare(&body).unwrap();
        let rays: Vec<_> = (0..32)
            .map(|bin| SensorRay::new(point(0.0, 0.0), bin as f64 * TAU / 32.0, 500.0).unwrap())
            .collect();
        let mut scratch = RayQueryScratch::default();
        let mut previous = None;
        for _ in 0..3 {
            let mut meter = RayWorkMeter::new(1);
            assert_eq!(
                view.trace_packet(&rays, 99, 9.0, 1.0, &mut meter, &mut scratch)
                    .unwrap(),
                [RayIntersection::Clear; 32]
            );
            assert_eq!(meter.diagnostics().cells, 1);
            assert!(!meter.diagnostics().saturated);
            if let Some(capacity) = previous {
                assert_eq!(scratch.packet_capacity(), capacity);
            }
            previous = Some(scratch.packet_capacity());
        }
        assert!(view
            .trace_packet(&[], 99, 9.0, 1.0, &mut RayWorkMeter::new(1), &mut scratch)
            .is_err());
        assert!(view
            .trace_packet(&rays, 0, 9.0, 1.0, &mut RayWorkMeter::new(1), &mut scratch)
            .is_err());
    }
}
