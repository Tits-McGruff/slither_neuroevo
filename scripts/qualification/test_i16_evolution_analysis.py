"""Small statistical correctness checks for the paired evolution reduction."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("i16_evolution_analysis", Path(__file__).with_name("analyze-i16-evolution.py"))
ANALYSIS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ANALYSIS)


class PairedStatisticsTest(unittest.TestCase):
    def test_zero_effect_is_not_evidence_of_difference(self):
        result = ANALYSIS.paired([0.0] * 5)
        self.assertEqual(result["seedPairs"], 5)
        self.assertEqual(result["meanDifference"], 0.0)
        self.assertEqual(result["twoSidedExactSignFlipP"], 1.0)
        self.assertEqual(result["exploratoryBootstrap95PercentInterval"], [0.0, 0.0])

    def test_five_consistent_effects_cannot_claim_p_below_point_zero_five(self):
        # Both all-positive and all-negative sign assignments are as extreme.
        result = ANALYSIS.paired([1.0] * 5)
        self.assertEqual(result["twoSidedExactSignFlipP"], 2 / 32)
        self.assertEqual(result["exploratoryBootstrap95PercentInterval"], [1.0, 1.0])

    def test_balanced_effects_and_interpolated_quantiles(self):
        result = ANALYSIS.paired([-2.0, -1.0, 0.0, 1.0, 2.0])
        self.assertEqual(result["meanDifference"], 0.0)
        self.assertEqual(result["twoSidedExactSignFlipP"], 1.0)
        self.assertEqual(ANALYSIS.percentile([0.0, 10.0], .25), 2.5)
        self.assertIsNone(ANALYSIS.paired([]))


if __name__ == "__main__":
    unittest.main()
