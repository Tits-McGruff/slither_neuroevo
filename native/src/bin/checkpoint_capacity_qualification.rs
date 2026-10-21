//! Explicit offline checkpoint-capacity candidates, with no production limit selection.
use slither_native::engine::checkpoint_capacity::{
    capacity_workload_options, run_checkpoint_archive_capacity, run_checkpoint_capacity,
    CheckpointCapacityPolicy,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
fn available_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    let status = rustix::fs::statvfs(path)?;
    status
        .f_bavail
        .checked_mul(status.f_frsize)
        .ok_or_else(|| "filesystem byte count overflow".into())
}

#[cfg(target_os = "windows")]
fn available_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: NUL-terminated UTF-16 input lives through the call; the single
    // non-null output points to one writable u64, and optional outputs are null.
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(available)
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let mut values = BTreeMap::new();
    while let Some(flag) = arguments.next() {
        if ![
            "--workspace",
            "--report",
            "--mode",
            "--population",
            "--graph",
            "--envelope-mib",
            "--memory-gib",
            "--source",
            "--lease",
            "--coexistence",
            "--saved-brains",
        ]
        .contains(&flag.as_str())
        {
            return Err(format!("unknown flag {flag}").into());
        }
        let value = arguments.next().ok_or("flag requires a value")?;
        if values.insert(flag, value).is_some() {
            return Err("duplicate flag".into());
        }
    }
    let get = |flag: &str| {
        values
            .get(flag)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {flag}"))
    };
    let workspace = PathBuf::from(get("--workspace")?);
    let output = PathBuf::from(get("--report")?);
    let mode = get("--mode")?;
    if !["audit", "compressed", "raw", "archive", "archive-raw"].contains(&mode) {
        return Err("mode must be audit/compressed/raw/archive/archive-raw".into());
    }
    let coexistence = match values
        .get("--coexistence")
        .map(String::as_str)
        .unwrap_or("none")
    {
        "none" => false,
        "library-inspector" => true,
        _ => return Err("coexistence must be none/library-inspector".into()),
    };
    if coexistence && mode == "audit" {
        return Err("coexistence requires an allocated workload".into());
    }
    let population: usize = get("--population")?.parse()?;
    if ![55, 300].contains(&population) {
        return Err("population must be 55 or 300".into());
    }
    let large_graph = match get("--graph")? {
        "large" => true,
        "small" => false,
        _ => return Err("graph must be large/small".into()),
    };
    let envelope: usize = get("--envelope-mib")?.parse()?;
    let memory_gib: usize = get("--memory-gib")?.parse()?;
    if !(1..=8).contains(&memory_gib) {
        return Err("offline memory must be 1..8 GiB".into());
    }
    let policy = CheckpointCapacityPolicy::new(envelope, mode == "raw" || mode == "archive-raw")?;
    // Exclusive creation prevents accidental use of an owner checkpoint directory.
    fs::create_dir(&workspace)?;
    let workspace = workspace.canonicalize()?;
    let available = available_bytes(&workspace)?;
    let reserve = (6 * envelope as u64 + 1024) * 1024 * 1024
        + if coexistence {
            slither_native::engine::coexistence_capacity::additional_disk_reserve_bytes()
        } else {
            0
        };
    if available < reserve {
        return Err(format!(
            "insufficient disk: {available} available, {reserve} required including reserve"
        )
        .into());
    }
    let options = capacity_workload_options(population, large_graph);
    let mut report = if mode.starts_with("archive") {
        let lease_path = PathBuf::from(get("--lease")?);
        let file = std::fs::File::open(lease_path)?;
        if file.metadata()?.len() > 64 * 1024 {
            return Err("lease exceeds 64 KiB".into());
        }
        let mut lease = String::new();
        std::io::Read::read_to_string(&mut std::io::Read::take(file, 64 * 1024 + 1), &mut lease)?;
        let saved_brains = if coexistence {
            let path = PathBuf::from(get("--saved-brains")?);
            let file = std::fs::File::open(path)?;
            if file.metadata()?.len() > 128 * 1024 {
                return Err("saved brain selection exceeds 128 KiB".into());
            }
            let entries: Vec<_> = serde_json::from_reader(file)?;
            if entries.len() != slither_native::engine::coexistence_capacity::LIBRARY_RECORDS {
                return Err("coexistence requires 100 selected saved brains".into());
            }
            entries
        } else {
            Vec::new()
        };
        run_checkpoint_archive_capacity(
            &options,
            Path::new(get("--source")?),
            &workspace,
            &lease,
            policy,
            memory_gib * 1024 * 1024 * 1024,
            &saved_brains,
        )?
    } else {
        run_checkpoint_capacity(
            &options,
            &workspace,
            policy,
            memory_gib * 1024 * 1024 * 1024,
            mode != "audit",
            coexistence,
        )?
    };
    report["diskAvailableBeforeBytes"] = available.into();
    report["diskRequiredIncludingReserveBytes"] = reserve.into();
    report["mode"] = mode.into();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    eprintln!("checkpoint capacity qualification completed ({mode}, {population} snakes)");
    Ok(())
}

/// Debug builds retain several large coordinator move temporaries. Match an
/// explicit offline stack budget rather than relying on Windows' main stack.
fn main() -> Result<(), Box<dyn Error>> {
    std::thread::Builder::new()
        .name("checkpoint-capacity".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| run().map_err(|error| error.to_string()))?
        .join()
        .map_err(|_| "checkpoint capacity worker panicked")?
        .map_err(Into::into)
}
