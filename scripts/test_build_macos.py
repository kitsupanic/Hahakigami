import unittest
from build_macos import RELEASE, release_environment, development_identities, select_identity, local_environment, verify_metadata, leaf_certificate_sha1, prepare_benchmark_output, benchmark_windows, benchmark_frontend_command, verify_entitlements, check_lite_options, lite_overlay, lite_build_arguments, LITE_ENTITLEMENTS, LITE_CONFIG, LITE_REQUIRED_ENTITLEMENTS

class LocalSigningTests(unittest.TestCase):
    def test_existing_development_identity_selection_fails_closed(self):
        first, second = 'A' * 40, 'B' * 40
        output = f'  1) {first} "Apple Development: Fixture (TEAM)"\n  2) {second} "Developer ID Application: Other (TEAM)"\n'
        self.assertEqual(development_identities(output), [first])
        self.assertEqual(select_identity([first]), first)
        self.assertEqual(select_identity([first, second], first.lower()), first)
        for identities, requested in [([], None), ([first, second], None), ([first], '-'), ([first], second)]:
            with self.assertRaises(ValueError): select_identity(identities, requested)
        self.assertEqual(select_identity([], adhoc=True), '-')
        with self.assertRaises(ValueError): select_identity([first], first, True)

    def test_local_environment_prevents_import_and_notarization_credentials(self):
        env = local_environment({'PATH': '/bin', 'APPLE_ID': 'fixture', 'APPLE_CERTIFICATE': 'fixture', 'APPLE_API_KEY': 'fixture', 'APPLE_SIGNING_IDENTITY': '-'}, 'A' * 40)
        self.assertEqual(env, {'PATH': '/bin', 'APPLE_SIGNING_IDENTITY': 'A' * 40})

    def test_verified_bundle_requires_sealing_and_stable_development_requirement(self):
        metadata = 'Identifier=app.hahakigami.desktop\nAuthority=Apple Development: Fixture\nTeamIdentifier=TEAM\nInfo.plist entries=10\nSealed Resources version=2\ndesignated => identifier "app.hahakigami.desktop" and anchor apple generic and certificate leaf[subject.CN] = "Apple Development: Fixture"\n'
        verify_metadata(metadata, 'app.hahakigami.desktop', False)
        verify_metadata(metadata.replace('designated => ', '# designated => '), 'app.hahakigami.desktop', False)
        for changed in [metadata.replace('app.hahakigami.desktop', 'other'), metadata.replace('Info.plist entries=10', 'Info.plist=not bound'), metadata.replace('Sealed Resources version=2', 'Sealed Resources=none'), metadata.replace('anchor apple generic', 'cdhash H"123"'), metadata.replace('anchor apple generic', 'anchor apple'), metadata.replace(' and certificate leaf[subject.CN]', ' and certificate 1[subject.CN]'), metadata.replace('designated => identifier "app.hahakigami.desktop"', 'designated => identifier "other"'), metadata.replace('Authority=Apple Development: Fixture', 'Signature=adhoc'), metadata.replace('TeamIdentifier=TEAM\n', '')]:
            with self.assertRaises(ValueError): verify_metadata(changed, 'app.hahakigami.desktop', False)

    def test_leaf_certificate_sha1_reads_signing_certificate(self):
        import platform
        if platform.system() != 'Darwin': self.skipTest('codesign is macOS-only')
        digest = leaf_certificate_sha1('/usr/bin/true')
        self.assertRegex(digest, r'^[0-9A-F]{40}$')

    def test_benchmark_output_rejects_unowned_or_redirected_paths(self):
        from pathlib import Path
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory).resolve()
            output = parent / 'new'
            self.assertEqual(prepare_benchmark_output(output), output)
            self.assertEqual(prepare_benchmark_output(output), output)
            other = parent / 'existing'; other.mkdir(); (other/'user-file').touch()
            with self.assertRaises(ValueError): prepare_benchmark_output(other)
            link = parent / 'link'; link.symlink_to(output, target_is_directory=True)
            with self.assertRaises(ValueError): prepare_benchmark_output(link/'child')
            dist = output/'dist'; dist.symlink_to(other,target_is_directory=True)
            with self.assertRaises(ValueError): prepare_benchmark_output(output)
            self.assertTrue((other/'user-file').exists())
            dist.unlink()
            (output/'.hahakigami-benchmark-build').write_text('foreign')
            with self.assertRaises(ValueError): prepare_benchmark_output(output)

    def test_benchmark_windows_use_private_webkit_storage(self):
        config = {'app': {'windows': [{'label': 'main', 'incognito': False, 'width': 1280}]}}
        result = benchmark_windows(config)
        self.assertTrue(result[0]['incognito'])
        self.assertEqual(result[0]['label'], 'main')
        self.assertEqual(result[0]['width'], 1280)
        self.assertFalse(config['app']['windows'][0]['incognito'])

    def test_framework_aware_benchmark_check(self):
        from pathlib import Path
        output = Path('/tmp/benchmark space')
        self.assertEqual(benchmark_frontend_command({'scripts': {'check': 'svelte-check'}}, output), "pnpm check && pnpm exec vite build --outDir '/tmp/benchmark space/dist' --emptyOutDir")
        self.assertTrue(benchmark_frontend_command({'scripts': {}}, output).startswith('pnpm exec tsc &&'))

    def test_release_selects_only_developer_id(self):
        first, second = 'A' * 40, 'B' * 40
        output = f'  1) {first} "Apple Development: Fixture (TEAM)"\n  2) {second} "Developer ID Application: Fixture (TEAM)"\n'
        self.assertEqual(development_identities(output, RELEASE), [second])
        self.assertEqual(select_identity([second], kind=RELEASE), second)
        with self.assertRaises(ValueError): select_identity([second], first, kind=RELEASE)

    def test_release_environment_requires_one_complete_notary_set_and_drops_import(self):
        base = {'PATH': '/bin', 'APPLE_CERTIFICATE': 'x', 'APPLE_CERTIFICATE_PASSWORD': 'x'}
        key = {'APPLE_API_KEY': 'k', 'APPLE_API_ISSUER': 'i', 'APPLE_API_KEY_PATH': '/k.p8'}
        self.assertEqual(release_environment({**base, **key}, 'B' * 40), {'PATH': '/bin', 'APPLE_SIGNING_IDENTITY': 'B' * 40, **key})
        account = {'APPLE_ID': 'a', 'APPLE_PASSWORD': 'p', 'APPLE_TEAM_ID': 'T'}
        self.assertEqual(release_environment({**base, **account}, 'B' * 40)['APPLE_TEAM_ID'], 'T')
        for partial in [base, {**base, 'APPLE_API_KEY': 'k', 'APPLE_API_ISSUER': 'i'}, {**base, **account, 'APPLE_PASSWORD': ''}]:
            with self.assertRaises(ValueError): release_environment(partial, 'B' * 40)

    def test_release_bundle_requires_developer_id_and_hardened_runtime(self):
        metadata = 'Identifier=app.hahakigami.desktop\nCodeDirectory v=20500 size=1 flags=0x10000(runtime) hashes=1+7 location=embedded\nAuthority=Developer ID Application: Fixture (TEAM)\nTeamIdentifier=TEAM\nSealed Resources version=2\ndesignated => identifier "app.hahakigami.desktop" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13] and certificate leaf[subject.OU] = TEAM\n'
        verify_metadata(metadata, 'app.hahakigami.desktop', False, RELEASE)
        for changed in [metadata.replace('0x10000(runtime)', '0x0(none)'), metadata.replace('Developer ID Application', 'Apple Development')]:
            with self.assertRaises(ValueError): verify_metadata(changed, 'app.hahakigami.desktop', False, RELEASE)


LITE_PLIST = '<?xml version="1.0"?><plist version="1.0"><dict>' + ''.join(f'<key>{key}</key><true/>' for key in LITE_REQUIRED_ENTITLEMENTS) + '</dict></plist>'


class LiteBuildTests(unittest.TestCase):
    def test_lite_cannot_be_combined_with_adhoc_benchmark_or_release_yet(self):
        from pathlib import Path
        check_lite_options(lite=True, release=False, adhoc=False, benchmark_output=None)
        check_lite_options(lite=False, release=True, adhoc=True, benchmark_output=None)
        for options in [{'adhoc': True}, {'benchmark_output': Path('/tmp/benchmark')}, {'release': True}]:
            with self.assertRaises(ValueError): check_lite_options(**{'lite': True, 'release': False, 'adhoc': False, 'benchmark_output': None, **options})

    def test_lite_bundle_must_carry_the_sandbox_and_its_other_entitlements(self):
        verify_entitlements(LITE_PLIST)
        verify_entitlements(LITE_PLIST.replace('<true/>', '\n\t<true/>'))
        self.assertEqual(len(LITE_REQUIRED_ENTITLEMENTS), 4)
        for key in LITE_REQUIRED_ENTITLEMENTS:
            with self.assertRaises(ValueError): verify_entitlements(LITE_PLIST.replace(f'<key>{key}</key><true/>', ''))
            with self.assertRaises(ValueError): verify_entitlements(LITE_PLIST.replace(f'<key>{key}</key><true/>', f'<key>{key}</key><false/>'))
        with self.assertRaises(ValueError): verify_entitlements('')

    def test_the_sandbox_failure_names_the_sandbox(self):
        with self.assertRaisesRegex(ValueError, 'App Sandbox'): verify_entitlements(LITE_PLIST.replace('<key>com.apple.security.app-sandbox</key><true/>', ''))

    def test_lite_overlay_names_the_lite_app(self):
        import json
        overlay = json.loads(LITE_CONFIG.read_text())
        self.assertEqual(lite_overlay(overlay), ('Hahakigami Lite', 'app.hahakigami.lite'))

    def test_shipped_entitlements_file_has_exactly_the_required_keys(self):
        import plistlib
        entitlements = plistlib.loads(LITE_ENTITLEMENTS.read_bytes())
        self.assertEqual(entitlements, {key: True for key in LITE_REQUIRED_ENTITLEMENTS})

    def test_lite_compiles_without_the_default_automation_feature(self):
        arguments = lite_build_arguments({'bundle': {'macOS': {'signingIdentity': 'A' * 40}}})
        self.assertEqual(arguments[arguments.index('--') + 1:], ['--no-default-features'])
        self.assertEqual(arguments[arguments.index('--features') + 1], 'lite')
        self.assertNotIn('automation', ' '.join(arguments))
        # Everything tauri itself reads comes before `--`; the overlay merges before the signing override.
        configs = [arguments[i + 1] for i, value in enumerate(arguments) if value == '--config']
        self.assertEqual(configs[0], str(LITE_CONFIG))
        self.assertIn('signingIdentity', configs[1])
        self.assertLess(arguments.index('--config'), arguments.index('--'))
