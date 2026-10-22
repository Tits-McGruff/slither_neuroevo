"""Immutable opaque family inputs for actual native paired performance runs."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "compare_sensor_workloads", Path(__file__).with_name("compare-sensor-workloads.py"))
DRIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DRIVER)


class CohortInputTests(unittest.TestCase):
    """Keep the exact source family fixed while native processes own admission."""

    def setUp(self):
        """Create only two small opaque input files in an exclusive test tree."""
        self.temporary = tempfile.TemporaryDirectory(prefix="slither-paired-cohort-")
        self.root = Path(self.temporary.name).resolve(strict=True)
        self.entries = []
        for index in range(2):
            file = f"family-{index}.slither-brain"
            raw = f"opaque input {index}, not a Python graph".encode("utf-8")
            (self.root / file).write_bytes(raw)
            self.entries.append({"file": file, "sha256": hashlib.sha256(raw).hexdigest()})
        self.index = self.root / "cohort.json"
        self.write()

    def tearDown(self):
        """Remove only this test's private temporary tree."""
        self.temporary.cleanup()

    def write(self, entries=None, schema=1):
        """Write one caller-controlled metadata index without interpreting artifacts."""
        self.index.write_text(json.dumps({
            "schemaVersion": schema, "artifacts": self.entries if entries is None else entries,
        }), encoding="utf-8")

    def test_binds_ordered_opaque_family_inputs_without_admitting_a_graph(self):
        """Checksums bind byte inputs only; the actual CLI must still call Rust."""
        sources = DRIVER.cohort_sources(self.index)
        self.assertEqual(list(sources), [self.index, *(self.root / e["file"] for e in self.entries)])
        self.assertEqual(list(sources.values())[1:], [e["sha256"] for e in self.entries])
        DRIVER.require_unchanged(sources)

    def test_rejects_replacement_or_removal_between_measured_processes(self):
        """Neither the index nor any family may change after the first process."""
        sources = DRIVER.cohort_sources(self.index)
        self.index.write_text(self.index.read_text(encoding="utf-8") + "\n", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "changed"):
            DRIVER.require_unchanged(sources)
        self.write()
        sources = DRIVER.cohort_sources(self.index)
        (self.root / self.entries[0]["file"]).write_bytes(b"replacement")
        with self.assertRaisesRegex(RuntimeError, "changed"):
            DRIVER.require_unchanged(sources)
        (self.root / self.entries[1]["file"]).unlink()
        with self.assertRaisesRegex(RuntimeError, "changed"):
            DRIVER.require_unchanged(sources)

    def test_rejects_escaping_and_nonportable_family_names(self):
        """Reject path redirects before opening any out-of-tree file."""
        for name in ("../escape.slither-brain", r"a\b.slither-brain",
                     "a/b.slither-brain", "C:escape.slither-brain", "\0.slither-brain"):
            self.write([dict(self.entries[0], file=name), self.entries[1]])
            with self.subTest(name=repr(name)):
                with self.assertRaisesRegex(RuntimeError, "confined"):
                    DRIVER.cohort_sources(self.index)

    def test_rejects_duplicate_missing_or_damaged_families(self):
        """A source receipt must identify each existing artifact exactly once."""
        for entries in ([self.entries[0], self.entries[0]],
                        [dict(self.entries[0], file="missing.slither-brain"), self.entries[1]],
                        [dict(self.entries[0], sha256="0" * 64), self.entries[1]]):
            self.write(entries)
            with self.assertRaises(RuntimeError):
                DRIVER.cohort_sources(self.index)

    def test_rejects_schema_family_count_and_oversized_index(self):
        """Keep metadata bounded before JSON parsing or source hashing."""
        for schema in (True, 0, 2, "1"):
            self.write(schema=schema)
            with self.assertRaisesRegex(RuntimeError, "schema"):
                DRIVER.cohort_sources(self.index)
        for entries in ([], self.entries[:1], self.entries * 9):
            self.write(entries)
            with self.assertRaisesRegex(RuntimeError, "2..16"):
                DRIVER.cohort_sources(self.index)
        self.index.write_bytes(b" " * (64 * 1024 + 1))
        with self.assertRaisesRegex(RuntimeError, "bounded"):
            DRIVER.cohort_sources(self.index)


if __name__ == "__main__":
    unittest.main()
