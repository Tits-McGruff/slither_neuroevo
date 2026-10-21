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


def original_json(evidence, record):
    """Read one small content-addressed JSON receipt without restoring an older population/corpus."""
    size = record["bytes"]
    require(type(size) is int and 0 <= size <= 4 * 1024**2, "bounded prior JSON required")
    require(all(isinstance(record[key], str) and re.fullmatch(r"[0-9a-f]{64}", record[key])
                for key in ("sha256", "gzipSha256")), "invalid prior digest")
    require(record["archive"] == "objects/" + record["sha256"] + ".gz", "prior content-addressed object required")
    archive = confined(evidence, record["archive"])
    require(archive.stat().st_size <= 4 * 1024**2 and digest(archive) == record["gzipSha256"], "prior compressed identity changed")
    with gzip.open(archive, "rb") as source:
        data = source.read(size + 1)
    require(len(data) == size and hashlib.sha256(data).hexdigest() == record["sha256"], "prior original identity changed")
    return json.loads(data)


def verify_prefix(root, prior, matrix):
    """Recompute observed initial/1,000-update equality from both immutable study receipts."""
    prefix = json.loads((root / "prefix-replay.json").read_bytes())
    prior = prior.resolve(strict=True)
    path = prior / "evidence.json"
    require(path.stat().st_size <= 1024**2 and digest(path) == prefix["previousEvidenceSha256"], "prior evidence identity changed")
    manifest = json.loads(path.read_bytes())
    require(manifest["kind"] == "replicated-ablation-evidence-v1" and manifest["status"] == "measured", "prior measured study required")
    files = manifest["files"]
    require(isinstance(files, list) and len(files) <= 2048, "bounded prior inventory required")
    records = {item["path"]: item for item in files}
    require(len(records) == len(files), "duplicate prior receipt")
    old_corpus = original_json(prior, records["corpus/corpus.json"])
    corpus = json.loads((root / "corpus/corpus.json").read_bytes())
    require(corpus["objects"] == old_corpus["objects"], "packed prefix inputs differ")
    for item in old_corpus["objects"]:
        old_record = records["corpus/" + item["file"]]
        require(old_record["bytes"] == item["bytes"] and old_record["sha256"] == item["sha256"], "prior packed input receipt differs")
    require(prefix["schemaVersion"] == 1 and prefix["update"] == 1000
            and prefix["allPackedInputsAndSequenceReceiptsExact"] is True
            and prefix["allInitialAndPrefixWeightsExact"] is True, "exact prefix witness required")
    expected = []
    for cell in matrix["cells"]:
        relative = f"seed-{cell['seed']}-fold-{cell['validationSeed']}/summary.json"
        old = original_json(prior, records[relative])
        current = json.loads(confined(root, relative).read_bytes())
        require(old["environment"] == current["environment"], "prefix compute environment differs")
        require(old["config"]["updates"] == 1000 and current["config"]["updates"] == matrix["updatesPerVariant"], "prefix update budgets differ")
        for key in ("batch_size", "bptt_steps", "burn_in_steps", "learning_rate", "gradient_clip", "seed", "validationSeed", "nativeSteps", "nativeWorkers"):
            require(old["config"][key] == current["config"][key], "prefix configuration differs")
        previous = {item["variant"]: item for item in old["variants"]}
        require(len(previous) == len(old["variants"]) == len(current["variants"]) == len(matrix["variants"]), "prefix variant exposure differs")
        for item in current["variants"]:
            before = previous[item["variant"]]
            points = [point for point in item["training"]["validationCurve"] if point["update"] == 1000]
            require(len(points) == 1, "one measured 1,000-update point required")
            require(item["initialWeightsSha256"] == before["initialWeightsSha256"]
                    and points[0]["weightsSha256"] == before["trainedWeightsSha256"], "prefix packed weights differ")
            expected.append({"seed": cell["seed"], "validationSeed": cell["validationSeed"], "variant": item["variant"],
                             "initialWeightsSha256": item["initialWeightsSha256"], "weightsAt1000Sha256": points[0]["weightsSha256"]})
    require(prefix["comparisons"] == expected, "prefix witness differs from original paired receipts")
    return len(expected)


def verify(evidence, previous_evidence=None):
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
        prefix = root / "prefix-replay.json"
        require(prefix.is_file() == (previous_evidence is not None), "prefix witness and explicit prior evidence must be supplied together")
        prefix_comparisons = verify_prefix(root, previous_evidence, matrix) if previous_evidence is not None else None
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
                "releaseQualified": matrix["releaseQualified"], "maturityQualified": matrix["maturityQualified"],
                **({"prefixComparisonsExact": prefix_comparisons} if prefix_comparisons is not None else {})}


def main():
    """Require the retained evidence directory explicitly; no output can replace owner state."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--previous-evidence", type=Path)
    args = parser.parse_args()
    print(json.dumps(verify(args.evidence, args.previous_evidence)))


if __name__ == "__main__":
    main()
