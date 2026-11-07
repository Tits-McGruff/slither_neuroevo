//! Value checks for authoritative state.

use super::*;

pub(crate) fn validate_f32_slice(
    field: &'static str,
    outer_index: usize,
    values: &[f32],
) -> Result<(), StateError> {
    if values.iter().any(|value| !value.is_finite()) {
        return Err(StateError::NonFinite {
            field,
            index: outer_index,
        });
    }
    Ok(())
}

pub(super) fn validate_point(
    field: &'static str,
    index: usize,
    point: WorldPoint,
) -> Result<(), StateError> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err(StateError::NonFinite { field, index });
    }
    Ok(())
}

pub(super) fn validate_positive_f64(field: &'static str, value: f64) -> Result<(), StateError> {
    if !value.is_finite() || value <= 0.0 {
        return invalid(field, "value must be finite and positive");
    }
    Ok(())
}

pub(super) fn validate_nonnegative_f64(field: &'static str, value: f64) -> Result<(), StateError> {
    if !value.is_finite() || value < 0.0 {
        return invalid(field, "value must be finite and non-negative");
    }
    Ok(())
}

pub(super) fn validate_text(field: &'static str, value: &str) -> Result<(), StateError> {
    if value.is_empty() || value.as_bytes().contains(&0) {
        return invalid(field, "text must be non-empty and contain no NUL bytes");
    }
    Ok(())
}

pub(super) fn validate_sha256(field: &'static str, value: &str) -> Result<(), StateError> {
    let Some(hexadecimal) = value.strip_prefix("sha256:") else {
        return invalid(field, "must use sha256:<64 lowercase hexadecimal digits>");
    };
    if hexadecimal.len() != 64
        || !hexadecimal
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return invalid(field, "must use sha256:<64 lowercase hexadecimal digits>");
    }
    Ok(())
}

pub(super) fn validate_sha256_hex(field: &'static str, value: &str) -> Result<(), StateError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return invalid(
            field,
            "must contain 64 lowercase SHA-256 hexadecimal digits",
        );
    }
    Ok(())
}

pub(super) fn validate_build_class(field: &'static str, value: &str) -> Result<(), StateError> {
    if !matches!(value, "production" | "test-hooks") {
        return invalid(field, "must be production or test-hooks");
    }
    Ok(())
}

pub(super) fn invalid<T>(field: &'static str, reason: &str) -> Result<T, StateError> {
    Err(invalid_error(field, reason))
}

pub(super) fn invalid_error(field: &'static str, reason: &str) -> StateError {
    StateError::InvalidField {
        field,
        reason: reason.to_owned(),
    }
}

pub(super) fn checked_add(
    left: usize,
    right: usize,
    context: &'static str,
) -> Result<usize, StateError> {
    left.checked_add(right)
        .ok_or(StateError::ArithmeticOverflow { context })
}

pub(super) fn checked_sum(values: &[usize]) -> Result<usize, StateError> {
    values.iter().try_fold(0usize, |total, value| {
        checked_add(total, *value, "state memory total")
    })
}
