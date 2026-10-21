"""Boundary regressions for the retained-evidence restore, without running archived code."""
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("verify_replicated_ablation", Path(__file__).with_name("verify-replicated-ablation.py"))
READER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(READER)


class EvidenceBoundaryTests(unittest.TestCase):
    """Invalid paths, limits and compressed bytes must fail before any archived reader starts."""

    def setUp(self):
        """Create only an exclusively owned, small evidence root."""
        self.temporary = tempfile.TemporaryDirectory(prefix="slither-ablation-reader-test-")
        self.root = Path(self.temporary.name).resolve(strict=True)
        self.assertEqual(self.root.parent, Path(tempfile.gettempdir()).resolve(strict=True))
        (self.root / "objects").mkdir()
        original = b"{}"
        encoded = gzip.compress(original, mtime=0)
        digest = hashlib.sha256(original).hexdigest()
        self.record = {"path": "matrix.json", "archive": "objects/" + digest + ".gz", "bytes": len(original),
                       "sha256": digest, "gzipSha256": hashlib.sha256(encoded).hexdigest()}
        (self.root / self.record["archive"]).write_bytes(encoded)
        self.manifest = {"schemaVersion": 1, "kind": "replicated-ablation-evidence-v1", "status": "measured", "files": [self.record]}
        self.child_patch = patch.object(READER.subprocess, "run")
        self.child = self.child_patch.start()

    def tearDown(self):
        """A malformed archive never executes its claimed code; remove the exact owned root."""
        self.child.assert_not_called()
        self.child_patch.stop()
        self.assertEqual(self.root.resolve(strict=True).parent, Path(tempfile.gettempdir()).resolve(strict=True))
        self.temporary.cleanup()

    def rejected(self):
        """Persist the malformed fixture and require a validation failure."""
        (self.root / "evidence.json").write_text(json.dumps(self.manifest), encoding="utf-8")
        with self.assertRaises(ValueError):
            READER.verify(self.root)

    def test_rejects_aliases_traversal_and_platform_paths(self):
        """Every original path is confined before scratch output can be opened."""
        for name in ("../matrix.json", "/matrix.json", "C:/matrix.json", "a\\matrix.json", ".", "a/./matrix.json", "a//matrix.json"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                READER.confined(self.root, name)
        self.assertEqual(READER.confined(self.root, "cell/full/initial.slither-brain"), self.root / "cell/full/initial.slither-brain")

    def test_rejects_duplicate_paths_and_noninteger_or_oversized_lengths(self):
        """Receipt arithmetic cannot admit duplicate destinations or bool-as-int bounds."""
        for size in (True, -1, 128 * 1024**2 + 1):
            with self.subTest(size=size):
                self.record["bytes"] = size
                self.rejected()
        self.record["bytes"] = 2
        self.manifest["files"].append(dict(self.record))
        self.rejected()

    def test_rejects_changed_compressed_bytes(self):
        """A matching filename cannot substitute for the exact compressed object."""
        self.record["gzipSha256"] = "0" * 64
        self.rejected()

    def test_rejects_expansion_beyond_declared_original_length(self):
        """A small compressed input cannot expand past the admitted receipt."""
        encoded = gzip.compress(b"0123456789", mtime=0)
        (self.root / self.record["archive"]).write_bytes(encoded)
        self.record["gzipSha256"] = hashlib.sha256(encoded).hexdigest()
        self.rejected()

    def test_rejects_original_digest_mismatch_after_valid_decompression(self):
        """Decoded length alone cannot establish the measured original bytes."""
        encoded = gzip.compress(b"[]", mtime=0)
        (self.root / self.record["archive"]).write_bytes(encoded)
        self.record["gzipSha256"] = hashlib.sha256(encoded).hexdigest()
        self.rejected()



class PrefixBoundaryTests(unittest.TestCase):
    """Join verified study metadata without training or executing an older archived reader."""

    def setUp(self):
        """Build a small opaque predecessor and one paired metadata-only prefix."""
        self.temporary = tempfile.TemporaryDirectory(prefix="slither-curve-prefix-test-")
        self.base = Path(self.temporary.name).resolve(strict=True)
        self.prior = self.base / "prior"
        self.root = self.base / "current"
        (self.prior / "objects").mkdir(parents=True)
        (self.root / "corpus").mkdir(parents=True)
        (self.root / "seed-1-fold-2").mkdir()
        self.config = {"batch_size": 32, "bptt_steps": 64, "burn_in_steps": 32,
                       "learning_rate": 0.0003, "gradient_clip": 1, "seed": 1,
                       "validationSeed": 2, "nativeSteps": 600, "nativeWorkers": 1}
        initial = hashlib.sha256(b"initial-opaque-weights").hexdigest()
        trained = hashlib.sha256(b"trained-opaque-weights").hexdigest()
        environment = {"device": "cpu", "threads": 2, "torch": "opaque-test-build"}
        old = {"environment": environment, "config": dict(self.config, updates=1000), "variants": [
            {"variant": "full", "initialWeightsSha256": initial, "trainedWeightsSha256": trained}]}
        self.current = {"environment": dict(environment), "config": dict(self.config, updates=2000), "variants": [
            {"variant": "full", "initialWeightsSha256": initial,
             "training": {"validationCurve": [{"update": 1000, "weightsSha256": trained}]}}]}
        labels = b"data"
        label_sha = hashlib.sha256(labels).hexdigest()
        corpus = {"objects": [{"file": "labels.f32", "bytes": 4, "sha256": label_sha}]}
        files = [self.object("corpus/corpus.json", json.dumps(corpus).encode()),
                 self.object("corpus/labels.f32", labels),
                 self.object("seed-1-fold-2/summary.json", json.dumps(old).encode())]
        manifest = {"kind": "replicated-ablation-evidence-v1", "status": "measured", "files": files}
        (self.prior / "evidence.json").write_text(json.dumps(manifest), encoding="utf-8")
        (self.root / "corpus/corpus.json").write_text(json.dumps(corpus), encoding="utf-8")
        self.prefix = {"schemaVersion": 1, "update": 1000,
                       "previousEvidenceSha256": READER.digest(self.prior / "evidence.json"),
                       "allPackedInputsAndSequenceReceiptsExact": True,
                       "allInitialAndPrefixWeightsExact": True,
                       "comparisons": [{"seed": 1, "validationSeed": 2, "variant": "full",
                                        "initialWeightsSha256": initial, "weightsAt1000Sha256": trained}]}
        self.matrix = {"cells": [{"seed": 1, "validationSeed": 2}], "updatesPerVariant": 2000,
                       "variants": {"full": {}}}
        self.publish()

    def tearDown(self):
        """Remove only this fixture's exclusive root."""
        self.temporary.cleanup()

    def object(self, name, raw):
        """Create an actual content-addressed compressed predecessor receipt."""
        sha = hashlib.sha256(raw).hexdigest()
        encoded = gzip.compress(raw, mtime=0)
        archive = "objects/" + sha + ".gz"
        (self.prior / archive).write_bytes(encoded)
        return {"path": name, "archive": archive, "bytes": len(raw), "sha256": sha,
                "gzipSha256": hashlib.sha256(encoded).hexdigest()}

    def publish(self):
        """Publish changed metadata only within the fixture tree."""
        (self.root / "prefix-replay.json").write_text(json.dumps(self.prefix), encoding="utf-8")
        (self.root / "seed-1-fold-2/summary.json").write_text(json.dumps(self.current), encoding="utf-8")

    def test_recomputes_observed_prefix_from_both_source_receipts(self):
        """Receipt equality is derived, rather than accepted from witness booleans."""
        self.assertEqual(READER.verify_prefix(self.root, self.prior, self.matrix), 1)
        self.prefix["comparisons"].append(self.prefix["comparisons"][0])
        self.publish()
        with self.assertRaisesRegex(ValueError, "witness differs"):
            READER.verify_prefix(self.root, self.prior, self.matrix)

    def test_rejects_changed_packed_weights_and_training_configuration(self):
        """Matching witness text cannot conceal a changed training path."""
        self.current["config"]["gradient_clip"] = 2
        self.publish()
        with self.assertRaisesRegex(ValueError, "configuration differs"):
            READER.verify_prefix(self.root, self.prior, self.matrix)
        self.current["config"]["gradient_clip"] = 1
        self.current["variants"][0]["training"]["validationCurve"][0]["weightsSha256"] = "0" * 64
        self.publish()
        with self.assertRaisesRegex(ValueError, "packed weights differ"):
            READER.verify_prefix(self.root, self.prior, self.matrix)

    def test_rejects_changed_predecessor_and_compressed_source(self):
        """An explicit predecessor is independently bound to its compressed originals."""
        self.prefix["previousEvidenceSha256"] = "0" * 64
        self.publish()
        with self.assertRaisesRegex(ValueError, "prior evidence identity"):
            READER.verify_prefix(self.root, self.prior, self.matrix)
        self.prefix["previousEvidenceSha256"] = READER.digest(self.prior / "evidence.json")
        self.publish()
        manifest = json.loads((self.prior / "evidence.json").read_bytes())
        (self.prior / manifest["files"][0]["archive"]).write_bytes(b"damaged")
        with self.assertRaisesRegex(ValueError, "prior compressed identity"):
            READER.verify_prefix(self.root, self.prior, self.matrix)

if __name__ == "__main__":
    unittest.main()
