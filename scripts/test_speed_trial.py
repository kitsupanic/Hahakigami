import unittest
from unittest.mock import patch
from pathlib import Path
from types import SimpleNamespace
import subprocess
import tempfile
import json
from speed_trial import summarize, run, requested_workers, manifest_record


def record(variant, number, elapsed=100):
    return dict(variant=variant, scope='fixture', phase='warm', round=number, elapsed_ms=elapsed,
                state='completed', success=True, entries=10, directories=2, errors=0,
                allocated_bytes=200, peak_rss_bytes=1000)


class SummaryTests(unittest.TestCase):
    def runs(self):
        return [record(v, n, 100 if v == 'baseline' else 80) for n in range(1, 4) for v in ('baseline', 'candidate')]

    def test_initial_excluded_and_no_acceptance_claim(self):
        runs = self.runs()
        initial = record('candidate', 0, 9999)
        initial['phase'] = 'initial'
        summary = summarize(runs + [initial])['fixture']
        self.assertEqual(summary['warm_median_ms'], {'baseline': 100, 'candidate': 80})
        self.assertTrue(summary['equivalent_aggregate_speed_gate'])
        self.assertFalse(summary['acceptance_met'])
        self.assertFalse(summary['app_level_comparison'])
        self.assertEqual(len(summary['initial_runs']), 1)

    def test_failure_incomplete_missing_pair_and_coverage_drift_fail_gate(self):
        for mutation in ({'success': False}, {'state': 'cancelled'}, {'errors': 1}, {'allocated_bytes': 201}, {'directories': 3}):
            with self.subTest(mutation=mutation):
                runs = self.runs()
                runs[-1].update(mutation)
                self.assertFalse(summarize(runs)['fixture']['equivalent_aggregate_speed_gate'])
        self.assertFalse(summarize(self.runs()[:-1])['fixture']['equivalent_aggregate_speed_gate'])

    def test_no_successful_timings(self):
        runs = self.runs()
        for r in runs:
            r['success'] = False
        self.assertIsNone(summarize(runs)['fixture']['candidate_over_baseline_ratio'])


class RunnerTests(unittest.TestCase):
    def test_supplied_manifest_preserves_snapshot_and_fingerprint(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'manifest.json'
            path.write_text(json.dumps({'scanner.rs': 'snapshot-hash'}))
            manifest = manifest_record(path)
            self.assertEqual(manifest['contents'], {'scanner.rs': 'snapshot-hash'})
            self.assertEqual(manifest['path'], str(path.resolve()))
            self.assertEqual(len(manifest['sha256']), 64)
            self.assertIsNone(manifest_record(None))

    def test_worker_overrides_use_variant_and_shared_default(self):
        args = SimpleNamespace(workers=8, candidate_workers=12, baseline_workers=None)
        self.assertEqual(requested_workers(args, 'candidate'), 12)
        self.assertEqual(requested_workers(args, 'baseline'), 8)
        self.assertEqual(requested_workers(SimpleNamespace(workers=4), 'candidate'), 4)

    def test_mutated_binary_rejects_result(self):
        args = SimpleNamespace(workers=8, timeout=1)
        result = SimpleNamespace(returncode=0, stderr='', stdout='{}')
        with patch('speed_trial.file_hash', side_effect=['original', 'changed']), patch('speed_trial.subprocess.run', return_value=result), patch('builtins.print'):
            result = run('candidate', Path('/tmp/binary'), 'original', 'fixture', '/tmp/fixture', 'warm', 1, args)
        self.assertFalse(result['success'])
        self.assertIn('changed during scan', result['error'])

    def test_timeout_recorded_without_losing_report(self):
        args = SimpleNamespace(workers=8, timeout=1)
        with patch('speed_trial.file_hash', return_value='original'), patch('speed_trial.subprocess.run', side_effect=subprocess.TimeoutExpired('scan', 1)), patch('builtins.print'):
            result = run('candidate', Path('/tmp/binary'), 'original', 'fixture', '/tmp/fixture', 'warm', 1, args)
        self.assertFalse(result['success'])
        self.assertIn('timed out', result['error'])
        self.assertGreaterEqual(result['wall_ms'], 0)


if __name__ == '__main__':
    unittest.main()
