import unittest
from unittest.mock import patch
from pathlib import Path
from types import SimpleNamespace
from qos_trial import rotated_order, validate, summarize, run


def fixture(mode='userInitiated'):
    return {'workers': 1, 'backend': 'Bulk', 'state': 'Completed', 'scan_qos': mode,
            'stats': {'entries': 2, 'directories': 1, 'errors': 0, 'logical_bytes': 3, 'allocated_bytes': 4, 'elapsed_ms': 5},
            'worker_stats': [{'id': 1, 'entries': 1, 'errors': 0, 'qos': mode, 'qosError': None, 'cpuUs': None}]}


class QosTests(unittest.TestCase):
    def test_subset_rotation_and_summary(self):
        modes = ('automatic', 'userInitiated')
        self.assertEqual(rotated_order(1, modes), ('userInitiated', 'automatic'))
        runs = [{'mode': m, 'phase': 'warm', 'success': True, 'raw': fixture(m)} for m in modes]
        summary = summarize(runs, modes)
        self.assertTrue(summary['warm_aggregate_coverage_stable'])
        self.assertEqual(set(summary['warm_median_ms']), set(modes))
        self.assertNotIn('background', summary['percent_elapsed_change_vs_automatic'])

    def test_rotated_order(self):
        self.assertEqual(rotated_order(0), ('automatic', 'userInitiated', 'background'))
        self.assertEqual(rotated_order(1), ('userInitiated', 'background', 'automatic'))
        self.assertEqual(rotated_order(3), rotated_order(0))

    def test_validation_and_automatic_inherited_class(self):
        self.assertTrue(validate(fixture(), 1, 'userInitiated')['accounting_valid'])
        raw = fixture('automatic')
        raw['worker_stats'][0]['qos'] = 'default'
        validate(raw, 1, 'automatic')

    def test_missing_failure_and_wrong_qos_rejected(self):
        for field, value in [('scan_qos', None), ('state', 'Cancelled'), ('workers', 2)]:
            raw = fixture()
            raw[field] = value
            with self.assertRaises(ValueError):
                validate(raw, 1, 'userInitiated')
        for field, value in [('qosError', 22), ('qos', 'background')]:
            raw = fixture()
            raw['worker_stats'][0][field] = value
            with self.assertRaises(ValueError):
                validate(raw, 1, 'userInitiated')
        raw = fixture()
        del raw['worker_stats'][0]['qosError']
        with self.assertRaises(ValueError):
            validate(raw, 1, 'userInitiated')

    def test_mutation_rejects_evidence(self):
        with patch('qos_trial.file_hash', side_effect=['hash', 'changed']), patch('qos_trial.subprocess.run', return_value=SimpleNamespace(returncode=0, stderr='', stdout='{}')):
            result = run(Path('/tmp/scan'), 'hash', Path('/'), 1, 'automatic', 0, 1)
        self.assertFalse(result['success'])
        self.assertIn('changed during', result['error'])

    def test_initial_excluded_and_drift_gated(self):
        runs = [{'mode': m, 'phase': 'warm', 'success': True, 'raw': fixture(m)} for m in rotated_order(0)]
        runs.append({'mode': 'automatic', 'phase': 'initial', 'success': True, 'raw': fixture('automatic')})
        runs[-1]['raw']['stats']['elapsed_ms'] = 999
        self.assertEqual(summarize(runs)['warm_median_ms']['automatic'], 5)
        self.assertTrue(summarize(runs)['warm_aggregate_coverage_stable'])
        runs[1]['raw']['stats']['entries'] = 9
        self.assertFalse(summarize(runs)['warm_aggregate_coverage_stable'])
        self.assertFalse(summarize(runs)['acceptance_met'])


if __name__ == '__main__':
    unittest.main()
