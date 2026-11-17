import unittest
from worker_diagnostics_trial import analysis


def raw():
    return {'workers': 2, 'stats': {'entries': 6, 'errors': 1, 'elapsed_ms': 10}, 'finalization_us': 50,
            'worker_stats': [{'id': 1, 'entries': 3, 'directories': 1, 'errors': 0, 'cpuUs': 4000, 'busyUs': 8000, 'metadataUs': 3000, 'qos': 'default'},
                             {'id': 2, 'entries': 2, 'directories': 1, 'errors': 1, 'cpuUs': 3000, 'busyUs': 6000, 'metadataUs': 2000, 'qos': 'default'}]}


class AnalysisTests(unittest.TestCase):
    def test_root_excluded_and_cpu_threads(self):
        report = analysis(raw())
        self.assertTrue(report['accounting_valid'])
        self.assertEqual(report['average_cpu_threads'], .7)
        self.assertEqual(report['phase_wall_fraction_of_worker_busy']['metadataUs'], 5000 / 14000)

    def test_missing_cpu_and_zero_elapsed(self):
        record = raw()
        record['worker_stats'][0]['cpuUs'] = None
        self.assertIsNone(analysis(record)['average_cpu_threads'])
        record = raw()
        record['stats']['elapsed_ms'] = 0
        self.assertIsNone(analysis(record)['average_cpu_threads'])

    def test_accounting_drift_and_duplicate_workers(self):
        for field, value in [('entries', 7), ('errors', 2)]:
            record = raw()
            record['stats'][field] = value
            self.assertFalse(analysis(record)['accounting_valid'])
        record = raw()
        record['worker_stats'][1]['id'] = 1
        self.assertFalse(analysis(record)['accounting_valid'])

    def test_descriptor_attempts_and_unknown_old_reports(self):
        record = raw()
        self.assertIsNone(analysis(record)['absolute_reopen_attempts'])
        record['descriptor_limit'] = 192
        record['descriptor_peak'] = 180
        for row in record['worker_stats']:
            row['relativeOpens'] = 1
            row['absoluteReopens'] = 2
        result = analysis(record)
        self.assertEqual(result['relative_open_attempts'], 2)
        self.assertEqual(result['absolute_reopen_attempts'], 4)
        self.assertEqual(result['descriptor_limit'], 192)
        self.assertEqual(result['descriptor_peak'], 180)


if __name__ == '__main__':
    unittest.main()
