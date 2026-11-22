#!/usr/bin/env python3
"""Build the macOS app with an existing keychain identity; never grant access.

Default: local build signed with Apple Development. --release: Developer ID build, notarized and stapled.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


DEVELOPMENT, RELEASE = 'Apple Development', 'Developer ID Application'
# Notarization credentials tauri reads; either set is enough. Certificate import vars are never passed.
NOTARY_CREDENTIALS = (('APPLE_API_KEY', 'APPLE_API_ISSUER', 'APPLE_API_KEY_PATH'), ('APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'))


def development_identities(output, kind=DEVELOPMENT):
    return re.findall(rf'^\s*\d+\) ([0-9A-Fa-f]{{40}}) "{kind}: [^"\n]+"\s*$', output, re.MULTILINE)


def select_identity(identities, requested=None, adhoc=False, kind=DEVELOPMENT):
    identities = list(dict.fromkeys(value.upper() for value in identities))
    if adhoc:
        if requested:
            raise ValueError('Choose either --identity or --adhoc.')
        return '-'
    if requested:
        if requested.upper() not in identities:
            raise ValueError(f'--identity must match an existing valid {kind} certificate SHA-1.')
        return requested.upper()
    if len(identities) != 1:
        raise ValueError(f'Expected one existing {kind} identity. Choose its SHA-1 with --identity' +
                         ('.' if kind == RELEASE else '; --adhoc explicitly opts into unstable permission identity.'))
    return identities[0]


def local_environment(environment, identity):
    # Use an existing keychain identity only. Never import a certificate or upload for notarization.
    env = {key: value for key, value in environment.items() if not key.startswith('APPLE_')}
    env['APPLE_SIGNING_IDENTITY'] = identity
    return env


def release_environment(environment, identity):
    """Local environment plus one complete set of notarization credentials; fail rather than ship unnotarized."""
    env = local_environment(environment, identity)
    for names in NOTARY_CREDENTIALS:
        if all(environment.get(name) for name in names):
            env.update({name: environment[name] for name in names})
            return env
    raise ValueError('Release builds need notarization credentials: ' +
                     ' or '.join(', '.join(names) for names in NOTARY_CREDENTIALS) + '.')


def verify_metadata(metadata, identifier, adhoc, kind=DEVELOPMENT):
    if f'Identifier={identifier}\n' not in metadata:
        raise ValueError('Signed identifier does not match the configured bundle identifier.')
    if 'Info.plist=not bound' in metadata or 'Sealed Resources=none' in metadata:
        raise ValueError('The application bundle is not sealed.')
    is_adhoc = 'Signature=adhoc' in metadata
    if is_adhoc != adhoc:
        raise ValueError('The resulting signature differs from the requested signing mode.')
    if not adhoc:
        if f'Authority={kind}:' not in metadata or not re.search(r'^TeamIdentifier=[A-Z0-9]+$', metadata, re.MULTILINE):
            raise ValueError(f'No {kind} authority/team in the signature.')
        # Notarization rejects bundles without the hardened runtime.
        if kind == RELEASE and not re.search(r'^CodeDirectory .*flags=0x[0-9a-f]*\([^)]*\bruntime\b', metadata, re.MULTILINE):
            raise ValueError('Release bundle is not signed with the hardened runtime.')
        requirement = next((line.removeprefix('# ') for line in metadata.splitlines()
                            if line.removeprefix('# ').startswith('designated => ')), '')
        if not requirement:
            raise ValueError('Could not read the designated requirement.')
        if 'cdhash ' in requirement:
            raise ValueError('The designated requirement still depends on a particular executable hash.')
        for needed in (f'identifier "{identifier}"', 'anchor apple generic', 'certificate leaf['):
            if needed not in requirement:
                raise ValueError(f'The designated requirement lacks {needed!r}.')


def leaf_certificate_sha1(bundle):
    """SHA-1 of the certificate the bundle was signed with (codesign writes the leaf as <prefix>0)."""
    with tempfile.TemporaryDirectory() as directory:
        subprocess.run(['/usr/bin/codesign', '-d', f'--extract-certificates={directory}/cert', str(bundle)],
                       check=True, capture_output=True, timeout=30)
        return hashlib.sha1(Path(directory, 'cert0').read_bytes()).hexdigest().upper()


def prepare_benchmark_output(output):
    if not output.is_absolute() or output.resolve() != output or output == Path('/'):
        raise ValueError('Benchmark output must be absolute with no symlink ancestors.')
    output.mkdir(parents=True, exist_ok=True)
    marker = output / '.hahakigami-benchmark-build'
    if marker.is_symlink():
        raise ValueError('Benchmark build marker cannot be a symlink.')
    if marker.exists():
        if not marker.is_file() or marker.read_text() != 'owned benchmark build v1\n':
            raise ValueError('Benchmark build ownership mismatch.')
    else:
        if any(output.iterdir()):
            raise ValueError('Refusing nonempty unowned benchmark build directory.')
        with marker.open('x') as stream:
            stream.write('owned benchmark build v1\n')
    if (output / 'dist').is_symlink():
        raise ValueError('Benchmark dist cannot redirect cleanup through a symlink.')
    return output


def benchmark_windows(config):
    return [{**window, 'incognito': True} for window in config['app']['windows']]

def benchmark_frontend_command(package, output):
    """Use the app's framework-aware check, including frozen React baselines."""
    import shlex
    check = 'pnpm check' if 'check' in package.get('scripts', {}) else 'pnpm exec tsc'
    return check + ' && pnpm exec vite build --outDir ' + shlex.quote(str(output / 'dist')) + ' --emptyOutDir'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--identity', help='SHA-1 of an existing valid Apple Development identity')
    parser.add_argument('--benchmark-output', type=Path, help='Build an isolated benchmark-only app into a new absolute output directory.')
    parser.add_argument('--adhoc', action='store_true', help='Explicitly build without a stable development identity')
    parser.add_argument('--release', action='store_true', help='Developer ID build for other Macs: notarize, staple, add a DMG')
    args = parser.parse_args()
    kind = RELEASE if args.release else DEVELOPMENT
    if platform.system() != 'Darwin':
        parser.error('This local build command requires macOS.')
    try:
        if args.release and (args.adhoc or args.benchmark_output):
            raise ValueError('--release cannot be combined with --adhoc or --benchmark-output.')
        identities = [] if args.adhoc else development_identities(subprocess.run(
            ['/usr/bin/security', 'find-identity', '-v', '-p', 'codesigning'], check=True,
            capture_output=True, text=True, timeout=30).stdout, kind)
        identity = select_identity(identities, args.identity, args.adhoc, kind)
        config = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())
        name, identifier = config['productName'], config['identifier']
        if Path(name).name != name:
            raise ValueError('Product name must be one bundle filename.')
        if args.benchmark_output and (not args.benchmark_output.is_absolute() or args.adhoc):
            raise ValueError('Benchmark bundles require absolute output and stable development signing.')
        output = args.benchmark_output
        if output:
            prepare_benchmark_output(output)
        print(f'Building with an existing {kind} identity.' if identity != '-' else
              'Building ad-hoc: macOS grants may need to be renewed after each build.', flush=True)
        environment = (release_environment if args.release else local_environment)(os.environ, identity)
        overrides = {'bundle': {'macOS': {'signingIdentity': identity}}}
        command = ['pnpm', 'tauri', 'build', '--bundles', 'app,dmg' if args.release else 'app']
        if output:
            environment['CARGO_TARGET_DIR'] = str(output / 'target')
            environment['VITE_PERFORMANCE_BENCHMARK'] = '1'
            command += ['--features', 'performance-benchmark']
            overrides['app'] = {'windows': benchmark_windows(config)}
            overrides['build'] = {
                'frontendDist': str(output / 'dist'),
                'beforeBuildCommand': benchmark_frontend_command(json.loads((ROOT / 'package.json').read_text()), output),
            }
        command += ['--config', json.dumps(overrides)]
        subprocess.run(command, cwd=ROOT, env=environment, check=True)
        # Cargo honours CARGO_TARGET_DIR (relative to cwd=ROOT), so verify the bundle it just built.
        bundle = ROOT / environment.get('CARGO_TARGET_DIR', 'target') / 'release/bundle/macos' / f'{name}.app'
        subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(bundle)], check=True)
        display = subprocess.run(['/usr/bin/codesign', '--display', '--verbose=4', '--requirements', '-', str(bundle)],
                                 check=True, capture_output=True, text=True)
        verify_metadata(display.stdout + display.stderr, identifier, args.adhoc, kind)
        if identity != '-' and leaf_certificate_sha1(bundle) != identity:
            raise ValueError('The bundle was not signed with the selected identity.')
        if args.release:
            subprocess.run(['/usr/bin/xcrun', 'stapler', 'validate', str(bundle)], check=True, timeout=60)
            subprocess.run(['/usr/sbin/spctl', '--assess', '--type', 'execute', '--verbose', str(bundle)], check=True, timeout=60)
            dmgs = sorted(bundle.parents[1].glob('dmg/*.dmg'))
            print(f'Verified notarized app: {bundle}\nDisk image to share: {dmgs[-1] if dmgs else "not found"}', flush=True)
        else:
            print(f'Verified local app: {bundle}\nFull Disk Access remains a manual macOS grant.', flush=True)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f'Build failed: {error}\n')


if __name__ == '__main__':
    main()
