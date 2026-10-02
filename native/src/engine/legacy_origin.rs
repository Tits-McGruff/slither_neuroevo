//! Bounded source facts retained when an older population begins a new run.

use serde::{Deserialize, Serialize};

/// Maximum encoded origin record, independent of population size.
pub const MAX_LEGACY_ORIGIN_BYTES: usize = 4096;

/// Representation actually consumed by the compatibility reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyPopulationSource {
    /// Per-slot TypeScript checkpoint rows.
    #[serde(rename = "typescript-v2")]
    TypeScriptV2,
    /// Length-prefixed genome JSON inside a SQLite gzip BLOB.
    #[serde(rename = "legacy-gzip")]
    Gzip,
    /// Population JSON inside a SQLite parent row.
    #[serde(rename = "legacy-json")]
    EmbeddedJson,
    /// An original browser population JSON upload.
    #[serde(rename = "browser-json")]
    BrowserJson,
}

/// Explicit completeness of the source, separate from the new run's own saved state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyPopulationCompleteness {
    /// The compatibility reader imports population/graph/settings only.
    #[serde(rename = "population-only")]
    PopulationOnly,
}

/// Immutable bounded source provenance carried by every checkpoint of the converted lineage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyPopulationOrigin {
    /// Origin contract version.
    pub version: u32,
    /// Actual source representation.
    pub source_format: LegacyPopulationSource,
    /// Positive original SQLite row identity, absent for a browser file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_snapshot_id: Option<u64>,
    /// Original lineage when the source supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_run_id: Option<String>,
    /// Exact positive source generation, retained as a fixed-width hexadecimal scalar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_generation: Option<String>,
    /// Original seed; it does not seed the new run's missing continuation state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_seed: Option<u32>,
    /// Original uploaded file digest when one complete source file was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
    /// Honest source completeness.
    pub completeness: LegacyPopulationCompleteness,
    /// Always false: generated replacement state cannot make the old source exact.
    pub exact_continuation: bool,
}

impl LegacyPopulationOrigin {
    /// Create the population-only record; admission validates all optional source facts.
    pub fn new(source_format: LegacyPopulationSource) -> Self {
        Self {
            version: 1,
            source_format,
            source_snapshot_id: None,
            source_run_id: None,
            source_generation: None,
            source_seed: None,
            source_sha256: None,
            completeness: LegacyPopulationCompleteness::PopulationOnly,
            exact_continuation: false,
        }
    }

    /// Reject impossible source claims before admitting or restoring authority.
    pub fn validate(&self) -> Result<(), &'static str> {
        let browser = self.source_format == LegacyPopulationSource::BrowserJson;
        if self.version != 1 || self.exact_continuation {
            return Err("unsupported or exact legacy population origin");
        }
        if browser != self.source_snapshot_id.is_none()
            || self
                .source_snapshot_id
                .is_some_and(|id| id == 0 || id > 9_007_199_254_740_991)
        {
            return Err("legacy origin snapshot identity disagrees with its source format");
        }
        if self
            .source_run_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.contains('\0'))
        {
            return Err("legacy origin lineage is empty, oversized or contains NUL");
        }
        if self.source_generation.as_ref().is_some_and(|value| {
            value.len() != 16
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || u64::from_str_radix(value, 16).map_or(true, |generation| generation == 0)
        }) {
            return Err("legacy origin generation is not a positive canonical u64");
        }
        if self.source_sha256.as_ref().is_some_and(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err("legacy origin source digest is not lowercase SHA-256");
        }
        Ok(())
    }

    /// Encode only this bounded record; population bytes never enter JSON here.
    pub fn encode(&self) -> Result<String, String> {
        self.validate().map_err(str::to_owned)?;
        let encoded = serde_json::to_string(self).map_err(|error| error.to_string())?;
        if encoded.len() > MAX_LEGACY_ORIGIN_BYTES {
            return Err("legacy origin exceeds its bounded metadata budget".into());
        }
        Ok(encoded)
    }

    /// Decode one bounded record and independently validate its meaning.
    pub fn decode(encoded: &str) -> Result<Self, String> {
        if encoded.len() > MAX_LEGACY_ORIGIN_BYTES {
            return Err("legacy origin exceeds its bounded metadata budget".into());
        }
        let value: Self = serde_json::from_str(encoded).map_err(|error| error.to_string())?;
        value.validate().map_err(str::to_owned)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unknown fields and invented exact continuation must not become trusted source facts.
    #[test]
    fn legacy_origin_rejects_invalid_claims_and_oversized_records() {
        let base = LegacyPopulationOrigin::new(LegacyPopulationSource::BrowserJson);
        let mut value = serde_json::to_value(&base).unwrap();
        for (key, invalid) in [
            ("version", serde_json::json!(2)),
            ("exactContinuation", serde_json::json!(true)),
            ("sourceSnapshotId", serde_json::json!(1)),
            ("sourceGeneration", serde_json::json!("0000000000000000")),
            ("sourceGeneration", serde_json::json!("000000000000000A")),
            ("sourceSeed", serde_json::json!(4_294_967_296_u64)),
            ("sourceRunId", serde_json::json!("bad\u{0000}lineage")),
            ("sourceSha256", serde_json::json!("A".repeat(64))),
            ("invented", serde_json::json!(true)),
        ] {
            let mut invalid_value = value.clone();
            invalid_value[key] = invalid;
            assert!(
                LegacyPopulationOrigin::decode(&invalid_value.to_string()).is_err(),
                "{key}"
            );
        }
        value["sourceFormat"] = serde_json::json!("typescript-v2");
        assert!(LegacyPopulationOrigin::decode(&value.to_string()).is_err());
        assert!(LegacyPopulationOrigin::decode(&" ".repeat(MAX_LEGACY_ORIGIN_BYTES + 1)).is_err());
    }

    /// Preserve optional facts without confusing the original seed with the new authority.
    #[test]
    fn legacy_origin_round_trips_browser_and_database_facts() {
        for source in [
            LegacyPopulationSource::BrowserJson,
            LegacyPopulationSource::TypeScriptV2,
            LegacyPopulationSource::Gzip,
            LegacyPopulationSource::EmbeddedJson,
        ] {
            let mut origin = LegacyPopulationOrigin::new(source);
            if source != LegacyPopulationSource::BrowserJson {
                origin.source_snapshot_id = Some(17);
            }
            origin.source_run_id = Some("old-run".into());
            origin.source_generation = Some("ffffffffffffffff".into());
            origin.source_seed = Some(u32::MAX);
            origin.source_sha256 = Some("a".repeat(64));
            assert_eq!(
                LegacyPopulationOrigin::decode(&origin.encode().unwrap()).unwrap(),
                origin
            );
        }
    }
}
