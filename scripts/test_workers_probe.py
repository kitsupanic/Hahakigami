import copy
import unittest
from workers_probe import verify

class WorkersProbeTests(unittest.TestCase):
    def test_caps_snapshot_and_production_auto(self):
        raw = {'settings': {'autoScope':'volume','maxWorkers':14}, 'scan': {'state':'completed','workerMode':'auto','workerSource':'hardware','workers':7}, 'capturedWorkers':7, 'automaticAfterLegacyReload': {'workerMode':'auto','qos':'automatic','resolvedWorkers':7,'defaultWorkers':7}}
        verify(raw)
        for mutation in ('oversubscribed','folder','calibrated','running','snapshot','legacy_override'):
            changed = copy.deepcopy(raw)
            if mutation == 'oversubscribed': changed['scan']['workers'] = 14
            elif mutation == 'folder': changed['settings']['autoScope'] = 'folder'
            elif mutation == 'calibrated': changed['scan']['workerSource'] = 'calibrated'
            elif mutation == 'running': changed['scan']['state'] = 'running'
            elif mutation == 'snapshot': changed['capturedWorkers'] = 3
            else: changed['automaticAfterLegacyReload']['qos'] = 'background'
            with self.assertRaises(ValueError, msg=mutation): verify(changed)

    def test_folder_scan_may_use_six_workers_but_not_seven(self):
        raw = {'settings': {'autoScope':'folder','maxWorkers':14,'defaultWorkers':6}, 'scan': {'state':'completed','workerMode':'auto','workerSource':'hardware','workers':6}, 'capturedWorkers':6, 'automaticAfterLegacyReload': {'workerMode':'auto','qos':'automatic','resolvedWorkers':6,'defaultWorkers':6}}
        verify(raw)
        raw['scan']['workers'] = raw['capturedWorkers'] = 7
        with self.assertRaises(ValueError): verify(raw)
