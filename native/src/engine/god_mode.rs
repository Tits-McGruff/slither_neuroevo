//! Ordered God Mode mutations applied before the next authoritative step.

use super::state::{WorldPoint, WorldState};
use std::error::Error;
use std::fmt::{Display, Formatter};

/// Result of translating one live snake and its complete swept state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GodModeMovePublication {
    /// Exact public frame-v1 identity selected by the browser.
    pub frame_v1_id: u32,
    /// Clamped authoritative head X.
    pub x: f64,
    /// Clamped authoritative head Y.
    pub y: f64,
}

/// Translate one live snake by the largest delta that keeps its entire body in bounds.
pub(crate) fn apply_god_mode_move(
    world: &mut WorldState,
    world_radius: f64,
    frame_v1_id: u32,
    target: WorldPoint,
) -> Result<GodModeMovePublication, GodModeError> {
    if frame_v1_id == 0 || !target.x.is_finite() || !target.y.is_finite() {
        return Err(GodModeError::InvalidRequest);
    }
    let snake_index = world
        .snakes
        .iter()
        .position(|snake| snake.frame_v1_id == frame_v1_id && snake.alive)
        .ok_or(GodModeError::MissingOrDeadSnake(frame_v1_id))?;
    let snake = &world.snakes[snake_index];
    let body_end = snake
        .body
        .start
        .checked_add(snake.body.len)
        .filter(|end| *end <= world.body_points.len())
        .ok_or(GodModeError::InvalidBody)?;
    let body = &world.body_points[snake.body.start..body_end];
    let dx = target.x - snake.position.x;
    let dy = target.y - snake.position.y;
    let radial_limit = (world_radius - snake.radius).max(0.0);
    let scale = maximum_translation_scale(body, dx, dy, radial_limit)?
        .min(maximum_translation_scale(
            std::slice::from_ref(&snake.position),
            dx,
            dy,
            radial_limit,
        )?)
        .min(maximum_translation_scale(
            std::slice::from_ref(&snake.previous_position),
            dx,
            dy,
            radial_limit,
        )?);
    if scale <= f64::EPSILON && (dx.abs() > 1.0e-9 || dy.abs() > 1.0e-9) {
        return Err(GodModeError::NoValidTranslation);
    }
    let applied_dx = dx * scale;
    let applied_dy = dy * scale;
    let translated_head = translate(snake.position, applied_dx, applied_dy)?;
    let translated_previous = translate(snake.previous_position, applied_dx, applied_dy)?;
    for point in body {
        translate(*point, applied_dx, applied_dy)?;
    }
    let snake = &mut world.snakes[snake_index];
    snake.position = translated_head;
    snake.previous_position = translated_previous;
    for point in &mut world.body_points[snake.body.start..body_end] {
        point.x += applied_dx;
        point.y += applied_dy;
    }
    Ok(GodModeMovePublication {
        frame_v1_id,
        x: translated_head.x,
        y: translated_head.y,
    })
}

fn translate(point: WorldPoint, dx: f64, dy: f64) -> Result<WorldPoint, GodModeError> {
    let translated = WorldPoint {
        x: point.x + dx,
        y: point.y + dy,
    };
    if !translated.x.is_finite() || !translated.y.is_finite() {
        return Err(GodModeError::InvalidRequest);
    }
    Ok(translated)
}

fn maximum_translation_scale(
    points: &[WorldPoint],
    dx: f64,
    dy: f64,
    limit: f64,
) -> Result<f64, GodModeError> {
    let delta_squared = dx * dx + dy * dy;
    let limit_squared = limit * limit;
    if !delta_squared.is_finite() || !limit_squared.is_finite() {
        return Err(GodModeError::InvalidRequest);
    }
    let mut scale = 1.0_f64;
    for point in points {
        let current_error = point.x * point.x + point.y * point.y - limit_squared;
        if !current_error.is_finite() || current_error > 1.0e-6 {
            return Err(GodModeError::InvalidBody);
        }
        if delta_squared <= f64::EPSILON {
            continue;
        }
        let linear = 2.0 * (point.x * dx + point.y * dy);
        let discriminant = linear * linear - 4.0 * delta_squared * current_error;
        if !linear.is_finite() || !discriminant.is_finite() || discriminant < 0.0 {
            return Err(GodModeError::InvalidBody);
        }
        let exit_scale = (-linear + discriminant.sqrt()) / (2.0 * delta_squared);
        if exit_scale.is_finite() {
            scale = scale.min(exit_scale);
        }
    }
    Ok(scale.clamp(0.0, 1.0))
}

/// Recoverable rejection of one requested God Mode move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GodModeError {
    InvalidRequest,
    MissingOrDeadSnake(u32),
    InvalidBody,
    NoValidTranslation,
}

impl Display for GodModeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest => write!(formatter, "God Mode move coordinates must be finite"),
            Self::MissingOrDeadSnake(id) => {
                write!(formatter, "snake {id} is missing or already dead")
            }
            Self::InvalidBody => write!(formatter, "snake body is outside valid world bounds"),
            Self::NoValidTranslation => {
                write!(formatter, "translation cannot keep the body in bounds")
            }
        }
    }
}

impl Error for GodModeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::{BodyRange, SnakeKind, SnakeState};

    fn snake(position: WorldPoint, body_len: usize) -> SnakeState {
        SnakeState {
            id: 1,
            frame_v1_id: 7,
            kind: SnakeKind::Evolved,
            alive: true,
            population_slot: Some(0),
            brain: None,
            baseline_slot: None,
            baseline_strategy: None,
            position,
            previous_position: position,
            direction: 0.0,
            radius: 5.0,
            speed: 0.0,
            boost: false,
            age_seconds: 0.0,
            food: 0.0,
            points: 0.0,
            kills: 0,
            target_length: body_len as f64,
            fitness: 0.0,
            turn: 0.0,
            previous_turn: 0.0,
            input_boost: false,
            previous_input_boost: false,
            control_accumulator_seconds: 0.0,
            delivered_observation_points: 0.0,
            body: BodyRange {
                start: 0,
                len: body_len,
            },
            skin: 0,
        }
    }

    #[test]
    fn translates_head_previous_position_and_complete_body_together() {
        let mut world = WorldState {
            snakes: vec![snake(WorldPoint { x: 10.0, y: 10.0 }, 2)],
            body_points: vec![
                WorldPoint { x: 10.0, y: 10.0 },
                WorldPoint { x: 5.0, y: 10.0 },
            ],
            ..WorldState::default()
        };
        let result = apply_god_mode_move(&mut world, 100.0, 7, WorldPoint { x: 30.0, y: 40.0 })
            .expect("interior translation");
        assert_eq!((result.x, result.y), (30.0, 40.0));
        assert_eq!(
            world.snakes[0].previous_position,
            WorldPoint { x: 30.0, y: 40.0 }
        );
        assert_eq!(world.body_points[1], WorldPoint { x: 25.0, y: 40.0 });
    }
}
