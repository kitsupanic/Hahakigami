import unittest
from pathlib import Path
import tempfile
from unittest.mock import patch

from clone_accounting_benchmark import execute, normalize, raw_coverage, summarize, validate


class CloneBenchmarkTests(unittest.TestCase):
    def test_execute_resolves_relative_output_before_use(self):
        with tempfile.TemporaryDirectory() as tmp, patch('clone_accounting_benchmark.validate', side_effect=ValueError('stop')) as check:
            with patch('pathlib.Path.cwd', return_value=Path(tmp)), self.assertRaises(ValueError):
                execute({}, Path('out'))
        self.assertTrue(check.call_args.args[1].is_absolute())

    def test_native_uses_actual_engine_timer_not_process_wall(self):
        result = normalize({'success': True, 'wall_ms': 400, 'raw': {
            'stats': {'elapsed_ms': 123, 'entries': 10, 'allocated_bytes': 100,
                      'errors': 0, 'incomplete': False}, 'peak_rss_native_units': 4096,
            'accounting': {'deduction_bytes': 100}, 'workers': 3}}, 'native')
        self.assertEqual(result['elapsed_ms'], 123)
        self.assertEqual(result['accounting']['deduction_bytes'], 100)
        self.assertEqual(result['peak_rss_bytes'], 4096)
        self.assertTrue(result['coverage_complete'])

    def test_missing_timer_does_not_manufacture_speed(self):
        self.assertFalse(normalize({'success': True, 'raw': {}}, 'native')['success'])

    def test_policy_change_alone_is_not_raw_path_loss(self):
        report = {'complete': True, 'unknown_fields': 0, 'missing_paths': {'only_0': 0, 'only_1': 0},
                  'mismatches': {'allocated_bytes': 0}, 'equivalent': False, 'policy_compatible': False}
        with patch('clone_accounting_benchmark.compare', return_value=report):
            result = raw_coverage('baseline', 'candidate')
        self.assertTrue(result['raw_fields_match'])
        self.assertFalse(result['equivalent'])
        report['mismatches']['allocated_bytes'] = 1
        with patch('clone_accounting_benchmark.compare', return_value=report):
            self.assertFalse(raw_coverage('baseline', 'candidate')['raw_fields_match'])

    def test_no_exclusions_or_initial_values_enter_warm_median(self):
        rows = [{'scope': 'fixture', 'layer': 'native', 'role': role, 'phase': 'warm',
                 'success': True, 'elapsed_ms': value, 'entries': 10, 'allocated_bytes': allocation,
                 'peak_rss_bytes': 1000} for role, allocation in [('baseline', 200), ('candidate', 100)]
                for value in [10, 11, 12, 13, 900]]
        rows.append({**rows[0], 'phase': 'initial', 'elapsed_ms': 2000})
        summary = summarize(rows)[0]
        self.assertEqual(summary['baseline']['elapsed_ms'], 12)
        self.assertEqual(summary['candidate']['allocated_bytes'], [100] * 5)
        rows[0]['success'] = False
        self.assertFalse(summarize(rows)[0]['baseline']['complete'])

    def test_reference_requires_all_layers_and_exporter_and_enters_summary(self):
        with tempfile.TemporaryDirectory() as root:
            config = {'scopes': {'fixture': root}, 'executables': [
                {'role': role, 'layer': layer} for role in ('baseline', 'candidate', 'reference')
                for layer in ('native', 'app')],
                'exporters': [{'role': role} for role in ('baseline', 'candidate', 'reference')]}
            validate(config, Path(root).parent / 'outside-owned-output')
            config['exporters'].pop()
            with self.assertRaises(ValueError):
                validate(config, Path(root).parent / 'outside-owned-output')
        rows = [{'scope': 'fixture', 'layer': 'native', 'role': role, 'phase': 'warm',
                 'success': True, 'elapsed_ms': elapsed, 'entries': 10, 'allocated_bytes': 100,
                 'peak_rss_bytes': 1000} for role, elapsed in [('baseline', 20), ('candidate', 8), ('reference', 6)]
                for _ in range(5)]
        result = summarize(rows)[0]
        self.assertEqual(result['time_ratio'], 0.4)
        self.assertEqual(result['reference_time_ratio'], 8 / 6)


if __name__ == '__main__':
    unittest.main()
