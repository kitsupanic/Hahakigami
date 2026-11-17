#!/usr/bin/env python3
"""Build the local macOS app with an existing development identity; never grant access."""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def development_identities(output):
    return re.findall(r'^\s*\d+\) ([0-9A-Fa-f]{40}) "Apple Development: [^"\n]+"\s*$', output, re.MULTILINE)


def select_identity(identities, requested=None, adhoc=False):
    identities = list(dict.fromkeys(value.upper() for value in identities))
    if adhoc:
        if requested:
            raise ValueError('Choose either --identity or --adhoc.')
        return '-'
    if requested:
        if requested.upper() not in identities:
            raise ValueError('--identity must match an existing valid Apple Development certificate SHA-1.')
        return requested.upper()
    if len(identities) != 1:
        raise ValueError('Expected one existing Apple Development identity. Choose its SHA-1 with --identity; --adhoc explicitly opts into unstable permission identity.')
    return identities[0]


def local_environment(environment, identity):
    # Use an existing keychain identity only. Never import a certificate or upload for notarization.
    env = {key: value for key, value in environment.items() if not key.startswith('APPLE_')}
    env['APPLE_SIGNING_IDENTITY'] = identity
    return env


def verify_metadata(metadata, identifier, adhoc):
    if f'Identifier={identifier}\n' not in metadata:
        raise ValueError('Signed identifier does not match the configured bundle identifier.')
    if 'Info.plist=not bound' in metadata or 'Sealed Resources=none' in metadata:
        raise ValueError('The application bundle is not sealed.')
    is_adhoc = 'Signature=adhoc' in metadata
    if is_adhoc != adhoc:
        raise ValueError('The resulting signature differs from the requested signing mode.')
    if not adhoc:
        if 'Authority=Apple Development:' not in metadata or not re.search(r'^TeamIdentifier=[A-Z0-9]+$', metadata, re.MULTILINE):
            raise ValueError('No Apple Development authority/team in the signature.')
        requirement = next((line.removeprefix('# ') for line in metadata.splitlines()
                            if line.removeprefix('# ').startswith('designated => ')), '')
        if not requirement:
            raise ValueError('Could not read the designated requirement.')
        if 'cdhash ' in requirement:
            raise ValueError('The designated requirement still depends on a particular executable hash.')


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
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('This local build command requires macOS.')
    try:
        identities = [] if args.adhoc else development_identities(subprocess.run(
            ['/usr/bin/security', 'find-identity', '-v', '-p', 'codesigning'], check=True,
            capture_output=True, text=True, timeout=30).stdout)
        identity = select_identity(identities, args.identity, args.adhoc)
        config = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())
        name, identifier = config['productName'], config['identifier']
        if Path(name).name != name:
            raise ValueError('Product name must be one bundle filename.')
        if args.benchmark_output and (not args.benchmark_output.is_absolute() or args.adhoc):
            raise ValueError('Benchmark bundles require absolute output and stable development signing.')
        output = args.benchmark_output
        if output:
            prepare_benchmark_output(output)
        print('Building with an existing Apple Development identity.' if identity != '-' else
              'Building ad-hoc: macOS grants may need to be renewed after each build.', flush=True)
        environment = local_environment(os.environ, identity)
        overrides = {'bundle': {'macOS': {'signingIdentity': identity}}}
        command = ['pnpm', 'tauri', 'build', '--bundles', 'app']
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
        bundle = (output / 'target' if output else ROOT / 'target') / 'release/bundle/macos' / f'{name}.app'
        subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(bundle)], check=True)
        display = subprocess.run(['/usr/bin/codesign', '--display', '--verbose=4', '--requirements', '-', str(bundle)],
                                 check=True, capture_output=True, text=True)
        verify_metadata(display.stdout + display.stderr, identifier, args.adhoc)
        print(f'Verified local app: {bundle}\nFull Disk Access remains a manual macOS grant.', flush=True)
    except (ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f'Local build failed: {error}\n')


if __name__ == '__main__':
    main()
