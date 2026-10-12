//! Structural sensor identities, separated from numeric run environments.
//! V4 is an explicitly unfrozen qualification candidate. A profile, meaning,
//! geometry or normalization change changes its digest even at equal width.

use super::sensor_layout::{SensorLayout, MAX_SENSOR_BINS, MIN_SENSOR_BINS, SENSOR_SCALAR_LABELS};
use super::state::{NormalizedSetting, NormalizedSettingValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::{Display, Formatter};

/// Structural profile; v3 remains the frozen historical observation contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SensorProfile {
    V3,
    Basic,
    Motion,
}

/// Distinguishes qualified released contracts from pre-freeze prototypes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReleaseStatus {
    Frozen,
    Candidate,
}

/// One scalar's exact structural label and formula/encoding meaning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScalarMeaning {
    label: String,
    meaning: String,
}

/// One complete angular channel, in flattened channel-major order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SensorChannelMeaning {
    label: String,
    offset: usize,
    meaning: String,
    absent_value: f32,
}
impl SensorChannelMeaning {
    pub fn label(&self) -> &str {
        &self.label
    }
    pub const fn offset(&self) -> usize {
        self.offset
    }
    pub fn meaning(&self) -> &str {
        &self.meaning
    }
    pub const fn absent_value(&self) -> f32 {
        self.absent_value
    }
}

/// Canonical metadata, built once outside sensing hot paths. Environment
/// ranges/costs/detail limits and cadence/life deployment rules are excluded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SensorContractIdentity {
    schema_version: u32,
    layout_version: u32,
    profile: SensorProfile,
    bins: usize,
    input_size: usize,
    scalar_count: usize,
    normalization_revision: u32,
    geometry_revision: u32,
    release_status: ReleaseStatus,
    scalar_encoding: String,
    angular_geometry: String,
    validity_encoding: String,
    scalar_semantics: Box<[ScalarMeaning]>,
    channels: Box<[SensorChannelMeaning]>,
    order: Box<[String]>,
}

/// Static cold-construction table; semantic strings are copied only once.
struct ContractDefinition {
    version: u32,
    labels: &'static [&'static str],
    scalars: &'static [&'static str],
    channels: &'static [(&'static str, &'static str, f32)],
}

impl SensorContractIdentity {
    /// Construct the frozen v3 or initial v4 Basic/Motion qualification shape.
    pub fn new(profile: SensorProfile, bins: usize) -> Result<Self, SensorContractError> {
        if !(MIN_SENSOR_BINS..=MAX_SENSOR_BINS).contains(&bins) {
            return Err(SensorContractError::Bins);
        }
        let ContractDefinition {
            version,
            labels,
            scalars,
            channels,
        } = match profile {
            SensorProfile::V3 => ContractDefinition {
                version: 3,
                labels: &SENSOR_SCALAR_LABELS,
                scalars: &V3_SCALAR_MEANINGS,
                channels: &V3_CHANNELS,
            },
            SensorProfile::Basic => ContractDefinition {
                version: 4,
                labels: &V4_SCALAR_LABELS,
                scalars: &V4_SCALAR_MEANINGS,
                channels: &V4_CHANNELS[..6],
            },
            SensorProfile::Motion => ContractDefinition {
                version: 4,
                labels: &V4_SCALAR_LABELS,
                scalars: &V4_SCALAR_MEANINGS,
                channels: &V4_CHANNELS,
            },
        };
        let scalar_count = labels.len();
        let input_size = scalar_count + channels.len() * bins;
        let scalar_semantics: Vec<_> = labels
            .iter()
            .zip(scalars)
            .map(|(label, meaning)| ScalarMeaning {
                label: (*label).into(),
                meaning: (*meaning).into(),
            })
            .collect();
        let mut order: Vec<_> = labels.iter().map(|label| (*label).to_owned()).collect();
        let mut channel_semantics = Vec::with_capacity(channels.len());
        for (index, (label, meaning, absent)) in channels.iter().enumerate() {
            channel_semantics.push(SensorChannelMeaning {
                label: (*label).into(),
                offset: scalar_count + index * bins,
                meaning: (*meaning).into(),
                absent_value: *absent,
            });
            order.extend((0..bins).map(|bin| format!("{label}_{bin}")));
        }
        Ok(Self {
            schema_version: 1, layout_version: version, profile, bins, input_size, scalar_count,
            normalization_revision: if version == 3 { 1 } else { 0 },
            geometry_revision: if version == 3 { 1 } else { 0 },
            release_status: if version == 3 { ReleaseStatus::Frozen } else { ReleaseStatus::Candidate },
            scalar_encoding: "bipolar(x)=2*clamp(x,0,1)-1; arithmetic in f64, final finite Float32; signed zero preserved in numeric samples".into(),
            angular_geometry: "channel-major; centers=-pi+2*pi*i/B relative to heading; centered half-open sectors; +pi wraps to -pi".into(),
            validity_encoding: if version == 3 { "v3 body query cap forces zero raw clearance; no explicit validity channels" } else { "all finite Float32; booleans/presence -1/+1; incomplete body/ray clearance -1 with scalar saturation +1; nearest motion fields zero when presence -1; urgency/density absent zero" }.into(),
            scalar_semantics: scalar_semantics.into_boxed_slice(), channels: channel_semantics.into_boxed_slice(), order: order.into_boxed_slice(),
        })
    }

    /// Reconstruct the recorded structural contract; legacy v3 has no extension.
    /// V4 always carries an exact digest, so a same-width or revised meaning rejects.
    pub fn from_settings(settings: &[NormalizedSetting]) -> Result<Self, SensorContractError> {
        if settings.windows(2).any(|pair| pair[0].path >= pair[1].path) {
            return Err(SensorContractError::Settings);
        }
        let value = |path: &str| {
            settings
                .binary_search_by(|entry| entry.path.as_str().cmp(path))
                .ok()
                .map(|index| &settings[index].value)
        };
        let integer = |path| match value(path) {
            Some(NormalizedSettingValue::Integer(value)) => Ok(*value),
            _ => Err(SensorContractError::Settings),
        };
        let version = integer("sense.layoutVersion")?;
        if integer("brain.sensorVersion")? != version {
            return Err(SensorContractError::Settings);
        }
        let bins =
            usize::try_from(integer("sense.bubbleBins")?).map_err(|_| SensorContractError::Bins)?;
        let profile = match version {
            3 => SensorProfile::V3,
            4 if integer("sense.contractVersion")? == 1 => match integer("sense.profile")? {
                0 => SensorProfile::Basic,
                1 => SensorProfile::Motion,
                _ => return Err(SensorContractError::Settings),
            },
            _ => return Err(SensorContractError::Settings),
        };
        let identity = Self::new(profile, bins)?;
        if version == 3 {
            if [
                "sense.contractVersion",
                "sense.profile",
                "sense.contractSha256",
            ]
            .iter()
            .any(|path| value(path).is_some())
            {
                return Err(SensorContractError::Settings);
            }
        } else if !matches!(value("sense.contractSha256"), Some(NormalizedSettingValue::Text(digest)) if *digest == identity.sha256_hex()?)
        {
            return Err(SensorContractError::StructuralMismatch);
        }
        Ok(identity)
    }

    /// Stamp a newly constructed v4 candidate; never migrate an existing owner record.
    pub fn record_v4_settings(
        &self,
        settings: &mut Vec<NormalizedSetting>,
    ) -> Result<(), SensorContractError> {
        let profile = match self.profile {
            SensorProfile::Basic => 0,
            SensorProfile::Motion => 1,
            SensorProfile::V3 => return Err(SensorContractError::Settings),
        };
        let digest = self.sha256_hex()?;
        for path in [
            "brain.sensorVersion",
            "sense.layoutVersion",
            "sense.bubbleBins",
        ] {
            if settings.iter().filter(|entry| entry.path == path).count() != 1 {
                return Err(SensorContractError::Settings);
            }
        }
        for entry in settings.iter_mut() {
            match entry.path.as_str() {
                "brain.sensorVersion" | "sense.layoutVersion" => {
                    entry.value = NormalizedSettingValue::Integer(4)
                }
                "sense.bubbleBins" => {
                    entry.value = NormalizedSettingValue::Integer(self.bins as i64)
                }
                _ => {}
            }
        }
        settings.retain(|entry| {
            ![
                "sense.contractVersion",
                "sense.profile",
                "sense.contractSha256",
            ]
            .contains(&entry.path.as_str())
        });
        settings.extend([
            NormalizedSetting {
                path: "sense.contractVersion".into(),
                value: NormalizedSettingValue::Integer(1),
            },
            NormalizedSetting {
                path: "sense.profile".into(),
                value: NormalizedSettingValue::Integer(profile),
            },
            NormalizedSetting {
                path: "sense.contractSha256".into(),
                value: NormalizedSettingValue::Text(digest),
            },
        ]);
        settings.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        Ok(())
    }

    pub const fn layout_version(&self) -> u32 {
        self.layout_version
    }

    /// Lowercase canonical digest carried by settings, saved brains and handshakes.
    pub fn sha256_hex(&self) -> Result<String, SensorContractError> {
        use std::fmt::Write;
        let mut text = String::with_capacity(64);
        for byte in self.sha256()? {
            write!(&mut text, "{byte:02x}").map_err(|_| SensorContractError::Encoding)?;
        }
        Ok(text)
    }

    pub const fn profile(&self) -> SensorProfile {
        self.profile
    }
    pub const fn bins(&self) -> usize {
        self.bins
    }
    pub const fn input_size(&self) -> usize {
        self.input_size
    }
    pub const fn scalar_count(&self) -> usize {
        self.scalar_count
    }
    pub fn channels(&self) -> &[SensorChannelMeaning] {
        &self.channels
    }
    pub fn order(&self) -> &[String] {
        &self.order
    }
    pub fn is_frozen(&self) -> bool {
        self.release_status == ReleaseStatus::Frozen
    }

    /// Stable struct-key order, UTF-8 JSON and domain-separated SHA256.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, SensorContractError> {
        serde_json::to_vec(self).map_err(|_| SensorContractError::Encoding)
    }
    pub fn sha256(&self) -> Result<[u8; 32], SensorContractError> {
        let mut hash = Sha256::new();
        hash.update(b"slither.sensor-contract.v1\0");
        hash.update(self.canonical_bytes()?);
        Ok(hash.finalize().into())
    }

    /// Bounded strict ingress rejects unsupported revisions or equal-width
    /// reinterpretations; accepting a width alone would lose compatibility.
    pub fn decode_supported(bytes: &[u8]) -> Result<Self, SensorContractError> {
        if bytes.len() > 65_536 {
            return Err(SensorContractError::Encoding);
        }
        let supplied: Self =
            serde_json::from_slice(bytes).map_err(|_| SensorContractError::Encoding)?;
        let expected = Self::new(supplied.profile, supplied.bins)?;
        if supplied.canonical_bytes()? != expected.canonical_bytes()? {
            return Err(SensorContractError::StructuralMismatch);
        }
        Ok(expected)
    }
}

impl SensorLayout {
    /// Existing v3 layouts acquire structural metadata without changing bytes,
    /// labels, default settings or the historical native sensor fixtures.
    pub fn contract_identity(&self) -> Result<SensorContractIdentity, SensorContractError> {
        let identity = SensorContractIdentity::new(self.profile, self.bins)?;
        let expected = match self.profile {
            SensorProfile::V3 => SensorLayout::new(self.bins),
            profile => SensorLayout::v4(profile, self.bins),
        }
        .map_err(|_| SensorContractError::StructuralMismatch)?;
        if *self != expected {
            return Err(SensorContractError::StructuralMismatch);
        }
        Ok(identity)
    }
}

/// Exact scalar labels for the initial pre-freeze v4 candidate.
pub const V4_SCALAR_LABELS: [&str; 24] = [
    "heading_sin",
    "heading_cos",
    "tactical_size",
    "radius_norm",
    "speed_norm",
    "spendable_points_norm",
    "boost_seconds_norm",
    "boost_cost_norm",
    "boost_eligible",
    "actual_boost",
    "held_boost_request",
    "held_turn",
    "previous_applied_turn",
    "angular_velocity_norm",
    "speed_change_norm",
    "delivered_wallet_delta_norm",
    "current_life_points_norm",
    "assessment_points_norm",
    "life_age_norm",
    "generation_remaining_fraction",
    "life_budget_used_norm",
    "first_sample_of_life",
    "pellet_query_saturated",
    "body_or_ray_query_saturated",
];
const V4_SCALAR_MEANINGS: [&str; 24] = [
    "sin(absolute heading)", "cos(absolute heading)",
    "bipolar(d/(d+40)); d=max(bodyLength-configuredStartLength,0); fraction also scales sensor ranges",
    "bipolar(radius/configuredMaximumRadius)", "bipolar(speed/configuredBoostSpeed)",
    "bipolar(spendablePoints/(spendablePoints+140))",
    "bipolar(T/(T+2)); T=max(spendablePoints-minBoostPoints,0)/actualEffectiveCost; zero cost=+1",
    "bipolar(actualEffectiveCost/(actualEffectiveCost+7)); physical movement size curve",
    "actual movement eligibility; zero positive spend impossible => -1; false=-1 true=+1",
    "actual boost false=-1 true=+1", "held boost request false=-1 true=+1",
    "clamp(held turn,-1,+1)", "clamp(previous applied turn,-1,+1)",
    "clamp(completed fixed-step heading change/fixedDt/configuredTurnRate,-1,+1)",
    "clamp(completed fixed-step speed change/configuredBoostSpeed,-1,+1)",
    "delta/(abs(delta)+10); delta=wallet minus previous delivered wallet; probes do not consume",
    "bipolar(lifePoints/(lifePoints+200)); evolved ledger or ordinary physical points",
    "bipolar(selectedAssessment/(selectedAssessment+1000)); no population-relative normalization",
    "bipolar(age/(age+60))", "clamp(1-generationElapsed/configuredDuration,0,1)",
    "evolved off=-1; finite M=Float32(-livesStarted/M); unlimited=+1; baseline/external/retained=0",
    "+1 until accepted external or consumed internal observation in this physical life, then -1; takeover preserves flag",
    "pellet query saturated false=-1 true=+1", "body or capsule-ray query incomplete false=-1 true=+1",
];
const V4_CHANNELS: [(&str, &str, f32); 15] = [
    ("near_food", "ring[0,near]; sum((1-distance/far)*clamp(value/foodValue,0,6)); bipolar(strength/(strength+foodK))", -1.0),
    ("far_food", "ring(near,far]; same bounded distance/value weight and saturation as near ring", -1.0),
    ("body_sector", "bipolar(min other-body segment clearance/near); collision radii scaled by configured hitScale; capped/incomplete raw clearance zero", 1.0),
    ("body_ray", "bipolar(first capsule intersection distance/near); ray through sector center; sum of collision radii with hitScale; tangency closed; incomplete=-1", 1.0),
    ("wall_ray", "bipolar(circular boundary ray clearance/near); arena radius minus observer collision radius", 1.0),
    ("head_clearance", "bipolar(min other-head disc clearance/near) in current relative bearing sector", 1.0),
    ("head_presence", "nearest head selected by minimum clearance then full internal ID; false=-1 true=+1", -1.0),
    ("head_bearing_residual", "clamp(wrapped nearest relative bearing minus sector center divided by pi/B,-1,+1)", 0.0),
    ("head_relative_length", "clamp(log2(nearest bodyLength/observer bodyLength)/4,-1,+1)", 0.0),
    ("head_heading_sin", "sin(nearest heading minus observer heading)", 0.0),
    ("head_heading_cos", "cos(nearest heading minus observer heading)", 0.0),
    ("head_velocity_forward", "clamp((nearestVelocity-observerVelocity) dot observerForward/(2*configuredBoostSpeed),-1,+1)", 0.0),
    ("head_velocity_lateral", "clamp((nearestVelocity-observerVelocity) dot observerLeft/(2*configuredBoostSpeed),-1,+1)", 0.0),
    ("head_collision_urgency", "max(1-TTC/2) over all relevant held-motion disc intersections within 2s; sector is relative contact bearing; no collision=0", 0.0),
    ("head_density", "count/(count+4) for relevant heads in current bearing sector", 0.0),
];
const V3_SCALAR_MEANINGS: [&str; 19] = [
    "sin(absolute heading)",
    "cos(absolute heading)",
    "bipolar(clamp((length-start)/(maximum-start),0,1))",
    "clamp((wallet-minBoost)/max(minBoost,1e-6),-1,+1)",
    "clamp(2*ln(1+selectedAssessment)/ln(1+max(generationPeakSelectedAssessment,0.001))-1,-1,+1)",
    "bipolar(speed/configuredBoostSpeed)",
    "actual boost false=-1 true=+1",
    "clamp(2*selectedAssessment/max(generationPeakSelectedAssessment,1)-1,-1,+1)",
    "clamp((wallet-previousDeliveredWallet)/10,-1,+1); probes do not consume",
    "bipolar(length/configuredMaximumLength)",
    "clamp((wallet-minBoost)/max(minBoost,1),-1,+1)",
    "bipolar(actualEffectiveCost/max(3*baseCost,1))",
    "bipolar((worldRadius-centerDistance)/worldRadius)",
    "bipolar(1-nearestFoodDistance/far)",
    "sin(nearest relative food bearing), absent=0",
    "cos(nearest relative food bearing), absent=0",
    "bipolar(1-nearestOtherBodyRawClearance/near)",
    "bipolar(1-nearestOtherHeadRawClearance/near)",
    "bipolar(age/max(configuredDuration,1))",
];
const V3_CHANNELS: [(&str, &str, f32); 4] = [
    ("food", "sum((1-distance/far)*clamp(value/foodValue,0,6)); bipolar(strength/(strength+foodK))", -1.0),
    ("hazard", "bipolar(min other-body segment clearance/near); complete segment geometry; hitScale; cap saturates zero raw clearance", 1.0),
    ("wall", "bipolar((center boundary ray distance-observerRadius)/near)", 1.0),
    ("head", "bipolar(min other-head clearance/near); hitScale", 1.0),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SensorContractError {
    Bins,
    Encoding,
    StructuralMismatch,
    /// Incomplete, noncanonical or inconsistent structural settings.
    Settings,
}
impl Display for SensorContractError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid sensor contract: {self:?}")
    }
}
impl std::error::Error for SensorContractError {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_wire_fixtures_are_exact_native_canonical_contracts() {
        let records: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("fixtures/sensor-contract-shapes.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(records.as_array().unwrap().len(), 7);
        for record in records.as_array().unwrap() {
            let profile: SensorProfile = serde_json::from_value(record["profile"].clone()).unwrap();
            let contract =
                SensorContractIdentity::new(profile, record["bins"].as_u64().unwrap() as usize)
                    .unwrap();
            assert_eq!(
                record["canonicalJson"],
                String::from_utf8(contract.canonical_bytes().unwrap()).unwrap()
            );
            assert_eq!(record["sha256"], contract.sha256_hex().unwrap());
        }
    }

    #[test]
    fn persisted_settings_bind_profiles_revisions_and_same_width_meanings() {
        use crate::engine::step_config::typescript_default_settings;
        let mut basic = typescript_default_settings(12, 2);
        let identity = SensorContractIdentity::new(SensorProfile::Basic, 20).unwrap();
        identity.record_v4_settings(&mut basic).unwrap();
        assert_eq!(
            SensorContractIdentity::from_settings(&basic).unwrap(),
            identity
        );
        for path in [
            "sense.contractVersion",
            "sense.profile",
            "sense.contractSha256",
        ] {
            let mut missing = basic.clone();
            missing.retain(|entry| entry.path != path);
            assert!(SensorContractIdentity::from_settings(&missing).is_err());
        }
        let motion = SensorContractIdentity::new(SensorProfile::Motion, 8).unwrap();
        assert_eq!(motion.input_size(), identity.input_size());
        let mut reinterpreted = basic.clone();
        for entry in &mut reinterpreted {
            match entry.path.as_str() {
                "sense.profile" => entry.value = NormalizedSettingValue::Integer(1),
                "sense.bubbleBins" => entry.value = NormalizedSettingValue::Integer(8),
                _ => {}
            }
        }
        assert_eq!(
            SensorContractIdentity::from_settings(&reinterpreted),
            Err(SensorContractError::StructuralMismatch)
        );
        motion.record_v4_settings(&mut reinterpreted).unwrap();
        assert_eq!(
            SensorContractIdentity::from_settings(&reinterpreted).unwrap(),
            motion
        );
        let legacy = typescript_default_settings(12, 2);
        assert_eq!(
            SensorContractIdentity::from_settings(&legacy)
                .unwrap()
                .profile(),
            SensorProfile::V3
        );
        let mut partial_legacy = legacy;
        partial_legacy.push(NormalizedSetting {
            path: "sense.profile".into(),
            value: NormalizedSettingValue::Integer(1),
        });
        partial_legacy.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        assert!(SensorContractIdentity::from_settings(&partial_legacy).is_err());
    }

    #[test]
    fn layouts_have_exact_labels_offsets_and_ceiling_widths() {
        for bins in 8..=32 {
            for (profile, scalar, channel) in [
                (SensorProfile::V3, 19, 4),
                (SensorProfile::Basic, 24, 6),
                (SensorProfile::Motion, 24, 15),
            ] {
                let identity = SensorContractIdentity::new(profile, bins).unwrap();
                assert_eq!(identity.input_size(), scalar + channel * bins);
                assert_eq!(identity.order().len(), identity.input_size());
                assert!(identity.input_size() <= 512);
                for (index, description) in identity.channels().iter().enumerate() {
                    assert_eq!(description.offset(), scalar + index * bins);
                    assert_eq!(
                        identity.order()[description.offset()],
                        format!("{}_0", description.label())
                    );
                }
            }
        }
        let basic = SensorContractIdentity::new(SensorProfile::Basic, 16).unwrap();
        assert_eq!(basic.input_size(), 120);
        assert_eq!(
            SensorContractIdentity::new(SensorProfile::Motion, 32)
                .unwrap()
                .input_size(),
            504
        );
        assert_eq!(&basic.order()[..24], V4_SCALAR_LABELS);
        assert!(!basic.is_frozen());
    }
    #[test]
    fn v3_identity_preserves_every_existing_native_layout() {
        for bins in 8..=32 {
            let layout = SensorLayout::new(bins).unwrap();
            let identity = layout.contract_identity().unwrap();
            assert!(identity.is_frozen());
            assert_eq!(identity.order(), layout.order.as_ref());
            assert_eq!(identity.input_size(), layout.input_size);
        }
    }
    #[test]
    fn equal_width_profiles_have_different_structural_identity() {
        let basic = SensorContractIdentity::new(SensorProfile::Basic, 20).unwrap();
        let motion = SensorContractIdentity::new(SensorProfile::Motion, 8).unwrap();
        assert_eq!(basic.input_size(), motion.input_size());
        assert_ne!(basic.sha256().unwrap(), motion.sha256().unwrap());
    }
    #[test]
    fn strict_ingress_rejects_meaning_geometry_revision_and_same_width_reinterpretation() {
        let original = SensorContractIdentity::new(SensorProfile::Motion, 16).unwrap();
        let bytes = original.canonical_bytes().unwrap();
        assert_eq!(
            SensorContractIdentity::decode_supported(&bytes).unwrap(),
            original
        );
        for selector in 0..6 {
            let mut changed = original.clone();
            match selector {
                0 => changed.channels[0].meaning.push_str("changed"),
                1 => changed.channels[3].absent_value = 0.0,
                2 => changed.normalization_revision += 1,
                3 => changed.angular_geometry.push_str("changed"),
                4 => changed.order.swap(0, 1),
                _ => changed.release_status = ReleaseStatus::Frozen,
            }
            assert_ne!(changed.sha256().unwrap(), original.sha256().unwrap());
            assert_eq!(
                SensorContractIdentity::decode_supported(&changed.canonical_bytes().unwrap()),
                Err(SensorContractError::StructuralMismatch)
            );
        }
        assert_eq!(
            SensorContractIdentity::decode_supported(&vec![b' '; 65_537]),
            Err(SensorContractError::Encoding)
        );
        assert_eq!(
            SensorContractIdentity::new(SensorProfile::Basic, 7),
            Err(SensorContractError::Bins)
        );
        assert_eq!(
            SensorContractIdentity::new(SensorProfile::Motion, 33),
            Err(SensorContractError::Bins)
        );
    }
}
