"""Recompute disjoint final-seed native outcomes and enforce aggregate-only resource evidence."""
import gzip
import hashlib
import json
from pathlib import Path
import re
import statistics
import unittest

BASE = Path(__file__).resolve().parents[2] / "docs/performance/sensor-v4"
ROOT = BASE / "final-native-tactics-cuda8000-windows-v1"
SOURCE = BASE / "heldout-curves-cuda8000-windows-v1"
METRICS = ("survivingTrials", "survivalSeconds", "foodEarned", "kills")
VARIANTS = {"full", "no-bearing", "no-heading", "no-velocity", "no-urgency", "no-density", "core-only"}
SCENARIOS = {"approaching", "boost-escape", "corridor", "crossing", "crowd-opening",
             "food-body-trap", "head-on", "larger-threat", "open-wander", "predicted-crossing",
             "receding", "saturation", "smaller-intercept", "wall-escape"}


def digest(data):
    """Hash retained opaque bytes without executing archived sources."""
    return hashlib.sha256(data).hexdigest()


def totals(trials):
    """Recompute physical gameplay totals directly from native trial receipts."""
    return dict(zip(METRICS, (sum(t["alive"] for t in trials),
                             sum(t["aliveSeconds"] for t in trials),
                             sum(t["foodEarned"] for t in trials),
                             sum(t["kills"] for t in trials))))


class FinalNativeEvidenceTests(unittest.TestCase):
    """Bound all archives and bind models to the original fixed-fold study before interpretation."""

    @classmethod
    def setUpClass(cls):
        """Read confined ordinary files with an eight-MiB total decompression ceiling."""
        cls.manifest = json.loads((ROOT / "evidence.json").read_text(encoding="utf-8"))
        cls.files = {}
        records = cls.manifest["files"]
        assert len(records) == 49
        total = 0
        for record in records:
            name = record["path"]
            assert re.fullmatch(r"(?:[a-zA-Z0-9_-]+/)*[a-zA-Z0-9_-]+\.(?:json|py|slither-brain)", name)
            assert name not in cls.files
            path = ROOT / (name + ".gz")
            assert path.is_file() and not path.is_symlink() and path.stat().st_size <= 4 * 1024**2
            size = record["bytes"]
            assert type(size) is int and 0 < size <= 4 * 1024**2
            encoded = path.read_bytes()
            assert digest(encoded) == record["gzipSha256"]
            with gzip.open(path, "rb") as stream:
                raw = stream.read(size + 1)
            assert len(raw) == size and digest(raw) == record["sha256"]
            total += size
            assert total < 8 * 1024**2
            cls.files[name] = raw
        cls.design = cls.document("design.json")
        cls.summary = cls.document("summary.json")

    @classmethod
    def document(cls, name):
        """Interpret only an already checksum-verified JSON original."""
        return json.loads(cls.files[name])

    def test_fixed_models_and_disjoint_final_seeds_bind_the_completed_source_study(self):
        """The complete fixed fold is evaluated without selecting models using final outcomes."""
        source_bytes = (SOURCE / "evidence.json").read_bytes().replace(b"\r\n", b"\n")
        self.assertEqual(digest(b"slither-evidence-manifest-utf8-lf-v1\0" + source_bytes),
                         self.manifest["sourceManifestCanonicalSha256"])
        source_manifest = json.loads(source_bytes)
        source_files = {record["path"]: record for record in source_manifest["files"]}
        for name, old in (("source-matrix.json", "matrix.json"), ("source-corpus.json", "corpus/corpus.json")):
            self.assertEqual(digest(self.files[name]), source_files[old]["sha256"])
        self.assertEqual(digest(self.files["source-matrix.json"]), self.manifest["sourceMatrixSha256"])
        self.assertEqual(self.design["sourceMatrixSha256"], self.manifest["sourceMatrixSha256"])
        self.assertEqual(self.design["sourceNativeSha256"], self.manifest["nativeSourceSha256"])
        self.assertEqual(self.design["sourceNativeCommit"], self.manifest["nativeCommit"])
        self.assertEqual(self.design["runnerSha256"], self.document("source-matrix.json")["runnerSha256"]["tactical"])
        self.assertEqual(digest(self.files["design.json"]), self.summary["designSha256"])
        corpus_seeds = {row["seed"] for row in self.document("source-corpus.json")["sequences"]}
        self.assertEqual(corpus_seeds, set(range(1, 9)))
        self.assertEqual(set(self.design["sourceCorpusSeeds"]), corpus_seeds)
        self.assertEqual(self.design["finalSeeds"], list(range(33, 41)))
        self.assertFalse(corpus_seeds.intersection(self.design["finalSeeds"]))
        self.assertEqual(self.design["trainingFold"], 2)
        self.assertEqual(self.design["initializationSeeds"], [1, 2, 3])
        self.assertEqual(set(self.design["variants"]), VARIANTS)
        rows = self.summary["models"]
        self.assertEqual(len(rows), 21)
        self.assertEqual({(r["initializationSeed"], r["variant"]) for r in rows},
                         {(seed, variant) for seed in (1, 2, 3) for variant in VARIANTS})
        for row in rows:
            old = f"seed-{row['initializationSeed']}-fold-2/{row['variant']}/trained.slither-brain"
            self.assertEqual(digest(self.files[row["model"]]), source_files[old]["sha256"])
            self.assertEqual(row["modelSha256"], source_files[old]["sha256"])
            self.assertEqual(row["reportSha256"], digest(self.files[row["report"]]))

    def test_actual_native_trials_cover_every_scenario_seed_with_identical_physical_references(self):
        """Every model executes actual Rust sensors, inference and physics on the same 112 cases."""
        reference = None
        for row in self.summary["models"]:
            report = self.document(row["report"])
            self.assertEqual(report["sourceSha256"], self.manifest["nativeSourceSha256"])
            self.assertEqual(report["target"], "x86_64-pc-windows-msvc")
            self.assertEqual((report["workers"], report["firstSeed"], report["seedCount"], report["maximumSteps"]),
                             (1, 33, 8, 600))
            candidate = report["candidate"]
            model = self.files[row["model"]]
            self.assertEqual(candidate["artifactSha256"], digest(model))
            self.assertEqual(model[:8], b"SLTHBRN1")
            self.assertEqual(candidate["weightsSha256"], model[24:56].hex())
            self.assertEqual((candidate["inputSize"], candidate["controlIntervalSeconds"],
                              candidate["lifeStateRule"], candidate["sourceKind"]),
                             (264, 1 / 60, "reset", "distilled"))
            if reference is None:
                reference = report["referenceV1"]
            self.assertEqual(report["referenceV1"], reference)
            for name in ("trials", "referenceV1"):
                trials = report[name]
                self.assertEqual(len(trials), 112)
                self.assertEqual({(t["scenario"], t["seed"]) for t in trials},
                                 {(scenario, seed) for scenario in SCENARIOS for seed in range(33, 41)})
                for trial in trials:
                    self.assertTrue(0 < trial["steps"] <= 600)
                    self.assertGreater(trial["observations"], 0)
                    self.assertTrue(trial["sharedBatchBudgetHeld"])
                    self.assertRegex(trial["trajectoryWitnessSha256"], r"^[0-9a-f]{64}$")
                    self.assertEqual(trial["policy"], "canonical-student" if name == "trials" else "v1")
                    if name == "trials":
                        self.assertEqual(trial["policyWorldReads"], 0)
                        self.assertEqual(trial["behaviorDraws"], 0)
            for metric, actual in totals(report["trials"]).items():
                self.assertAlmostEqual(actual, row[metric], places=8)

    def test_macro_outcomes_preserve_tradeoffs_and_do_not_select_a_profile(self):
        """Aggregate physical outcomes without treating synthetic scenarios as representative ecology."""
        for variant, aggregate in self.summary["variants"].items():
            selected = [r for r in self.summary["models"] if r["variant"] == variant]
            self.assertEqual(len(selected), 3)
            for metric in METRICS:
                values = [row[metric] for row in selected]
                self.assertAlmostEqual(aggregate[metric]["mean"], statistics.mean(values), places=8)
                self.assertEqual(aggregate[metric]["minimum"], min(values))
                self.assertEqual(aggregate[metric]["maximum"], max(values))
        reference = totals(self.document(self.summary["models"][0]["report"])["referenceV1"])
        full = self.summary["variants"]["full"]
        self.assertEqual(reference["survivingTrials"], 58)
        self.assertAlmostEqual(reference["foodEarned"], 335.1102894, places=6)
        self.assertGreater(full["survivingTrials"]["mean"], reference["survivingTrials"])
        self.assertLess(full["foodEarned"]["mean"], reference["foodEarned"])
        for record in (self.manifest, self.summary):
            self.assertTrue(record["measurementComplete"])
            self.assertFalse(record["releaseQualified"])
            self.assertFalse(record["maturityQualified"])
            self.assertIsNone(record["profileChoice"])
        self.assertIn("no independent final teacher-agreement", self.manifest["scope"])
        self.assertIn("first approximately 15 seconds absent", self.manifest["resourceCoverage"])

    def test_published_windows_wsl_gpu_observations_are_aggregate_only(self):
        """Strict field allowlists reject process inventories, names, PIDs and arbitrary command output."""
        resources = self.document("aggregate-resources.json")
        self.assertEqual(len(resources), 93)
        observer_sha = digest(self.files["system-resource-observer.py"])
        for row in resources:
            self.assertEqual(row["platform"], "Windows")
            self.assertIs(row["processDetailsIncluded"], False)
            self.assertEqual(row["observerSourceSha256"], observer_sha)
            self.assertLessEqual(set(row), {"schemaVersion", "utc", "platform", "gpu", "host", "wsl",
                                           "processDetailsIncluded", "attribution", "collectionSeconds",
                                           "sample", "observerSourceSha256"})
            self.assertLessEqual(set(row["host"]), {"available", "cpuPercent", "totalMemoryKiB", "freeMemoryKiB"})
            self.assertLessEqual(set(row["wsl"]), {"available", "distributions", "error", "exitCode", "oversized"})
            for distribution in row["wsl"].get("distributions", []):
                self.assertLessEqual(set(distribution), {"available", "cpuTicks", "loadAverage", "memoryKiB",
                                                        "error", "exitCode", "oversized"})
                self.assertLessEqual(set(distribution.get("memoryKiB", {})),
                                     {"MemTotal", "MemFree", "MemAvailable", "Buffers", "Cached"})
            self.assertLessEqual(set(row["gpu"]), {"available", "rows", "error", "exitCode", "oversized"})
            for gpu in row["gpu"].get("rows", []):
                self.assertLessEqual(set(gpu), {"name", "utilization.gpu [%]", "utilization.memory [%]",
                                               "memory.used [MiB]", "memory.total [MiB]", "power.draw [W]",
                                               "temperature.gpu"})


if __name__ == "__main__":
    unittest.main()
