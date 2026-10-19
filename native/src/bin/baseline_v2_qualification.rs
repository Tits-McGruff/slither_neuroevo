//! Bounded actual-native tactical batches, with no server or owner database.
use slither_native::engine::baseline_qualification::{
    run_student_tactical_qualification, run_tactical_qualification_with_corpus,
    StudentTacticalReport, TacticalOptions, TacticalReport, TacticalStudent,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{Read, Write};

#[derive(serde::Serialize)]
#[serde(untagged)]
enum Report {
    Teacher(TacticalReport),
    Student(StudentTacticalReport),
}

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
            "--student-artifact",
            "--seed-start",
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
    let options = TacticalOptions {
        seed_count: get("--seed-count", "16").parse()?,
        steps: get("--steps", "180").parse()?,
        workers: get("--workers", "1").parse()?,
        cadence_seconds: get("--cadence", "0.016666666666666666").parse()?,
    };
    let report = if let Some(path) = values.get("--student-artifact") {
        if values.contains_key("--corpus-directory") {
            return Err("student evaluation cannot export teacher corpora".into());
        }
        let file = std::fs::File::open(path)?;
        let maximum = slither_native::engine::brain_artifact::MAX_BRAIN_ARTIFACT_BYTES as u64;
        if !file.metadata()?.is_file() || file.metadata()?.len() > maximum {
            return Err("student artifact exceeds ordinary portable admission".into());
        }
        let mut bytes = Vec::new();
        file.take(maximum + 1).read_to_end(&mut bytes)?;
        let student = TacticalStudent::from_artifact(&bytes)?;
        Report::Student(run_student_tactical_qualification(
            options,
            &student,
            get("--seed-start", "1").parse()?,
        )?)
    } else {
        if values.contains_key("--seed-start") {
            return Err("--seed-start requires --student-artifact".into());
        }
        Report::Teacher(run_tactical_qualification_with_corpus(
            options,
            values.get("--corpus-directory").map(std::path::Path::new),
        )?)
    };
    let trials = match &report {
        Report::Teacher(value) => value.trials.len(),
        Report::Student(value) => value.trials.len(),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    println!("trials={trials} output={output}");
    Ok(())
}
