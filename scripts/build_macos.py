#!/usr/bin/env python3
"""Build the macOS app with an existing keychain identity; never grant access.

Default: local build signed with Apple Development. --release: Developer ID build, notarized and stapled.
--lite: sandboxed Hahakigami Lite, signed with Apple Development.
--lite --release: Lite for the Mac App Store, signed with Apple Distribution and wrapped in an installer .pkg signed
with Mac Installer Distribution. Upload is manual (Transporter); this script never receives App Store Connect credentials.
"""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import plistlib
import platform
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


LITE_CONFIG = ROOT / 'src-tauri/tauri.lite.conf.json'
# The App Sandbox plus what Lite uses inside it (ADR 0007); network.client is what lets WebKit render.
LITE_SANDBOX_ENTITLEMENT = 'com.apple.security.app-sandbox'
LITE_REQUIRED_ENTITLEMENTS = (LITE_SANDBOX_ENTITLEMENT, 'com.apple.security.files.user-selected.read-write',
                              'com.apple.security.files.bookmarks.app-scope', 'com.apple.security.network.client')
LITE_ENTITLEMENTS = ROOT / 'src-tauri/entitlements/lite.entitlements'
DEVELOPMENT, RELEASE = 'Apple Development', 'Developer ID Application'
APPLE_DISTRIBUTION, INSTALLER = 'Apple Distribution', 'Mac Installer Distribution'
# The Mac App Store needs the profile the person downloaded for the Lite bundle ID inside the app. The script only
# embeds a path it is given; it never creates or downloads one.
PROVISIONING_PROFILE_NAME = 'embedded.provisionprofile'
# Notarization credentials tauri reads; either set is enough. Certificate import vars are never passed.
NOTARY_CREDENTIALS = (('APPLE_API_KEY', 'APPLE_API_ISSUER', 'APPLE_API_KEY_PATH'), ('APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'))
# Anything that could upload to App Store Connect (Transporter, altool, fastlane). A team ID alone is not a secret.
STORE_CREDENTIAL_NAMES = {name for names in NOTARY_CREDENTIALS for name in names} - {'APPLE_TEAM_ID'} | {'API_PRIVATE_KEYS_DIR'}
STORE_CREDENTIAL_PREFIXES = ('APP_STORE_CONNECT', 'FASTLANE_')


def identity_names(output, kind=DEVELOPMENT):
    """(SHA-1, full name) of each listed identity of one kind, like `Apple Distribution: Name (TEAMID)`."""
    return re.findall(rf'^\s*\d+\) ([0-9A-Fa-f]{{40}}) "({kind}: [^"\n]+)"\s*$', output, re.MULTILINE)


def development_identities(output, kind=DEVELOPMENT):
    return [sha for sha, _ in identity_names(output, kind)]


def identity_name(output, sha, kind):
    """The full name of the selected identity. Fails when another valid identity has the same name, because tools
    that select by name (productbuild) could not tell the two apart."""
    named = {value.upper(): name for value, name in identity_names(output, kind)}
    name = named.get(sha.upper())
    if name is None:
        raise ValueError(f'The selected {kind} identity is not in the keychain listing.')
    if sum(other == name for other in named.values()) > 1:
        raise ValueError(f'More than one valid {kind} identity is named "{name}", so it cannot be chosen by name. '
                         'Remove the one you do not use from the keychain, or sign with a different certificate.')
    return name


def identity_team(name):
    """The Team ID Apple puts at the end of a certificate's common name."""
    match = re.search(r'\(([A-Z0-9]{10})\)$', name)
    if not match:
        raise ValueError(f'Could not read the Team ID from the identity name "{name}".')
    return match.group(1)


def select_identity(identities, requested=None, adhoc=False, kind=DEVELOPMENT, option='--identity'):
    identities = list(dict.fromkeys(value.upper() for value in identities))
    if adhoc:
        if requested:
            raise ValueError('Choose either --identity or --adhoc.')
        return '-'
    if requested:
        if requested.upper() not in identities:
            raise ValueError(f'{option} must match an existing valid {kind} certificate SHA-1.')
        return requested.upper()
    if len(identities) != 1:
        raise ValueError(f'Expected one existing {kind} identity. Choose its SHA-1 with {option}' +
                         ('; --adhoc explicitly opts into unstable permission identity.' if kind == DEVELOPMENT else '.'))
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


def appstore_environment(environment, identity):
    """Local environment for the App Store build; refuse (rather than drop) anything that could reach App Store Connect."""
    present = sorted(name for name, value in environment.items() if value and
                     (name in STORE_CREDENTIAL_NAMES or name.startswith(STORE_CREDENTIAL_PREFIXES)))
    if present:
        raise ValueError('The App Store build never receives App Store Connect or notarization credentials; unset: ' +
                         ', '.join(present) + '. Upload the .pkg manually with Transporter.')
    return local_environment(environment, identity)


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


def check_lite_options(lite, release, adhoc, benchmark_output):
    if lite and (adhoc or benchmark_output):
        raise ValueError('--lite cannot be combined with --adhoc or --benchmark-output.')


def check_store_options(lite, release, installer_identity, provisioning_profile):
    store = lite and release
    if (installer_identity or provisioning_profile) and not store:
        raise ValueError('--installer-identity and --provisioning-profile only apply to --lite --release.')
    if store and not provisioning_profile:
        raise ValueError('--lite --release needs --provisioning-profile: the path to the Mac App Store provisioning '
                         'profile for the Lite bundle ID that you downloaded from the developer account.')


def check_provisioning_profile(path):
    path = path.expanduser().resolve()
    if path.suffix != '.provisionprofile' or not path.is_file():
        raise ValueError('--provisioning-profile must be an existing .provisionprofile file.')
    return path


def with_provisioning_profile(overrides, profile):
    """Overrides that make tauri copy the profile to Contents/embedded.provisionprofile before signing."""
    macos = {**overrides['bundle']['macOS'], 'files': {PROVISIONING_PROFILE_NAME: str(profile)}}
    return {**overrides, 'bundle': {**overrides['bundle'], 'macOS': macos}}


def verify_provisioning_profile_embedded(bundle):
    embedded = Path(bundle) / 'Contents' / PROVISIONING_PROFILE_NAME
    if not embedded.is_file() or embedded.stat().st_size == 0:
        raise ValueError('The Lite bundle has no embedded provisioning profile.')


def installer_command(bundle, package, identity):
    return ['/usr/bin/productbuild', '--component', str(bundle), '/Applications', '--sign', identity, str(package)]


def verify_package_signature(output):
    """`pkgutil --check-signature` output: the leaf of the certificate chain must be Mac Installer Distribution."""
    if not re.search(rf'^\s*1\.\s+{INSTALLER}: ', output, re.MULTILINE):
        raise ValueError(f'The installer package is not signed with a {INSTALLER} certificate.')


APPLICATION_IDENTIFIER, TEAM_IDENTIFIER = 'com.apple.application-identifier', 'com.apple.developer.team-identifier'


def store_identifiers(profile, team, bundle_identifier):
    """(application identifier, team identifier) the Mac App Store build is signed with, read from the provisioning
    profile. Fails unless the profile is for this app and for the team of the Apple Distribution identity."""
    entitlements = profile.get('Entitlements') or {}
    expected = f'{team}.{bundle_identifier}'
    if entitlements.get(APPLICATION_IDENTIFIER) != expected:
        raise ValueError(f'The provisioning profile is not for {expected} (it says '
                         f'{entitlements.get(APPLICATION_IDENTIFIER)!r}).')
    if entitlements.get(TEAM_IDENTIFIER) != team or team not in (profile.get('TeamIdentifier') or []):
        raise ValueError(f'The provisioning profile belongs to another team than the Apple Distribution identity ({team}).')
    return entitlements[APPLICATION_IDENTIFIER], entitlements[TEAM_IDENTIFIER]


def read_provisioning_profile(path):
    """The profile's property list. `security cms -D` only decodes the file; it does not touch the keychain."""
    decoded = subprocess.run(['/usr/bin/security', 'cms', '-D', '-i', str(path)], check=True, capture_output=True, timeout=30)
    return plistlib.loads(decoded.stdout)


def store_entitlements(identifiers, base=LITE_ENTITLEMENTS):
    """The Lite entitlements plus the two identifiers a Mac App Store build needs. The shipped file stays as it is."""
    application_identifier, team_identifier = identifiers
    entitlements = plistlib.loads(Path(base).read_bytes())
    entitlements.update({APPLICATION_IDENTIFIER: application_identifier, TEAM_IDENTIFIER: team_identifier})
    return plistlib.dumps(entitlements)


def with_entitlements(overrides, path):
    """Overrides that make tauri sign with the entitlements file at `path`."""
    return {**overrides, 'bundle': {**overrides['bundle'], 'macOS': {**overrides['bundle']['macOS'], 'entitlements': str(path)}}}


def lite_overlay(overlay):
    """Product name and identifier of the Lite edition, from its Tauri config overlay."""
    return overlay['productName'], overlay['identifier']


def lite_build_arguments(overrides):
    """Extra `tauri build` arguments for Lite. The overlay (name, identifier, entitlements) merges before the
    signing overrides. Cargo features only add, so Lite leaves out the default `automation` feature (CLI, MCP)."""
    return ['--features', 'lite', '--config', str(LITE_CONFIG), '--config', json.dumps(overrides),
            '--', '--no-default-features']


def verify_entitlements(output, identifiers=None):
    """The Lite signature must carry the App Sandbox and the entitlements Lite uses inside it. A Mac App Store build
    (`identifiers` is the pair from `store_identifiers`) must also carry its application and team identifiers."""
    def granted(key):
        return re.search(rf'<key>{re.escape(key)}</key>\s*<true/>', output)
    if not granted(LITE_SANDBOX_ENTITLEMENT):
        raise ValueError('The Lite bundle signature does not carry the App Sandbox entitlement.')
    for key in LITE_REQUIRED_ENTITLEMENTS:
        if not granted(key):
            raise ValueError(f'The Lite bundle signature lacks the {key} entitlement.')
    for key, value in zip((APPLICATION_IDENTIFIER, TEAM_IDENTIFIER), identifiers or ()):
        if not re.search(rf'<key>{re.escape(key)}</key>\s*<string>{re.escape(value)}</string>', output):
            raise ValueError(f'The Lite bundle signature lacks the {key} entitlement ({value}).')


def leaf_certificate_sha1(bundle):
    """SHA-1 of the certificate the bundle was signed with (codesign writes the leaf as <prefix>0)."""
    with tempfile.TemporaryDirectory() as directory:
        subprocess.run(['/usr/bin/codesign', '-d', f'--extract-certificates={directory}/cert', str(bundle)],
                       check=True, capture_output=True, timeout=30)
        return hashlib.sha1(Path(directory, 'cert0').read_bytes()).hexdigest().upper()


def keychain_listing(kind):
    """`security find-identity` for one kind. Installer certificates are not code-signing identities, so list without a policy."""
    policy = [] if kind == INSTALLER else ['-p', 'codesigning']
    return subprocess.run(['/usr/bin/security', 'find-identity', '-v', *policy], check=True,
                          capture_output=True, text=True, timeout=30).stdout


def keychain_identities(kind):
    return development_identities(keychain_listing(kind), kind)


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
    parser.add_argument('--identity', help='SHA-1 of an existing valid signing identity (Apple Development, Developer ID Application with --release, Apple Distribution with --lite --release)')
    parser.add_argument('--installer-identity', help='SHA-1 of an existing valid Mac Installer Distribution identity (--lite --release)')
    parser.add_argument('--provisioning-profile', type=Path, help='Existing Mac App Store .provisionprofile for the Lite bundle ID (--lite --release)')
    parser.add_argument('--benchmark-output', type=Path, help='Build an isolated benchmark-only app into a new absolute output directory.')
    parser.add_argument('--adhoc', action='store_true', help='Explicitly build without a stable development identity')
    parser.add_argument('--lite', action='store_true', help='Sandboxed Hahakigami Lite build (Apple Development identity; with --release, a Mac App Store installer package)')
    parser.add_argument('--release', action='store_true', help='Developer ID build for other Macs: notarize, staple, add a DMG')
    args = parser.parse_args()
    store = args.lite and args.release
    kind = APPLE_DISTRIBUTION if store else RELEASE if args.release else DEVELOPMENT
    if platform.system() != 'Darwin':
        parser.error('This local build command requires macOS.')
    scratch = contextlib.ExitStack()
    try:
        if args.release and (args.adhoc or args.benchmark_output):
            raise ValueError('--release cannot be combined with --adhoc or --benchmark-output.')
        check_lite_options(args.lite, args.release, args.adhoc, args.benchmark_output)
        check_store_options(args.lite, args.release, args.installer_identity, args.provisioning_profile)
        profile = check_provisioning_profile(args.provisioning_profile) if store else None
        identity = select_identity([] if args.adhoc else keychain_identities(kind), args.identity, args.adhoc, kind)
        installers = keychain_listing(INSTALLER) if store else ''
        installer = select_identity(development_identities(installers, INSTALLER), args.installer_identity, kind=INSTALLER,
                                    option='--installer-identity') if store else None
        # productbuild may not take a SHA-1: it gets the identity's name, which must then be unambiguous.
        installer_name = identity_name(installers, installer, INSTALLER) if store else None
        config = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())
        name, identifier = lite_overlay(json.loads(LITE_CONFIG.read_text())) if args.lite else (config['productName'], config['identifier'])
        if Path(name).name != name:
            raise ValueError('Product name must be one bundle filename.')
        if args.benchmark_output and (not args.benchmark_output.is_absolute() or args.adhoc):
            raise ValueError('Benchmark bundles require absolute output and stable development signing.')
        output = args.benchmark_output
        if output:
            prepare_benchmark_output(output)
        print(f'Building with an existing {kind} identity.' if identity != '-' else
              'Building ad-hoc: macOS grants may need to be renewed after each build.', flush=True)
        environment = (appstore_environment if store else release_environment if args.release else local_environment)(os.environ, identity)
        overrides = {'bundle': {'macOS': {'signingIdentity': identity}}}
        identifiers = None
        if store:
            overrides = with_provisioning_profile(overrides, profile)
            # The App Store needs the signature to carry the application and team identifiers the profile grants.
            identifiers = store_identifiers(read_provisioning_profile(profile),
                                            identity_team(identity_name(keychain_listing(APPLE_DISTRIBUTION), identity, APPLE_DISTRIBUTION)),
                                            identifier)
            entitlements_file = Path(scratch.enter_context(tempfile.TemporaryDirectory())) / 'lite-store.entitlements'
            entitlements_file.write_bytes(store_entitlements(identifiers))
            overrides = with_entitlements(overrides, entitlements_file)
        command = ['pnpm', 'tauri', 'build', '--bundles', 'app,dmg' if args.release and not store else 'app']
        if output:
            environment['CARGO_TARGET_DIR'] = str(output / 'target')
            environment['VITE_PERFORMANCE_BENCHMARK'] = '1'
            command += ['--features', 'performance-benchmark']
            overrides['app'] = {'windows': benchmark_windows(config)}
            overrides['build'] = {
                'frontendDist': str(output / 'dist'),
                'beforeBuildCommand': benchmark_frontend_command(json.loads((ROOT / 'package.json').read_text()), output),
            }
        command += lite_build_arguments(overrides) if args.lite else ['--config', json.dumps(overrides)]
        subprocess.run(command, cwd=ROOT, env=environment, check=True)
        # Cargo honours CARGO_TARGET_DIR (relative to cwd=ROOT), so verify the bundle it just built.
        bundle = ROOT / environment.get('CARGO_TARGET_DIR', 'target') / 'release/bundle/macos' / f'{name}.app'
        subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(bundle)], check=True)
        display = subprocess.run(['/usr/bin/codesign', '--display', '--verbose=4', '--requirements', '-', str(bundle)],
                                 check=True, capture_output=True, text=True)
        verify_metadata(display.stdout + display.stderr, identifier, args.adhoc, kind)
        if args.lite:
            entitlements = subprocess.run(['/usr/bin/codesign', '--display', '--entitlements', ':-', str(bundle)],
                                          check=True, capture_output=True, text=True, timeout=30)
            verify_entitlements(entitlements.stdout + entitlements.stderr, identifiers)
        if identity != '-' and leaf_certificate_sha1(bundle) != identity:
            raise ValueError('The bundle was not signed with the selected identity.')
        if store:
            verify_provisioning_profile_embedded(bundle)
            package = bundle.parents[1] / 'pkg' / f'{name}.pkg'
            package.parent.mkdir(exist_ok=True)
            package.unlink(missing_ok=True)
            subprocess.run(installer_command(bundle, package, installer_name), check=True, timeout=600)
            check = subprocess.run(['/usr/sbin/pkgutil', '--check-signature', str(package)], check=True,
                                   capture_output=True, text=True, timeout=60)
            verify_package_signature(check.stdout + check.stderr)
            print(f'Verified App Store installer: {package}\nUpload it manually with Transporter.', flush=True)
        elif args.release:
            subprocess.run(['/usr/bin/xcrun', 'stapler', 'validate', str(bundle)], check=True, timeout=60)
            subprocess.run(['/usr/sbin/spctl', '--assess', '--type', 'execute', '--verbose', str(bundle)], check=True, timeout=60)
            dmgs = sorted(bundle.parents[1].glob('dmg/*.dmg'))
            print(f'Verified notarized app: {bundle}\nDisk image to share: {dmgs[-1] if dmgs else "not found"}', flush=True)
        else:
            print(f'Verified local app: {bundle}\nFull Disk Access remains a manual macOS grant.', flush=True)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f'Build failed: {error}\n')
    finally:
        scratch.close()


if __name__ == '__main__':
    main()
