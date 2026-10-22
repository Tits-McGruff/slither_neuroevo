"""Retained native sensing measurements are exact workload evidence, not a release pass."""
import gzip
import hashlib
import json
from pathlib import Path
import statistics
import unittest

ROOT = Path(__file__).resolve().parents[2] / "docs/performance/sensor-v4/sensing-profile-debian-v1"


class SensingProfileEvidenceTests(unittest.TestCase):
    """Recompute compact receipts, fixed-policy continuations and descriptive ABBA medians."""

    @classmethod
    def setUpClass(cls):
        cls.manifest = json.loads((ROOT / "evidence.json").read_text(encoding="utf-8"))
        cls.data = {}
        for name, receipt in cls.manifest["files"].items():
            assert receipt["bytes"] <= 2 * 1024**2
            encoded = (ROOT / (name + ".gz")).read_bytes()
            assert hashlib.sha256(encoded).hexdigest() == receipt["gzipSha256"]
            with gzip.open(ROOT / (name + ".gz"), "rb") as stream:
                raw = stream.read(receipt["bytes"] + 1)
            assert len(raw) == receipt["bytes"]
            assert hashlib.sha256(raw).hexdigest() == receipt["sha256"]
            cls.data[name] = raw

    def document(self, name):
        """Decode one checksum-verified retained report."""
        return json.loads(self.data[name])

    def equal_work(self, reports):
        """Compare actual native outputs, work and every captured teacher sample digest."""
        reference = reports[0]
        for report in reports:
            self.assertTrue(report["admitted"])
            self.assertTrue(report["capture"]["exactSamplesMatch"])
            self.assertTrue(report["capture"]["queueAccountingMatches"])
            self.assertEqual(report["capture"]["droppedSamples"], 0)
            self.assertEqual(report["trainedCohort"]["indexSha256"],
                             self.manifest["trainedCohortSha256"])
            for key in ("target", "rustc", "mathBackend", "sensorSha256", "parameterCount",
                        "continuationSha256", "consumerWork", "raw", "normalized", "trainedCohort"):
                self.assertEqual(report[key], reference[key], key)
            for key in ("sourceSampleSha256", "drainedSampleSha256"):
                self.assertEqual(report["capture"][key], reference["capture"][key], key)

    def test_sampled_phases_and_exact_unsampled_continuation(self):
        reports = [self.document("profile/" + name) for name in (
            "0-unsampled.json", "1-sampled.json", "2-unsampled.json")]
        self.equal_work(reports)
        for report in reports:
            self.assertEqual(report["sourceSha256"], self.manifest["profileNativeSourceSha256"])
            self.assertEqual(report["measuredSteps"], 1800)
            self.assertEqual(report["warmupSteps"], 600)
        profile = reports[1]["v4SensingProfile"]
        self.assertEqual(profile["samples"], self.manifest["sampledEvaluations"])
        self.assertGreater(profile["samples"], 8000)
        self.assertEqual(len(profile["phaseNames"]), 8)
        total = sum(profile["nanoseconds"])
        for phase, name, ns in zip(self.manifest["sampledPhases"],
                                   profile["phaseNames"], profile["nanoseconds"]):
            self.assertEqual(phase, {"name": name, "nanoseconds": ns, "fraction": ns / total})
        pellet_fraction = sum(phase["fraction"] for phase in self.manifest["sampledPhases"]
                              if phase["name"] in ("pelletQuery", "pelletEncoding"))
        self.assertGreater(pellet_fraction, 0.5)

    def test_rejected_candidates_have_unchanged_work_and_recomputed_medians(self):
        for candidate in ("octant", "heap"):
            comparison = self.document(candidate + "/comparison.json")
            self.assertEqual(hashlib.sha256(self.data[candidate + "/compare-driver.py"]).hexdigest(),
                             comparison["scriptSha256"])
            self.assertEqual(comparison["order"], ["baseline", "candidate", "candidate", "baseline"])
            reports = []
            for ordinal, role in enumerate(comparison["order"]):
                name = f"{ordinal}-{role}.json"
                report = self.document(candidate + "/" + name)
                record = comparison["records"][ordinal]
                self.assertEqual(record["reportSha256"],
                                 hashlib.sha256(self.data[candidate + "/" + name]).hexdigest())
                expected = (self.manifest["profileNativeSourceSha256"] if role == "baseline"
                            else self.manifest["candidateNativeSourceSha256"][candidate])
                self.assertEqual(report["sourceSha256"], expected)
                self.assertEqual(report["warmupSteps"], 600)
                self.assertEqual(report["measuredSteps"], 1200)
                self.assertEqual(report["completedSteps"], 1800)
                phases = report["nativePhaseCosts"]
                self.assertEqual(record["nativeMeanMs"], phases["totalMicros"] / 1200 / 1000)
                self.assertEqual(record["sensingMeanMs"], phases["sensingMicros"] / 1200 / 1000)
                self.assertEqual(record["nativeP95Ms"], report["stepCost"]["p95Ms"])
                reports.append(report)
            self.equal_work(reports)
            for role in ("baseline", "candidate"):
                for key in ("nativeMeanMs", "sensingMeanMs", "nativeP95Ms"):
                    measured = statistics.median(row[key] for row in comparison["records"]
                                                 if row["role"] == role)
                    self.assertEqual(comparison["medians"][role][key], measured)
            ratio = (comparison["medians"]["candidate"]["nativeMeanMs"] /
                     comparison["medians"]["baseline"]["nativeMeanMs"])
            self.assertGreater(ratio, 0.99)

    def test_query_subdivision_keeps_reference_outputs_and_records_host_load(self):
        report = self.document("query/sampled.json")
        reference = self.document("profile/1-sampled.json")
        self.equal_work([reference, report])
        self.assertEqual(report["sourceSha256"], self.manifest["queryProfileNativeSourceSha256"])
        summary = self.document("query/summary.json")
        self.assertTrue(summary["exactReferenceWork"])
        self.assertEqual(summary["v4SensingProfile"], report["v4SensingProfile"])
        profile = report["v4SensingProfile"]
        self.assertEqual(profile["version"], 2)
        self.assertEqual(profile["queryFields"], ["traversalNanos", "sortNanos", "samples",
                                                 "entriesVisited", "retainedCandidates",
                                                 "qualifyingCandidates"])
        travel, sort, samples, entries, retained, qualifying = profile["query"]
        self.assertEqual(samples, profile["samples"])
        self.assertGreater(samples, 8000)
        self.assertGreater(entries, qualifying)
        self.assertGreaterEqual(qualifying, retained)
        self.assertGreater(sort / (travel + sort), 0.4)
        self.assertLess(sort / (travel + sort), 0.45)
        self.assertLess(travel + sort, profile["nanoseconds"][1])
        observer_sha = hashlib.sha256(self.data["query/system-resource-observer.py"]).hexdigest()
        rows = [json.loads(line) for line in self.data["query/system-resources.jsonl"].splitlines()]
        self.assertGreaterEqual(len(rows), 2)
        for row in rows:
            self.assertEqual(row["observerSourceSha256"], observer_sha)
            self.assertEqual(row["platform"], "Linux")
            self.assertTrue(row["host"]["available"])
            self.assertGreaterEqual(row["collectionSeconds"], 0)
            self.assertIn("attribution", row)

    def test_scope_and_input_provenance_do_not_claim_a_gate_pass(self):
        self.assertFalse(self.manifest["performanceQualified"])
        self.assertFalse(self.manifest["maturityQualified"])
        prior = json.loads((ROOT.parent / "live-cohort-debian-v1/evidence.json").read_text(
            encoding="utf-8"))
        self.assertEqual(prior["nativeSourceSha256"], self.manifest["baseNativeSourceSha256"])
        self.assertEqual(prior["files"]["cohort.json"]["sha256"],
                         self.manifest["trainedCohortSha256"])
        self.assertIn("Shared host", self.manifest["scope"])
        self.assertIn("uninstrumented", self.manifest["caveats"])


if __name__ == "__main__":
    unittest.main()
