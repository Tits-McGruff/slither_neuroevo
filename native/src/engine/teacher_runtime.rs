//! Cold capture admission and identity construction from native authority.

use super::baseline_v2::{BaselineV2Mode, BaselineV2Settings};
use super::sensor_contract::{SensorContractIdentity, SensorProfile};
use super::state::AuthoritativeState;
use super::teacher_samples::TeacherContractV1;
use sha2::{Digest, Sha256};

/// Explicit diagnostic ceilings; these never reserve reliable controller output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TeacherCaptureLimits {
    pub pending_bytes: usize,
    pub queue_chunks: usize,
    pub queue_bytes: usize,
}

/// Ordered arm request bound to the expected current run/configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeacherCaptureRequest {
    pub capture_epoch: [u8; 16],
    pub expected_run_id: String,
    pub expected_config_revision: u64,
    pub baseline_slots: Box<[u32]>,
    pub limits: TeacherCaptureLimits,
}
impl TeacherCaptureRequest {
    /// Reject shape/ownership limits before an inbound request can allocate state.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.capture_epoch == [0; 16]
            || self.expected_run_id.is_empty()
            || self.expected_run_id.len() > 128
            || self.expected_run_id.contains('\0')
            || self.expected_config_revision == 0
            || self.baseline_slots.is_empty()
            || self.baseline_slots.len() > 120
            || self.baseline_slots.iter().any(|slot| *slot >= 120)
            || self
                .baseline_slots
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self.limits.pending_bytes == 0
            || self.limits.pending_bytes > 16 * 1024 * 1024
            || !(1..=65_536).contains(&self.limits.queue_chunks)
            || self.limits.queue_bytes == 0
            || self.limits.queue_bytes > 16 * 1024 * 1024
        {
            return Err("invalid teacher capture request");
        }
        Ok(())
    }
}

/// Derive all teacher/sensor/cadence identities from the admitted native run.
pub(crate) fn capture_contract(
    authority: &AuthoritativeState,
    request: &TeacherCaptureRequest,
) -> Result<(TeacherContractV1, SensorContractIdentity), String> {
    request.validate().map_err(str::to_owned)?;
    let state = authority.state();
    if request.expected_run_id != state.identity.run_id
        || request.expected_config_revision != state.identity.config_revision
        || request
            .baseline_slots
            .iter()
            .any(|slot| *slot as usize >= state.fixed_step.baseline_lifecycle.slots.len())
    {
        return Err("teacher capture owner/run/configuration mismatch".into());
    }
    let sensor = SensorContractIdentity::from_settings(&state.config.settings)
        .map_err(|error| error.to_string())?;
    let settings = BaselineV2Settings::from_settings(&state.config.settings)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "teacher capture requires baseline-v2".to_owned())?;
    if sensor.profile() == SensorProfile::V3 {
        return Err("teacher capture requires v4".into());
    }
    let mut teacher = Sha256::new();
    teacher.update(b"slither.baseline-teacher.v2\0");
    teacher.update(env!("SLITHER_NATIVE_SOURCE_SHA256").as_bytes());
    let contract = TeacherContractV1 {
        capture_epoch: request.capture_epoch,
        run_id: state.identity.run_id.clone(),
        config_sha256: super::state::decode_sha256_identity(
            "teacher.config",
            &state.identity.config_hash,
        )
        .map_err(|error| error.to_string())?,
        sensor_sha256: sensor.sha256().map_err(|error| error.to_string())?,
        teacher_sha256: teacher.finalize().into(),
        teacher_revision: super::baseline_v2::BASELINE_V2_VERSION,
        teacher_mode: match settings.mode {
            BaselineV2Mode::Deterministic => 0,
            BaselineV2Mode::Stochastic => 1,
        },
        control_interval_seconds: settings.control_interval_seconds,
        input_size: sensor.input_size(),
    };
    Ok((contract, sensor))
}
