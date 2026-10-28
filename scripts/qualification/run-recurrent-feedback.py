"""Run the offline feedback investigation against preserved qualification inputs.

Example (paths supplied by the operator, no production configuration access):
  python scripts/qualification/run-recurrent-feedback.py --checkout . \
      --artifacts /path/to/exact-396-motion-b16-v3 --phase full
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--phase", choices=("full", "trace"), required=True)
    parser.add_argument("--trace-windows", type=Path)
    args = parser.parse_args()
    checkout = args.checkout.resolve()
    artifacts = args.artifacts.resolve()
    destination = artifacts / "feedback-investigation-20261028"
    destination.mkdir(exist_ok=True)
    checkpoint = artifacts / "a99cedbfa8fb3d4f7ef028472cb460713ee78f4ed813f1f65122cc0c7feaa160.checkpoint-v3"
    expected = "f2230b4147180cdb32366a283388eac40a1bd3cacf30ca313a5470685c6be935"
    assert digest(checkpoint) == expected
    original_checksums = (artifacts / "SHA256SUMS").read_bytes()
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
    assert not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"], cwd=checkout, text=True).strip()
    env = dict(os.environ)
    env["PATH"] = str(Path.home() / ".cargo/bin") + os.pathsep + env["PATH"]
    env["RUSTFLAGS"] = "-C target-cpu=native"
    progress = {"phase": args.phase, "sourceCommit": commit, "defaultBackend": "rust-sse2-v1"}

    def run(name, command):
        progress.update(stage=name, status="running", updatedUnixSeconds=time.time())
        (destination / f"{args.phase}-progress.json").write_text(json.dumps(progress, indent=2) + "\n")
        print(name, "running", flush=True)
        started = time.monotonic()
        with (destination / f"{name}.log").open("w") as log:
            result = subprocess.run(command, cwd=checkout, env=env, stdout=log, stderr=subprocess.STDOUT, check=False)
        receipt = {"command": command, "sourceCommit": commit, "exitCode": result.returncode,
                   "wallSeconds": time.monotonic() - started, "rustflags": env["RUSTFLAGS"]}
        (destination / f"{name}-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if result.returncode:
            progress.update(status="failed", exitCode=result.returncode)
            (destination / f"{args.phase}-progress.json").write_text(json.dumps(progress, indent=2) + "\n")
            raise RuntimeError(f"{name} failed; see its log")
        print(name, "passed", flush=True)

    binary = checkout / "native/target/release/recurrent-feedback-investigation"
    common = [str(binary), "--checkpoint", str(checkpoint), "--observation-bank", str(artifacts / "real-sse2-w7.json"), "--workers", "7"]
    if args.phase == "full":
        run("focused-tests", ["cargo", "test", "--manifest-path", "native/Cargo.toml", "--release", "--features", "engine-test-hooks", "feedback_diagnostics", "--lib"])
        run("build-feedback", ["cargo", "build", "--manifest-path", "native/Cargo.toml", "--release", "--features", "engine-test-hooks", "--bin", "recurrent-feedback-investigation"])
        run("smoke", [*common, "--deliveries", "32", "--slots", "24,30,35", "--output", str(destination / "smoke")])
        run("full", [*common, "--deliveries", "10000", "--output", str(destination / "full")])
    else:
        assert args.trace_windows
        run("trace", [*common, "--deliveries", "10000", "--slots", "24,30,35", "--trace-windows", str(args.trace_windows.resolve()), "--output", str(destination / "trace")])
    assert digest(checkpoint) == expected
    assert (artifacts / "SHA256SUMS").read_bytes() == original_checksums
    progress.update(stage="complete", status="passed", updatedUnixSeconds=time.time(), originalArtifactManifestUnchanged=True)
    (destination / f"{args.phase}-progress.json").write_text(json.dumps(progress, indent=2) + "\n")


if __name__ == "__main__":
    main()
