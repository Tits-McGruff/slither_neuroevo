//! Bounded, offline-only committed tactical observations for canonical training.
//! This is a synthetic qualification corpus, not the production teacher transport.

use super::baseline_qualification::{TacticalOptions, TacticalTrial};
use super::checkpoint::{rename_noreplace, sync_parent_directory};
use super::sensor_contract::{SensorContractIdentity, SensorProfile};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
/// Independent offline output ceiling; no native runtime/checkpoint limit changes.
const MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sequence {
    scenario: &'static str,
    seed: u32,
    policy: &'static str,
    row_offset: u64,
    rows: u64,
    alive: bool,
    death_cause: Option<&'static str>,
    trajectory_witness_sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ObjectReceipt {
    file: &'static str,
    bytes: u64,
    sha256: String,
}

/// Streams fixed-width LE rows; metadata is published only after all files are synced.
pub(crate) struct TacticalCorpusWriter {
    root: PathBuf,
    files: [BufWriter<File>; 3],
    hashes: [Sha256; 3],
    rows: u64,
    sequences: Vec<Sequence>,
    previous_step: u32,
    sequence_start: u64,
    maximum_rows: u64,
    contract: SensorContractIdentity,
}
impl TacticalCorpusWriter {
    /// Prove worst-case output admission before exclusively creating any corpus path.
    pub(crate) fn new(
        root: &Path,
        options: TacticalOptions,
        profile: SensorProfile,
    ) -> Result<Self> {
        if !matches!(profile, SensorProfile::Basic | SensorProfile::Motion) {
            return Err("tactical corpus requires a v4 profile".into());
        }
        let contract = SensorContractIdentity::new(profile, options.bins)?;
        let row_bytes = (contract.input_size() as u64 + 3) * 4;
        let maximum_rows = u64::from(options.seed_count)
            .checked_mul(u64::from(options.steps))
            .and_then(|rows| rows.checked_mul(14 * 2))
            .ok_or("tactical corpus row bound overflow")?;
        if maximum_rows == 0 || maximum_rows > (MAX_BYTES - 1024 * 1024) / row_bytes {
            return Err("tactical corpus worst-case binary plus metadata exceeds 256 MiB".into());
        }
        fs::create_dir(root)?;
        let open = |name| -> Result<BufWriter<File>> {
            Ok(BufWriter::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(root.join(name))?,
            ))
        };
        Ok(Self {
            root: root.to_owned(),
            files: [
                open("observations.f32")?,
                open("actions.f32")?,
                open("steps.u32")?,
            ],
            hashes: std::array::from_fn(|_| Sha256::new()),
            rows: 0,
            sequences: Vec::new(),
            previous_step: 0,
            sequence_start: 0,
            maximum_rows,
            contract,
        })
    }
    pub(crate) fn begin(&mut self) {
        self.sequence_start = self.rows;
        self.previous_step = 0;
    }
    /// Call only after the complete physical step and matching subject outcome publish.
    pub(crate) fn committed(
        &mut self,
        step: u32,
        observation: &[f32],
        turn: f32,
        boost: bool,
    ) -> Result<()> {
        if observation.len() != self.contract.input_size()
            || observation.iter().any(|value| !value.is_finite())
            || !turn.is_finite()
            || !(-1.0..=1.0).contains(&turn)
            || step <= self.previous_step
            || self.rows >= self.maximum_rows
        {
            return Err("invalid or unadmitted committed tactical row".into());
        }
        for value in observation {
            let bytes = value.to_le_bytes();
            self.files[0].write_all(&bytes)?;
            self.hashes[0].update(bytes);
        }
        for value in [turn, if boost { 1.0 } else { 0.0 }] {
            let bytes = value.to_le_bytes();
            self.files[1].write_all(&bytes)?;
            self.hashes[1].update(bytes);
        }
        let bytes = step.to_le_bytes();
        self.files[2].write_all(&bytes)?;
        self.hashes[2].update(bytes);
        self.rows += 1;
        self.previous_step = step;
        Ok(())
    }
    pub(crate) fn end(&mut self, trial: &TacticalTrial) -> Result<()> {
        let rows = self.rows - self.sequence_start;
        let matching_policy = match self.contract.profile() {
            SensorProfile::Basic => matches!(
                trial.policy,
                "basic-v2-deterministic" | "basic-v2-stochastic"
            ),
            SensorProfile::Motion => matches!(
                trial.policy,
                "motion-v2-deterministic" | "motion-v2-stochastic"
            ),
            SensorProfile::V3 => false,
        };
        if !matching_policy
            || rows != trial.observations
            || rows == 0
            || self.sequences.len() >= 14 * 64 * 2
        {
            return Err("tactical corpus does not reconcile delivered decisions".into());
        }
        self.sequences.push(Sequence {
            scenario: trial.scenario,
            seed: trial.seed,
            policy: trial.policy,
            row_offset: self.sequence_start,
            rows,
            alive: trial.alive,
            death_cause: trial.death_cause,
            trajectory_witness_sha256: trial.trajectory_witness_sha256.clone(),
        });
        Ok(())
    }
    /// A final manifest is the completion marker; partial outputs never masquerade as a corpus.
    pub(crate) fn finish(mut self, options: TacticalOptions) -> Result<()> {
        if options.bins != self.contract.bins() {
            return Err("tactical corpus layout changed before publication".into());
        }
        for file in &mut self.files {
            file.flush()?;
            file.get_ref().sync_all()?;
        }
        if self.sequences.len() != options.seed_count as usize * 14 * 2 {
            return Err("incomplete tactical corpus sequence matrix".into());
        }
        let objects: Vec<_> = self
            .hashes
            .into_iter()
            .zip([
                (
                    "observations.f32",
                    self.rows * self.contract.input_size() as u64 * 4,
                ),
                ("actions.f32", self.rows * 8),
                ("steps.u32", self.rows * 4),
            ])
            .map(|(hash, (file, bytes))| ObjectReceipt {
                file,
                bytes,
                sha256: format!("{:x}", hash.finalize()),
            })
            .collect();
        let contract = self.contract;
        let metadata = serde_json::json!({"schemaVersion":1,"kind":"native-tactical-corpus-v1",
            "sourceSha256":crate::native_addon_source_sha256(),"target":crate::native_addon_build_target(),
            "rustc":crate::native_addon_rustc_version(),"mathBackend":super::inference::InferenceMathBackend::detect().label(),
            "sensorCanonicalJson":String::from_utf8(contract.canonical_bytes()?)?,"sensorSha256":contract.sha256_hex()?,
            "inputSize":contract.input_size(),"rows":self.rows,"controlIntervalSeconds":options.cadence_seconds,
            "maximumSteps":options.steps,"seedCount":options.seed_count,"workers":options.workers,
            "objects":objects,"sequences":self.sequences,"scope":"Synthetic native v4 deterministic/stochastic tactical teachers in the declared profile, real committed observations/actions and complete physical steps. Each sequence starts with zero baseline policy state and ends at death or the bounded horizon. No held/dead rows, resampling, production-world snapshots, trained-population distribution or release qualification is implied."});
        let bytes = serde_json::to_vec_pretty(&metadata)?;
        if bytes.len() > 1024 * 1024 {
            return Err("tactical corpus metadata exceeds 1 MiB".into());
        }
        let mut manifest = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.root.join(".corpus.json.pending"))?;
        manifest.write_all(&bytes)?;
        manifest.write_all(b"\n")?;
        manifest.sync_all()?;
        drop(manifest);
        rename_noreplace(
            &self.root.join(".corpus.json.pending"),
            &self.root.join("corpus.json"),
        )?;
        sync_parent_directory(&self.root)?;
        Ok(())
    }
}
