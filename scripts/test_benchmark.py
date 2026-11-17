import unittest
from pathlib import Path
import tempfile
from unittest.mock import patch
from benchmark import scanner_fingerprints, summarize


class CoverageGateTests(unittest.TestCase):
    def test_scanner_fingerprints_include_nested_modules(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = ['crates/disk-scan/src/lib.rs',
                     'crates/disk-scan/src/clones/accounting.rs',
                     'crates/disk-scan/src/tree/records.rs',
                     'crates/disk-scan/Cargo.toml', 'Cargo.lock']
            for relative in paths:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('before')
            with patch('benchmark.ROOT', root):
                before = scanner_fingerprints()
                self.assertEqual(set(before), set(paths))
                nested = 'crates/disk-scan/src/tree/records.rs'
                (root / nested).write_text('after')
                after = scanner_fingerprints()
                self.assertNotEqual(before[nested], after[nested])
                self.assertEqual({path for path in before if before[path] != after[path]}, {nested})

    def pair(self, entries=10, allocated=100, elapsed=5):
        return [dict(scope="fixture", app="hahakigami", phase="warm", round=1, success=True,
                     entries=10, allocated_bytes=100, errors=0, elapsed_ms=5),
                dict(scope="fixture", app="opendisk", phase="warm", round=1, success=True,
                     entries=entries, allocated_bytes=allocated, errors=0, elapsed_ms=elapsed)]

    def test_faster_with_missing_entries_does_not_pass(self):
        scope = summarize(self.pair(entries=20, elapsed=10))["fixture"]
        self.assertTrue(scope["raw_speed_target_met"])
        self.assertFalse(scope["equivalent_coverage_speed_gate"])
        self.assertFalse(scope["acceptance_met"])

    def test_allocated_difference_does_not_pass(self):
        self.assertFalse(summarize(self.pair(allocated=101))["fixture"]["equivalent_coverage_speed_gate"])

    def test_equal_aggregate_totals_are_not_app_acceptance(self):
        scope = summarize(self.pair())["fixture"]
        self.assertTrue(scope["equivalent_coverage_speed_gate"])
        self.assertFalse(scope["acceptance_met"])

    def test_failed_process_never_passes(self):
        runs = self.pair()
        runs[1]["success"] = False
        self.assertFalse(summarize(runs)["fixture"]["equivalent_coverage_speed_gate"])


if __name__ == "__main__":
    unittest.main()
