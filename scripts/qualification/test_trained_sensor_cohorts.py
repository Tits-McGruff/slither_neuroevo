"""Opaque learned-artifact restore boundaries; graph/weight admission stays in Rust."""
import gzip
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("trained_sensor_cohorts", Path(__file__).with_name("trained-sensor-cohorts.py"))
DRIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DRIVER)


class ArtifactRestoreTests(unittest.TestCase):
    """Reject altered or escaped receipts before publishing any restored input."""

    def setUp(self):
        """Allocate only an exclusively owned small study tree."""
        self.temporary = tempfile.TemporaryDirectory(prefix="slither-cohort-restore-")
        self.root = Path(self.temporary.name).resolve(strict=True)
        self.raw = b"opaque-native-artifact"
        self.sha = hashlib.sha256(self.raw).hexdigest()
        self.compressed = gzip.compress(self.raw, mtime=0)
        (self.root / "objects").mkdir()
        self.relative = "seed-1-fold-2/full/trained.slither-brain"
        self.archive = "objects/" + self.sha + ".gz"
        self.record = {"path": self.relative, "archive": self.archive, "bytes": len(self.raw),
                       "sha256": self.sha, "gzipSha256": hashlib.sha256(self.compressed).hexdigest()}
        (self.root / self.archive).write_bytes(self.compressed)

    def tearDown(self):
        """Remove only the temporary tree owned by this test."""
        self.temporary.cleanup()

    def restore(self, records=None, name="restored.slither-brain", replicated=True):
        """Exercise the real byte-only restore with isolated output."""
        return DRIVER.extract_artifact(self.root, {"files": records if records is not None else [self.record]},
                                       self.relative, self.root / name, replicated)

    def test_restores_exact_bytes_and_preserves_exclusive_output_for_both_formats(self):
        """Both existing archives and replicated content objects remain supported."""
        identity = self.restore()
        self.assertEqual(identity["artifactSha256"], self.sha)
        self.assertEqual((self.root / "restored.slither-brain").read_bytes(), self.raw)
        with self.assertRaises(FileExistsError):
            self.restore()
        self.record["archive"] = self.relative + ".gz"
        legacy = self.root / self.record["archive"]
        legacy.parent.mkdir(parents=True)
        legacy.write_bytes(self.compressed)
        self.restore(name="legacy.slither-brain", replicated=False)
        self.assertEqual((self.root / "legacy.slither-brain").read_bytes(), self.raw)

    def test_rejects_duplicate_missing_receipts_and_traversal(self):
        """Receipt selection and content paths cannot redirect the restore."""
        for records in [[], [self.record, self.record], [dict(self.record, archive="../outside.gz")]]:
            with self.assertRaises(RuntimeError):
                self.restore(records)
        self.assertFalse((self.root / "restored.slither-brain").exists())

    def test_rejects_digest_length_and_expansion_damage_before_publication(self):
        """Both encoded and decoded identity and byte bounds are checked."""
        bad = [dict(self.record, bytes=value) for value in [True, 0, -1, 64 * 1024**2 + 1]]
        bad += [dict(self.record, bytes=len(self.raw) - 1), dict(self.record, bytes=len(self.raw) + 1),
                dict(self.record, gzipSha256="0" * 64), dict(self.record, sha256="../bad")]
        for record in bad:
            with self.assertRaises(RuntimeError):
                self.restore([record])
        changed = gzip.compress(b"x" * len(self.raw), mtime=0)
        (self.root / self.archive).write_bytes(changed)
        self.record["gzipSha256"] = hashlib.sha256(changed).hexdigest()
        with self.assertRaises(RuntimeError):
            self.restore()
        self.assertFalse((self.root / "restored.slither-brain").exists())


if __name__ == "__main__":
    unittest.main()
