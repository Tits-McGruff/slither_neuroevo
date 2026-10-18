"""Check large Motion/B32 native gameplay and the current 512-MiB population rejection.

These controlled-step random-population probes do not qualify 1x scheduling,
trained policies, larger checkpoint admission, full-stack capacity or the soak.
"""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess


def require(condition, message):
    """Keep every evidence/admission check active under optimized Python too."""
    if not condition:
        raise RuntimeError(message)


def canonical_digest(path):
    """Bind portable Python source independently of checkout line endings."""
    return hashlib.sha256(path.read_text(encoding="utf-8").encode()).hexdigest()


def arguments():
    """Require an identified native binary and exclusive task-owned report directory."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--source-sha256", required=True)
    parser.add_argument("--environment", choices=("development", "oxygen-debian"), required=True)
    parser.add_argument("--workers", type=int, default=3)
    parser.add_argument("--steps", type=int, default=3600)
    parser.add_argument("--warmup", type=int, default=600)
    parser.add_argument("--deadline-seconds", type=int, default=900)
    args = parser.parse_args()
    if (not 2 <= args.workers <= 512 or not 1200 <= args.steps <= 60000
            or not 0 <= args.warmup <= 60000 or args.steps + args.warmup > 60000
            or not 1 <= args.deadline_seconds <= 3600):
        parser.error("bounded workers, at least 1200 steps and at most 60000 total required")
    if (len(args.source_sha256) != 64
            or any(c not in "0123456789abcdef" for c in args.source_sha256)):
        parser.error("exact lowercase native source identity required")
    args.executable = args.executable.resolve(strict=True)
    if not args.executable.is_file():
        parser.error("native executable must be a regular file")
    args.directory = args.directory.resolve()
    args.directory.mkdir()  # Exclusive; each native child owns and removes only its new workspace.
    return args


def main():
    """Verify early rejection first, then compare complete native one/multiple-worker continuations."""
    args = arguments()
    helper_path = Path(__file__).with_name("qualify-ray-hierarchy.py")
    spec = importlib.util.spec_from_file_location("large_workload_validation", helper_path)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    source = {p.name: canonical_digest(p) for p in (Path(__file__), helper_path)}
    common = ["--profile", "motion", "--bins", "32", "--graph", "large", "--respawn", "on",
              "--seed", "42", "--math-backend", "sse2", "--minimum", "1", "--cap", "256",
              "--batch", "65536", "--baseline", "v2-stochastic", "--baseline-cadence",
              "0.016666666666666666", "--capture", "on", "--environment", args.environment]
    records, reports, target = [], {}, None
    cases = [("large300-admission", "P1", 1, 0, 1),
             ("large55-w1", "P0", 1, args.warmup, args.steps),
             (f"large55-w{args.workers}", "P0", args.workers, args.warmup, args.steps)]
    for name, scenario, workers, warmup, steps in cases:
        output, workspace = args.directory / f"{name}.json", args.directory / f"{name}-workspace"
        subprocess.run([str(args.executable), *common, "--scenario", scenario,
                        "--workers", str(workers), "--warmup", str(warmup), "--steps", str(steps),
                        "--workspace", str(workspace), "--output", str(output)],
                       check=True, timeout=args.deadline_seconds)
        require(output.stat().st_size <= 4 * 1024**2, "native report exceeds bounded metadata")
        raw = output.read_bytes()
        report = json.loads(raw)
        require(report["sourceSha256"] == args.source_sha256, "native source differs; rebuild binary")
        current_target = report["target"], report["rustc"], report["cpuModel"]
        target = target or current_target
        require(target == current_target, "compiler/target/CPU changed between cases")
        require(not workspace.exists(), "native child did not clean its owned workspace")
        require(report["environment"] == args.environment, "wrong platform environment label")
        require(report["graphSize"] == "large" and report["sensorContract"]["inputSize"] == 504,
                "large Motion/B32 graph required")
        if name == "large300-admission":
            require(report["admitted"] is False and report["completedSteps"] == 0,
                    "current large population must reject before complete-step execution")
            require(report["population"] == 300 and "512.00 MiB" in report["rejection"]
                    and "494306 parameters" in report["rejection"], "wrong population admission boundary")
            require(report["observations"] == 0 and report["capture"] is None,
                    "rejected population must not sense or capture")
        else:
            helper.validate_report(report, scenario, "motion", 32, 256, 65536, workers,
                                   steps, warmup, expected_graph="large")
            require(report["parameterCount"] == 494306, "wrong canonical large graph")
            require(report["normalized"] and len(report["normalized"]) == 504,
                    "complete finite Motion/B32 sensor histogram required")
        reports[name] = report
        records.append({"name": name, "report": output.name,
                        "reportSha256": hashlib.sha256(raw).hexdigest(),
                        "admitted": report["admitted"], "peakRssBytes": report["peakRssBytes"],
                        "p95Within60hz": report["nativeStepP95Within60hz"],
                        "rayQualityBelowOnePercent": report["allConsumerRayIncompleteBelowOnePercent"]})
    reference, parallel = reports["large55-w1"], reports[f"large55-w{args.workers}"]
    for key in ["sourceSha256", "target", "rustc", "mathBackend", "sensorSha256", "parameterCount",
                "continuationSha256", "consumerWork", "raw", "normalized"]:
        require(reference[key] == parallel[key], f"worker count changed {key}")
    for key in ["sourceSampleSha256", "drainedSampleSha256", "samplesObserved", "dueTeacherHistogram"]:
        require(reference["capture"][key] == parallel["capture"][key], f"worker count changed capture {key}")
    summary = {"schemaVersion": 1, "gate": "large-motion32-native-workload-v1",
               "checksPassed": True, "releaseQualified": False,
               "nativeSourceSha256": args.source_sha256, "sourceCanonicalLfSha256": source,
               "environment": args.environment, "warmupSteps": args.warmup,
               "measuredSteps": args.steps, "workers": [1, args.workers],
               "checkpointEnvelopeBytes": 512 * 1024**2, "records": records,
               "workerContinuationExact": True,
               "scope": ("Seeded 55-snake large Motion/B32 graph and ten ordinary-delay v2 teachers; "
                         "actual controlled steps, capture and exact one/multiple-worker continuation. "
                         "The 300-snake case qualifies existing early rejection only. No larger native "
                         "checkpoint envelope, trained-policy distribution, 1x wall scheduler, Node/HTTP, "
                         "trainer/shadow coexistence or integrated soak is qualified.")}
    with (args.directory / "summary.json").open("x", encoding="utf-8", newline="\n") as stream:
        json.dump(summary, stream, indent=2, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    print("large_workload_checks_complete", flush=True)


if __name__ == "__main__":
    main()
