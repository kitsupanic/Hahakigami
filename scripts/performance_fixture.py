#!/usr/bin/env python3
"""Create an owned disposable fixture; never mutate arbitrary existing directories."""
import argparse
import ctypes
import json
import os
from pathlib import Path
import platform
import plistlib
import subprocess
import tempfile

MARKER = '.hahakigami-performance-fixture.json'


def apfs_volume(path):
    """Resolve an existing directory to its device; diskutil rejects arbitrary dirs."""
    try:
        output = subprocess.check_output(['/bin/df', '-P', str(Path(path).resolve())], text=True, timeout=30)
        lines = output.splitlines()
        if len(lines) != 2:
            raise RuntimeError('Cannot resolve containing volume from df output')
        device = lines[1].split()[0]
        if not device.startswith('/dev/'):
            raise RuntimeError(f'Containing filesystem has no disk device: {device}')
        data = subprocess.check_output(['/usr/sbin/diskutil', 'info', '-plist', device], timeout=30)
        info = plistlib.loads(data)
    except (OSError, subprocess.SubprocessError, plistlib.InvalidFileException) as error:
        raise RuntimeError('Unable to query containing filesystem; APFS status unknown') from error
    filesystem = info.get('FilesystemType')
    if not isinstance(filesystem, str):
        raise RuntimeError('diskutil did not report a filesystem type; APFS status unknown')
    if filesystem.lower() != 'apfs':
        raise ValueError(f'Fixture requires APFS; containing filesystem is {filesystem}')
    return {'device': device, 'filesystem': filesystem, 'mount_point': info.get('MountPoint')}


def create(parent, entries=100000, require_apfs=True, cache_eligible=False):
    if entries < 32:
        raise ValueError('At least 32 entries required')
    root = Path(tempfile.mkdtemp(prefix='hahakigami-fixture-', dir=parent))
    limitations = []
    try:
        if require_apfs:
            apfs_volume(root)
        for name in ('shallow', 'wide', 'deep'):
            (root / name).mkdir()
        current = root / 'deep'
        for index in range(8):
            current = current / str(index)
            current.mkdir()
        (root / 'shallow' / 'ordinary').write_bytes(b'fixture\n')
        with (root / 'shallow' / 'sparse').open('wb') as stream:
            stream.truncate(16 * 1024 * 1024)
        if cache_eligible:
            # Retain the same path/count while avoiding the scanner's documented
            # hard-link cache exclusion. The accounting fixture keeps real links.
            (root / 'shallow' / 'hardlink').write_bytes(b'fixture\n')
        else:
            os.link(root / 'shallow' / 'ordinary', root / 'shallow' / 'hardlink')
        os.symlink('../deep', root / 'shallow' / 'symlink')
        os.symlink('.', current / 'cycle')
        for name in ('newline\nname', '日本語', 'space name'):
            (root / 'wide' / name).touch()
        try:
            descriptor = os.open(os.fsencode(root / 'wide') + b'/native-\xff', os.O_CREAT | os.O_WRONLY, 0o600)
            os.close(descriptor)
        except OSError as error:
            import errno
            if error.errno != errno.EILSEQ:
                raise
            (root / 'wide' / 'native-é').touch()
            limitations.append('Filesystem rejects invalid UTF-8 names; Unicode native name substituted')
        if cache_eligible:
            # Clone events conservatively invalidate the volume-wide journal.
            # Keep shared-storage cases in the separate accounting fixture.
            (root / 'shallow' / 'clone').write_bytes(b'fixture\n')
        elif platform.system() == 'Darwin':
            lib = ctypes.CDLL('/usr/lib/libSystem.B.dylib', use_errno=True)
            if lib.clonefile(os.fsencode(root / 'shallow' / 'ordinary'), os.fsencode(root / 'shallow' / 'clone'), 0) != 0:
                raise OSError(ctypes.get_errno(), 'clonefile failed')
        else:
            (root / 'shallow' / 'clone').write_bytes(b'fixture\n')
            limitations.append('APFS clone unavailable on this host')
        # Include marker in the exact entry count, excluding root itself.
        (root / MARKER).write_text(json.dumps({'schema_version': 1, 'owned_root': str(root.resolve()), 'entries': entries, 'variant': 'cache-eligible' if cache_eligible else 'accounting', 'limitations': limitations, 'mutation_identity': [os.stat(root/'shallow'/'ordinary').st_dev, os.stat(root/'shallow'/'ordinary').st_ino], 'mutation_directory_identity': [os.stat(root/'shallow').st_dev, os.stat(root/'shallow').st_ino]}))
        count = sum(len(dirs) + len(files) for _, dirs, files in os.walk(root, followlinks=False))
        for index in range(entries - count):
            directory = (root / 'shallow', root / 'wide', current)[index % 3]
            (directory / f'item-{index:06d}').touch()
        return root
    except BaseException:
        import shutil
        shutil.rmtree(root)
        raise


def mutate(root):
    import stat
    root = Path(root).resolve()
    root_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        marker = os.open(MARKER, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=root_fd)
        with os.fdopen(marker) as stream:
            if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                raise ValueError('Invalid fixture marker')
            metadata = json.loads(stream.read(65537))
        if metadata.get('owned_root') != str(root):
            raise ValueError('Fixture ownership mismatch')
        directory = os.open('shallow', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=root_fd)
        try:
            info = os.fstat(directory)
            if metadata.get('mutation_directory_identity') != [info.st_dev, info.st_ino]:
                raise ValueError('Fixture directory identity changed')
            file = os.open('ordinary', os.O_WRONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
            try:
                info = os.fstat(file)
                if metadata.get('mutation_identity') != [info.st_dev, info.st_ino]:
                    raise ValueError('Fixture file identity changed')
                if not stat.S_ISREG(info.st_mode):
                    raise ValueError('Fixture target is not a regular file')
                os.ftruncate(file, 0)
                os.write(file, b'changed fixture\n')
            finally:
                os.close(file)
        finally:
            os.close(directory)
    finally:
        os.close(root_fd)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--parent', type=Path, required=True)
    parser.add_argument('--entries', type=int, default=100000)
    parser.add_argument('--cache-eligible', action='store_true', help='Use independent files instead of hard links and clones for incremental rescan measurements')
    args = parser.parse_args()
    print(create(args.parent, args.entries, cache_eligible=args.cache_eligible))
