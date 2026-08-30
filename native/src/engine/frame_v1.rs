//! Direct Rust packing for the browser's existing binary display-frame v1.
//!
//! The frame remains a compatibility format made only of little-endian
//! `f32` values on the two supported x86_64 targets. Simulation state supplies
//! the first four header fields and all entity data. A caller-supplied view
//! descriptor is echoed only as presentation metadata and never enters
//! authority. The current browser passes its locally smoothed camera as render
//! overrides, so these compatibility fields do not give Rust camera ownership.

use super::state::{AuthoritativeState, WorldState, FRAME_V1_MAX_EXACT_ID};
use std::error::Error;
use std::fmt;
use std::mem::size_of;

/// Display-frame v1 layout version implemented by this module.
pub const FRAME_V1_PACKER_VERSION: u32 = 1;
/// Number of Float32 values in the global frame header.
pub const FRAME_V1_HEADER_FLOATS: usize = 7;
/// Number of Float32 values before one alive snake's body coordinates.
pub const FRAME_V1_SNAKE_HEADER_FLOATS: usize = 8;
/// Number of Float32 values in one pellet record.
pub const FRAME_V1_PELLET_FLOATS: usize = 5;

/// Presentation-only values echoed into the final three frame-v1 header fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameV1ViewDescriptor {
    /// Browser camera center in world coordinates.
    pub camera_x: f64,
    /// Browser camera center in world coordinates.
    pub camera_y: f64,
    /// Browser presentation zoom; it must be finite and positive.
    pub zoom: f64,
}

impl Default for FrameV1ViewDescriptor {
    fn default() -> Self {
        Self {
            camera_x: 0.0,
            camera_y: 0.0,
            zoom: 1.0,
        }
    }
}

/// Small routing metadata produced without traversing the world again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameV1Metadata {
    /// Authoritative generation written into the frame.
    pub generation: u64,
    /// All authoritative snake records, including dead records omitted below.
    pub total_snakes: usize,
    /// Alive snake records actually included in the variable block.
    pub alive_snakes: usize,
    /// Pellet records included in the frame.
    pub pellets: usize,
    /// Number of Float32 values in the complete frame.
    pub float_length: usize,
    /// Exact byte length for welcome and socket-routing metadata.
    pub byte_length: usize,
}

/// Bounded frame packing failure. A failed pack leaves the caller buffer unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameV1Error {
    /// Checked layout arithmetic overflowed.
    ArithmeticOverflow { context: &'static str },
    /// A displayed integer cannot be represented exactly by frame-v1 Float32.
    InexactInteger {
        field: &'static str,
        index: usize,
        value: u64,
    },
    /// A floating-point value is non-finite or overflows when narrowed to Float32.
    InvalidFloat { field: &'static str, index: usize },
    /// An alive snake references body storage outside the supplied world.
    InvalidBodyRange { snake_id: u64 },
    /// A pellet kind is not part of the current four-value frame-v1 mapping.
    UnsupportedPelletKind { index: usize, kind: u32 },
    /// The complete frame exceeds the allocation already admitted with authority.
    FrameTooLarge {
        required_bytes: usize,
        maximum_bytes: usize,
    },
    /// The output allocation could not be reserved without exposing a partial frame.
    AllocationFailed { required_bytes: usize },
}

impl fmt::Display for FrameV1Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow { context } => {
                write!(
                    formatter,
                    "frame-v1 arithmetic overflow while calculating {context}"
                )
            }
            Self::InexactInteger {
                field,
                index,
                value,
            } => write!(
                formatter,
                "frame-v1 {field}[{index}] value {value} is not exactly representable as Float32"
            ),
            Self::InvalidFloat { field, index } => write!(
                formatter,
                "frame-v1 {field}[{index}] is non-finite or outside the Float32 range"
            ),
            Self::InvalidBodyRange { snake_id } => {
                write!(
                    formatter,
                    "frame-v1 snake {snake_id} has an invalid body range"
                )
            }
            Self::UnsupportedPelletKind { index, kind } => write!(
                formatter,
                "frame-v1 pellet[{index}] kind {kind} is outside the current 0..=3 mapping"
            ),
            Self::FrameTooLarge {
                required_bytes,
                maximum_bytes,
            } => write!(
                formatter,
                "frame-v1 requires {required_bytes} bytes but authority admits {maximum_bytes}"
            ),
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "frame-v1 could not reserve {required_bytes} wire bytes"
            ),
        }
    }
}

impl Error for FrameV1Error {}

/// Pack one validated authoritative state directly into caller-owned reusable memory.
///
/// The maximum comes from the frame allocation charged during state admission.
/// All fallible validation and sizing occurs before `output` is changed. The
/// returned byte length is intended to be cached by the frame-routing layer so
/// welcome refreshes never serialize the world merely to calculate one field.
pub fn pack_authoritative_frame_v1_into(
    authority: &AuthoritativeState,
    view: FrameV1ViewDescriptor,
    output: &mut Vec<u8>,
) -> Result<FrameV1Metadata, FrameV1Error> {
    let state = authority.state();
    pack_frame_v1_source_into(
        state.generation.generation,
        state.config.world_radius,
        &state.world,
        view,
        authority.memory_estimate().frame_bytes,
        output,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FrameV1Shape {
    generation: u64,
    total_snakes: usize,
    alive_snakes: usize,
    pellets: usize,
    float_length: usize,
    byte_length: usize,
}

#[allow(clippy::too_many_arguments)]
fn pack_frame_v1_source_into(
    generation: u64,
    world_radius: f64,
    world: &WorldState,
    view: FrameV1ViewDescriptor,
    maximum_bytes: usize,
    output: &mut Vec<u8>,
) -> Result<FrameV1Metadata, FrameV1Error> {
    let shape = preflight_frame(generation, world_radius, world, view, maximum_bytes)?;
    if shape.byte_length > output.capacity() {
        output
            .try_reserve_exact(shape.byte_length.saturating_sub(output.len()))
            .map_err(|_| FrameV1Error::AllocationFailed {
                required_bytes: shape.byte_length,
            })?;
    }
    output.resize(shape.byte_length, 0);

    let mut cursor = 0;
    write_f32(output, &mut cursor, generation as f32);
    write_f32(output, &mut cursor, shape.total_snakes as f32);
    write_f32(output, &mut cursor, shape.alive_snakes as f32);
    write_f32(output, &mut cursor, world_radius as f32);
    write_f32(output, &mut cursor, view.camera_x as f32);
    write_f32(output, &mut cursor, view.camera_y as f32);
    write_f32(output, &mut cursor, view.zoom as f32);

    for snake in &world.snakes {
        if !snake.alive {
            continue;
        }
        write_f32(output, &mut cursor, snake.frame_v1_id as f32);
        write_f32(output, &mut cursor, snake.radius as f32);
        write_f32(output, &mut cursor, snake.skin as f32);
        write_f32(output, &mut cursor, snake.position.x as f32);
        write_f32(output, &mut cursor, snake.position.y as f32);
        write_f32(output, &mut cursor, snake.direction as f32);
        write_f32(output, &mut cursor, if snake.boost { 1.0 } else { 0.0 });
        write_f32(output, &mut cursor, snake.body.len as f32);

        let body_end = snake.body.start + snake.body.len;
        for point in &world.body_points[snake.body.start..body_end] {
            write_f32(output, &mut cursor, point.x as f32);
            write_f32(output, &mut cursor, point.y as f32);
        }
    }

    write_f32(output, &mut cursor, shape.pellets as f32);
    for pellet in &world.pellets {
        write_f32(output, &mut cursor, pellet.position.x as f32);
        write_f32(output, &mut cursor, pellet.position.y as f32);
        write_f32(output, &mut cursor, pellet.value as f32);
        write_f32(output, &mut cursor, pellet.kind as f32);
        write_f32(output, &mut cursor, pellet.color as f32);
    }
    debug_assert_eq!(cursor, shape.byte_length);

    Ok(FrameV1Metadata {
        generation: shape.generation,
        total_snakes: shape.total_snakes,
        alive_snakes: shape.alive_snakes,
        pellets: shape.pellets,
        float_length: shape.float_length,
        byte_length: shape.byte_length,
    })
}

fn preflight_frame(
    generation: u64,
    world_radius: f64,
    world: &WorldState,
    view: FrameV1ViewDescriptor,
    maximum_bytes: usize,
) -> Result<FrameV1Shape, FrameV1Error> {
    exact_u64("generation", 0, generation)?;
    exact_usize("total_snakes", 0, world.snakes.len())?;
    exact_usize("pellet_count", 0, world.pellets.len())?;
    finite_f32("world_radius", 0, world_radius, true)?;
    finite_f32("camera_x", 0, view.camera_x, false)?;
    finite_f32("camera_y", 0, view.camera_y, false)?;
    finite_f32("zoom", 0, view.zoom, true)?;

    let mut alive_snakes = 0usize;
    let mut snake_floats = 0usize;
    for (index, snake) in world.snakes.iter().enumerate() {
        if !snake.alive {
            continue;
        }
        alive_snakes = checked_add(alive_snakes, 1, "alive snake count")?;
        exact_u64("snake_id", index, u64::from(snake.frame_v1_id))?;
        exact_u64("skin", index, u64::from(snake.skin))?;
        exact_usize("body_length", index, snake.body.len)?;
        finite_f32("snake_radius", index, snake.radius, true)?;
        finite_f32("snake_x", index, snake.position.x, false)?;
        finite_f32("snake_y", index, snake.position.y, false)?;
        finite_f32("snake_direction", index, snake.direction, false)?;
        let body_end = snake
            .body
            .start
            .checked_add(snake.body.len)
            .filter(|end| *end <= world.body_points.len())
            .ok_or(FrameV1Error::InvalidBodyRange { snake_id: snake.id })?;
        for (point_offset, point) in world.body_points[snake.body.start..body_end]
            .iter()
            .enumerate()
        {
            finite_f32("body_x", point_offset, point.x, false)?;
            finite_f32("body_y", point_offset, point.y, false)?;
        }
        let body_floats = checked_mul(snake.body.len, 2, "snake body coordinates")?;
        snake_floats = checked_add(
            snake_floats,
            checked_add(FRAME_V1_SNAKE_HEADER_FLOATS, body_floats, "one snake block")?,
            "all snake blocks",
        )?;
    }
    exact_usize("alive_snakes", 0, alive_snakes)?;

    for (index, pellet) in world.pellets.iter().enumerate() {
        finite_f32("pellet_x", index, pellet.position.x, false)?;
        finite_f32("pellet_y", index, pellet.position.y, false)?;
        finite_f32("pellet_value", index, pellet.value, true)?;
        if pellet.kind > 3 {
            return Err(FrameV1Error::UnsupportedPelletKind {
                index,
                kind: pellet.kind,
            });
        }
        exact_u64("pellet_color", index, u64::from(pellet.color))?;
    }

    let pellet_floats = checked_mul(world.pellets.len(), FRAME_V1_PELLET_FLOATS, "pellet fields")?;
    let float_length = checked_add(
        checked_add(
            checked_add(FRAME_V1_HEADER_FLOATS, snake_floats, "frame snakes")?,
            1,
            "pellet count field",
        )?,
        pellet_floats,
        "complete frame",
    )?;
    let byte_length = checked_mul(float_length, size_of::<f32>(), "frame byte length")?;
    if byte_length > maximum_bytes {
        return Err(FrameV1Error::FrameTooLarge {
            required_bytes: byte_length,
            maximum_bytes,
        });
    }
    Ok(FrameV1Shape {
        generation,
        total_snakes: world.snakes.len(),
        alive_snakes,
        pellets: world.pellets.len(),
        float_length,
        byte_length,
    })
}

fn exact_usize(field: &'static str, index: usize, value: usize) -> Result<(), FrameV1Error> {
    let value = u64::try_from(value).map_err(|_| FrameV1Error::ArithmeticOverflow {
        context: "frame integer conversion",
    })?;
    exact_u64(field, index, value)
}

fn exact_u64(field: &'static str, index: usize, value: u64) -> Result<(), FrameV1Error> {
    if value <= u64::from(FRAME_V1_MAX_EXACT_ID) {
        Ok(())
    } else {
        Err(FrameV1Error::InexactInteger {
            field,
            index,
            value,
        })
    }
}

fn finite_f32(
    field: &'static str,
    index: usize,
    value: f64,
    require_positive: bool,
) -> Result<(), FrameV1Error> {
    let narrowed = value as f32;
    if value.is_finite() && narrowed.is_finite() && (!require_positive || value > 0.0) {
        Ok(())
    } else {
        Err(FrameV1Error::InvalidFloat { field, index })
    }
}

fn write_f32(output: &mut [u8], cursor: &mut usize, value: f32) {
    let end = *cursor + size_of::<f32>();
    output[*cursor..end].copy_from_slice(&value.to_le_bytes());
    *cursor = end;
}

fn checked_add(left: usize, right: usize, context: &'static str) -> Result<usize, FrameV1Error> {
    left.checked_add(right)
        .ok_or(FrameV1Error::ArithmeticOverflow { context })
}

fn checked_mul(left: usize, right: usize, context: &'static str) -> Result<usize, FrameV1Error> {
    left.checked_mul(right)
        .ok_or(FrameV1Error::ArithmeticOverflow { context })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::{BodyRange, PelletState, SnakeKind, SnakeState, WorldPoint};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Fixture {
        view: FixtureView,
        source: FixtureSource,
        expected: FixtureExpected,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureView {
        camera_x: f64,
        camera_y: f64,
        zoom: f64,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureSource {
        generation: u64,
        world_radius: f64,
        snakes: Vec<FixtureSnake>,
        pellets: Vec<FixturePellet>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureSnake {
        id: u64,
        frame_v1_id: u32,
        radius: f64,
        skin: u32,
        x: f64,
        y: f64,
        direction: f64,
        boost: bool,
        alive: bool,
        body: Vec<FixturePoint>,
    }

    #[derive(Clone, Copy, Deserialize)]
    struct FixturePoint {
        x: f64,
        y: f64,
    }

    #[derive(Deserialize)]
    struct FixturePellet {
        id: u64,
        x: f64,
        y: f64,
        value: f64,
        kind: u32,
        color: u32,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureExpected {
        float_length: usize,
        byte_length: usize,
        float_bits: Vec<String>,
    }

    fn fixture() -> Fixture {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("frame-v1-reference.json");
        let json = std::fs::read_to_string(path).expect("retained frame-v1 fixture must read");
        serde_json::from_str(&json).expect("retained frame-v1 fixture must parse")
    }

    fn fixture_world(source: &FixtureSource) -> WorldState {
        let mut body_points = Vec::new();
        let mut snakes = Vec::with_capacity(source.snakes.len());
        for (slot, snake) in source.snakes.iter().enumerate() {
            let body_start = body_points.len();
            body_points.extend(snake.body.iter().map(|point| WorldPoint {
                x: point.x,
                y: point.y,
            }));
            let position = WorldPoint {
                x: snake.x,
                y: snake.y,
            };
            snakes.push(SnakeState {
                id: snake.id,
                frame_v1_id: snake.frame_v1_id,
                kind: SnakeKind::Evolved,
                alive: snake.alive,
                population_slot: Some(slot as u32),
                brain: None,
                baseline_slot: None,
                baseline_strategy: None,
                position,
                previous_position: position,
                direction: snake.direction,
                radius: snake.radius,
                speed: 0.0,
                boost: snake.boost,
                age_seconds: 0.0,
                food: 0.0,
                points: 0.0,
                kills: 0,
                target_length: snake.body.len() as f64,
                fitness: 0.0,
                turn: 0.0,
                previous_turn: 0.0,
                input_boost: false,
                previous_input_boost: false,
                control_accumulator_seconds: 0.0,
                delivered_observation_points: 0.0,
                body: BodyRange {
                    start: body_start,
                    len: snake.body.len(),
                },
                skin: snake.skin,
            });
        }
        WorldState {
            snakes,
            body_points,
            pellets: source
                .pellets
                .iter()
                .map(|pellet| PelletState {
                    id: pellet.id,
                    position: WorldPoint {
                        x: pellet.x,
                        y: pellet.y,
                    },
                    value: pellet.value,
                    kind: pellet.kind,
                    color: pellet.color,
                    owner: None,
                })
                .collect(),
            controller_leases: Vec::new(),
        }
    }

    fn decode_float_bits(value: &str) -> u32 {
        u32::from_str_radix(value.strip_prefix("0x").expect("Float32 hex prefix"), 16)
            .expect("Float32 hex value")
    }

    fn read_f32(output: &[u8], float_index: usize) -> f32 {
        let start = float_index * size_of::<f32>();
        let bytes: [u8; 4] = output[start..start + size_of::<f32>()]
            .try_into()
            .expect("one complete Float32 word");
        f32::from_le_bytes(bytes)
    }

    fn snake(id: u64, frame_v1_id: u32, alive: bool, skin: u32, body: BodyRange) -> SnakeState {
        let position = WorldPoint { x: 12.5, y: -3.25 };
        SnakeState {
            id,
            frame_v1_id,
            kind: SnakeKind::Evolved,
            alive,
            population_slot: Some(0),
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
            position,
            previous_position: position,
            direction: std::f64::consts::FRAC_PI_3,
            radius: 10.25,
            speed: 0.0,
            boost: true,
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
            skin,
        }
    }

    fn world() -> WorldState {
        let body_points = vec![
            WorldPoint { x: 12.5, y: -3.25 },
            WorldPoint { x: 8.0, y: -1.0 },
            WorldPoint { x: 2.0, y: 4.5 },
            WorldPoint { x: -9.0, y: 7.0 },
        ];
        let mut first = snake(1, 17, true, 2, BodyRange { start: 0, len: 3 });
        let mut dead = snake(2, 18, false, 0, BodyRange { start: 0, len: 0 });
        dead.position = WorldPoint { x: 99.0, y: 99.0 };
        let mut last = snake(
            3,
            FRAME_V1_MAX_EXACT_ID,
            true,
            1,
            BodyRange { start: 3, len: 1 },
        );
        last.position = body_points[3];
        last.previous_position = body_points[3];
        last.radius = 6.5;
        last.direction = -1.2;
        last.boost = false;
        first.population_slot = Some(0);
        last.population_slot = Some(1);
        WorldState {
            snakes: vec![first, dead, last],
            body_points,
            pellets: vec![
                PelletState {
                    id: 10,
                    position: WorldPoint { x: 1.0, y: 2.0 },
                    value: 1.5,
                    kind: 0,
                    color: 0,
                    owner: None,
                },
                PelletState {
                    id: 11,
                    position: WorldPoint { x: -3.0, y: 4.0 },
                    value: 2.0,
                    kind: 1,
                    color: 17,
                    owner: Some(1),
                },
                PelletState {
                    id: 12,
                    position: WorldPoint { x: 5.25, y: -6.75 },
                    value: 0.75,
                    kind: 2,
                    color: FRAME_V1_MAX_EXACT_ID,
                    owner: Some(3),
                },
                PelletState {
                    id: 13,
                    position: WorldPoint { x: 8.5, y: 9.25 },
                    value: 0.25,
                    kind: 3,
                    color: 0,
                    owner: None,
                },
            ],
            controller_leases: Vec::new(),
        }
    }

    #[test]
    fn view_defaults_are_neutral_and_success_reuses_the_caller_buffer() {
        let source = world();
        let mut output = Vec::with_capacity(256);
        let original_capacity = output.capacity();
        let metadata = pack_frame_v1_source_into(
            7,
            3_500.0,
            &source,
            FrameV1ViewDescriptor::default(),
            1_024,
            &mut output,
        )
        .unwrap();
        assert_eq!(read_f32(&output, 4), 0.0);
        assert_eq!(read_f32(&output, 5), 0.0);
        assert_eq!(read_f32(&output, 6), 1.0);
        assert_eq!(metadata.byte_length, metadata.float_length * 4);
        assert_eq!(metadata.total_snakes, 3);
        assert_eq!(metadata.alive_snakes, 2);
        assert_eq!(metadata.pellets, 4);
        assert_eq!(output.capacity(), original_capacity);
    }

    #[test]
    fn current_typescript_fixture_matches_every_float32_bit() {
        let fixture = fixture();
        let world = fixture_world(&fixture.source);
        let view = FrameV1ViewDescriptor {
            camera_x: fixture.view.camera_x,
            camera_y: fixture.view.camera_y,
            zoom: fixture.view.zoom,
        };
        let mut output = Vec::new();
        let metadata = pack_frame_v1_source_into(
            fixture.source.generation,
            fixture.source.world_radius,
            &world,
            view,
            fixture.expected.byte_length,
            &mut output,
        )
        .expect("current TypeScript fixture packs");
        assert_eq!(metadata.float_length, fixture.expected.float_length);
        assert_eq!(metadata.byte_length, fixture.expected.byte_length);
        assert_eq!(output.len(), fixture.expected.byte_length);
        assert_eq!(
            output.len() / size_of::<f32>(),
            fixture.expected.float_bits.len()
        );
        for (index, (actual, expected)) in output
            .chunks_exact(size_of::<f32>())
            .zip(&fixture.expected.float_bits)
            .enumerate()
        {
            let bytes: [u8; 4] = actual.try_into().expect("one complete Float32 word");
            assert_eq!(
                u32::from_le_bytes(bytes),
                decode_float_bits(expected),
                "Float32 word {index}"
            );
        }
    }

    #[test]
    fn every_late_failure_leaves_the_existing_output_unchanged() {
        let mut source = world();
        let mut output = vec![91, 92, 93];
        let original = output.clone();
        source.pellets.last_mut().unwrap().kind = 4;
        assert!(matches!(
            pack_frame_v1_source_into(
                7,
                3_500.0,
                &source,
                FrameV1ViewDescriptor::default(),
                1_024,
                &mut output,
            ),
            Err(FrameV1Error::UnsupportedPelletKind { index: 3, kind: 4 })
        ));
        assert_eq!(output, original);

        source.pellets.last_mut().unwrap().kind = 3;
        assert!(matches!(
            pack_frame_v1_source_into(
                7,
                3_500.0,
                &source,
                FrameV1ViewDescriptor::default(),
                4,
                &mut output,
            ),
            Err(FrameV1Error::FrameTooLarge { .. })
        ));
        assert_eq!(output, original);
    }

    #[test]
    fn frame_v1_rejects_inexact_identity_and_invalid_body_without_writing() {
        let mut source = world();
        let mut output = vec![3];
        source.snakes[0].frame_v1_id = FRAME_V1_MAX_EXACT_ID + 1;
        assert!(matches!(
            pack_frame_v1_source_into(
                7,
                3_500.0,
                &source,
                FrameV1ViewDescriptor::default(),
                1_024,
                &mut output,
            ),
            Err(FrameV1Error::InexactInteger {
                field: "snake_id",
                index: 0,
                ..
            })
        ));
        assert_eq!(output, vec![3]);

        source.snakes[0].frame_v1_id = 17;
        source.snakes[0].body.start = usize::MAX;
        source.snakes[0].body.len = 1;
        assert!(matches!(
            pack_frame_v1_source_into(
                7,
                3_500.0,
                &source,
                FrameV1ViewDescriptor::default(),
                usize::MAX,
                &mut output,
            ),
            Err(FrameV1Error::InvalidBodyRange { snake_id: 1 })
        ));
        assert_eq!(output, vec![3]);
    }
}
