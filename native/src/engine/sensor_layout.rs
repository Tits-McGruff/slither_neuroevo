//! Sensor-v3 labels, offsets, and input-size contract.
//!
//! The browser and Protocol 2 trainer currently consume one fixed v3 ordering:
//! nineteen scalar values followed by equally sized food, body-hazard, wall,
//! and other-head channels. Rust validates the owner-approved 8--32 bin range
//! instead of silently accepting an incompatible graph input size.

use std::error::Error;
use std::fmt;

/// Only supported sensor-layout version.
pub const SENSOR_LAYOUT_VERSION: &str = "v3";
/// Number of non-binned values at the start of every v3 observation.
pub const SENSOR_SCALAR_COUNT: usize = 19;
/// Number of equally sized angular channels in every v3 observation.
pub const SENSOR_CHANNEL_COUNT: usize = 4;
/// Smallest owner-supported angular bin count.
pub const MIN_SENSOR_BINS: usize = 8;
/// Largest owner-supported angular bin count.
pub const MAX_SENSOR_BINS: usize = 32;

/// Exact scalar prefix shared with `src/protocol/sensors.ts`.
pub const SENSOR_SCALAR_LABELS: [&str; SENSOR_SCALAR_COUNT] = [
    "heading_sin",
    "heading_cos",
    "size_norm",
    "boost_margin",
    "points_pct",
    "speed_norm",
    "boost_state",
    "points_norm",
    "points_delta_norm",
    "length_norm",
    "boost_points_frac",
    "boost_cost_norm",
    "wall_dist_norm",
    "nearest_food_dist_norm",
    "nearest_food_dir_sin",
    "nearest_food_dir_cos",
    "nearest_body_dist_norm",
    "nearest_head_dist_norm",
    "age_norm",
];

/// Exact public meaning of the score-delta value.
pub const POINTS_DELTA_SENSOR_DESCRIPTION: &str = "Score change accumulated since this snake's previous delivered sensor sample, or since construction for its first sample; unsampled control intervals accumulate. The value is divided by 10 and clamped to [-1, 1].";

/// Starting offsets for the four binned channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorOffsets {
    /// Food-density channel offset.
    pub food: usize,
    /// Body-hazard-clearance channel offset.
    pub hazard: usize,
    /// Wall-clearance channel offset.
    pub wall: usize,
    /// Other-head-clearance channel offset.
    pub head: usize,
}

/// Validated sensor-v3 layout used by graphs, observations, and handshakes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensorLayout {
    /// Number of angular bins in each binned channel.
    pub bins: usize,
    /// Number of scalar prefix values.
    pub scalar_count: usize,
    /// Number of binned channels.
    pub channel_count: usize,
    /// Total neural input length.
    pub input_size: usize,
    /// Starting offsets for each binned channel.
    pub offsets: SensorOffsets,
    /// Exact published label order.
    pub order: Box<[String]>,
}

impl SensorLayout {
    /// Construct and validate one sensor-v3 layout.
    pub fn new(bins: usize) -> Result<Self, SensorLayoutError> {
        if !(MIN_SENSOR_BINS..=MAX_SENSOR_BINS).contains(&bins) {
            return Err(SensorLayoutError::UnsupportedBinCount { bins });
        }
        let binned = bins
            .checked_mul(SENSOR_CHANNEL_COUNT)
            .ok_or(SensorLayoutError::SizeOverflow)?;
        let input_size = SENSOR_SCALAR_COUNT
            .checked_add(binned)
            .ok_or(SensorLayoutError::SizeOverflow)?;
        let offsets = SensorOffsets {
            food: SENSOR_SCALAR_COUNT,
            hazard: SENSOR_SCALAR_COUNT + bins,
            wall: SENSOR_SCALAR_COUNT + bins * 2,
            head: SENSOR_SCALAR_COUNT + bins * 3,
        };
        let mut order = Vec::with_capacity(input_size);
        order.extend(SENSOR_SCALAR_LABELS.iter().map(|label| (*label).to_owned()));
        for prefix in ["food", "hazard", "wall", "head"] {
            order.extend((0..bins).map(|index| format!("{prefix}_{index}")));
        }
        debug_assert_eq!(order.len(), input_size);
        Ok(Self {
            bins,
            scalar_count: SENSOR_SCALAR_COUNT,
            channel_count: SENSOR_CHANNEL_COUNT,
            input_size,
            offsets,
            order: order.into_boxed_slice(),
        })
    }

    /// Build the small network-handshake view without exposing engine state.
    #[must_use]
    pub fn specification(&self) -> SensorSpecification {
        SensorSpecification {
            sensor_count: self.input_size,
            order: self.order.clone(),
            layout_version: SENSOR_LAYOUT_VERSION,
        }
    }
}

/// Small sensor metadata payload suitable for a welcome message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensorSpecification {
    /// Total number of values in one observation.
    pub sensor_count: usize,
    /// Exact public label order.
    pub order: Box<[String]>,
    /// Stable version identifier.
    pub layout_version: &'static str,
}

/// Sensor-layout validation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorLayoutError {
    /// Requested bins fall outside the owner-supported v3 range.
    UnsupportedBinCount { bins: usize },
    /// Checked input-size arithmetic overflowed.
    SizeOverflow,
}

impl fmt::Display for SensorLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedBinCount { bins } => write!(
                formatter,
                "sensor-v3 bins must be in {MIN_SENSOR_BINS}..={MAX_SENSOR_BINS}; received {bins}"
            ),
            Self::SizeOverflow => write!(formatter, "sensor-v3 input-size arithmetic overflowed"),
        }
    }
}

impl Error for SensorLayoutError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_the_typescript_v3_contract() {
        let layout = SensorLayout::new(16).expect("default layout should validate");
        assert_eq!(layout.scalar_count, 19);
        assert_eq!(layout.channel_count, 4);
        assert_eq!(layout.input_size, 83);
        assert_eq!(layout.offsets.food, 19);
        assert_eq!(layout.offsets.hazard, 35);
        assert_eq!(layout.offsets.wall, 51);
        assert_eq!(layout.offsets.head, 67);
        assert_eq!(&layout.order[..19], SENSOR_SCALAR_LABELS);
        assert_eq!(layout.order[19], "food_0");
        assert_eq!(layout.order[34], "food_15");
        assert_eq!(layout.order[35], "hazard_0");
        assert_eq!(layout.order[82], "head_15");
        assert_eq!(layout.specification().layout_version, "v3");
    }

    #[test]
    fn every_owner_supported_bin_count_has_the_expected_length() {
        for bins in MIN_SENSOR_BINS..=MAX_SENSOR_BINS {
            let layout = SensorLayout::new(bins).expect("supported bins should validate");
            assert_eq!(layout.input_size, 19 + 4 * bins);
            assert_eq!(layout.order.len(), layout.input_size);
        }
    }

    #[test]
    fn bins_outside_the_owner_supported_range_fail_clearly() {
        assert_eq!(
            SensorLayout::new(7),
            Err(SensorLayoutError::UnsupportedBinCount { bins: 7 })
        );
        assert_eq!(
            SensorLayout::new(33),
            Err(SensorLayoutError::UnsupportedBinCount { bins: 33 })
        );
    }
}
