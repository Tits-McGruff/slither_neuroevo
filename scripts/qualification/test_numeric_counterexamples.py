"""Four minimized first-step scalar failures retain the original Float32 tolerance."""
import hashlib
import json
from pathlib import Path
import struct
import unittest


def f32(value):
    """Mirror the original comparison's Float32 threshold arithmetic."""
    return struct.unpack("<f", struct.pack("<f", value))[0]


class NumericalCounterexamples(unittest.TestCase):
    """Known old failures and corrected values cannot silently lose their gate."""

    def test_original_four_first_step_failures(self):
        """Keep exact LSTM/RRU first-state failures without whole trained-model collections."""
        root = Path(__file__).with_name("fixtures") / "scalar-threshold-v1"
        data = (root / "counterexamples.json").read_bytes()
        receipt = json.loads((root / "fixture.json").read_bytes())["files"]["counterexamples.json"]
        self.assertEqual(hashlib.sha256(data).hexdigest(), receipt["sha256"])
        cases = json.loads(data)
        self.assertEqual([x["case"] for x in cases], ["lstm", "lstm", "lstm", "rru"])
        for row in cases:
            threshold = f32(f32(1e-5) + f32(f32(1e-4) * abs(row["expected"])))
            self.assertGreater(abs(f32(row["before"] - row["expected"])), threshold)
            self.assertLessEqual(abs(f32(row["after"] - row["expected"])), threshold)
