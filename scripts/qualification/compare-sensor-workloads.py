"""Compare equivalent native sensor builds with adjacent ABBA workload runs."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import time


def require(condition, message):
    """Keep comparison checks active under every Python optimization mode."""
    if not condition:
        raise RuntimeError(message)


def digest(path):
    """Hash immutable artifact bytes without retaining the entire executable."""
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def child_cpu_before():
    """Snapshot cumulative completed-child CPU time on Unix."""
    if os.name == "nt":
        return None
    import resource  # pylint: disable=import-outside-toplevel
    usage = resource.getrusage(resource.RUSAGE_CHILDREN)
    return usage.ru_utime + usage.ru_stime


def child_cpu_after(process, before):
    """Read only this owned child's CPU, including all native worker threads."""
    if os.name != "nt":
        return child_cpu_before() - before
    from ctypes import wintypes  # pylint: disable=import-outside-toplevel
    handle = getattr(process, "_handle", None)
    if handle is None:
        return None
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    query = kernel.GetProcessTimes
    query.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    query.restype = wintypes.BOOL
    created, exited, system, user = [wintypes.FILETIME() for _ in range(4)]
    if not query(int(handle), ctypes.byref(created), ctypes.byref(exited), ctypes.byref(system), ctypes.byref(user)):
        raise ctypes.WinError(ctypes.get_last_error())
    ticks = lambda item: (item.dwHighDateTime << 32) + item.dwLowDateTime
    return (ticks(system) + ticks(user)) / 10_000_000.0


def parse_args():
    """Require explicit binaries and an exclusively created report directory."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--baseline-source", required=True)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--scenario", choices=["P0", "P1"], default="P1")
    parser.add_argument("--workers", type=int, default=3)
    parser.add_argument("--steps", type=int, default=1200)
    parser.add_argument("--environment", choices=["development", "oxygen-debian"], default="development")
    parser.add_argument("--deadline-seconds", type=int, default=900)
    args = parser.parse_args()
    if len(args.baseline_source) != 64 or any(c not in "0123456789abcdef" for c in args.baseline_source):
        parser.error("baseline source requires exact lowercase SHA-256")
    if not 1 <= args.steps <= 59995 or not 1 <= args.workers <= 512 or not 1 <= args.deadline_seconds <= 3600:
        parser.error("bounded steps/workers/deadline required")
    args.baseline = args.baseline.resolve(strict=True)
    args.candidate = args.candidate.resolve(strict=True)
    if not args.baseline.is_file() or not args.candidate.is_file() or args.baseline == args.candidate:
        parser.error("distinct regular native executables required")
    args.directory = args.directory.resolve()
    args.directory.mkdir()  # Exclusive; never reuse or remove owner files.
    return args


def main():
    """Run fixed semantics-preserving comparisons; no result relaxes qualification gates."""
    script_sha = digest(Path(__file__))
    args = parse_args()
    binaries = {"baseline": args.baseline, "candidate": args.candidate}
    binary_sha = {name: digest(path) for name, path in binaries.items()}
    sources = {"baseline": args.baseline_source}
    common = ["--scenario", args.scenario, "--profile", "motion", "--bins", "16", "--graph", "modest",
              "--respawn", "on", "--seed", "42", "--workers", str(args.workers), "--math-backend", "sse2",
              "--minimum", "1", "--cap", "256", "--batch", "65536", "--baseline", "v2-stochastic",
              "--baseline-cadence", "0.016666666666666666", "--capture", "on", "--warmup", "5",
              "--steps", str(args.steps), "--environment", args.environment]
    records = []
    reference = None
    for index, role in enumerate(["baseline", "candidate", "candidate", "baseline"]):
        require(digest(binaries[role]) == binary_sha[role], "executable changed during comparison")
        stem = f"{index}-{role}"
        output = args.directory / (stem + ".json")
        command = [str(binaries[role]), *common, "--workspace", str(args.directory / (stem + "-workspace")), "--output", str(output)]
        before = child_cpu_before()
        started = time.perf_counter()
        process = subprocess.Popen(command, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        try:
            code = process.wait(timeout=args.deadline_seconds)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise
        wall = time.perf_counter() - started
        cpu = child_cpu_after(process, before)
        require(digest(binaries[role]) == binary_sha[role], "executable changed during native run")
        require(code == 0, f"native comparison case failed: {stem} exit {code}")
        require(output.stat().st_size <= 4 * 1024 * 1024, "native report exceeds admission")
        report = json.loads(output.read_bytes())
        require(report["admitted"] and report["version"] == 4, "admitted version-4 workload required")
        require(report["capture"]["exactSamplesMatch"] is True and report["capture"]["queueAccountingMatches"], "exact capture and ownership reconciliation required")
        sources.setdefault(role, report["sourceSha256"])
        require(sources[role] == report["sourceSha256"], "wrong or changing source identity")
        if role == "candidate":
            require(sources[role] != sources["baseline"], "candidate is the baseline build")
        if reference is None:
            reference = report
        for key in ["target", "rustc", "mathBackend", "sensorSha256", "parameterCount", "continuationSha256", "consumerWork", "normalized"]:
            require(reference[key] == report[key], f"comparison changed {key}")
        for key in ["sourceSampleSha256", "drainedSampleSha256"]:
            require(reference["capture"][key] == report["capture"][key], f"comparison changed {key}")
        phases = report["nativePhaseCosts"]
        require(phases["samples"] == args.steps, "phase sample count mismatch")
        require(sum(value for key, value in phases.items() if key not in ["samples", "totalMicros"]) == phases["totalMicros"], "phase costs do not partition service")
        record = {"role": role, "report": output.name, "reportSha256": digest(output), "sourceSha256": report["sourceSha256"],
                  "binarySha256": binary_sha[role], "nativeMeanMs": phases["totalMicros"] / args.steps / 1000,
                  "sensingMeanMs": phases["sensingMicros"] / args.steps / 1000, "nativeP95Ms": report["stepCost"]["p95Ms"],
                  "processCpuSeconds": cpu, "processWallSeconds": wall, "effectiveProcessCores": None if cpu is None else cpu / wall}
        records.append(record)
        print(json.dumps(record, sort_keys=True), flush=True)
    medians = {role: {key: statistics.median(row[key] for row in records if row["role"] == role)
                      for key in ["nativeMeanMs", "sensingMeanMs", "nativeP95Ms"]} for role in binaries}
    require(digest(Path(__file__)) == script_sha, "comparison script changed during run")
    result = {"version": 1, "scriptSha256": script_sha, "arguments": common, "order": [row["role"] for row in records],
              "equivalent": True, "records": records, "medians": medians,
              "caveat": "Adjacent ABBA process runs on a shared host; descriptive timing evidence, not a release pass or statistical confidence claim. Native reports exclude observer/startup from step cost; child CPU/wall includes the entire CLI process. Exact equality is asserted for these same-target workloads only, not promised across revisions/platforms. Reports and executables are immutable during the comparison; no owner database, controller, or runtime defaults change."}
    with (args.directory / "comparison.json").open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2, allow_nan=False)
        stream.write("\n")
    print(json.dumps({"medians": medians, "equivalent": True}), flush=True)


if __name__ == "__main__":
    main()
