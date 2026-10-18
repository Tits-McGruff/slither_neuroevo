"""Qualify committed native capture and byte pressure without a spool or trainer."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


# Each independently admitted native pending/queue ownership ceiling.
BUDGET = 16 * 1024 * 1024


def require(condition, message):
    """Keep correctness checks active even when Python optimization is enabled."""
    if not condition:
        raise RuntimeError(message)


def digest(path):
    """Fingerprint immutable inputs and retained report bytes with bounded memory."""
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def parse_args():
    """Require an existing executable and a new, exclusive retained-report directory."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--workers", type=int, default=3)
    parser.add_argument("--steps", type=int, default=1200)
    parser.add_argument("--environment", choices=["development", "oxygen-debian"], default="development")
    parser.add_argument("--deadline-seconds", type=int, default=900)
    args = parser.parse_args()
    if not 2 <= args.workers <= 512 or not 1200 <= args.steps <= 60000 or not 1 <= args.deadline_seconds <= 3600:
        parser.error("require workers 2..512, steps 1200..60000 and deadline 1..3600")
    args.executable = args.executable.resolve(strict=True)
    if not args.executable.is_file():
        parser.error("native executable must be a regular file")
    args.directory = args.directory.resolve()
    args.directory.mkdir()  # Exclusive; workspaces are removed by the native owner.
    return args


def validate_report(report, steps):
    """Check workload identity and row/cadence accounting without accepting a timing gate."""
    require(report["version"] in (4, 5) and report["admitted"], "admitted version-4/5 workload required")
    require(report["population"] == 55 and report["baselineCount"] == 10, "P0 with ten teachers required")
    require(report["teacherRespawnDelaySeconds"] == 0.5, "recorded minimum admitted teacher delay required")
    require(report["baselineMode"] == "v2-stochastic" and report["baselineCadenceSeconds"] == 1 / 60, "exact 60 Hz v2 required")
    require(report["completedSteps"] == steps and report["measuredSteps"] == steps, "complete measured steps required")
    capture = report["capture"]
    if capture is None:
        return
    observed = capture["samplesObserved"]
    histogram = capture["dueTeacherHistogram"]
    require(len(observed) == 10 and all(count > 0 for count in observed), "all actual teachers must contribute")
    require(len(histogram) == 11 and sum(histogram) == steps, "due teacher histogram must cover every step")
    require(sum(index * count for index, count in enumerate(histogram)) == sum(observed), "due histogram must reconcile exact rows")
    require(sum(observed) == report["consumerWork"][1]["observations"], "capture must equal authoritative delivered observations")
    streak = capture["longestTenTeacherStreakSteps"]
    require(0 <= streak <= histogram[10] and bool(streak) == bool(histogram[10]), "ten-teacher streak must match actual due boundaries")
    require(capture["samplesPerSimulatedSecond"] <= 600, "capture cannot fabricate held or dead teacher rows")
    require(capture["pendingBudgetBytes"] == BUDGET and capture["queueBudgetBytes"] == BUDGET, "retain both 16 MiB capture ceilings")
    require(0 < capture["queuePayloadBudgetBytes"] < BUDGET, "record actual payload after ownership reserves")
    require(capture["queuePeakBytes"] <= capture["queuePayloadBudgetBytes"], "encoded ownership must stay admitted")
    require(capture["queueAccountingMatches"], "observed/drained/dropped ownership must reconcile")
    require(capture["productionWatermark"] == capture["releasedThrough"], "final drain must release all assigned sequences")
    require(capture["countLimitedDroppedChunks"] + capture["byteLimitedDroppedChunks"] == capture["droppedChunks"], "drop causes must partition complete dropped chunks")
    require(sum(observed) == sum(capture["samplesDrained"]) + capture["droppedSamples"], "complete dropped rows must reconcile")


def main():
    """Compare off/fast/parallel/byte-limited gameplay with immutable source and sample witnesses."""
    script_sha = digest(Path(__file__))
    args = parse_args()
    executable_sha = digest(args.executable)
    common = ["--scenario", "P0", "--profile", "motion", "--bins", "32", "--graph", "modest",
              "--respawn", "on", "--seed", "42", "--math-backend", "sse2", "--minimum", "1",
              "--cap", "256", "--batch", "65536", "--baseline", "v2-stochastic",
              "--baseline-cadence", "0.016666666666666666", "--teacher-respawn-delay", "0.5",
              "--warmup", "0", "--steps", str(args.steps), "--environment", args.environment]
    cases = [("off-single", 1, False, 1), ("fast-single", 1, True, 1),
             ("fast-parallel", args.workers, True, 1), ("byte-pressure-parallel", args.workers, True, args.steps)]
    reference = None
    captured_reference = None
    records = []
    for name, workers, enabled, drain_steps in cases:
        require(digest(args.executable) == executable_sha, "native executable changed during cohort")
        output = args.directory / (name + ".json")
        command = [str(args.executable), *common, "--workers", str(workers), "--capture", "on" if enabled else "off"]
        if enabled:
            command += ["--capture-queue-chunks", "8192", "--capture-drain-steps", str(drain_steps)]
        command += ["--workspace", str(args.directory / (name + "-workspace")), "--output", str(output)]
        process = subprocess.Popen(command, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        try:
            code = process.wait(timeout=args.deadline_seconds)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise
        require(code == 0, f"native capture case failed: {name} exit {code}")
        require(digest(args.executable) == executable_sha, "native executable changed during case")
        require(output.stat().st_size <= 4 * 1024 * 1024, "native report exceeds admission")
        report = json.loads(output.read_bytes())
        validate_report(report, args.steps)
        if reference is None:
            reference = report
        for key in ["sourceSha256", "target", "rustc", "mathBackend", "sensorSha256", "parameterCount",
                    "continuationSha256", "consumerWork", "raw", "normalized"]:
            require(reference[key] == report[key], f"capture/worker pressure changed {key}")
        capture = report["capture"]
        require((capture is not None) == enabled, "capture mode mismatch")
        if enabled:
            if captured_reference is None:
                captured_reference = capture
            for key in ["samplesObserved", "sourceSampleSha256", "dueTeacherHistogram", "longestTenTeacherStreakSteps"]:
                require(captured_reference[key] == capture[key], f"capture transport changed {key}")
            if drain_steps == 1:
                require(capture["droppedChunks"] == 0 and capture["exactSamplesMatch"] is True, "fast drain must preserve exact bytes")
                require(captured_reference["drainedSampleSha256"] == capture["drainedSampleSha256"], "workers changed drained bytes")
            else:
                require(capture["byteLimitedDroppedChunks"] > 0 and capture["countLimitedDroppedChunks"] == 0, "slow case must reach actual byte pressure")
                require(capture["queuePeakChunks"] < 8192, "descriptor count must not be the pressure limit")
                require(capture["exactSamplesMatch"] is None, "dropped data cannot receive an exactness pass")
        record = {"name": name, "report": output.name, "reportSha256": digest(output), "workers": workers,
                  "capture": enabled, "nativeP95Ms": report["stepCost"]["p95Ms"], "peakRssBytes": report["peakRssBytes"]}
        if enabled:
            for key in ["samplesPerSimulatedSecond", "longestTenTeacherStreakSteps", "queuePayloadBudgetBytes",
                        "queuePeakChunks", "queuePeakBytes", "droppedChunks", "droppedSamples", "droppedBytes",
                        "countLimitedDroppedChunks", "byteLimitedDroppedChunks", "productionWatermark", "releasedThrough"]:
                record[key] = capture[key]
        records.append(record)
        print(json.dumps(record, sort_keys=True), flush=True)
    require(digest(Path(__file__)) == script_sha, "qualification script changed during cohort")
    result = {"version": 1, "scriptSha256": script_sha, "executableSha256": executable_sha,
              "sourceSha256": reference["sourceSha256"], "commonArguments": common, "records": records,
              "authorityEquivalent": True, "fastCaptureExact": True, "wholeChunkBytePressure": True,
              "caveat": "Actual ten-slot 60 Hz native gameplay, with admitted 0.5-second death gaps. A ten-teacher streak proves only that measured interval; the aggregate rate includes dead/held teachers and is not a sustained 600-row/s pass. Controlled simulated wall times; no Node/spool/HTTP/trainer/shadow, real-time wall-debt or six-hour resource qualification. Native timings are descriptive shared-host results. No production default or resource ceiling changes."}
    with (args.directory / "qualification.json").open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2, allow_nan=False)
        stream.write("\n")
    print(json.dumps({"authorityEquivalent": True, "fastCaptureExact": True, "wholeChunkBytePressure": True}), flush=True)


if __name__ == "__main__":
    main()
