"""Public resource evidence must never carry the host's application inventory."""
import copy
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "resource_counter_export", Path(__file__).with_name("export-resource-counters.py"))
EXPORTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EXPORTER)


def observation():
    """Supply realistic Windows/WSL counters plus text that must not be published."""
    return {"schemaVersion": 1, "utc": "2026-10-24T00:10:00+00:00", "platform": "Windows",
            "sample": 0, "observerSourceSha256": "a" * 64, "processDetailsIncluded": False,
            "collectionSeconds": 1.5, "attribution": "PRIVATE-ATTRIBUTION",
            "host": {"available": True, "cpuPercent": 30, "totalMemoryKiB": 64000,
                     "freeMemoryKiB": 32000, "unknown": "PRIVATE-UNKNOWN"},
            "wsl": {"available": True, "distributions": [
                {"available": True, "cpuTicks": [1, 2, 3, 4],
                 "loadAverage": [0.1, 0.2, 0.3],
                 "memoryKiB": {"MemTotal": 1000, "MemAvailable": 400}},
                {"available": False, "error": "PRIVATE-ERROR", "exitCode": 1, "oversized": False}]},
            "gpu": {"available": True, "rows": [
                {"name": "PRIVATE-GPU-MODEL", "utilization.gpu [%]": "95",
                 "utilization.memory [%]": "15", "memory.used [MiB]": "3000",
                 "memory.total [MiB]": "8000", "power.draw [W]": "125.54",
                 "temperature.gpu": "[N/A]"}]}}


class PublicResourceCounterTests(unittest.TestCase):
    """Check every retained data family, refusal path and non-overwriting CLI."""

    def test_windows_wsl_gpu_values_survive_without_names_or_free_text(self):
        original = observation()
        before = copy.deepcopy(original)
        result = EXPORTER.public_snapshot(original)
        self.assertEqual(original, before)
        self.assertNotIn("PRIVATE", json.dumps(result))
        self.assertEqual(result["host"], {"available": True, "cpuPercent": 30,
                                         "totalMemoryKiB": 64000, "freeMemoryKiB": 32000})
        self.assertEqual(result["wsl"]["distributions"][0], original["wsl"]["distributions"][0])
        self.assertEqual(result["wsl"]["distributions"][1],
                         {"available": False, "error": "counter unavailable", "exitCode": 1, "oversized": False})
        self.assertEqual(result["gpu"]["rows"][0]["utilization.gpu [%]"], "95")
        self.assertEqual(result["gpu"]["rows"][0]["temperature.gpu"], "[N/A]")
        self.assertNotIn("name", result["gpu"]["rows"][0])

    def test_linux_and_missing_counters_never_imply_an_idle_host(self):
        data = observation()
        data["platform"] = "Linux"
        data["host"] = data.pop("wsl")["distributions"][0]
        data["gpu"] = {"available": False, "error": "PRIVATE-ERROR"}
        result = EXPORTER.public_snapshot(data)
        self.assertEqual(result["host"], data["host"])
        self.assertFalse(result["gpu"]["available"])
        self.assertEqual(result["gpu"]["error"], "counter unavailable")
        self.assertNotIn("wsl", result)
        data["host"] = {"available": False}
        self.assertEqual(EXPORTER.public_snapshot(data)["host"], {"available": False})

    def test_debug_markers_and_private_keys_fail_even_in_unknown_fields(self):
        for marker in [True, 0, None, "false"]:
            data = observation()
            data["processDetailsIncluded"] = marker
            with self.subTest(marker=marker), self.assertRaisesRegex(ValueError, "process details"):
                EXPORTER.public_snapshot(data)
        data = observation()
        del data["processDetailsIncluded"]
        with self.assertRaisesRegex(ValueError, "process details"):
            EXPORTER.public_snapshot(data)
        for key in ["processes", "gpuProcesses", "PID", "pids", "process_name", "processName",
                    "commandLine", "IDProcess", "distribution", "distributionName",
                    "distribution_id", "hostname", "username"]:
            data = observation()
            data["unknown"] = [{"nested": {key: "PRIVATE"}}]
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "private"):
                EXPORTER.public_snapshot(data)

    def test_typed_fields_cannot_smuggle_private_strings_or_nonfinite_values(self):
        changes = [
            ("host", "cpuPercent", "PRIVATE"), ("host", "cpuPercent", True),
            ("host", "freeMemoryKiB", 1.5), ("host", "cpuPercent", 101),
            ("top", "collectionSeconds", float("nan")), ("top", "collectionSeconds", float("inf")),
            ("top", "collectionSeconds", 10**300), ("top", "sample", True),
            ("top", "utc", "PRIVATE"), ("top", "utc", "2026-10-24T10:10:00+10:00"),
            ("top", "observerSourceSha256", "PRIVATE"),
        ]
        for group, key, value in changes:
            data = observation()
            (data if group == "top" else data[group])[key] = value
            with self.subTest(group=group, key=key, value=value), self.assertRaises(ValueError):
                EXPORTER.public_snapshot(data)
        for key in EXPORTER.GPU_FIELDS:
            data = observation()
            data["gpu"]["rows"][0][key] = "PRIVATE"
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "GPU"):
                EXPORTER.public_snapshot(data)
        data = observation()
        data["wsl"]["distributions"][0]["cpuTicks"][0] = "PRIVATE"
        with self.assertRaises(ValueError):
            EXPORTER.public_snapshot(data)
        data = observation()
        data["wsl"]["distributions"][0]["memoryKiB"]["MemTotal"] = "PRIVATE"
        with self.assertRaises(ValueError):
            EXPORTER.public_snapshot(data)

    def test_bounded_shapes_and_duplicate_json_keys_reject(self):
        for group, key, value in [
            ("gpu", "rows", [{}] * 33), ("wsl", "distributions", [{}] * 17),
            ("host", "available", 1), ("gpu", "rows", [{"name": "PRIVATE"}]),
        ]:
            data = observation()
            data[group][key] = value
            with self.subTest(group=group, key=key), self.assertRaises(ValueError):
                EXPORTER.public_snapshot(data)
        for raw in [b'{"processDetailsIncluded":true,"processDetailsIncluded":false}',
                    b'{"value":NaN}', b'{"value":Infinity}', b'{"value":']:
            with self.subTest(raw=raw), self.assertRaisesRegex(ValueError, "invalid resource JSON"):
                EXPORTER.decode_json(raw)

    def test_complete_validation_precedes_file_creation_and_preserves_originals(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "local.jsonl"
            destination = Path(root) / "public.jsonl"
            first = observation()
            second = observation()
            second["sample"] = 1
            source.write_text(json.dumps(first) + "\n" + json.dumps(second) + "\n", encoding="utf-8")
            raw = source.read_bytes()
            count, size = EXPORTER.export_counters(source, destination)
            self.assertEqual((count, size), (2, destination.stat().st_size))
            self.assertNotIn(b"PRIVATE", destination.read_bytes())
            self.assertEqual(source.read_bytes(), raw)
            public = destination.read_bytes()
            with self.assertRaises(FileExistsError):
                EXPORTER.export_counters(source, destination)
            self.assertEqual(destination.read_bytes(), public)
            with self.assertRaises(FileExistsError):
                EXPORTER.export_counters(source, source)
            self.assertEqual(source.read_bytes(), raw)
            destination.unlink()
            for bad in [b"invalid JSON", json.dumps({**second, "processDetailsIncluded": True}).encode()]:
                source.write_bytes(json.dumps(first).encode() + b"\n" + bad + b"\n")
                with self.assertRaises(ValueError):
                    EXPORTER.export_counters(source, destination)
                self.assertFalse(destination.exists())

    def test_stream_bounds_and_changed_receipts_leave_no_public_file(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "local.jsonl"
            destination = Path(root) / "public.jsonl"
            first = observation()
            for second in [observation(), {**observation(), "observerSourceSha256": "b" * 64, "sample": 1}]:
                source.write_text(json.dumps(first) + "\n" + json.dumps(second) + "\n", encoding="utf-8")
                with self.assertRaises(ValueError):
                    EXPORTER.export_counters(source, destination)
                self.assertFalse(destination.exists())
            source.write_text(json.dumps(first) + "\n", encoding="utf-8")
            for constant in ["MAX_LINE_BYTES", "MAX_INPUT_BYTES", "MAX_OUTPUT_BYTES", "MAX_SAMPLES"]:
                with patch.object(EXPORTER, constant, 1 if constant != "MAX_SAMPLES" else 0):
                    with self.subTest(constant=constant), self.assertRaises(ValueError):
                        EXPORTER.export_counters(source, destination)
                self.assertFalse(destination.exists())
            source.write_bytes(b"")
            with self.assertRaises(ValueError):
                EXPORTER.export_counters(source, destination)
            self.assertFalse(destination.exists())

    def test_cli_emits_only_counts_or_fixed_errors_without_input_values_or_paths(self):
        with tempfile.TemporaryDirectory(prefix="PRIVATE-") as root:
            source = Path(root) / "PRIVATE-local.jsonl"
            destination = Path(root) / "PRIVATE-public.jsonl"
            source.write_text(json.dumps({**observation(), "processDetailsIncluded": True}) + "\n",
                              encoding="utf-8")
            result = subprocess.run([sys.executable, "-B", str(Path(EXPORTER.__file__)),
                                     "--input", str(source), "--output", str(destination)],
                                    capture_output=True, check=False, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertNotIn(b"PRIVATE", result.stdout + result.stderr)
            self.assertFalse(destination.exists())
            source.unlink()
            missing = subprocess.run([sys.executable, "-B", str(Path(EXPORTER.__file__)),
                                      "--input", str(source), "--output", str(destination)],
                                     capture_output=True, check=False, timeout=10)
            self.assertEqual(missing.returncode, 2)
            self.assertNotIn(b"PRIVATE", missing.stdout + missing.stderr)
            source.write_text(json.dumps(observation()) + "\n", encoding="utf-8")
            accepted = subprocess.run([sys.executable, "-B", str(Path(EXPORTER.__file__)),
                                       "--input", str(source), "--output", str(destination)],
                                      capture_output=True, check=False, timeout=10)
            self.assertEqual(accepted.returncode, 0)
            self.assertIn(b"Exported 1 aggregate observations", accepted.stdout)
            self.assertNotIn(b"PRIVATE", accepted.stdout + accepted.stderr + destination.read_bytes())

    def test_profile_clock_resource_receipts_contain_only_guarded_aggregate_counters(self):
        """Re-export every actual Windows/WSL row without admitting an inventory field."""
        root = Path(__file__).resolve().parents[2] / "docs/performance/sensor-v4/profile-width-wall-windows-v1"
        manifest = json.loads((root / "evidence.json").read_bytes())
        receipt = manifest["files"]["resources.json"]
        encoded = (root / "resources.json.gz").read_bytes()
        self.assertEqual(hashlib.sha256(encoded).hexdigest(), receipt["gzipSha256"])
        with gzip.open(root / "resources.json.gz", "rb") as stream:
            raw = stream.read(receipt["bytes"] + 1)
        self.assertEqual(len(raw), receipt["bytes"])
        self.assertEqual(hashlib.sha256(raw).hexdigest(), receipt["sha256"])
        sequences = json.loads(raw)
        self.assertEqual({key: len(value) for key, value in sequences.items()},
                         manifest["resourceSamples"])
        for rows in sequences.values():
            for row in rows:
                self.assertEqual(EXPORTER.public_snapshot(row), row)
                self.assertFalse(row["processDetailsIncluded"])

    def test_real_retained_windows_debian_and_micro_counters_are_accepted(self):
        root = Path(__file__).resolve().parents[2] / "docs/performance/sensor-v4/pellet-key-screening-v1"
        count = 0
        for platform in ["windows", "debian", "micro"]:
            rows = json.loads(gzip.decompress((root / platform / "resources.json.gz").read_bytes()))
            for sequence in rows.values():
                for row in sequence:
                    public = EXPORTER.public_snapshot(row)
                    self.assertEqual(public["host"], row["host"])
                    if row["gpu"]["available"]:
                        self.assertEqual(public["gpu"], row["gpu"])
                    else:
                        self.assertFalse(public["gpu"]["available"])
                    self.assertEqual(public.get("wsl"), row.get("wsl"))
                    count += 1
        self.assertGreaterEqual(count, 19)


if __name__ == "__main__":
    unittest.main()
