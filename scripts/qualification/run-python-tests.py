"""Explicit source versus required historical qualification test selection."""
import argparse
from pathlib import Path
import sys
import unittest

HISTORICAL = {"test_cuda_curve_evidence.py", "test_final_native_evidence.py",
              "test_final_teacher_agreement.py", "test_native_dot_evidence.py",
              "test_sensing_profile_evidence.py", "test_resource_counter_evidence.py"}


def main():
    """Selecting historical/all fails on unavailable external evidence; nothing is skipped."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kind", choices=("source", "evidence", "all"), default="all")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    suite = unittest.TestSuite()
    for path in sorted(root.glob("test_*.py")):
        historical = path.name in HISTORICAL
        if args.kind == "all" or historical == (args.kind == "evidence"):
            suite.addTests(unittest.defaultTestLoader.discover(str(root), pattern=path.name))
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    sys.exit(not result.wasSuccessful())


if __name__ == "__main__":
    main()
