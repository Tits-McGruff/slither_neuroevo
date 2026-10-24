"""Bind independent final teacher agreement to native rows, fixed models and aggregate counters."""
from collections import defaultdict
import gzip
import hashlib
import json
from pathlib import Path
from historical_evidence import required_root
import re
import statistics
import struct
import unittest

BASE = required_root() / "docs/performance/sensor-v4"
ROOT = BASE / "final-teacher-agreement-windows-v1"
SOURCE = BASE / "final-native-tactics-cuda8000-windows-v1"
VARIANTS = {"full", "core-only", "no-bearing", "no-heading", "no-velocity", "no-urgency", "no-density"}
POLICIES = {"motion-v2-deterministic", "motion-v2-stochastic"}


def digest(data):
    """Hash bounded originals without executing archived source."""
    return hashlib.sha256(data).hexdigest()


class FinalTeacherAgreementTests(unittest.TestCase):
    """Verify original bytes and recompute coverage, metrics and exact-observation variation."""

    @classmethod
    def setUpClass(cls):
        """Read confined regular archives under the measured preset's 128-MiB total ceiling."""
        cls.manifest = json.loads((ROOT / "evidence.json").read_text(encoding="utf-8"))
        cls.files = {}
        total = 0
        assert len(cls.manifest["files"]) == 44
        for record in cls.manifest["files"]:
            name, size = record["path"], record["bytes"]
            assert re.fullmatch(r"(?:[a-zA-Z0-9_-]+/)*[a-zA-Z0-9_-]+\.(?:json|py|f32|u32)", name)
            assert name not in cls.files and type(size) is int and 0 <= size < 128 * 1024**2
            path = ROOT / (name + ".gz")
            assert path.resolve().is_relative_to(ROOT.resolve())
            assert path.is_file() and not path.is_symlink() and path.stat().st_size < 32 * 1024**2
            encoded = path.read_bytes()
            assert digest(encoded) == record["gzipSha256"]
            with gzip.open(path, "rb") as stream:
                raw = stream.read(size + 1)
            assert len(raw) == size and digest(raw) == record["sha256"]
            total += size
            assert total < 128 * 1024**2
            cls.files[name] = raw
        cls.design = cls.document("design.json")
        cls.summary = cls.document("agreement-summary.json")
        cls.corpus = cls.document("corpus/corpus.json")
        cls.final = [row for row in cls.corpus["sequences"] if row["seed"] in range(33, 41)]

    @classmethod
    def document(cls, name):
        """Decode one already verified JSON original."""
        return json.loads(cls.files[name])

    def test_native_matrix_selects_only_disjoint_final_rows_and_preserves_originals(self):
        """The frozen generator's other seeds cannot enter evaluation or model selection."""
        self.assertEqual(self.design["evaluatedSeeds"], list(range(33, 41)))
        self.assertEqual(self.design["sourceTrainingSeeds"], list(range(1, 9)))
        self.assertEqual(self.design["trainingFold"], 2)
        self.assertEqual(self.design["steps"], 96)
        self.assertEqual(self.design["workers"], 1)
        self.assertEqual(self.design["nativeCommit"], self.manifest["nativeCommit"])
        self.assertEqual(self.corpus["sourceSha256"], self.manifest["nativeSourceSha256"])
        self.assertEqual(self.corpus["seedCount"], 40)
        self.assertEqual(self.corpus["maximumSteps"], 96)
        self.assertEqual(self.corpus["inputSize"], 264)
        self.assertEqual(digest(self.files["corpus/corpus.json"]), self.summary["corpusMetadataSha256"])
        self.assertEqual(digest(self.files["design.json"]), self.summary["designSha256"])
        report = self.document("teacher-report.json")
        self.assertEqual(report["sourceSha256"], self.manifest["nativeSourceSha256"])
        self.assertEqual((report["seedCount"], report["maximumSteps"], report["workers"]), (40, 96, 1))
        offset = 0
        for row in self.corpus["sequences"]:
            self.assertEqual(row["rowOffset"], offset)
            self.assertTrue(1 <= row["rows"] <= 96)
            self.assertIn(row["policy"], POLICIES)
            offset += row["rows"]
        self.assertEqual(offset, self.corpus["rows"])
        for item in self.corpus["objects"]:
            data = self.files["corpus/" + item["file"]]
            self.assertEqual((len(data), digest(data)), (item["bytes"], item["sha256"]))
        self.assertEqual(len(self.files["corpus/observations.f32"]), offset * 264 * 4)
        self.assertEqual(len(self.files["corpus/actions.f32"]), offset * 8)
        self.assertEqual(len(self.files["corpus/steps.u32"]), offset * 4)
        scenarios = {row["scenario"] for row in self.final}
        self.assertEqual(len(scenarios), 14)
        self.assertEqual(len(self.final), 224)
        self.assertEqual({(row["scenario"], row["seed"], row["policy"]) for row in self.final},
                         {(scenario, seed, policy) for scenario in scenarios
                          for seed in range(33, 41) for policy in POLICIES})
        self.assertFalse({row["seed"] for row in self.final}.intersection(self.design["sourceTrainingSeeds"]))
        self.assertEqual(sum(row["rows"] for row in self.final), 19978)
        self.assertEqual(self.summary["excludedCollectedSeeds"], list(range(1, 33)))

    def test_fixed_models_and_imported_evaluator_sources_are_exact(self):
        """All 21 models come from the predeclared fold; metrics do not choose a best initialization."""
        source_bytes = (SOURCE / "evidence.json").read_bytes().replace(b"\r\n", b"\n")
        expected = digest(b"slither-evidence-manifest-utf8-lf-v1\0" + source_bytes)
        self.assertEqual(expected, self.manifest["modelSourceManifestCanonicalSha256"])
        self.assertEqual(expected, self.summary["sourceStudyManifestCanonicalSha256"])
        source_files = {r["path"]: r for r in json.loads(source_bytes)["files"]}
        models = self.summary["models"]
        self.assertEqual(len(models), 21)
        self.assertEqual({(row["initializationSeed"], row["variant"]) for row in models},
                         {(seed, variant) for seed in (1, 2, 3) for variant in VARIANTS})
        for model in models:
            receipt = source_files[model["model"]]
            encoded = (SOURCE / (model["model"] + ".gz")).read_bytes()
            self.assertEqual(digest(encoded), receipt["gzipSha256"])
            with gzip.open(SOURCE / (model["model"] + ".gz"), "rb") as stream:
                raw = stream.read(2 * 1024**2 + 1)
            self.assertEqual(len(raw), receipt["bytes"])
            self.assertEqual(digest(raw), model["modelSha256"])
            self.assertEqual(digest(raw), receipt["sha256"])
            self.assertEqual(raw[:8], b"SLTHBRN1")
            self.assertEqual(raw[24:56].hex(), model["weightsSha256"])
            self.assertEqual(digest(self.files[model["report"]]), model["reportSha256"])
            original = self.document(model["report"])
            self.assertEqual(original, {k: v for k, v in model.items()
                                        if k not in ("report", "reportSha256")})
        self.assertEqual(self.summary["sourceFiles"], self.manifest["evaluatorSourceFiles"])
        self.assertEqual(len(self.summary["sourceFiles"]), 9)
        for name, expected in self.summary["sourceFiles"].items():
            self.assertEqual(digest(self.files["evaluator/" + name]), expected)

    def test_recompute_all_unweighted_groups_and_macro_means(self):
        """Every sequence/policy/scenario contributes real rows; no event weighting changes validation."""
        for row in self.summary["models"]:
            agreement = row["agreement"]
            self.assertEqual((agreement["rows"], agreement["sequences"]), (19978, 224))
            for group_name in ("perScenario", "perPolicy", "perScenarioPolicy"):
                groups = agreement[group_name]
                self.assertEqual(len(groups), {"perScenario": 14, "perPolicy": 2,
                                               "perScenarioPolicy": 28}[group_name])
                self.assertEqual(sum(g["rows"] for g in groups.values()), 19978)
                for field, metric in (("turnSquaredError", "turnMse"),
                                      ("turnAbsoluteError", "turnMae"),
                                      ("boostDisagreements", "boostDisagreement"),
                                      ("lossSum", "loss")):
                    total = sum(g[field] for g in groups.values())
                    self.assertAlmostEqual(total / 19978, agreement[metric], places=10)
                for group in groups.values():
                    self.assertGreater(group["rows"], 0)
                    self.assertTrue(0 <= group["boostDisagreements"] <= group["rows"])
                    self.assertTrue(0 <= group["turnAbsoluteError"] <= 2 * group["rows"])
                    self.assertTrue(0 <= group["turnSquaredError"] <= 4 * group["rows"])
            self.assertGreaterEqual(agreement["turnMse"] + 1e-7, agreement["turnMae"]**2)
        for variant, aggregate in self.summary["variants"].items():
            records = [row["agreement"] for row in self.summary["models"] if row["variant"] == variant]
            self.assertEqual(len(records), 3)
            for metric, values in aggregate.items():
                original = [row[metric] for row in records]
                self.assertAlmostEqual(values["mean"], statistics.mean(original), places=12)
                self.assertEqual(values["minimum"], min(original))
                self.assertEqual(values["maximum"], max(original))
        for item in (self.manifest, self.summary):
            self.assertTrue(item["measurementComplete"])
            self.assertFalse(item["maturityQualified"])
            self.assertFalse(item["releaseQualified"])
            self.assertIsNone(item["profileChoice"])
        self.assertEqual(self.summary["device"], "cpu")
        self.assertEqual(self.summary["optimizerUpdates"], 0)

    def test_exact_observation_variation_uses_only_final_rows_and_preserves_small_support(self):
        """Recompute repeated-label statistics with independent list-based population variances."""
        observations, actions = (self.files["corpus/" + name]
                                 for name in ("observations.f32", "actions.f32"))
        groups = {"all": defaultdict(list), **{policy: defaultdict(list) for policy in POLICIES}}
        for sequence in self.final:
            for row in range(sequence["rowOffset"], sequence["rowOffset"] + sequence["rows"]):
                observation = observations[row * 1056:(row + 1) * 1056]
                action = struct.unpack_from("<ff", actions, row * 8)
                self.assertIn(action[1], (0.0, 1.0))
                for partition in ("all", sequence["policy"]):
                    groups[partition][observation].append(action)
        report = self.document("conditional-label-variation.json")
        self.assertEqual(report["rows"], 19978)
        for policy, cells in groups.items():
            actual = report["groups"][policy]
            repeats = [labels for labels in cells.values() if len(labels) > 1]
            count = sum(len(labels) for labels in repeats)
            self.assertEqual(actual["distinctObservations"], len(cells))
            self.assertEqual(actual["repeatedGroups"], len(repeats))
            self.assertEqual(actual["repeatedRows"], count)
            self.assertEqual(actual["rows"], sum(len(labels) for labels in cells.values()))
            variance = sum(len(labels) * statistics.pvariance(action[0] for action in labels)
                           for labels in repeats) / count
            minority = sum(min(sum(action[1] for action in labels),
                               len(labels) - sum(action[1] for action in labels))
                           for labels in repeats) / count
            self.assertAlmostEqual(actual["turnVarianceOverRepeatedRows"], variance, places=12)
            self.assertEqual(actual["boostMinorityRateOverRepeatedRows"], minority)
        self.assertEqual(report["groups"]["motion-v2-stochastic"]["repeatedRows"], 8)
        self.assertIn("do not bound a recurrent student's attainable error", report["scope"])

    def test_every_retained_resource_record_is_aggregate_only(self):
        """Fail publication evidence if Windows/WSL/GPU process or application fields appear."""
        observer = digest(self.files["system-resource-observer.py"])
        for name, count in self.manifest["resourceSamples"].items():
            rows = self.document(name)
            self.assertEqual(len(rows), count)
            self.assertGreaterEqual(count, 2)
            for row in rows:
                self.assertIs(row["processDetailsIncluded"], False)
                self.assertEqual(row["observerSourceSha256"], observer)
                self.assertLessEqual(set(row), {"schemaVersion", "utc", "platform", "gpu", "host",
                                               "wsl", "processDetailsIncluded", "attribution",
                                               "collectionSeconds", "sample", "observerSourceSha256"})
                self.assertLessEqual(set(row["host"]),
                                     {"available", "cpuPercent", "totalMemoryKiB", "freeMemoryKiB"})
                self.assertLessEqual(set(row.get("wsl", {})),
                                     {"available", "distributions", "error", "exitCode", "oversized"})
                for distribution in row.get("wsl", {}).get("distributions", []):
                    self.assertLessEqual(set(distribution),
                                         {"available", "cpuTicks", "loadAverage", "memoryKiB",
                                          "error", "exitCode", "oversized"})
                    self.assertLessEqual(set(distribution.get("memoryKiB", {})),
                                         {"MemTotal", "MemFree", "MemAvailable", "Buffers", "Cached"})
                self.assertLessEqual(set(row["gpu"]), {"available", "rows", "error", "exitCode", "oversized"})
                for gpu in row["gpu"].get("rows", []):
                    self.assertLessEqual(set(gpu), {"name", "utilization.gpu [%]", "utilization.memory [%]",
                                                   "memory.used [MiB]", "memory.total [MiB]",
                                                   "power.draw [W]", "temperature.gpu"})


if __name__ == "__main__":
    unittest.main()
