import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import plistlib
import subprocess
import json
from performance_fixture import create, mutate, apfs_volume


class FixtureTests(unittest.TestCase):
    def test_cache_eligible_variant_preserves_count_and_independent_files(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch('performance_fixture.ctypes.CDLL',side_effect=AssertionError('Cache fixture must not clone files')):
                root = create(directory, 64, require_apfs=False, cache_eligible=True)
            self.assertEqual(sum(len(dirs)+len(files) for _,dirs,files in os.walk(root)),64)
            files = [path for path in root.rglob('*') if path.is_file() and not path.is_symlink()]
            self.assertTrue(all(path.stat().st_nlink == 1 for path in files))
            self.assertEqual(len({(path.stat().st_dev,path.stat().st_ino) for path in files}),len(files))
            metadata = json.loads((root/'.hahakigami-performance-fixture.json').read_text())
            self.assertEqual(metadata['variant'],'cache-eligible')
            mutate(root)
            self.assertEqual((root/'shallow/ordinary').read_bytes(),b'changed fixture\n')
            self.assertEqual((root/'shallow/hardlink').read_bytes(),b'fixture\n')
            self.assertEqual((root/'shallow/clone').read_bytes(),b'fixture\n')

    def test_exact_count_and_ownership(self):
        with tempfile.TemporaryDirectory() as d:
            root = create(d, 64, require_apfs=False)
            self.assertEqual(sum(len(a)+len(b) for _,a,b in os.walk(root)),64)
            self.assertEqual((root/'shallow/ordinary').stat().st_ino,(root/'shallow/hardlink').stat().st_ino)
            self.assertTrue((root/'shallow/symlink').is_symlink())
            self.assertTrue(any(name.startswith(b'native-') for name in os.listdir(os.fsencode(root/'wide'))))
            mutate(root)
            self.assertEqual((root/'shallow/hardlink').read_bytes(),b'changed fixture\n')
            with self.assertRaises(FileNotFoundError): mutate(Path(d))

    def test_directory_resolves_device_before_diskutil(self):
        with patch('performance_fixture.subprocess.check_output', side_effect=[
            'Filesystem 512-blocks Used Available Capacity Mounted on\n/dev/disk3s5 100 50 50 50% /System/Volumes/Data\n',
            plistlib.dumps({'FilesystemType':'apfs','MountPoint':'/System/Volumes/Data'})
        ]) as query:
            info = apfs_volume('/tmp/folder with spaces')
            self.assertEqual(info['device'],'/dev/disk3s5')
            self.assertEqual(query.call_args_list[1].args[0],['/usr/sbin/diskutil','info','-plist','/dev/disk3s5'])

    def test_query_failure_distinct_from_non_apfs(self):
        df = 'Filesystem 512-blocks Used Available Capacity Mounted on\n/dev/disk3s5 100 50 50 50% /data\n'
        with patch('performance_fixture.subprocess.check_output', side_effect=[df,plistlib.dumps({'FilesystemType':'hfs'})]):
            with self.assertRaisesRegex(ValueError,'containing filesystem is hfs'): apfs_volume('/tmp')
        with patch('performance_fixture.subprocess.check_output', side_effect=subprocess.CalledProcessError(1,['df'])):
            with self.assertRaisesRegex(RuntimeError,'status unknown'): apfs_volume('/tmp')


class MutationIsolationTests(unittest.TestCase):
    def test_redirected_leaf_and_ancestor_do_not_modify_outside_data(self):
        import os
        import tempfile
        from pathlib import Path
        from performance_fixture import create, mutate
        with tempfile.TemporaryDirectory() as parent:
            outside=Path(parent)/'outside';outside.write_bytes(b'keep this')
            for kind in ('symlink','hardlink','ancestor'):
                root=create(parent,32,require_apfs=False)
                target=root/'shallow'/'ordinary'
                if kind=='ancestor':
                    alternate=Path(parent)/'alternate';alternate.mkdir(exist_ok=True)
                    (alternate/'ordinary').write_bytes(b'keep this')
                    (root/'shallow').rename(root/'original-shallow')
                    (root/'shallow').symlink_to(alternate,target_is_directory=True)
                else:
                    target.unlink()
                    target.symlink_to(outside) if kind=='symlink' else os.link(outside,target)
                with self.assertRaises((ValueError,OSError)):mutate(root)
                self.assertEqual(outside.read_bytes(),b'keep this')
                if kind=='ancestor':self.assertEqual((alternate/'ordinary').read_bytes(),b'keep this')
