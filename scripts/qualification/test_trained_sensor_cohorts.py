"""Opaque learned-artifact restore boundaries; graph/weight admission stays in Rust."""
import hashlib
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("trained_sensor_cohorts", Path(__file__).with_name("trained-sensor-cohorts.py"))
DRIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DRIVER)


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
