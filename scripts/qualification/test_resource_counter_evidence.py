"""Required complete resource sequence checks against verified external evidence."""
import gzip
import hashlib
import json
from pathlib import Path
import unittest
from historical_evidence import required_root
from test_resource_counter_export import EXPORTER


class HistoricalResourceCounterTests(unittest.TestCase):
    """All original sequence lengths, hashes and privacy assertions stay mandatory."""
    def test_profile_clock_resource_receipts_contain_only_guarded_aggregate_counters(self):
        """Re-export every actual Windows/WSL row without admitting an inventory field."""
        base = required_root() / "docs/performance/sensor-v4"
        for capsule in ("profile-width-wall-windows-v1", "profile-width-wall-debian-v1"):
            root = base / capsule
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
        root = required_root() / "docs/performance/sensor-v4/pellet-key-screening-v1"
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
