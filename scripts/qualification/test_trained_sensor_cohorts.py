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

    def test_digest_stream_bounds_reads_including_concurrently_enlarged_inputs(self):
        """Repeated identity checks cannot read an unbounded replacement into memory."""
        path = self.root / "identity.bin"
        path.write_bytes(b"abc")
        self.assertEqual(DRIVER.digest(path, maximum=3), hashlib.sha256(b"abc").hexdigest())
        path.write_bytes(b"abcd")
        with self.assertRaises(RuntimeError):
            DRIVER.digest(path, maximum=3)

    def test_native_debug_binary_can_exceed_artifact_bound_without_widening_imports(self):
        """Large Linux executable hashing remains streaming; brain byte limits stay fixed."""
        path = self.root / "synthetic-debug-binary"
        with path.open("wb") as stream:
            stream.truncate(64 * 1024**2 + 1)
        self.assertEqual(len(DRIVER.executable_digest(path)), 64)
        with self.assertRaises(RuntimeError):
            DRIVER.digest(path)
        with path.open("wb") as stream:
            stream.truncate(512 * 1024**2 + 1)
        with self.assertRaises(RuntimeError):
            DRIVER.executable_digest(path)

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


class SensorReportTests(unittest.TestCase):
    """Reject requested/native/source identity drift before comparing measurements."""

    def report(self, profile, bins, replicated=False):
        """Model the small public identity surface emitted after native admission."""
        width = 24 + (6 if profile == "basic" else 15) * bins
        descriptor = {"inputSize": width, "sensorSha256": "a" * 64}
        result = {"sensorContract": {"layoutVersion": 4, "profile": profile,
                                    "bins": bins, "inputSize": width},
                  "sensorSha256": "a" * 64}
        if replicated:
            result["trainedCohort"] = {"families": [{"artifact": dict(descriptor)},
                                                   {"artifact": dict(descriptor)}]}
        else:
            result["trainedArtifact"] = descriptor
        return result

    def test_all_admitted_widths_and_source_modes_bind_exact_native_descriptor(self):
        """Every B8..32 request uses the requested profile and every family descriptor."""
        for profile in ("basic", "motion"):
            for bins in range(8, 33):
                for replicated in (False, True):
                    report = self.report(profile, bins, replicated)
                    self.assertEqual(DRIVER.validate_sensor_report(report, profile, bins, replicated),
                                     "a" * 64)

    def test_profile_width_version_or_digest_drift_rejects(self):
        """Matching array sizes alone cannot reinterpret an artifact sensor contract."""
        for key, value in (("profile", "motion"), ("bins", 9), ("bins", True),
                           ("inputSize", 144), ("inputSize", True), ("layoutVersion", 3)):
            report = self.report("basic", 8)
            report["sensorContract"][key] = value
            with self.assertRaises(RuntimeError):
                DRIVER.validate_sensor_report(report, "basic", 8, False)
        for bad_digest in ("A" * 64, "../bad", None):
            report = self.report("motion", 32)
            report["sensorSha256"] = bad_digest
            with self.assertRaises(RuntimeError):
                DRIVER.validate_sensor_report(report, "motion", 32, False)
        for replicated in (False, True):
            for key, value in (("inputSize", 264), ("sensorSha256", "b" * 64)):
                report = self.report("motion", 32, replicated)
                descriptor = (report["trainedCohort"]["families"][-1]["artifact"]
                              if replicated else report["trainedArtifact"])
                descriptor[key] = value
                with self.assertRaises(RuntimeError):
                    DRIVER.validate_sensor_report(report, "motion", 32, replicated)
        report = self.report("motion", 32, True)
        report["trainedCohort"]["families"] = []
        with self.assertRaises(RuntimeError):
            DRIVER.validate_sensor_report(report, "motion", 32, True)

    def test_variant_selection_preserves_legacy_and_rejects_unavailable_comparisons(self):
        """Basic and direct artifacts have one Full graph; old Motion studies keep both."""
        self.assertEqual(DRIVER.selected_variants("motion", None), ["full", "core-only"])
        self.assertEqual(DRIVER.selected_variants("basic", None), ["full"])
        self.assertEqual(DRIVER.selected_variants("motion", None, True), ["full"])
        self.assertEqual(DRIVER.selected_variants("motion", ["core-only"]), ["core-only"])
        for profile, variants, direct in (
                ("basic", ["core-only"], False), ("motion", ["core-only"], True),
                ("motion", ["full", "full"], False), ("motion", [], False),
                ("motion", ["other"], False), ("v3", None, False)):
            with self.assertRaises(RuntimeError):
                DRIVER.selected_variants(profile, variants, direct)


if __name__ == "__main__":
    unittest.main()
