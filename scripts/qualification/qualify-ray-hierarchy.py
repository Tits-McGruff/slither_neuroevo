"""Measure admitted v4 ray quality across profiles, widths and bounded budgets."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def require(condition, message):
    """Preserve admission and evidence checks under optimized Python execution."""
    if not condition:
        raise RuntimeError(message)


def digest(path):
    """Hash immutable artifacts with bounded memory."""
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def parse_args():
    """Require explicit immutable native input and a new report directory."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--source-sha256", required=True)
    parser.add_argument("--reuse-directory", type=Path)
    parser.add_argument("--workers", type=int, default=3)
    parser.add_argument("--steps", type=int, default=1200)
    parser.add_argument("--environment", choices=["development", "oxygen-debian"], default="development")
    parser.add_argument("--deadline-seconds", type=int, default=900)
    args = parser.parse_args()
    if not 2 <= args.workers <= 512 or not 1200 <= args.steps <= 59995 or not 1 <= args.deadline_seconds <= 3600:
        parser.error("require workers 2..512, steps 1200..59995 and deadline 1..3600")
    if len(args.source_sha256) != 64 or any(value not in "0123456789abcdef" for value in args.source_sha256):
        parser.error("exact lowercase native source SHA-256 required")
    if args.reuse_directory is not None:
        args.reuse_directory = args.reuse_directory.resolve(strict=True)
        if not args.reuse_directory.is_dir():
            parser.error("reuse directory must exist")
    args.executable = args.executable.resolve(strict=True)
    if not args.executable.is_file():
        parser.error("native executable must be a regular file")
    args.directory = args.directory.resolve()
    args.directory.mkdir()  # Exclusive; the native owner removes only its own workspaces.
    return args


def validate_report(report, scenario, profile, bins, cap, batch, workers, steps):
    """Reconcile actual committed observations and work without awarding a release pass."""
    population = 55 if scenario == "P0" else 300
    require(report["version"] == 4 and report["admitted"], "admitted version-4 workload required")
    require(report["population"] == population and report["baselineCount"] == 10, "wrong physical population")
    contract = report["sensorContract"]
    require(contract["layoutVersion"] == 4 and contract["profile"] == profile and contract["bins"] == bins, "wrong sensor contract")
    require(report["seed"] == 42 and report["workers"] == workers and report["mathBackend"] == "rust-sse2-v1", "wrong seeded backend")
    require(report["respawn"] and report["graphSize"] == "modest", "unlimited modest population required")
    require(report["baselineMode"] == "v2-stochastic" and report["baselineCadenceSeconds"] == 1 / 60, "exact 60 Hz v2 required")
    require(report["teacherRespawnDelaySeconds"] == 20, "ordinary teacher replacement delay required")
    require(report["rayMinimum"] == 1 and report["rayCap"] == cap and report["rayBatch"] == batch, "wrong ray admission")
    require(report["measuredSteps"] == steps and report["warmupSteps"] == 5 and report["completedSteps"] == steps + 5, "complete measured steps required")
    work = report["consumerWork"]
    minimum = report["minimumDueObservations"]
    maximum = report["maximumDueObservations"]
    observed = report["observations"]
    require(0 < minimum <= maximum <= population, "actual nonzero evolved due envelope required")
    require(len(work) == 3 and work[0]["observations"] == observed, "evolved delivery accounting must reconcile")
    require(minimum * steps <= observed <= maximum * steps, "deliveries must fit the actual boundary envelope")
    require(work[1]["observations"] > 0 and work[2]["observations"] == 0, "honest baseline/external sample counts required")
    require(minimum <= report["minimumDueConsumers"] <= report["maximumDueConsumers"] <= population + 10, "actual all-consumer envelope required")
    require(maximum <= report["maximumDueConsumers"], "consumer envelope must include evolved deliveries")
    require(report["sharedBatchBudgetHeld"] is True, "one shared fair ray budget required")
    for consumer in work:
        require(consumer["v4Observations"] == consumer["observations"], "all delivered sensors must be v4")
        require(sum(consumer[key] for key in ["rayCells", "rayCandidates", "rayIntersections"]) == consumer["rayChargedUnits"], "work units must partition charges")
        require(consumer["rayChargedUnits"] <= cap * consumer["observations"], "per-observation cap exceeded")
        require(0 <= consumer["incompleteObservations"] <= consumer["incompleteDirections"] <= bins * consumer["observations"], "invalid incomplete accounting")
    count = sum(row["observations"] for row in work)
    incomplete = sum(row["incompleteObservations"] for row in work)
    require(sum(row["rayChargedUnits"] for row in work) <= batch * steps, "batch accounting exceeded")
    fraction = incomplete / count
    require(report["allConsumerRayIncompleteFraction"] == fraction, "pooled fraction must reconcile actual observations")
    require(report["allConsumerRayIncompleteBelowOnePercent"] == (fraction < 0.01), "quality flag must match actual fraction")
    require(all(row["samples"] == observed and row["nonfinite"] == 0 for row in report["normalized"]), "finite normalized observations required")
    capture = report["capture"]
    require(capture is not None and capture["exactSamplesMatch"] is True and capture["queueAccountingMatches"], "exact committed capture required")
    require(sum(capture["samplesObserved"]) == work[1]["observations"] == sum(capture["samplesDrained"]), "delivered teacher rows must reconcile")
    require(capture["droppedChunks"] == 0 and capture["productionWatermark"] == capture["releasedThrough"], "fast capture must drain and release every chunk")


def main():
    """Run widths and budget probes, plus an exact same-target worker continuation check."""
    script_sha = digest(Path(__file__))
    args = parse_args()
    binary_sha = digest(args.executable)
    source = args.source_sha256
    reference = None
    target = None
    records = []
    cases = [("P1", profile, bins, 256, 65536, args.workers)
             for profile in ["basic", "motion"] for bins in [8, 16, 32]]
    cases += [("P0", profile, 16, 256, 65536, args.workers) for profile in ["basic", "motion"]]
    cases += [("P1", "motion", 16, cap, batch, args.workers)
              for cap, batch in [(128, 65536), (256, 32768), (256, 16384)]]
    cases += [("P1", "motion", 16, 256, 65536, 1)]
    for scenario, profile, bins, cap, batch, workers in cases:
        require(digest(args.executable) == binary_sha, "native executable changed during cohort")
        name = f"{scenario}-{profile}-b{bins}-c{cap}-q{batch}-w{workers}"
        output = args.directory / (name + ".json")
        command = [str(args.executable), "--scenario", scenario, "--profile", profile, "--bins", str(bins),
                   "--graph", "modest", "--respawn", "on", "--seed", "42", "--workers", str(workers),
                   "--math-backend", "sse2", "--minimum", "1", "--cap", str(cap), "--batch", str(batch),
                   "--baseline", "v2-stochastic", "--baseline-cadence", "0.016666666666666666", "--capture", "on",
                   "--warmup", "5", "--steps", str(args.steps), "--environment", args.environment,
                   "--workspace", str(args.directory / (name + "-workspace")), "--output", str(output)]
        reused = args.reuse_directory / output.name if args.reuse_directory is not None else None
        reused = reused if reused is not None and reused.exists() else None
        if reused is None:
            process = subprocess.Popen(command, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
            try:
                code = process.wait(timeout=args.deadline_seconds)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                raise
            require(code == 0, f"native case failed: {name} exit {code}")
            report_path = output
        else:
            require(reused.is_file() and not reused.is_symlink(), "reuse requires a regular report")
            report_path = reused
        require(digest(args.executable) == binary_sha, "native executable changed during case")
        require(report_path.stat().st_size <= 4 * 1024 * 1024, "native report exceeds admission")
        raw = report_path.read_bytes()
        report_sha = hashlib.sha256(raw).hexdigest()
        report = json.loads(raw)
        validate_report(report, scenario, profile, bins, cap, batch, workers, args.steps)
        require(report["sourceSha256"] == source and report["environment"] == args.environment, "source/environment mismatch")
        target = target or (report["target"], report["rustc"], report["cpuModel"])
        require(target == (report["target"], report["rustc"], report["cpuModel"]), "target/compiler/CPU changed across cases")
        if reused is not None:
            require(digest(reused) == report_sha, "reuse report changed during validation")
            with output.open("xb") as stream:
                stream.write(raw)
        require(digest(output) == report_sha, "retained report changed during validation")
        if (scenario, profile, bins, cap, batch) == ("P1", "motion", 16, 256, 65536):
            if reference is None:
                reference = report
            else:
                for key in ["sourceSha256", "target", "rustc", "mathBackend", "sensorSha256", "parameterCount",
                            "continuationSha256", "consumerWork", "raw", "normalized"]:
                    require(reference[key] == report[key], f"worker count changed {key}")
                for key in ["sourceSampleSha256", "drainedSampleSha256", "samplesObserved", "dueTeacherHistogram"]:
                    require(reference["capture"][key] == report["capture"][key], f"worker count changed capture {key}")
        work = report["consumerWork"]
        record = {"name": name, "report": output.name, "reportSha256": report_sha, "reused": reused is not None,
                  "incompleteFraction": report["allConsumerRayIncompleteFraction"],
                  "consumerFractions": [None if not row["observations"] else row["incompleteObservations"] / row["observations"] for row in work],
                  "consumerObservations": [row["observations"] for row in work],
                  "nativeMeanMs": report["nativePhaseCosts"]["totalMicros"] / args.steps / 1000,
                  "nativeP95Ms": report["stepCost"]["p95Ms"], "peakRssBytes": report["peakRssBytes"]}
        records.append(record)
        print(json.dumps(record, sort_keys=True), flush=True)
    require(digest(Path(__file__)) == script_sha, "qualification script changed during cohort")
    result = {"version": 1, "scriptSha256": script_sha, "binarySha256": binary_sha, "sourceSha256": source,
              "steps": args.steps, "seed": 42, "workerContinuationMatches": True, "records": records,
              "caveat": "Seeded random-population native gameplay on a shared host; these are width and budget probes, not a complete cap matrix, trained-policy distribution, external-controller qualification, tactical/ablation result, 1x wall-debt test, statistical timing claim or production default selection. Different ray completion can change subsequent gameplay. Actual due counts may temporarily fall below population under bounded respawn. Reused reports retain exact original bytes and are independently revalidated against the requested native source, workload and target. Worker equality is asserted only for matching source/build/target/environment and workload. Ordinary teacher replacement can reduce live teacher counts; capture does not claim 600 rows/second."}
    with (args.directory / "qualification.json").open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2, allow_nan=False)
        stream.write("\n")


if __name__ == "__main__":
    main()
