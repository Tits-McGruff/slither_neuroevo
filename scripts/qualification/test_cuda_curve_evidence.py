"""Bind the completed CUDA matrix to compiled work, native tensors and late held-out curves."""
import gzip
import hashlib
import json
from pathlib import Path
import statistics
import unittest

ROOT = Path(__file__).resolve().parents[2] / "docs/performance/sensor-v4/heldout-curves-cuda8000-windows-v1"


class CudaCurveEvidenceTests(unittest.TestCase):
    """Inspect all 63 receipts without training, simulation or application inventories."""

    @classmethod
    def setUpClass(cls):
        """Index retained objects; the full restore verifier separately recomputes the paired matrix."""
        cls.manifest = json.loads((ROOT / "evidence.json").read_text(encoding="utf-8"))
        cls.records = {r["path"]: r for r in cls.manifest["files"]}
        assert len(cls.records) == len(cls.manifest["files"])
        cls.cache = {}
        cls.matrix = cls.document("matrix.json")
        cls.cells = [(f"seed-{c['seed']}-fold-{c['validationSeed']}", c) for c in cls.matrix["cells"]]
        cls.cells = [(n, cls.document(n + "/summary.json")) for n, _ in cls.cells]

    @classmethod
    def original(cls, name):
        """Bound and verify each small compressed object before decoding; never load the large corpus here."""
        if name not in cls.cache:
            r = cls.records[name]
            assert type(r["bytes"]) is int and 0 <= r["bytes"] <= 2 * 1024**2
            assert r["archive"] == "objects/" + r["sha256"] + ".gz"
            encoded = (ROOT / r["archive"]).read_bytes()
            assert hashlib.sha256(encoded).hexdigest() == r["gzipSha256"]
            with gzip.open(ROOT / r["archive"], "rb") as stream:
                raw = stream.read(r["bytes"] + 1)
            assert len(raw) == r["bytes"] and hashlib.sha256(raw).hexdigest() == r["sha256"]
            cls.cache[name] = raw
        return cls.cache[name]

    @classmethod
    def document(cls, name):
        """Decode one checksum-verified metadata object."""
        return json.loads(cls.original(name))

    def test_actual_compiled_work_covers_the_complete_grid_and_default_window(self):
        """Nine cells and seven variants really use CUDA compilation for all 504,000 updates."""
        self.assertEqual(self.manifest["matrixSha256"], self.records["matrix.json"]["sha256"])
        self.assertEqual({(s["config"]["seed"], s["config"]["validationSeed"]) for _, s in self.cells},
                         {(seed, fold) for seed in (1, 2, 3) for fold in (2, 4, 6)})
        variants = []
        for _, s in self.cells:
            self.assertEqual(s["environment"]["device"], "cuda:0")
            self.assertEqual([s["config"][k] for k in ("batch_size", "burn_in_steps", "bptt_steps")], [32, 32, 64])
            self.assertEqual({v["variant"] for v in s["variants"]}, set(self.matrix["variants"]))
            variants.extend(s["variants"])
        self.assertEqual(len(variants), 63)
        for v in variants:
            self.assertEqual(v["training"]["updates"], 8000)
            self.assertEqual(v["training"]["windowExecution"],
                             {"requested": "cudagraphs", "compiledWindowCalls": 8000,
                              "eagerWindowCalls": 0, "compiledSignatures": 1})

    def test_native_parity_tensors_bind_each_opaque_end_model(self):
        """All measured 256-delivery prefixes retain native tensors and exact trained weights."""
        for name, s in self.cells:
            for v in s["variants"]:
                base = name + "/" + v["variant"] + "/"
                r = self.document(base + "native-parity/receipt.json")
                self.assertEqual(r["steps"], 256)
                self.assertEqual(r["sourceSha256"], self.manifest["nativeSourceSha256"])
                self.assertEqual(r["layoutSha256"], v["layoutSha256"])
                self.assertEqual(r["weightsSha256"], v["trainedWeightsSha256"])
                for field, file in [("weightsSha256", "trained-weights.f32"),
                                    ("observationsSha256", "parity-observations.f32"),
                                    ("outputsSha256", "native-parity/outputs.f32"),
                                    ("statesSha256", "native-parity/states.f32")]:
                    self.assertEqual(self.records[base + file]["sha256"], r[field])
                    self.original(base + file)
                self.assertEqual(v["parity"]["steps"], 256)
                self.assertLessEqual(v["parity"]["outputMaxAbs"], 1e-5)
                self.assertLessEqual(v["parity"]["stateMaxAbs"], 1e-5)
                artifact = self.original(base + "trained.slither-brain")
                self.assertEqual(artifact[:8], b"SLTHBRN1")
                self.assertEqual(artifact[24:56].hex(), v["trainedWeightsSha256"])

    def test_late_curves_do_not_establish_maturity_or_select_a_profile(self):
        """Recompute macro means and preserve Full improvement and Core-only endpoint regression."""
        for name, aggregate in self.matrix["variants"].items():
            curves = [{p["update"]: p for p in next(v for v in s["variants"] if v["variant"] == name)
                       ["training"]["validationCurve"]} for _, s in self.cells]
            self.assertEqual([p["update"] for p in aggregate["heldOutLearningCurve"]],
                             [0, 250, 500, 1000, 1500, 2000, 4000, 6000, 8000])
            for p in aggregate["heldOutLearningCurve"]:
                self.assertAlmostEqual(p["measurements"]["turnMae"]["mean"],
                                       statistics.mean(c[p["update"]]["turnMae"] for c in curves), places=14)
        full = self.matrix["variants"]["full"]["heldOutLearningCurve"]
        self.assertLess(full[-1]["measurements"]["turnMae"]["mean"], .93 * full[-2]["measurements"]["turnMae"]["mean"])
        core = self.matrix["variants"]["core-only"]["heldOutLearningCurve"]
        self.assertGreater(core[-1]["measurements"]["turnMae"]["mean"], core[-2]["measurements"]["turnMae"]["mean"])
        self.assertFalse(self.matrix["maturityQualified"])
        self.assertFalse(self.matrix["releaseQualified"])
        self.assertIsNone(self.matrix["profileChoice"])


if __name__ == "__main__":
    unittest.main()
