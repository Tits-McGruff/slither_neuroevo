"""Compare repeated long native workloads without assuming cross-build trajectory equivalence."""
import hashlib
import importlib.util
import json
from pathlib import Path
import statistics
import subprocess
import time

root = Path.cwd()
scratch = root / "native/target/ray-bin-comparison-20261018"
spec = importlib.util.spec_from_file_location("comparison_helpers", root / "scripts/qualification/compare-sensor-workloads.py")
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
expected = {"baseline": "0f7dcaeea16629d01b0ff679a15e724c5099eda121e3776bd829c9016f99cfb1", "candidate": "e56a9c7324149cab36e1ace1849f543494dfb50a82786b722c91ecf9f7144615"}
binaries = {role: scratch / (role + ".exe") for role in expected}
fingerprints = {role: helpers.digest(path) for role, path in binaries.items()}
rows = []
witnesses = {}
for index, role in enumerate(["baseline", "candidate", "candidate", "baseline"]):
    stem = str(index + 1) + "-" + role
    output = scratch / (stem + ".json")
    workspace = scratch / (stem + ".workspace")
    helpers.require(not output.exists() and not workspace.exists(), "exclusive case output required")
    args = [str(binaries[role]), "--scenario", "P1", "--profile", "motion", "--bins", "16", "--graph", "modest", "--respawn", "on", "--seed", "42", "--workers", "5", "--math-backend", "sse2", "--minimum", "1", "--cap", "256", "--batch", "65536", "--baseline", "v2-stochastic", "--baseline-cadence", "0.016666666666666666", "--capture", "on", "--warmup", "600", "--steps", "3600", "--workspace", str(workspace), "--output", str(output)]
    before = helpers.child_cpu_before()
    started = time.monotonic()
    process = subprocess.Popen(args, cwd=root, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        stdout, stderr = process.communicate(timeout=900)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        raise
    cpu = helpers.child_cpu_after(process, before)
    helpers.require(process.returncode == 0, "native case failed: " + stderr.decode(errors="replace")[-4096:])
    helpers.require(len(stdout) + len(stderr) <= 256 * 1024, "native output admission")
    helpers.require(helpers.digest(binaries[role]) == fingerprints[role], "executable changed during measurement")
    helpers.require(output.stat().st_size <= 4 * 1024 ** 2, "report admission")
    report = json.loads(output.read_bytes())
    helpers.require(report["admitted"] and report["version"] == 5 and report["sourceSha256"] == expected[role], "source-identified admitted report required")
    helpers.require(report["capture"]["exactSamplesMatch"] is True and report["capture"]["queueAccountingMatches"], "committed capture must reconcile")
    witness = {key: report[key] for key in ["continuationSha256", "consumerWork", "raw", "normalized"]}
    witness["capture"] = {key: report["capture"][key] for key in ["sourceSampleSha256", "drainedSampleSha256", "samplesObserved", "samplesDrained"]}
    if role in witnesses:
        helpers.require(witness == witnesses[role], "same-build trajectory, raw samples, work and capture must repeat exactly")
    else:
        witnesses[role] = witness
    row = {"role": role, "report": output.name, "reportSha256": helpers.digest(output), "wallSeconds": time.monotonic() - started, "cpuSeconds": cpu, "meanMs": report["nativePhaseCosts"]["totalMicros"] / 3600 / 1000, "p95Ms": report["stepCost"]["p95Ms"], "allObservations": sum(item["observations"] for item in report["consumerWork"]), "incompleteFraction": report["allConsumerRayIncompleteFraction"], "qualityGate": report["allConsumerRayIncompleteBelowOnePercent"], "continuationSha256": report["continuationSha256"]}
    rows.append(row)
    print(json.dumps(row), flush=True)
summary = {"version": 1, "sources": expected, "binarySha256": fingerprints, "scope": "One seed and shared Windows host; Motion B16 modest graph, five workers, 600 warmup and 3600 measured steps. Exact 60-Hz baseline cadence and unchanged ray limits. Changed truncation can change gameplay, so cross-build trajectories need not agree and timing is descriptive. This is not real-clock, trained-policy, tactical or default-freeze qualification.", "reproducibleWithinEachBuild": True, "crossBuildContinuationEqual": witnesses["baseline"]["continuationSha256"] == witnesses["candidate"]["continuationSha256"], "rows": rows, "medians": {role: {key: statistics.median(row[key] for row in rows if row["role"] == role) for key in ["meanMs", "p95Ms", "cpuSeconds", "incompleteFraction"]} for role in expected}}
with (scratch / "summary.json").open("x", encoding="utf-8") as stream:
    json.dump(summary, stream, indent=2)
    stream.write("\n")
print(json.dumps(summary["medians"]), flush=True)
