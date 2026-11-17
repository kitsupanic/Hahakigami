import base64
import json
from pathlib import Path
import tempfile
import unittest
from coverage_compare import compare


class CoverageTests(unittest.TestCase):
    def write(self, path, rows, **header):
        root = {'type':'node', 'path_base64':'', 'kind':'directory', 'logical_bytes':0, 'allocated_bytes':0, 'summary':False}
        nodes = [root, *rows]
        path.write_text('\n'.join(json.dumps(r) for r in [{'type':'header','schema_version':1,'complete':True,'accounting_policy':'allocated','mount_policy':'same','size_scope':'own','node_count':len(nodes),**header}, *nodes, {'type':'footer','schema_version':1,'complete':True,'node_count':len(nodes)}]))


    def node(self, raw=b'bad-\xff', **extra):
        return {'type': 'node', 'path_base64': base64.b64encode(raw).decode(), 'kind': 'file', 'logical_bytes': 1, 'allocated_bytes': 4096, 'summary': False, **extra}

    def test_native_paths_and_missing(self):
        with tempfile.TemporaryDirectory() as d:
            a, b = Path(d)/'a', Path(d)/'b'
            self.write(a, [self.node()]); self.write(b, [self.node()])
            self.assertTrue(compare(a,b)['equivalent'])
            self.write(b, [self.node(b'other')])
            self.assertEqual(compare(a,b)['missing_paths']['only_0'], 1)

    def test_summary_incomplete_and_accounting(self):
        with tempfile.TemporaryDirectory() as d:
            a, b = Path(d)/'a', Path(d)/'b'
            self.write(a, [self.node(summary=True)]); self.write(b, [self.node(summary=True)])
            self.assertFalse(compare(a,b)['equivalent'])
            self.write(a, [self.node()]); self.write(b, [self.node()], complete=False)
            self.assertFalse(compare(a,b)['equivalent'])
            self.write(b, [self.node()], accounting_policy='clones')
            self.assertFalse(compare(a,b)['equivalent'])

    def test_duplicates_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            a, b = Path(d)/'a', Path(d)/'b'
            self.write(a, [self.node(),self.node()]); self.write(b, [])
            with self.assertRaises(ValueError): compare(a,b)

    def test_excluded_branch_and_recorded_errors_override_complete_claim(self):
        with tempfile.TemporaryDirectory() as d:
            a,b=Path(d)/'a',Path(d)/'b'
            self.write(a,[self.node(b'blocked',excluded=True)])
            self.write(b,[self.node(b'blocked',excluded=True)])
            report=compare(a,b)
            self.assertFalse(report['equivalent'])
            self.assertEqual(report['excluded_branches'][0]['count'],1)
            self.assertEqual(report['excluded_branches'][0]['path_samples_base64'],[base64.b64encode(b'blocked').decode()])
            self.write(a,[],issue_count=1,issue_samples=[{'error_kind':'PermissionDenied'}]);self.write(b,[])
            self.assertFalse(compare(a,b)['equivalent'])

    def test_empty_or_truncated_export_cannot_prove_coverage(self):
        with tempfile.TemporaryDirectory() as d:
            a,b=Path(d)/'a',Path(d)/'b'
            self.write(a,[self.node()]);self.write(b,[self.node()])
            b.write_text('\n'.join(b.read_text().splitlines()[:-1]))
            self.assertFalse(compare(a,b)['equivalent'])
            header={'type':'header','schema_version':1,'complete':True,'accounting_policy':'allocated','mount_policy':'same','size_scope':'own'}
            a.write_text(json.dumps(header));b.write_text(json.dumps(header))
            self.assertFalse(compare(a,b)['equivalent'])
