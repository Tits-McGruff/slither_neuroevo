"""Qualify the opt-in affine-i16/SSE2-recurrent backend on preserved mature inputs.

Run on the Linux qualification host with --checkout and --artifacts pointing to
the clean experiment checkout and the existing exact-396-motion-b16-v3 folder.
Outputs are generated artifacts; no production configuration is read or written.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def cpu():
    return list(map(int, Path("/proc/stat").read_text().splitlines()[0].split()[1:9]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    checkout = args.checkout.resolve()
    artifacts = args.artifacts.resolve()
    destination = artifacts / "mixed-affine-20261028"
    destination.mkdir(exist_ok=False)
    checkpoint = artifacts / "a99cedbfa8fb3d4f7ef028472cb460713ee78f4ed813f1f65122cc0c7feaa160.checkpoint-v3"
    expected = "f2230b4147180cdb32366a283388eac40a1bd3cacf30ca313a5470685c6be935"
    assert digest(checkpoint) == expected
    preserved = {path: path.read_bytes() for path in [artifacts / "SHA256SUMS", artifacts / "feedback-investigation-20261028/SHA256SUMS"]}
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
    assert not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=checkout, text=True).strip()
    env = dict(os.environ)
    env["PATH"] = str(Path.home() / ".cargo/bin") + os.pathsep + env["PATH"]
    env["RUSTFLAGS"] = "-C target-cpu=native"
    (destination / "tmp").mkdir()
    env["TMPDIR"] = str(destination / "tmp")
    graph = "87d6630df32fd9fea900835db0daee478f799507d3fedb242ad63680640fbfd0"
    progress = {"sourceCommit": commit, "defaultBackend": "rust-sse2-v1"}
    (destination / "configuration.json").write_text(json.dumps({
        **progress, "checkpointSha256": expected, "graphLayoutSha256": graph,
        "population": 85, "workers": 7, "sensors": "Motion/B16, body and wall rays, 264 inputs",
        "measuredSimulationSecondsPerBackend": 1000, "warmupSteps": 300,
        "compilerFlags": env["RUSTFLAGS"], "isolatedWarmup": 50, "isolatedPasses": 500,
        "isolatedRepetitions": 8, "feedbackDeliveries": 10000,
    }, indent=2) + "\n")

    def stage(name, status, **fields):
        progress.update(stage=name, status=status, updatedUnixSeconds=time.time(), **fields)
        temporary = destination / "progress.tmp"
        temporary.write_text(json.dumps(progress, indent=2) + "\n")
        temporary.replace(destination / "progress.json")
        print(name, status, flush=True)

    def run(name, command):
        stage(name, "running")
        started = time.monotonic()
        before = cpu()
        samples = []
        peak = 0
        with (destination / f"{name}.log").open("w") as log:
            process = subprocess.Popen(command, cwd=checkout, env=env, stdout=log, stderr=subprocess.STDOUT)
            while process.poll() is None:
                try:
                    values = Path(f"/proc/{process.pid}/stat").read_text().rsplit(")", 1)[1].split()
                    rss = int(values[21]) * os.sysconf("SC_PAGE_SIZE")
                    peak = max(peak, rss)
                    samples.append({"wallSeconds": time.monotonic() - started,
                                    "processCpuSeconds": (int(values[11]) + int(values[12])) / os.sysconf("SC_CLK_TCK"),
                                    "rssBytes": rss, "aggregateCpuTicks": cpu()})
                except (FileNotFoundError, ProcessLookupError):
                    pass
                time.sleep(1)
            code = process.wait()
        delta = [b - a for a, b in zip(before, cpu())]
        receipt = {"command": command, "sourceCommit": commit, "exitCode": code,
                   "wallSeconds": time.monotonic() - started, "rustflags": env["RUSTFLAGS"],
                   "sampledPeakRssBytes": peak, "aggregateCpuTicksDelta": delta,
                   "aggregateCpuStealFraction": delta[7] / sum(delta) if sum(delta) else None,
                   "samples": samples}
        (destination / f"{name}-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if code:
            stage(name, "failed", exitCode=code)
            raise RuntimeError(f"{name} failed; see its log")
        stage(name, "passed")

    def load(name):
        return json.loads((destination / name).read_text())

    def validate_game(report, steps):
        assert report["admitted"], report.get("rejection")
        assert report["population"] == 85 and report["parameterCount"] == 1397096
        assert report["graphLayoutSha256"] == graph
        sensor = report["sensorContract"]
        assert sensor["profile"] == "motion" and sensor["bins"] == 16 and sensor["inputSize"] == 264
        assert {"body_ray", "wall_ray"} <= {v["label"] for v in sensor["channels"]}
        assert (report["rayMinimum"], report["rayCap"], report["rayBatch"]) == (1, 256, 65536)
        assert report["workers"] in (1, 7) and report["completedSteps"] == steps

    try:
        run("build", ["cargo", "build", "--manifest-path", "native/Cargo.toml", "--release", "--features", "engine-test-hooks",
                      "--bin", "sensor-v4-qualification", "--bin", "evolved-i16-qualification"])
        qualifier = str(checkout / "native/target/release/sensor-v4-qualification")
        benchmark = str(checkout / "native/target/release/evolved-i16-qualification")
        common = ["--population", "85", "--profile", "motion", "--bins", "16", "--respawn", "off", "--seed", "42",
                  "--minimum", "1", "--cap", "256", "--batch", "65536", "--pellet-checks", "3000", "--segment-checks", "2200",
                  "--baseline", "v2-stochastic", "--baseline-cadence", "0.016666666666666666", "--environment", "oxygen-debian",
                  "--evolved-checkpoint", str(checkpoint)]
        for workers in (1, 7):
            name = f"controlled-mixed-w{workers}"
            run(name, [qualifier, *common, "--math-backend", "i16-affine-fma128-sse2", "--workers", str(workers),
                       "--clock", "controlled", "--warmup", "30", "--steps", "600",
                       "--workspace", str(destination / f"{name}-workspace"), "--output", str(destination / f"{name}.json")])
            validate_game(load(f"{name}.json"), 630)
        assert load("controlled-mixed-w1.json")["continuationSha256"] == load("controlled-mixed-w7.json")["continuationSha256"]
        run("evolved-inference", [benchmark, "--checkpoint", str(checkpoint), "--observation-bank", str(artifacts / "real-sse2-w7.json"),
                                 "--candidate", "i16-affine-fma128-sse2", "--brains", "85", "--workers", "7", "--warmup", "50",
                                 "--passes", "500", "--repetitions", "8", "--deliveries", "10000", "--require-exact-recurrent", "true",
                                 "--output", str(destination / "evolved-inference")])
        result = load("evolved-inference/report.json")
        assert result["graphLayoutSha256"] == graph and result["parametersPerGenome"] == 1397096
        assert result["brains"] == 85 and result["workers"] == 7 and result["requireExactRecurrent"]
        for name, backend in [("real-sse2-w7", "sse2"), ("real-mixed-w7", "i16-affine-fma128-sse2"), ("real-i16-w7", "i16-matrix-fma128")]:
            run(name, [qualifier, *common, "--math-backend", backend, "--workers", "7", "--clock", "real", "--warmup", "300", "--steps", "60000",
                       "--workspace", str(destination / f"{name}-workspace"), "--output", str(destination / f"{name}.json")])
            validate_game(load(f"{name}.json"), 60300)
        # Validation runs after the measured workloads, never concurrently with them.
        for name, command in [
            ("fmt", ["cargo", "fmt", "--manifest-path", "native/Cargo.toml", "--", "--check"]),
            ("check", ["cargo", "check", "--manifest-path", "native/Cargo.toml"]),
            ("tests-release", ["cargo", "test", "--manifest-path", "native/Cargo.toml", "--release", "--features", "engine-test-hooks"]),
            ("clippy", ["cargo", "clippy", "--manifest-path", "native/Cargo.toml", "--all-targets", "--all-features", "--", "-D", "warnings"]),
        ]:
            run(name, command)
        assert digest(checkpoint) == expected
        assert all(path.read_bytes() == contents for path, contents in preserved.items())
        stage("complete", "passed", worker1And7ContinuationMatched=True, originalArtifactManifestsUnchanged=True)
    except Exception as error:
        stage(progress.get("stage", "unknown"), "failed", failure=str(error))
        raise


if __name__ == "__main__":
    main()
