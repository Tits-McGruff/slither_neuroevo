"""Verify retained inference timing and numeric correction without executing archived code."""
import array
import ast
import gzip
import hashlib
import json
import math
from pathlib import Path
import re
import statistics
import struct
import sys
import unittest

BASE = Path(__file__).resolve().parents[2] / "docs/performance/sensor-v4"


def sha(data):
    """Hash opaque retained input/output bytes."""
    return hashlib.sha256(data).hexdigest()


def capsule(name):
    """Read confined regular files under bounded compressed and decoded byte ceilings."""
    root = BASE / name
    manifest = json.loads((root / "evidence.json").read_text(encoding="utf-8"))
    assert 0 < len(manifest["files"]) <= 64
    expected_paths = {"evidence.json"} | {name + ".gz" for name in manifest["files"]}
    assert {p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()} == expected_paths
    files = {}
    total = 0
    for name, record in manifest["files"].items():
        assert re.fullmatch(r"(?:[A-Za-z0-9_-]+/)*[A-Za-z0-9_-]+\.(?:json|rs|py|ts|f32|slither-brain)", name)
        path = root / (name + ".gz")
        assert path.is_file() and not path.is_symlink() and path.stat().st_size <= 4 * 1024**2
        size = record["bytes"]
        assert type(size) is int and 0 < size <= 4 * 1024**2
        encoded = path.read_bytes()
        assert sha(encoded) == record["gzipSha256"]
        with gzip.open(path, "rb") as stream:
            raw = stream.read(size + 1)
        assert len(raw) == size and sha(raw) == record["sha256"]
        total += size
        assert total <= 8 * 1024**2
        files[name] = raw
    return manifest, files


def floats(raw):
    """Interpret only finite retained Float32 result fixtures, never run a graph."""
    assert len(raw) % 4 == 0
    values = array.array("f")
    values.frombytes(raw)
    if sys.byteorder != "little":
        values.byteswap()
    assert all(math.isfinite(x) for x in values)
    return values


def f32(value):
    """Reproduce the comparison gate's Float32 arithmetic."""
    return struct.unpack("<f", struct.pack("<f", value))[0]


def violations(actual, expected, atol, rtol):
    """Recompute per-value Torch-style absolute/relative thresholds."""
    assert len(actual) == len(expected)
    return sum(abs(f32(x - y)) > f32(f32(atol) + f32(f32(rtol) * abs(y)))
               for x, y in zip(actual, expected))


def brain_metadata(raw):
    """Check a bounded raw artifact envelope before reading inert JSON metadata."""
    header = struct.Struct("<8sIB3xII32s32s")
    assert header.size <= len(raw) <= 4 * 1024**2
    magic, length, encoding, count, stored, digest, stored_digest = header.unpack_from(raw)
    assert magic == b"SLTHBRN1" and encoding == 0 and raw[13:16] == bytes(3)
    assert 0 < length <= 1024**2 and 0 < count <= 1000000 and stored == count * 4
    assert len(raw) == header.size + length + stored
    weights = raw[header.size + length:]
    assert hashlib.sha256(weights).digest() == digest == stored_digest
    assert len(floats(weights)) == count
    return json.loads(raw[header.size:header.size + length])


def aggregate_only(test, rows, observer):
    """Reject host/process inventories while retaining aggregate Windows/WSL/GPU counters."""
    for row in rows:
        test.assertIs(row["processDetailsIncluded"], False)
        test.assertEqual(row["observerSourceSha256"], sha(observer))
        test.assertLessEqual(set(row), {"schemaVersion", "utc", "platform", "gpu", "host", "wsl",
                                      "processDetailsIncluded", "attribution", "collectionSeconds",
                                      "sample", "observerSourceSha256"})
        hosts = [row["host"]]
        test.assertLessEqual(set(row["host"]), {"available", "cpuTicks", "loadAverage", "memoryKiB",
                                              "cpuPercent", "totalMemoryKiB", "freeMemoryKiB",
                                              "error", "exitCode", "oversized"})
        if "wsl" in row:
            test.assertEqual(row["platform"], "Windows")
            test.assertLessEqual(set(row["wsl"]), {"available", "distributions", "error", "exitCode", "oversized"})
            for item in row["wsl"].get("distributions", []):
                test.assertLessEqual(set(item), {"available", "cpuTicks", "loadAverage", "memoryKiB",
                                                "error", "exitCode", "oversized"})
                hosts.append(item)
        for host in hosts:
            memory = host.get("memoryKiB", {})
            test.assertLessEqual(set(memory), {"MemTotal", "MemFree", "MemAvailable", "Buffers", "Cached"})
            test.assertTrue(all(type(x) is int for x in memory.values()))
        test.assertLessEqual(set(row["gpu"]), {"available", "rows", "error", "exitCode", "oversized"})
        for gpu in row["gpu"].get("rows", []):
            test.assertLessEqual(set(gpu), {"name", "utilization.gpu [%]", "utilization.memory [%]",
                                          "memory.used [MiB]", "memory.total [MiB]",
                                          "power.draw [W]", "temperature.gpu"})


class NativeDotPerformanceEvidenceTests(unittest.TestCase):
    """Separate fixed-input speed evidence from still-failing live workload gates."""

    @classmethod
    def setUpClass(cls):
        """Load bounded exact originals and their unchanged source dependency."""
        cls.manifest, cls.files = capsule("native-dot-performance-v1")
        _, cls.prior = capsule("native-large-costs-debian-v1")

    def document(self, name):
        """Decode one already verified retained JSON file."""
        return json.loads(self.files[name])

    def test_actual_kernel_sources_and_paired_medians(self):
        """Four accumulators bind actual native source, not only a favorable prototype."""
        for platform in ["windows", "debian"]:
            metadata = self.document(platform + "/metadata.json")
            result = self.document(platform + "/result.json")
            self.assertEqual(metadata["sourceSha256"], sha(self.files["micro/benchmark.rs"]))
            self.assertEqual(metadata["referenceSha256"], sha(self.files["micro/simd_kernels.rs"]))
            self.assertEqual(metadata["candidateSha256"], sha(self.files["micro/candidate_kernels.rs"]))
            self.assertEqual(result["comparisons"], 11776)
            self.assertEqual(len(result["records"]), 112)
            for cell in metadata["summary"]:
                rows = [x for x in result["records"] if all(x[k] == cell[k] for k in
                        ["accumulators", "width", "streaming"])]
                self.assertEqual([x["pass"] for x in rows], list(range(8)))
                self.assertTrue(all(x["oldFirst"] == (x["pass"] % 4 in [0, 3]) for x in rows))
                self.assertAlmostEqual(statistics.median(x["candidateSeconds"] / x["originalSeconds"]
                                                        for x in rows), cell["medianCandidateRatio"], places=14)
                if cell["accumulators"] == 4 and cell["streaming"]:
                    self.assertLess(cell["medianCandidateRatio"], 1)
        self.assertEqual(self.manifest["selectedAccumulators"], 4)

    def test_real_population_pairs_have_identical_work_and_numeric_probe_bounds(self):
        """Recompute native single-worker v3 timings under equal build contracts."""
        summary = self.document("population/summary.json")
        self.assertEqual(len(summary["records"]), 16)
        for record in summary["records"]:
            prefix = "population/" + record["scenario"] + "-" + str(record["pass"])
            old = self.document(prefix + "-reference.json")
            new = self.document(prefix + "-candidate.json")
            self.assertNotIn("command", old)
            self.assertNotIn("command", new)
            for data in [old, new]:
                self.assertNotIn("hostname", data["environment"])
                self.assertNotIn("distributionId", data["environment"])
                self.assertEqual(set(data["environment"]), {
                    "architecture", "availableParallelism", "capturedAtEpochMs", "cpuModel",
                    "declaration", "fixtureRssBytes", "operatingSystem", "processPeakRssBytes",
                    "totalMemoryBytes"})
            self.assertEqual(old["workload"], new["workload"])
            self.assertEqual(old["source"]["nativeBuildContractSha256"], new["source"]["nativeBuildContractSha256"])
            self.assertEqual(new["source"]["nativeSourceSha256"], self.manifest["nativeSourceSha256"])
            self.assertEqual(record["referenceMeanMs"], old["result"]["wholePopulationPassMs"]["mean"])
            self.assertEqual(record["candidateMeanMs"], new["result"]["wholePopulationPassMs"]["mean"])
            for name in ["outputs", "recurrent"]:
                a = floats(bytes.fromhex(old["result"]["oneStepComparisonProbe"][name + "F32LeHex"]))
                b = floats(bytes.fromhex(new["result"]["oneStepComparisonProbe"][name + "F32LeHex"]))
                error = max(abs(x-y) for x, y in zip(a, b))
                self.assertEqual(error, record["maxProbeAbs"][name])
                self.assertLessEqual(error, old["result"]["oneStepComparisonProbe"]["absoluteTolerance"])
        for cell in summary["summary"]:
            rows = [r for r in summary["records"] if r["scenario"] == cell["scenario"]]
            self.assertAlmostEqual(statistics.median(r["candidateMeanMs"] / r["referenceMeanMs"]
                                                    for r in rows), cell["medianCandidateRatio"], places=14)
            self.assertLess(cell["medianCandidateRatio"], 1)

    def test_live_capture_and_durability_do_not_hide_failed_timing(self):
        """Retain actual P0/P1 work with exact trainer restart and three cold-restored boundaries."""
        self.assertFalse(self.manifest["performanceQualified"])
        self.assertFalse(self.manifest["fullCapacitySoakQualified"])
        for case in ["P0", "P1"]:
            report = self.document("live-" + case + "/report.json")
            self.assertEqual(report["source"]["slitherRevision"], self.manifest["slitherCommit"])
            self.assertEqual(report["source"]["nativeSourceSha256"], self.manifest["nativeSourceSha256"])
            self.assertEqual(report["source"]["hostSha256"], sha(self.prior["host/live-distillation.ts"]))
            self.assertEqual(report["source"]["trainerGateSha256"], sha(self.prior["pyrl/live_distillation_gate.py"]))
            self.assertEqual(report["resume"]["updates"], 250)
            self.assertTrue(report["resume"]["restartExact"])
            self.assertTrue(report["capture"]["noDrops"])
            self.assertFalse(report["summary"]["meetsMeasuredGates"])
            self.assertEqual(report["catalogOverlap"]["before"]["recordCount"], 100)
            self.assertEqual(report["catalogOverlap"]["before"], report["catalogOverlap"]["afterColdRestart"])
            self.assertEqual(len(report["durability"]["barriers"]), 3)
            self.assertTrue(report["durability"]["persistenceRestartExact"])
            before, after = (report["phaseCosts"][x]["counters"] for x in ["before", "after"])
            for key, value in self.manifest["workloads"][case]["phaseDeltas"].items():
                self.assertEqual(str(int(after[key], 16)-int(before[key], 16)), value)
            self.assertEqual(self.manifest["workloads"][case]["phaseDeltas"]["terminalStepSamples"], "3")

    def test_live_students_are_retained_before_disposable_databases_are_removed(self):
        """Keep the exact update-250 immutable candidates used by every live shadow."""
        for case in ["P0", "P1"]:
            raw = self.files["live-" + case + "/student.slither-brain"]
            digest = sha(raw)
            self.assertEqual(digest, self.manifest["retainedStudents"][case])
            report = self.document("live-" + case + "/report.json")
            metadata = brain_metadata(raw)
            self.assertEqual(metadata["graphManifest"]["totalParameters"], 494306)
            self.assertEqual(metadata["graphManifest"]["inputSize"], 504)
            self.assertEqual(metadata["provenance"]["sourceKind"], "distilled")
            details = metadata["provenance"]["details"]
            self.assertEqual(int(details["updates"], 16), self.manifest["retainedStudentUpdates"])
            self.assertEqual(details["trainerSourceSha256"], report["train"]["trainerSourceSha256"])
            for boundary in ["before", "after"]:
                self.assertEqual(len(report["shadows"][boundary]), 4)
                self.assertTrue(all(x["artifactSha256"] == digest for x in report["shadows"][boundary]))

    def test_published_resource_context_excludes_inventories(self):
        """Only aggregate counters survive into these performance receipts."""
        for name in ["windows/resources.json", "debian/before-resources.json", "debian/after-resources.json",
                     "live-P0/resources.json", "live-P1/resources.json"]:
            observer = self.files["windows/system-resource-observer.py"] if name.startswith("windows/") else self.prior["host/system-resource-observer.py"]
            aggregate_only(self, self.document(name), observer)


class NativeDotNumericEvidenceTests(unittest.TestCase):
    """Recompute the frozen first-step regressions and unchanged strict roundtrip contract."""

    @classmethod
    def setUpClass(cls):
        """Load finite input/output fixtures under the same bounded archive admission."""
        cls.manifest, cls.files = capsule("native-dot-numerics-v1")
        _, cls.prior = capsule("native-large-costs-debian-v1")

    def document(self, name):
        """Decode checksum-verified JSON only."""
        return json.loads(self.files[name])

    def test_all_named_optimizer_roundtrips_preserve_source_and_bounds(self):
        """Both backends cover every operation and large recurrent family on CPU/CUDA."""
        expected = {"dense", "mlp", "gru", "lstm", "rru", "mixed", "large-gru", "large-lstm", "large-rru"}
        digest = hashlib.sha256(b"pyrl.canonical-training-source.v1\0")
        for name in ["canonical_graph.py", "canonical_parity.py", "sequence_distillation.py"]:
            raw = self.files["pyrl/" + name] if "pyrl/" + name in self.files else self.prior["pyrl/" + name]
            digest.update(name.encode())
            digest.update(raw.replace(b"\r\n", b"\n"))
        for device in ["cpu", "cuda"]:
            report = self.document(device + "-roundtrip.json")
            self.assertEqual(report["trainerSourceSha256"], digest.hexdigest())
            self.assertEqual(report["dtype"], "Float32")
            self.assertFalse(report["tf32"])
            self.assertEqual(len(report["cases"]), 18)
            self.assertEqual({x["case"] for x in report["cases"]}, expected)
            for case in report["cases"]:
                self.assertEqual(case["nativeSourceSha256"], self.manifest["nativeSourceSha256"])
                self.assertEqual(case["weightsSha256"], case["optimization"]["afterSha256"])
                self.assertNotEqual(case["optimization"]["beforeSha256"], case["weightsSha256"])
                self.assertEqual(case["optimization"]["trainingSamples"], 2048)
                self.assertEqual(case["optimization"]["burnInSamples"], 1024)
                for field in ["outputErrorGrowth", "stateErrorGrowth"]:
                    self.assertEqual([x["step"] for x in case[field]], [1, 64, 128, 256])
                    self.assertLess(max(x["maxAbs"] for x in case[field]), 1e-3)
        self.assertEqual(sha(self.files["driver.py"]), self.document("cpu-roundtrip.json")["driverSha256"])

    def test_original_scalar_failures_are_not_relabelled_or_dropped(self):
        """Old/current scalar bytes match and two first-step failures remain explicitly recorded."""
        old = self.document("cpu-before.json")
        self.assertFalse(old["extendedPassed"])
        self.assertTrue(old["extendedSse2Passed"])
        failed = [x for x in old["extendedCases"] if not x["passed"]]
        self.assertEqual({(x["case"], x["backend"]) for x in failed},
                         {("large-lstm", "rust-scalar-v1"), ("large-rru", "rust-scalar-v1")})
        self.assertTrue(self.document("cuda-before.json")["extendedPassed"])
        for item in self.document("scalar-baseline.json")["results"]:
            prefix = "cases/" + item["case"].removeprefix("large-") + "/"
            before, legacy = (self.document(prefix + "receipt-" + x + ".json") for x in ["before", "legacy"])
            self.assertEqual(before["sourceSha256"], self.manifest["previousSourceSha256"])
            self.assertEqual(legacy["sourceSha256"], self.manifest["legacySourceSha256"])
            for field in ["outputsSha256", "statesSha256"]:
                self.assertEqual(before[field], legacy[field])
                self.assertEqual(before[field], item[field])

    def test_corrected_same_exports_satisfy_original_first_and_long_limits(self):
        """Derive numerical pass/fail from actual old/new Float32 states and frozen Torch results."""
        for kind, state_width, first_failures in [("lstm", 192, 3), ("rru", 96, 1)]:
            prefix = "cases/" + kind + "/"
            before, after = (self.document(prefix + "receipt-" + x + ".json") for x in ["before", "after"])
            self.assertEqual(after["sourceSha256"], self.manifest["nativeSourceSha256"])
            self.assertEqual(after["steps"], 256)
            self.assertEqual(sha(self.files[prefix + "weights.f32"]), before["weightsSha256"])
            self.assertEqual(before["weightsSha256"], after["weightsSha256"])
            self.assertEqual(sha(self.files["cases/observations.f32"]), after["observationsSha256"])
            self.assertGreater(len(floats(self.files[prefix + "weights.f32"])), 400000)
            for field, width in [("outputs", 2), ("states", state_width)]:
                actual = floats(self.files[prefix + "torch-" + field + ".f32"])
                old = floats(self.files[prefix + "before-" + field + ".f32"])
                new = floats(self.files[prefix + "after-" + field + ".f32"])
                self.assertEqual(len(actual), 256 * width)
                self.assertEqual(violations(actual[:width], old[:width], 1e-5, 1e-4),
                                 first_failures if field == "states" else 0)
                self.assertEqual(violations(actual[:width], new[:width], 1e-5, 1e-4), 0)
                self.assertEqual(violations(actual, new, 1e-3, 1e-3), 0)
                self.assertEqual(sha(self.files[prefix + "before-" + field + ".f32"]), before[field + "Sha256"])
                self.assertEqual(sha(self.files[prefix + "after-" + field + ".f32"]), after[field + "Sha256"])

    def test_debian_gap_and_rejected_prototype_remain_explicit(self):
        """Recompute the unresolved first-state failure instead of extending a Windows pass."""
        self.assertFalse(self.manifest["roundtripQualified"])
        self.assertTrue(self.manifest["windowsCpuRoundtripQualified"])
        self.assertTrue(self.manifest["windowsCudaRoundtripQualified"])
        self.assertFalse(self.manifest["debianCpuRoundtripQualified"])
        report = self.document("debian/cpu-extended.json")
        self.assertEqual(len(report["cases"]), 12)
        self.assertEqual(len(report["extendedCases"]), 6)
        self.assertFalse(report["extendedPassed"])
        self.assertFalse(report["extendedSse2Passed"])
        self.assertEqual(report["driverSha256"], sha(self.files["debian/collect-debian.py"]))
        failed = [x for x in report["extendedCases"] if not x["passed"]]
        self.assertEqual({(x["case"], x["backend"]) for x in failed},
                         {("large-lstm", "rust-scalar-v1"), ("large-lstm", "rust-sse2-v1")})
        torch_states = floats(self.files["debian/torch-states.f32"])
        self.assertEqual(len(torch_states), 256 * 192)
        for backend in ["scalar", "sse2"]:
            actual = floats(self.files["debian/" + backend + "-states.f32"])
            receipt = self.document("debian/" + backend + "-receipt.json")
            self.assertEqual(receipt["sourceSha256"], self.manifest["nativeSourceSha256"])
            self.assertEqual(receipt["statesSha256"], sha(self.files["debian/" + backend + "-states.f32"]))
            self.assertEqual(receipt["observationsSha256"], sha(self.files["cases/observations.f32"]))
            self.assertEqual(violations(torch_states[:192], actual[:192], 1e-5, 1e-4), 1)
            self.assertEqual(violations(torch_states, actual, 1e-3, 1e-3), 0)
            case = next(x for x in failed if x["backend"] == "rust-" + backend + "-v1")
            self.assertEqual(case["weightsSha256"], receipt["weightsSha256"])
            self.assertEqual(case["checks"]["state"]["first"]["mismatchCount"], 1)
            self.assertTrue(case["checks"]["state"]["sequence"]["passed"])
        rejected = self.document("debian/rejected-optimizer-prototype.json")
        self.assertFalse(rejected["passed"])
        self.assertFalse(rejected["productionAdopted"])
        self.assertEqual(rejected["prototypeDriverSha256"],
                         sha(self.files["debian/probe-stable-cpu-optimizer.py"]))
        self.assertEqual(rejected["failure"]["atol"], 1e-5)
        self.assertEqual(rejected["failure"]["rtol"], 1e-4)
        self.assertEqual(rejected["failure"]["mismatchCount"], 1)

    def test_numeric_context_is_aggregate_only(self):
        """Numeric qualification must not expose competing applications or WSL names."""
        for name in ["before-resources.json", "after-resources.json", "debian/resources.json"]:
            observer = self.prior["host/system-resource-observer.py"] if name.startswith("debian/") else self.files["system-resource-observer.py"]
            aggregate_only(self, self.document(name), observer)


class CanonicalCpuDotEvidenceTests(unittest.TestCase):
    """Bind the actual corrected CPU reference to fixed-export and optimizer parity."""

    @classmethod
    def setUpClass(cls):
        """Read only confined checksummed data; archived scripts remain inert."""
        cls.manifest, cls.files = capsule("canonical-cpu-dot-v1")
        _, cls.previous = capsule("native-dot-numerics-v1")

    def document(self, name):
        """Decode one admitted JSON file."""
        return json.loads(self.files[name])

    def test_actual_optimizer_gate_on_all_three_targets(self):
        """Retain every backend/family after real optimization with the existing bounds."""
        digest = hashlib.sha256(b"pyrl.canonical-training-source.v1\0")
        for name in ["canonical_graph.py", "canonical_parity.py", "sequence_distillation.py"]:
            digest.update(name.encode())
            digest.update(self.files["pyrl/" + name].replace(b"\r\n", b"\n"))
        self.assertEqual(digest.hexdigest(), self.manifest["trainerSourceSha256"])
        expected = {"dense", "mlp", "gru", "lstm", "rru", "mixed",
                    "large-gru", "large-lstm", "large-rru"}
        for path in ["windows/cpu-roundtrip.json", "windows/cuda-roundtrip.json",
                     "debian/cpu-roundtrip.json"]:
            report = self.document(path)
            self.assertEqual(report["trainerSourceSha256"], digest.hexdigest())
            self.assertEqual(report["driverSha256"], sha(self.files["qualify.py"]))
            self.assertEqual(report["generatorSha256"],
                             sha(self.files["pyrl/generate_canonical_graph_fixtures.py"]))
            self.assertEqual(report["dtype"], "Float32")
            self.assertFalse(report["tf32"])
            self.assertEqual(len(report["cases"]), 18)
            self.assertEqual({x["case"] for x in report["cases"]}, expected)
            for name in expected:
                cases = [x for x in report["cases"] if x["case"] == name]
                self.assertEqual({x["backend"] for x in cases}, {"rust-scalar-v1", "rust-sse2-v1"})
                for case in cases:
                    self.assertEqual(case["nativeSourceSha256"], self.manifest["nativeSourceSha256"])
                    self.assertEqual(case["weightsSha256"], case["optimization"]["afterSha256"])
                    self.assertNotEqual(case["weightsSha256"], case["optimization"]["beforeSha256"])
                    self.assertEqual(case["optimization"]["trainingSamples"], 2048)
                    self.assertEqual(case["optimization"]["burnInSamples"], 1024)
                    for field in ["outputErrorGrowth", "stateErrorGrowth"]:
                        self.assertEqual([x["step"] for x in case[field]], [1, 64, 128, 256])
                        self.assertLess(max(x["maxAbs"] for x in case[field]), 1e-3)
        def comparison_ast(raw):
            """Ignore line numbers while preserving the complete unchanged comparison code."""
            return ast.dump(next(x for x in ast.parse(raw).body
                                 if isinstance(x, ast.FunctionDef) and x.name == "_compare"))
        self.assertEqual(comparison_ast(self.files["pyrl/canonical_parity.py"]),
                         comparison_ast(self.previous["pyrl/canonical_parity.py"]))
        self.assertTrue(self.manifest["roundtripQualified"])
        self.assertFalse(self.manifest["performanceQualified"])
        self.assertFalse(self.manifest["fullCapacitySoakQualified"])

    def test_cuda_optimizer_exports_remain_identical(self):
        """A CPU-only arithmetic correction preserves every prior CUDA trained export."""
        previous = {(x["case"], x["backend"]): x for x in
                    json.loads(self.previous["cuda-roundtrip.json"])["cases"]}
        for case in self.document("windows/cuda-roundtrip.json")["cases"]:
            old = previous[(case["case"], case["backend"])]
            self.assertEqual(case["weightsSha256"], old["weightsSha256"])
            self.assertEqual(case["optimization"], old["optimization"])

    def test_general_accuracy_uses_actual_helper_and_independent_oracle(self):
        """Cover all CPU matrix shapes without selecting just the formerly failing LSTM."""
        for platform in ["windows", "debian"]:
            report = self.document(platform + "/accuracy.json")
            self.assertFalse(report["prototype"])
            self.assertEqual(report["block"], 16)
            self.assertEqual(report["driverSha256"], sha(self.files["accuracy.py"]))
            self.assertEqual(report["canonicalGraphSha256"], sha(self.files["pyrl/canonical_graph.py"]))
            rows = report["records"]
            self.assertEqual(len(rows), 384)
            self.assertEqual({(x["seed"], x["width"], x["rows"], x["batch"]) for x in rows},
                             {(seed, width, outputs, batch) for seed in range(4)
                              for width in [32, 51, 127, 128, 129, 147, 216, 248, 256, 264, 504, 512]
                              for outputs in [2, 32, 96, 256] for batch in [1, 32]})
            self.assertLess(max(x["compensatedRelativeSumAbs"] for x in rows), 2 * 2**-23)
            self.assertLess(sum(x["compensatedMaxAbs"]**2 for x in rows),
                            sum(x["directMaxAbs"]**2 for x in rows))

    def test_original_debian_export_is_corrected_without_retraining(self):
        """Compare the identical failing weights and native arrays with the new CPU reference."""
        report = self.document("fixed-lstm/report.json")
        self.assertTrue(report["passed"])
        self.assertTrue(report["fixedExport"])
        self.assertEqual(report["driverSha256"], sha(self.files["fixed-export.py"]))
        self.assertEqual(report["torchStatesSha256"], sha(self.files["fixed-lstm/torch-states.f32"]))
        torch_states = floats(self.files["fixed-lstm/torch-states.f32"])
        manifest = self.document("fixed-lstm/manifest.json")
        self.assertEqual(manifest["totalParameters"] * 4, len(self.files["fixed-lstm/weights.f32"]))
        for backend in ["scalar", "sse2"]:
            receipt = json.loads(self.previous["debian/" + backend + "-receipt.json"])
            self.assertEqual(receipt["weightsSha256"], sha(self.files["fixed-lstm/weights.f32"]))
            row = next(x for x in report["results"] if x["backend"] == "rust-" + backend + "-v1")
            self.assertEqual(row["nativeReceipt"], receipt)
            expected = floats(self.previous["debian/" + backend + "-states.f32"])
            self.assertEqual(violations(torch_states[:192], expected[:192], 1e-5, 1e-4), 0)
            self.assertEqual(violations(torch_states, expected, 1e-3, 1e-3), 0)

    def test_contemporaneous_resources_remain_aggregate_only(self):
        """Retain actual Windows/running-WSL/Debian/GPU totals without inventories."""
        for platform in ["windows", "debian"]:
            rows = self.document(platform + "/resources.json")
            self.assertGreaterEqual(len(rows), 2)
            self.assertEqual([x["sample"] for x in rows], list(range(len(rows))))
            aggregate_only(self, rows, self.files[platform + "/system-resource-observer.py"])


class CanonicalCpuRuntimeEvidenceTests(unittest.TestCase):
    """Separate faster actual CPU updates from still-unqualified integrated timing."""

    @classmethod
    def setUpClass(cls):
        """Load current code/data and exact earlier precision/native references."""
        cls.manifest, cls.files = capsule("canonical-cpu-runtime-v1")
        _, cls.previous = capsule("canonical-cpu-dot-v1")
        _, cls.native = capsule("native-dot-numerics-v1")

    def document(self, name):
        """Decode admitted original JSON without executing retained source."""
        return json.loads(self.files[name])

    def test_paired_actual_updates_bind_input_source_and_order(self):
        """Recompute fixed-input optimizer medians from every order-balanced pair."""
        self.assertFalse(self.manifest["performanceQualified"])
        self.assertFalse(self.manifest["fullCapacitySoakQualified"])
        for platform in ["windows", "debian"]:
            report = self.document(platform + "/benchmark.json")
            self.assertFalse(report["prototype"])
            self.assertEqual(report["driverSha256"], sha(self.files["benchmark-production.py"]))
            self.assertEqual(report["referenceHelperSha256"], sha(self.previous["pyrl/canonical_graph.py"]))
            self.assertEqual(report["candidateHelperSha256"], sha(self.files["pyrl/canonical_graph.py"]))
            self.assertEqual(report["cpuThreads"], 2)
            self.assertEqual(len(report["records"]), 16)
            for case in ["modest", "large-lstm"]:
                native = self.document(platform + "/" + case + "-native.json")
                self.assertEqual(native["sourceSha256"], self.manifest["nativeSourceSha256"])
                ratios, reference, candidate = [], [], []
                for index in range(4):
                    rows = [x for x in report["records"] if x["case"] == case and x["pass"] == index]
                    self.assertEqual([x["mode"] for x in rows],
                                     ["reference", "candidate"] if index in [0, 3] else ["candidate", "reference"])
                    pair = {x["mode"]: x for x in rows}
                    for row in rows:
                        self.assertEqual(row["inputWeightsSha256"], native["weightsSha256"])
                        self.assertEqual(row["optimization"]["beforeSha256"], native["weightsSha256"])
                        self.assertNotEqual(row["optimization"]["afterSha256"], native["weightsSha256"])
                        self.assertEqual(row["optimization"]["trainingSamples"], 2048)
                        self.assertEqual(row["optimization"]["burnInSamples"], 1024)
                        self.assertEqual(row["nativeSourceSha256"], self.manifest["nativeSourceSha256"])
                    a, b = pair["reference"], pair["candidate"]
                    self.assertEqual((a["inputSize"], a["totalParameters"]), (b["inputSize"], b["totalParameters"]))
                    self.assertLessEqual(abs(a["optimization"]["loss"] - b["optimization"]["loss"]),
                                         1e-5 + 1e-5 * abs(a["optimization"]["loss"]))
                    reference.append(a["seconds"])
                    candidate.append(b["seconds"])
                    ratios.append(b["seconds"] / a["seconds"])
                summary = next(x for x in self.manifest["summary"] if x["platform"] == platform and x["case"] == case)
                self.assertEqual(summary["medianReferenceSeconds"], statistics.median(reference))
                self.assertEqual(summary["medianCandidateSeconds"], statistics.median(candidate))
                self.assertEqual(summary["medianCandidateRatio"], statistics.median(ratios))
                self.assertLess(summary["medianCandidateRatio"], 1)

    def test_final_source_preserves_every_native_optimizer_roundtrip(self):
        """All families/backends retain real optimizer updates and the previous hard tolerances."""
        digest = hashlib.sha256(b"pyrl.canonical-training-source.v1\0")
        for name in ["canonical_graph.py", "canonical_parity.py", "sequence_distillation.py"]:
            digest.update(name.encode())
            digest.update(self.files["pyrl/" + name].replace(b"\r\n", b"\n"))
        self.assertEqual(digest.hexdigest(), self.manifest["trainerSourceSha256"])
        expected = {"dense", "mlp", "gru", "lstm", "rru", "mixed", "large-gru", "large-lstm", "large-rru"}
        for name in ["windows/cpu-roundtrip.json", "windows/cuda-roundtrip.json", "debian/cpu-roundtrip.json"]:
            report = self.document(name)
            self.assertEqual(report["trainerSourceSha256"], digest.hexdigest())
            self.assertEqual(report["driverSha256"], sha(self.files["qualify.py"]))
            self.assertEqual(report["generatorSha256"], sha(self.previous["pyrl/generate_canonical_graph_fixtures.py"]))
            self.assertEqual(report["dtype"], "Float32")
            self.assertFalse(report["tf32"])
            self.assertEqual(len(report["cases"]), 18)
            self.assertEqual({(x["case"], x["backend"]) for x in report["cases"]},
                             {(case, backend) for case in expected
                              for backend in ["rust-scalar-v1", "rust-sse2-v1"]})
            for case in report["cases"]:
                self.assertEqual(case["nativeSourceSha256"], self.manifest["nativeSourceSha256"])
                self.assertEqual(case["weightsSha256"], case["optimization"]["afterSha256"])
                self.assertNotEqual(case["weightsSha256"], case["optimization"]["beforeSha256"])
                for field in ["outputErrorGrowth", "stateErrorGrowth"]:
                    self.assertEqual([x["step"] for x in case[field]], [1, 64, 128, 256])
                    self.assertLess(max(x["maxAbs"] for x in case[field]), 1e-3)
        before = next(x for x in ast.parse(self.previous["pyrl/canonical_parity.py"]).body
                      if isinstance(x, ast.FunctionDef) and x.name == "_compare")
        after = next(x for x in ast.parse(self.files["pyrl/canonical_parity.py"]).body
                     if isinstance(x, ast.FunctionDef) and x.name == "_compare")
        self.assertEqual(ast.dump(before), ast.dump(after))

    def test_original_failure_stays_fixed_and_cuda_exports_are_identical(self):
        """Reuse exact old native arrays, not a retrained replacement or a wider limit."""
        report = self.document("fixed-lstm/report.json")
        self.assertTrue(report["fixedExport"])
        self.assertTrue(report["passed"])
        self.assertEqual(report["driverSha256"], sha(self.files["fixed-export.py"]))
        actual = floats(self.files["fixed-lstm/torch-states.f32"])
        self.assertEqual(report["torchStatesSha256"], sha(self.files["fixed-lstm/torch-states.f32"]))
        for backend in ["scalar", "sse2"]:
            expected = floats(self.native["debian/" + backend + "-states.f32"])
            row = next(x for x in report["results"] if x["backend"] == "rust-" + backend + "-v1")
            self.assertEqual(row["nativeReceipt"], json.loads(self.native["debian/" + backend + "-receipt.json"]))
            self.assertEqual(violations(actual[:192], expected[:192], 1e-5, 1e-4), 0)
            self.assertEqual(violations(actual, expected, 1e-3, 1e-3), 0)
        previous = {(x["case"], x["backend"]): x for x in json.loads(self.previous["windows/cuda-roundtrip.json"])["cases"]}
        for case in self.document("windows/cuda-roundtrip.json")["cases"]:
            old = previous[(case["case"], case["backend"])]
            self.assertEqual(case["optimization"], old["optimization"])
            self.assertEqual(case["weightsSha256"], old["weightsSha256"])

    def test_accuracy_covers_threshold_tails_batches_and_shapes(self):
        """Retain broad independent Float64 comparisons including direct short rows."""
        for platform in ["windows", "debian"]:
            report = self.document(platform + "/accuracy.json")
            self.assertFalse(report["prototype"])
            self.assertEqual(report["driverSha256"], sha(self.files["accuracy.py"]))
            self.assertEqual(report["canonicalGraphSha256"], sha(self.files["pyrl/canonical_graph.py"]))
            rows = report["records"]
            self.assertEqual(len(rows), 384)
            self.assertEqual({(x["seed"], x["width"], x["rows"], x["batch"]) for x in rows},
                             {(seed, width, outputs, batch) for seed in range(4)
                              for width in [32, 51, 127, 128, 129, 147, 216, 248, 256, 264, 504, 512]
                              for outputs in [2, 32, 96, 256] for batch in [1, 32]})
            for row in rows:
                if row["width"] < self.manifest["minimumCpuWidth"]:
                    self.assertEqual(row["compensatedMaxAbs"], row["directMaxAbs"])
            self.assertLess(max(x["compensatedRelativeSumAbs"] for x in rows), 2 * 2**-23)
            self.assertLess(sum(x["compensatedMaxAbs"]**2 for x in rows),
                            sum(x["directMaxAbs"]**2 for x in rows))

    def test_all_measurements_have_aggregate_resource_brackets(self):
        """Reject inventories in continuous and final Windows/WSL/Linux/GPU snapshots."""
        observer = self.files["system-resource-observer.py"]
        for platform in ["windows", "debian"]:
            for phase in ["qualification", "benchmark"]:
                for suffix in ["resources", "after-resources"]:
                    rows = self.document(platform + "/" + phase + "-" + suffix + ".json")
                    self.assertGreaterEqual(len(rows), 1)
                    self.assertEqual([x["sample"] for x in rows], list(range(len(rows))))
                    if suffix == "after-resources":
                        self.assertEqual(len(rows), 1)
                    aggregate_only(self, rows, observer)


class PublishedDotEvidencePrivacyTests(unittest.TestCase):
    """Reject private host inventories across every retained JSON document."""

    def test_json_documents_exclude_inventory_fields(self):
        """App/process names, commands, PIDs and distribution identities cannot be published."""
        forbidden = {"hostname", "username", "commandline", "command", "processes",
                     "processname", "pid", "pids", "distributionname", "distributionid"}

        def visit(value):
            """Walk bounded already admitted JSON data, including nested reports."""
            if isinstance(value, dict):
                self.assertFalse(forbidden.intersection(key.lower() for key in value))
                for item in value.values():
                    visit(item)
            elif isinstance(value, list):
                for item in value:
                    visit(item)

        for name in ["native-dot-performance-v1", "native-dot-numerics-v1", "canonical-cpu-dot-v1", "canonical-cpu-runtime-v1"]:
            _, files = capsule(name)
            for path, raw in files.items():
                if path.endswith((".json", ".slither-brain")):
                    with self.subTest(capsule=name, file=path):
                        visit(brain_metadata(raw) if path.endswith(".slither-brain") else json.loads(raw))


if __name__ == "__main__":
    unittest.main()
