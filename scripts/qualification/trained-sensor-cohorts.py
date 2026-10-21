"""Measure retained learned clones or seed families through native P0/P1 workloads."""
import argparse
import gzip
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import time


def require(condition, message):
    """Keep evidence assertions active even under optimized Python."""
    if not condition:
        raise RuntimeError(message)


def digest(path):
    """Hash one immutable bounded input or report."""
    return hashlib.sha256(path.read_bytes()).hexdigest()


def extract_artifact(study, evidence, relative, output, replicated):
    """Restore opaque checksummed bytes; Rust remains the artifact admission authority."""
    matches = [item for item in evidence["files"] if item["path"] == relative]
    require(len(matches) == 1, "one retained artifact receipt required")
    receipt = matches[0]
    require(type(receipt["bytes"]) is int and 0 < receipt["bytes"] <= 64 * 1024**2,
            "artifact byte admission")
    require(all(isinstance(receipt[key], str) and re.fullmatch(r"[0-9a-f]{64}", receipt[key])
                for key in ("sha256", "gzipSha256")), "artifact digest syntax")
    expected = "objects/" + receipt["sha256"] + ".gz" if replicated else relative + ".gz"
    require(receipt["archive"] == expected, "unexpected artifact path")
    compressed = (study / expected).resolve(strict=True)
    require(compressed.is_relative_to(study) and compressed.is_file(), "artifact archive escapes study")
    require(compressed.stat().st_size <= 64 * 1024**2 and digest(compressed) == receipt["gzipSha256"],
            "retained artifact archive changed")
    with gzip.open(compressed, "rb") as stream:
        opaque = stream.read(receipt["bytes"] + 1)
    require(len(opaque) == receipt["bytes"] and hashlib.sha256(opaque).hexdigest() == receipt["sha256"],
            "retained artifact bytes changed")
    with output.open("xb") as stream:
        stream.write(opaque)
    return {"studyPath": relative, "artifactSha256": receipt["sha256"]}


def main():
    """Run serial worker pairs; successful measurements never promote defaults."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--study", required=True, type=Path)
    parser.add_argument("--replicated", action="store_true", help="Mix three initialization seeds per graph from replicated evidence")
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--warmup", type=int, default=600)
    parser.add_argument("--steps", type=int, default=3600)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--environment", choices=["development", "oxygen-debian"], default="development")
    args = parser.parse_args()
    if (not 2 <= args.workers <= 512 or not 1 <= args.steps <= 60000
            or not 0 <= args.warmup <= 60000 or args.steps + args.warmup > 60000
            or not 0 <= args.seed <= 0xffffffff):
        parser.error("bounded workers, steps, warmup and u32 seed required")
    executable = args.executable.resolve(strict=True)
    study = args.study.resolve(strict=True)
    directory = args.directory.resolve()
    require(executable.is_file(), "native executable required")
    require(shutil.disk_usage(directory.parent).free >= 2 * 1024**3, "native disk reserve required")
    directory.mkdir()  # Exclusive: never reuse or remove owner paths.
    binary_sha = digest(executable)
    script_sha = digest(Path(__file__))
    evidence_path = study / "evidence.json"
    require(evidence_path.stat().st_size <= 1024**2, "bounded evidence manifest required")
    evidence_sha = digest(evidence_path)
    evidence = json.loads(evidence_path.read_bytes())
    require(isinstance(evidence["files"], list) and len(evidence["files"]) <= 2048, "bounded evidence inventory required")
    if args.replicated:
        require(evidence["schemaVersion"] == 1 and evidence["kind"] == "replicated-ablation-evidence-v1"
                and evidence["status"] == "measured", "measured replicated evidence required")
    records, inputs = [], {}
    source = None
    all_source_files = []
    for variant in ["full", "core-only"]:
        source_files = []
        if args.replicated:
            families, entries = [], []
            for seed in (1, 2, 3):
                relative = f"seed-{seed}-fold-2/{variant}/trained.slither-brain"
                member = directory / f"{variant}-seed-{seed}.slither-brain"
                identity = extract_artifact(study, evidence, relative, member, True)
                families.append(identity)
                entries.append({"file": member.name, "sha256": identity["artifactSha256"]})
                source_files.append((member, identity["artifactSha256"]))
            artifact = directory / (variant + "-cohort.json")
            with artifact.open("x", encoding="utf-8") as stream:
                json.dump({"schemaVersion": 1, "artifacts": entries}, stream, indent=2)
                stream.write("\n")
            inputs[variant] = {"families": families, "indexSha256": digest(artifact)}
            source_files.append((artifact, inputs[variant]["indexSha256"]))
        else:
            relative = variant + "/trained.slither-brain"
            artifact = directory / (variant + ".slither-brain")
            inputs[variant] = extract_artifact(study, evidence, relative, artifact, False)
            source_files.append((artifact, inputs[variant]["artifactSha256"]))
        all_source_files.extend(source_files)
        source_flag = "--trained-cohort" if args.replicated else "--trained-artifact"
        identity_key = "trainedCohort" if args.replicated else "trainedArtifact"
        for scenario, population in [("P0", 55), ("P1", 300)]:
            reference = None
            for workers in [1, args.workers]:
                stem = f"{variant}-{scenario}-w{workers}"
                output = directory / (stem + ".json")
                require(digest(executable) == binary_sha and all(digest(path) == sha for path, sha in source_files), "input changed")
                command = [str(executable), source_flag, str(artifact), "--scenario", scenario,
                           "--profile", "motion", "--bins", "16", "--respawn", "on", "--seed", str(args.seed),
                           "--workers", str(workers), "--math-backend", "sse2", "--minimum", "1", "--cap", "512",
                           "--batch", "131072", "--baseline", "v2-stochastic", "--baseline-cadence", "0.016666666666666666",
                           "--capture", "on", "--warmup", str(args.warmup), "--steps", str(args.steps),
                           "--environment", args.environment, "--workspace", str(directory / (stem + "-workspace")),
                           "--output", str(output)]
                started = time.perf_counter()
                with subprocess.Popen(command, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0) as process:
                    try:
                        code = process.wait(timeout=1800)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
                        raise
                elapsed = time.perf_counter() - started
                require(code == 0, f"native workload failed: {stem}")
                require(all(digest(path) == sha for path, sha in source_files), "learned sources changed during measurement")
                require(output.stat().st_size <= 4 * 1024**2, "report byte admission")
                report = json.loads(output.read_bytes())
                require(report["admitted"] and report["version"] == (7 if args.replicated else 6)
                        and report["graphSize"] == ("trained-cohort" if args.replicated else "trained-artifact"),
                        "trained workload admission required")
                if args.replicated:
                    cohort = report["trainedCohort"]
                    require(cohort["indexSha256"] == inputs[variant]["indexSha256"]
                            and 0 < cohort["sourceReservationBytes"] <= 512 * 1024**2, "cohort identity/admission changed")
                    reserve = report["authoritativeMemoryReservationBytes"]
                    require(type(reserve) is int and reserve > 0
                            and reserve + cohort["sourceReservationBytes"] <= 4 * 1024**3,
                            "source and authority construction ceiling exceeded")
                    require([item["artifact"]["artifactSha256"] for item in cohort["families"]]
                            == [item["artifactSha256"] for item in families], "wrong learned families")
                    require(len({item["artifact"]["weightsSha256"] for item in cohort["families"]}) == 3,
                            "distinct packed policies required")
                    require([item["populationSlots"] for item in cohort["families"]]
                            == [len(range(index, population, 3)) for index in range(3)], "wrong family slot assignment")
                else:
                    require(report["trainedArtifact"]["artifactSha256"] == inputs[variant]["artifactSha256"], "wrong learned policy")
                require(report["population"] == population and report["workers"] == workers and report["seed"] == args.seed,
                        "wrong cohort identity")
                require(report["completedSteps"] == args.steps + args.warmup and report["measuredSteps"] == args.steps,
                        "incomplete workload")
                require(report["sharedBatchBudgetHeld"] and report["observations"] > 0, "missing bounded observations")
                require(report["warmupSteps"] == args.warmup and report["baselineCount"] == 10, "wrong workload")
                require(all(hist["nonfinite"] == 0 for hist in report["raw"] + report["normalized"]), "nonfinite distribution")
                capture = report["capture"]
                require(capture["exactSamplesMatch"] is True and capture["queueAccountingMatches"], "capture reconciliation failed")
                source = source or report["sourceSha256"]
                require(report["sourceSha256"] == source, "native source changed")
                if reference is not None:
                    for key in ["sourceSha256", "target", "rustc", "mathBackend", "sensorSha256", identity_key,
                                "parameterCount", "continuationSha256", "consumerWork", "raw", "normalized"]:
                        require(reference[key] == report[key], f"worker continuation changed {key}")
                    for key in ["sourceSampleSha256", "drainedSampleSha256"]:
                        require(reference["capture"][key] == capture[key], f"worker capture changed {key}")
                reference = report
                row = {"variant": variant, "scenario": scenario, "workers": workers, "report": output.name,
                       "sha256": digest(output), "processWallSeconds": elapsed,
                       "meanNativeMs": report["nativePhaseCosts"]["totalMicros"] / args.steps / 1000,
                       "p95NativeMs": report["stepCost"]["p95Ms"],
                       "incompleteFraction": report["allConsumerRayIncompleteFraction"],
                       "observations": report["observations"], "peakRssBytes": report["peakRssBytes"]}
                records.append(row)
                print(json.dumps(row, sort_keys=True), flush=True)
    require(digest(executable) == binary_sha and digest(Path(__file__)) == script_sha
            and digest(evidence_path) == evidence_sha
            and all(digest(path) == sha for path, sha in all_source_files), "measurement inputs changed")
    summary = {"version": 1, "measurementComplete": True, "releaseQualified": False,
               "nativeSourceSha256": source, "binarySha256": binary_sha, "scriptSha256": script_sha,
               "studyEvidenceSha256": evidence_sha, "inputs": inputs, "records": records,
               "replicatedFamilies": args.replicated, "maturityQualified": False,
               "steps": args.steps, "warmup": args.warmup, "seed": args.seed,
               "caveat": ("Three independently seeded learned policy families per graph. " if args.replicated
                          else "One cloned learned policy per cohort. ") + "Trained on short synthetic teacher tactics; maturity is not inferred. "
                         "Evolved life/economic input domain differs from teacher training. Full and core graphs still sense Motion/B16. "
                         "Controlled native steps exclude observer/startup/checkpoint, Node/browser/network, background wall debt and trainer contention. "
                         "Serial shared-host timings are descriptive, not a real-clock, representative-population, integrated soak or default-promotion pass. "
                         "Exact worker equality applies only within this source/build/target/seed/configuration."}
    with (directory / "summary.json").open("x", encoding="utf-8") as stream:
        json.dump(summary, stream, indent=2, allow_nan=False)
        stream.write("\n")


if __name__ == "__main__":
    main()
