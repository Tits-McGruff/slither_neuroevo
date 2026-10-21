"""Verify retained measured ablations with their archived stdlib-only matrix reader.

This restores only task-owned scratch. It never trains a model or runs simulation.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tempfile


def require(condition, message):
    """Keep evidence checks active with optimized Python as well."""
    if not condition:
        raise ValueError(message)


def digest(path):
    """Stream original artifact identity with bounded memory."""
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            value.update(block)
    return value.hexdigest()


def confined(root, name):
    """Reject traversal, platform aliases and links outside the selected evidence root."""
    require(isinstance(name, str) and re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", name),
            "invalid evidence path")
    relative = PurePosixPath(name)
    require(not relative.is_absolute() and all(part not in (".", "..") for part in name.split("/")),
            "evidence traversal")
    path = root.joinpath(*relative.parts)
    require(path.resolve().is_relative_to(root), "evidence path escapes root")
    return path


def verify(evidence):
    """Restore bounded immutable inputs, recompute all paired cells, and remove exact scratch."""
    evidence = evidence.resolve(strict=True)
    manifest_path = evidence / "evidence.json"
    require(manifest_path.stat().st_size <= 1024**2, "oversized evidence manifest")
    manifest = json.loads(manifest_path.read_bytes())
    require(manifest["schemaVersion"] == 1 and manifest["kind"] == "replicated-ablation-evidence-v1"
            and manifest["status"] == "measured", "completed measured evidence required")
    records = manifest["files"]
    require(isinstance(records, list) and 1 <= len(records) <= 2048, "bounded evidence inventory required")
    paths, total = set(), 0
    for record in records:
        name = record["path"]
        confined(evidence, name)
        require(name not in paths, "duplicate evidence path")
        paths.add(name)
        size = record["bytes"]
        require(type(size) is int and 0 <= size <= 128 * 1024**2, "bounded original file required")
        total += size
        require(total <= 256 * 1024**2, "total decoded evidence exceeds admission")
        for field in ("sha256", "gzipSha256"):
            require(isinstance(record[field], str) and re.fullmatch(r"[0-9a-f]{64}", record[field]), "invalid digest")
        require(record["archive"] == "objects/" + record["sha256"] + ".gz", "content-addressed object required")
    temporary_root = Path(tempfile.gettempdir()).resolve(strict=True)
    require(shutil.disk_usage(temporary_root).free >= 1024**3 + total, "restore must preserve one GiB disk reserve")
    with tempfile.TemporaryDirectory(prefix="slither-replicated-ablation-") as temporary:
        root = Path(temporary).resolve(strict=True)
        require(root.parent == temporary_root and root.name.startswith("slither-replicated-ablation-"), "foreign scratch")
        for record in records:
            archive = confined(evidence, record["archive"])
            require(archive.is_file() and archive.stat().st_size <= 64 * 1024**2, "bounded compressed object required")
            require(digest(archive) == record["gzipSha256"], "compressed evidence changed")
            output = confined(root, record["path"])
            output.parent.mkdir(parents=True, exist_ok=True)
            count, value = 0, hashlib.sha256()
            with gzip.open(archive, "rb") as source, output.open("xb") as target:
                while block := source.read(65536):
                    count += len(block)
                    require(count <= record["bytes"], "decoded object exceeds receipt")
                    value.update(block)
                    target.write(block)
            require(count == record["bytes"] and value.hexdigest() == record["sha256"], "original evidence changed")
        original = root / "matrix.json"
        require(digest(original) == manifest["matrixSha256"], "original measured matrix changed")
        matrix = json.loads(original.read_bytes())
        corpus_path = root / "corpus/corpus.json"
        require(digest(corpus_path) == matrix["corpusManifestSha256"], "matrix corpus identity changed")
        corpus = json.loads(corpus_path.read_bytes())
        require(corpus["sourceSha256"] == matrix["corpusNativeSourceSha256"] == manifest["nativeSourceSha256"], "native corpus source changed")
        for item in corpus["objects"]:
            path = confined(root / "corpus", item["file"])
            require(path.stat().st_size == item["bytes"] and digest(path) == item["sha256"], "corpus object changed")
        arguments = [sys.executable, "-B", "-m", "pyrl_trainer.ablation_matrix", "--output", str(root / "recomputed.json"),
                     "--updates", str(matrix["updatesPerVariant"])]
        for seed in matrix["initializationSeeds"]:
            arguments.extend(["--initialization-seed", str(seed)])
        for fold in matrix["validationSeeds"]:
            arguments.extend(["--validation-seed", str(fold)])
        for cell in matrix["cells"]:
            arguments.extend(["--study", str(root / f"seed-{cell['seed']}-fold-{cell['validationSeed']}")])
        environment = dict(os.environ, PYTHONPATH=str(root), PYTHONNOUSERSITE="1")
        subprocess.run(arguments, cwd=root, env=environment, check=True, timeout=180)
        require(json.loads((root / "recomputed.json").read_bytes()) == matrix, "recomputed paired matrix differs")
        # The context manager removes only this exclusively created, revalidated root.
        require(root.resolve(strict=True).parent == temporary_root and not root.is_symlink(), "scratch identity changed")
        return {"kind": manifest["kind"], "files": len(records), "cells": matrix["cellCount"],
                "matrixSha256": manifest["matrixSha256"], "recomputedExact": True,
                "releaseQualified": matrix["releaseQualified"], "maturityQualified": matrix["maturityQualified"]}


def main():
    """Require the retained evidence directory explicitly; no output can replace owner state."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(verify(args.evidence)))


if __name__ == "__main__":
    main()
