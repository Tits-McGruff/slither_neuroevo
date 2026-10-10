//! Process CPU admission for the persistent Rust calculation pool.

use std::fmt;
use std::sync::OnceLock;

/// Startup CPU estimate shared by constructor, activation, and pool allocation.
static AVAILABLE_CALCULATION_WORKERS: OnceLock<usize> = OnceLock::new();

/// Detect process-available logical CPUs once; unknown parallelism admits only serial work.
pub fn available_calculation_workers() -> usize {
    *AVAILABLE_CALCULATION_WORKERS.get_or_init(|| {
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
    })
}

/// Requested manual count and the CPU ceiling against which it was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalculationWorkerCountError {
    /// Requested worker count; zero never means automatic selection.
    pub count: usize,
    /// Process-available CPU count captured at validation.
    pub maximum: usize,
}

impl fmt::Display for CalculationWorkerCountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "calculation workers {} must be from 1 to {} (available logical CPUs)",
            self.count, self.maximum
        )
    }
}

impl std::error::Error for CalculationWorkerCountError {}

/// Validate against a supplied CPU count so tests do not depend on the runner's hardware.
fn validate_with_maximum(count: usize, maximum: usize) -> Result<(), CalculationWorkerCountError> {
    if count == 0 || count > maximum {
        return Err(CalculationWorkerCountError { count, maximum });
    }
    Ok(())
}

/// Enforce the same detected CPU ceiling at each native admission boundary.
pub fn validate_calculation_workers(count: usize) -> Result<(), CalculationWorkerCountError> {
    validate_with_maximum(count, available_calculation_workers())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_range_tracks_supplied_cpu_counts() {
        for maximum in [1, 2, 8, 16, 32, 64] {
            for count in 1..=maximum {
                assert!(validate_with_maximum(count, maximum).is_ok());
            }
            for count in [0, maximum + 1, usize::MAX] {
                let error = validate_with_maximum(count, maximum).unwrap_err();
                assert_eq!(error, CalculationWorkerCountError { count, maximum });
                assert!(error.to_string().contains(&format!("1 to {maximum}")));
            }
        }
    }

    #[test]
    fn detected_limit_is_positive_and_stable_across_threads() {
        let maximum = available_calculation_workers();
        assert!(maximum > 0);
        assert!(validate_calculation_workers(maximum).is_ok());
        assert_eq!(
            std::thread::spawn(available_calculation_workers)
                .join()
                .unwrap(),
            maximum
        );
        assert_eq!(
            validate_calculation_workers(0).unwrap_err().maximum,
            maximum
        );
    }
}
