"""Measure retained learned clone policies through native P0/P1 workloads."""
import argparse
import gzip
import hashlib
import json
import os
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


def main():
    """Run serial worker pairs; successful measurements never promote defaults."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--study", required=True, type=Path)
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
    evidence_sha = digest(evidence_path)
    evidence = json.loads(evidence_path.read_bytes())
    records, inputs = [], {}
    source = None
    for variant in ["full", "core-only"]:
        relative = variant + "/trained.slither-brain"
        receipt = next(item for item in evidence["files"] if item["path"] == relative)
        require(receipt["archive"] == relative + ".gz", "unexpected artifact path")
        compressed = study / receipt["archive"]
        require(digest(compressed) == receipt["gzipSha256"], "retained artifact archive changed")
        require(0 < receipt["bytes"] <= 64 * 1024**2, "artifact byte admission")
        with gzip.open(compressed, "rb") as stream:
            opaque = stream.read(receipt["bytes"] + 1)
        require(len(opaque) == receipt["bytes"] and hashlib.sha256(opaque).hexdigest() == receipt["sha256"],
                "retained artifact bytes changed")
        artifact = directory / (variant + ".slither-brain")
        artifact.write_bytes(opaque)
        inputs[variant] = {"studyPath": relative, "artifactSha256": receipt["sha256"]}
        for scenario, population in [("P0", 55), ("P1", 300)]:
            reference = None
            for workers in [1, args.workers]:
                stem = f"{variant}-{scenario}-w{workers}"
                output = directory / (stem + ".json")
                require(digest(executable) == binary_sha and digest(artifact) == receipt["sha256"], "input changed")
                command = [str(executable), "--trained-artifact", str(artifact), "--scenario", scenario,
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
                require(output.stat().st_size <= 4 * 1024**2, "report byte admission")
                report = json.loads(output.read_bytes())
                require(report["admitted"] and report["version"] == 6 and report["graphSize"] == "trained-artifact",
                        "trained workload admission required")
                require(report["trainedArtifact"]["artifactSha256"] == receipt["sha256"], "wrong learned policy")
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
                    for key in ["sourceSha256", "target", "rustc", "mathBackend", "sensorSha256", "trainedArtifact",
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
            and digest(evidence_path) == evidence_sha, "measurement inputs changed")
    summary = {"version": 1, "measurementComplete": True, "releaseQualified": False,
               "nativeSourceSha256": source, "binarySha256": binary_sha, "scriptSha256": script_sha,
               "studyEvidenceSha256": evidence_sha, "inputs": inputs, "records": records,
               "steps": args.steps, "warmup": args.warmup, "seed": args.seed,
               "caveat": "One cloned learned policy per cohort, trained on short synthetic teacher tactics. "
                         "Evolved life/economic input domain differs from teacher training. Full and core graphs still sense Motion/B16. "
                         "Controlled native steps exclude observer/startup/checkpoint, Node/browser/network, background wall debt and trainer contention. "
                         "Serial shared-host timings are descriptive, not a real-clock, representative-population, Linux, soak or default-promotion pass. "
                         "Exact worker equality applies only within this source/build/target/seed/configuration."}
    with (directory / "summary.json").open("x", encoding="utf-8") as stream:
        json.dump(summary, stream, indent=2, allow_nan=False)
        stream.write("\n")


if __name__ == "__main__":
    main()
