//! Emit retained Stage 3 managed-checkpoint round-trip evidence.

use slither_native::engine::checkpoint_fixture::run_stage3_roundtrip_evidence;
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Disposable managed-file root removed after the evidence report is complete.
struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    /// Create one process- and time-qualified directory below the operating-system temp root.
    fn create() -> Result<Self, Box<dyn Error>> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "slither-stage3-checkpoint-roundtrip-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Parse the sole required output-path argument.
fn output_path() -> Result<PathBuf, Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let flag = arguments.next().ok_or("missing --output argument")?;
    if flag != "--output" {
        return Err(format!("unexpected argument {flag:?}; expected --output PATH").into());
    }
    let path = arguments.next().ok_or("missing path after --output")?;
    if arguments.next().is_some() {
        return Err("unexpected extra arguments".into());
    }
    Ok(PathBuf::from(path))
}

/// Write and sync one evidence artifact after all assertions pass.
fn write_report(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Run small, P0, and P2 fixtures through the real managed checkpoint codec.
fn main() -> Result<(), Box<dyn Error>> {
    let output = output_path()?;
    let temporary = TemporaryDirectory::create()?;
    let report = run_stage3_roundtrip_evidence(&temporary.0).map_err(std::io::Error::other)?;
    let bytes = serde_json::to_vec_pretty(&report)?;
    write_report(&output, &bytes)?;
    println!(
        "wrote {} checkpoint scenarios to {}",
        report.scenarios.len(),
        output.display()
    );
    Ok(())
}
