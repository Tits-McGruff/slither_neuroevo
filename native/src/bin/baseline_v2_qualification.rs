//! Bounded actual-native tactical batches, with no server or owner database.
use slither_native::engine::baseline_qualification::{
    run_tactical_qualification_with_corpus, TacticalOptions,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mut values = BTreeMap::new();
    while let Some(flag) = args.next() {
        if ![
            "--seed-count",
            "--steps",
            "--workers",
            "--cadence",
            "--output",
            "--corpus-directory",
        ]
        .contains(&flag.as_str())
        {
            return Err(format!("unknown option {flag}").into());
        }
        let value = args.next().ok_or("option requires a value")?;
        if values.insert(flag, value).is_some() {
            return Err("duplicate option".into());
        }
    }
    let get = |name: &str, default: &str| {
        values
            .get(name)
            .map(String::as_str)
            .unwrap_or(default)
            .to_owned()
    };
    let output = values.get("--output").ok_or("--output is required")?;
    if std::path::Path::new(output).exists() {
        return Err("report already exists".into());
    }
    let report = run_tactical_qualification_with_corpus(
        TacticalOptions {
            seed_count: get("--seed-count", "16").parse()?,
            steps: get("--steps", "180").parse()?,
            workers: get("--workers", "1").parse()?,
            cadence_seconds: get("--cadence", "0.016666666666666666").parse()?,
        },
        values.get("--corpus-directory").map(std::path::Path::new),
    )?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    println!("trials={} output={output}", report.trials.len());
    Ok(())
}
