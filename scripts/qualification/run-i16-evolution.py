"""Paired sustained evolution of the existing SSE2 and full matrix-i16 backends.

Linux qualification only. Use a clean experiment checkout and the preserved
exact-396 artifact directory. A new output directory is mandatory. No production
configuration is read or written, and no equality of descendants is required.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

SEEDS = (104729, 130363, 155921, 196613, 262147)
BACKENDS = {"sse2": "rust-sse2-v1", "i16-matrix-fma128": "rust-i16-matrix-fma128-v1"}
CHECKPOINT = "a99cedbfa8fb3d4f7ef028472cb460713ee78f4ed813f1f65122cc0c7feaa160.checkpoint-v3"
CHECKPOINT_SHA = "f2230b4147180cdb32366a283388eac40a1bd3cacf30ca313a5470685c6be935"
GRAPH_SHA = "87d6630df32fd9fea900835db0daee478f799507d3fedb242ad63680640fbfd0"


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def cpu():
    return list(map(int, Path("/proc/stat").read_text().splitlines()[0].split()[1:9]))


def save(path, value):
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def manifest(root):
    files = sorted(p for p in root.rglob("*") if p.is_file() and p != root / "SHA256SUMS" and not p.name.endswith(".tmp"))
    (root / "SHA256SUMS").write_text("".join(f"{digest(p)}  {p.relative_to(root).as_posix()}\n" for p in files))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    checkout, artifacts, output = args.checkout.resolve(), args.artifacts.resolve(), args.output.resolve()
    checkpoint = artifacts / CHECKPOINT
    assert digest(checkpoint) == CHECKPOINT_SHA
    # Preserve the established three artifact sets. Their manifests are immutable.
    preserved = {p: p.read_bytes() for p in (artifacts / "SHA256SUMS", artifacts / "feedback-investigation-20261028/SHA256SUMS", artifacts / "mixed-affine-20261028/SHA256SUMS")}
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
    assert not subprocess.check_output(["git", "status", "--porcelain"], cwd=checkout, text=True).strip()
    output.mkdir(exist_ok=False)
    env = dict(os.environ)
    env["PATH"] = str(Path.home() / ".cargo/bin") + os.pathsep + env["PATH"]
    env["RUSTFLAGS"] = "-C target-cpu=native"
    temporary = output / "tmp"
    temporary.mkdir()
    env["TMPDIR"] = str(temporary)
    progress = {"sourceCommit": commit, "stage": "starting", "completedRuns": 0}
    cpu_name = next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name"))
    save(output / "configuration.json", {"sourceCommit": commit, "startingCommit": "a98e1aa3671e8b819acef588ccc4e23357f187dc", "checkpointSha256": CHECKPOINT_SHA,
        "graphLayoutSha256": GRAPH_SHA, "population": 85, "parametersPerGenome": 1397096, "workers": 7, "seeds": SEEDS,
        "additionalGenerations": 20, "recordedGenerationSeconds": 480, "clock": "controlled; ordinary complete fixed steps at 60 Hz",
        "sensors": "Motion/B16 with body/wall rays, 264 inputs", "backendIdentities": BACKENDS, "defaultBackend": "rust-sse2-v1",
        "compilerFlags": env["RUSTFLAGS"], "cpuModel": cpu_name, "logicalCpus": os.cpu_count(),
        "rustc": subprocess.check_output(["rustc", "--version"], env=env, text=True).strip(),
        "comparison": "paired evolutionary outcomes, never descendant/hash/trajectory equality", "withinPairOrder": "alternates SSE2-first and i16-first"})

    def stage(name, status, **fields):
        progress.update(stage=name, status=status, updatedUnixSeconds=time.time(), **fields)
        save(output / "progress.json", progress)
        print(name, status, flush=True)

    def run(name, command, directory=None, validator=None):
        stage(name, "running")
        started, before = time.monotonic(), cpu()
        samples, peak, last_audit = [], 0, None
        with (output / f"{name}.log").open("w") as log:
            process = subprocess.Popen(command, cwd=checkout, env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                while process.poll() is None:
                    try:
                        values = Path(f"/proc/{process.pid}/stat").read_text().rsplit(")", 1)[1].split()
                        rss = int(values[21]) * os.sysconf("SC_PAGE_SIZE")
                        peak = max(peak, rss)
                        sample = {"wallSeconds": time.monotonic() - started, "processCpuSeconds": (int(values[11]) + int(values[12])) / os.sysconf("SC_CLK_TCK"), "rssBytes": rss, "aggregateCpuTicks": cpu()}
                        samples.append(sample)
                    except (FileNotFoundError, ProcessLookupError):
                        pass
                    if directory:
                        initial = directory / "evolution/initial.json"
                        if initial.exists() and last_audit is None:
                            try:
                                value = json.loads(initial.read_text())
                            except json.JSONDecodeError:
                                pass  # Initial file is flushed once, before the first step.
                            else:
                                validator(value)
                                last_audit = value
                        rows = directory / "evolution/generations.jsonl"
                        if rows.exists():
                            count = sum(1 for _ in rows.open())
                            if count != progress.get("completedGenerations", -1):
                                stage(name, "running", completedGenerations=count)
                    time.sleep(5)
                code = process.wait()
            except BaseException:
                process.terminate()
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                raise
        delta = [b - a for a, b in zip(before, cpu())]
        save(output / f"{name}-receipt.json", {"sourceCommit": commit, "command": command, "exitCode": code,
            "wallSeconds": time.monotonic() - started, "rustflags": env["RUSTFLAGS"], "sampledPeakRssBytes": peak,
            "aggregateCpuTicksDelta": delta, "aggregateCpuStealFraction": delta[7] / sum(delta) if sum(delta) else None, "samples": samples})
        if code:
            raise RuntimeError(f"{name} exited {code}; preserved its log and partial artifacts")
        if directory and last_audit is None:
            validator(json.loads((directory / "evolution/initial.json").read_text()))
        stage(name, "passed")

    common_hashes, common_settings, source_sha, contract_sha = None, None, None, None

    def validate_initial(value, seed, backend):
        nonlocal common_hashes, common_settings, source_sha, contract_sha
        assert value["backend"] == BACKENDS[backend] and str(value["seed"]) == str(seed)
        assert value["initialGeneration"] == 1 and value["generationSeconds"] == 480
        assert value["graphLayoutSha256"] == GRAPH_SHA and value["parametersPerGenome"] == 1397096
        genomes = value["population"]["genomes"]
        assert len(genomes) == 85 and all(len(g["matrices"]) == 12 for g in genomes)
        assert value["population"]["aggregateQuantization"]["clippingCount"] == 0
        hashes = [g["canonicalWeightsSha256"] for g in genomes]
        settings = value["settings"]
        if common_hashes is None:
            common_hashes, common_settings = hashes, settings
            source_sha, contract_sha = value["nativeSourceSha256"], value["buildContractSha256"]
        assert hashes == common_hashes, "starting canonical populations differ"
        assert settings == common_settings, "paired evolutionary settings differ"
        assert value["nativeSourceSha256"] == source_sha and value["buildContractSha256"] == contract_sha

    try:
        run("build", ["cargo", "build", "--manifest-path", "native/Cargo.toml", "--release", "--features", "engine-test-hooks", "--bin", "sensor-v4-qualification"])
        # Finish native validation before any measured process starts.
        for name, command in (
            ("fmt", ["cargo", "fmt", "--manifest-path", "native/Cargo.toml", "--", "--check"]),
            ("check", ["cargo", "check", "--manifest-path", "native/Cargo.toml"]),
            ("clippy", ["cargo", "clippy", "--manifest-path", "native/Cargo.toml", "--all-targets", "--all-features", "--", "-D", "warnings"]),
            ("tests-default", ["cargo", "test", "--manifest-path", "native/Cargo.toml"]),
            ("tests-release-hooks", ["cargo", "test", "--manifest-path", "native/Cargo.toml", "--release", "--features", "engine-test-hooks"]),
        ):
            run(name, command)
        binary = str(checkout / "native/target/release/sensor-v4-qualification")
        common = ["--population", "85", "--workers", "7", "--profile", "motion", "--bins", "16", "--respawn", "off",
            "--minimum", "1", "--cap", "256", "--batch", "65536", "--pellet-checks", "3000", "--segment-checks", "2200",
            "--baseline", "v2-stochastic", "--baseline-cadence", "0.016666666666666666", "--environment", "oxygen-debian",
            "--evolved-checkpoint", str(checkpoint), "--clock", "controlled", "--warmup", "0", "--steps", "1200000", "--evolve-generations", "20"]
        for pair, seed in enumerate(SEEDS):
            order = list(BACKENDS) if pair % 2 == 0 else list(reversed(BACKENDS))
            for backend in order:
                name = f"seed-{seed}-{backend}"
                directory = output / name
                directory.mkdir()
                stage(name, "starting", completedGenerations=0)
                run(name, [binary, *common, "--seed", str(seed), "--math-backend", backend,
                    "--workspace", str(directory / "workspace"), "--output", str(directory / "report.json"),
                    "--evolution-diagnostics", str(directory / "evolution")], directory,
                    lambda value: validate_initial(value, seed, backend))
                report = json.loads((directory / "report.json").read_text())
                assert report["admitted"] and report["population"] == 85 and report["workers"] == 7
                assert report["parameterCount"] == 1397096 and report["graphLayoutSha256"] == GRAPH_SHA
                sensors = report["sensorContract"]
                assert (sensors["profile"], sensors["bins"], sensors["inputSize"]) == ("motion", 16, 264)
                assert {"body_ray", "wall_ray"} <= {v["label"] for v in sensors["channels"]}
                rows = [json.loads(line) for line in (directory / "evolution/generations.jsonl").read_text().splitlines()]
                assert len(rows) == 20 and [r["generation"] for r in rows] == list(range(1, 21))
                assert report["evolvedPopulation"]["sourceGeneration"] == 21
                for row in rows:
                    assert len(row["fitnessBySlot"]) == 85 and row["normalEngineWallSeconds"] > 0
                    assert row["reproduction"]["eliteCopies"] + row["reproduction"]["offspring"] == 85
                    assert not row["completedLifeRingTruncated"]
                save(directory / "final-checkpoint.json", {"file": rows[-1]["checkpointFile"],
                    "sha256": digest(directory / "evolution" / rows[-1]["checkpointFile"]), "logicalRootSha256": rows[-1]["checkpointLogicalRootSha256"]})
                progress["completedRuns"] += 1
                stage(name, "passed")
                # Small per-run manifest supports off-host preservation between runs.
                manifest(directory)
        assert digest(checkpoint) == CHECKPOINT_SHA
        assert all(p.read_bytes() == content for p, content in preserved.items())
        stage("complete", "passed", completedGenerationsPerRun=20, originalArtifactManifestsUnchanged=True)
    except BaseException as error:
        stage(progress.get("stage", "unknown"), "failed", failure=str(error))
        raise
    finally:
        manifest(output)


if __name__ == "__main__":
    main()
