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


if __name__ == "__main__":
    unittest.main()
