import unittest
from build_macos import RELEASE, release_environment, development_identities, select_identity, local_environment, verify_metadata, leaf_certificate_sha1, prepare_benchmark_output, benchmark_windows, benchmark_frontend_command, verify_entitlements, check_lite_options, lite_overlay, lite_build_arguments, LITE_ENTITLEMENTS, LITE_CONFIG, LITE_REQUIRED_ENTITLEMENTS, APPLE_DISTRIBUTION, INSTALLER, check_store_options, appstore_environment, with_provisioning_profile, installer_command, verify_package_signature, verify_provisioning_profile_embedded, check_provisioning_profile, PROVISIONING_PROFILE_NAME, identity_names, identity_name, identity_team, store_identifiers, store_entitlements, with_entitlements

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
    def test_lite_cannot_be_combined_with_adhoc_or_benchmark(self):
        from pathlib import Path
        check_lite_options(lite=True, release=False, adhoc=False, benchmark_output=None)
        check_lite_options(lite=True, release=True, adhoc=False, benchmark_output=None)
        check_lite_options(lite=False, release=True, adhoc=True, benchmark_output=None)
        for options in [{'adhoc': True}, {'benchmark_output': Path('/tmp/benchmark')}, {'release': True, 'adhoc': True}]:
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


def listing(*entries):
    return ''.join(f'  {number}) {sha * 40} "{name}"\n' for number, (sha, name) in enumerate(entries, 1))


class AppStoreBuildTests(unittest.TestCase):
    def test_each_signing_kind_selects_only_its_own_certificate(self):
        output = listing(('A', 'Apple Development: Fixture (TEAM)'), ('B', 'Apple Distribution: Fixture (TEAM)'),
                         ('C', 'Mac Installer Distribution: Fixture (TEAM)'), ('D', 'Developer ID Application: Fixture (TEAM)'),
                         ('E', '3rd Party Mac Developer Installer: Fixture (TEAM)'))
        self.assertEqual(development_identities(output, APPLE_DISTRIBUTION), ['B' * 40])
        self.assertEqual(development_identities(output, INSTALLER), ['C' * 40])
        self.assertEqual(development_identities(output), ['A' * 40])

    def test_a_missing_identity_fails_naming_the_kind_and_never_falls_back(self):
        development, distribution, installer = 'A' * 40, 'B' * 40, 'C' * 40
        output = listing(('A', 'Apple Development: Fixture (TEAM)'), ('D', 'Developer ID Application: Fixture (TEAM)'))
        for kind, option in [(APPLE_DISTRIBUTION, '--identity'), (INSTALLER, '--installer-identity')]:
            with self.assertRaisesRegex(ValueError, f'{kind}.*{option}'):
                select_identity(development_identities(output, kind), kind=kind, option=option)
            # Another kind's SHA-1 is not accepted, even when asked for by name.
            with self.assertRaises(ValueError): select_identity(development_identities(output, kind), development, kind=kind, option=option)
        self.assertEqual(select_identity([distribution], kind=APPLE_DISTRIBUTION), distribution)
        self.assertEqual(select_identity([distribution, installer], installer.lower(), kind=INSTALLER, option='--installer-identity'), installer)
        with self.assertRaises(ValueError): select_identity([distribution, installer], kind=APPLE_DISTRIBUTION, option='--identity')
        with self.assertRaises(ValueError): select_identity([distribution], '-', kind=APPLE_DISTRIBUTION)

    def test_only_the_store_release_takes_an_installer_identity_or_a_provisioning_profile(self):
        from pathlib import Path
        profile = Path('/tmp/fixture.provisionprofile')
        check_store_options(lite=True, release=True, installer_identity='C' * 40, provisioning_profile=profile)
        check_store_options(lite=False, release=True, installer_identity=None, provisioning_profile=None)
        check_store_options(lite=True, release=False, installer_identity=None, provisioning_profile=None)
        for options in [{'lite': False, 'installer_identity': 'C' * 40}, {'lite': False, 'provisioning_profile': profile},
                        {'release': False, 'installer_identity': 'C' * 40}, {'release': False, 'provisioning_profile': profile},
                        {'provisioning_profile': None}]:
            with self.assertRaises(ValueError): check_store_options(**{'lite': True, 'release': True, 'installer_identity': None, 'provisioning_profile': profile, **options})

    def test_the_store_environment_refuses_app_store_connect_and_notarization_credentials(self):
        base = {'PATH': '/bin', 'APPLE_CERTIFICATE': 'x', 'APPLE_CERTIFICATE_PASSWORD': 'x', 'APPLE_TEAM_ID': 'TEAM'}
        self.assertEqual(appstore_environment(base, 'B' * 40), {'PATH': '/bin', 'APPLE_SIGNING_IDENTITY': 'B' * 40})
        for credentials in [{'APPLE_API_KEY': 'k'}, {'APPLE_API_ISSUER': 'i'}, {'APPLE_API_KEY_PATH': '/k.p8'}, {'APPLE_ID': 'a'},
                            {'APPLE_PASSWORD': 'p'}, {'APP_STORE_CONNECT_API_KEY': 'k'}, {'API_PRIVATE_KEYS_DIR': '/keys'},
                            {'FASTLANE_PASSWORD': 'p'}, {'FASTLANE_APPLE_APPLICATION_SPECIFIC_PASSWORD': 'p'}]:
            with self.assertRaises(ValueError) as caught: appstore_environment({**base, **credentials}, 'B' * 40)
            self.assertIn(next(iter(credentials)), str(caught.exception))
            self.assertNotIn('/keys', str(caught.exception))
        # An empty variable carries no credential.
        appstore_environment({**base, 'APPLE_ID': ''}, 'B' * 40)

    def test_the_provisioning_profile_is_embedded_by_path_and_the_overrides_are_not_mutated(self):
        from pathlib import Path
        overrides = {'bundle': {'macOS': {'signingIdentity': 'B' * 40}}}
        result = with_provisioning_profile(overrides, Path('/profiles/Lite Store.provisionprofile'))
        self.assertEqual(result['bundle']['macOS'], {'signingIdentity': 'B' * 40, 'files': {PROVISIONING_PROFILE_NAME: '/profiles/Lite Store.provisionprofile'}})
        self.assertEqual(PROVISIONING_PROFILE_NAME, 'embedded.provisionprofile')
        self.assertNotIn('files', overrides['bundle']['macOS'])

    def test_the_provisioning_profile_must_be_an_existing_provisionprofile_file(self):
        from pathlib import Path
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            profile = Path(directory).resolve() / 'Lite.provisionprofile'
            with self.assertRaises(ValueError): check_provisioning_profile(profile)
            profile.write_bytes(b'profile')
            self.assertEqual(check_provisioning_profile(profile), profile)
            other = Path(directory) / 'Lite.mobileprovision'; other.write_bytes(b'profile')
            with self.assertRaises(ValueError): check_provisioning_profile(other)
            with self.assertRaises(ValueError): check_provisioning_profile(Path(directory))

    def test_the_built_app_must_contain_the_embedded_profile(self):
        from pathlib import Path
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            bundle = Path(directory) / 'Hahakigami Lite.app'
            (bundle / 'Contents').mkdir(parents=True)
            with self.assertRaisesRegex(ValueError, 'provisioning profile'): verify_provisioning_profile_embedded(bundle)
            (bundle / 'Contents' / PROVISIONING_PROFILE_NAME).write_bytes(b'')
            with self.assertRaisesRegex(ValueError, 'provisioning profile'): verify_provisioning_profile_embedded(bundle)
            (bundle / 'Contents' / PROVISIONING_PROFILE_NAME).write_bytes(b'profile')
            verify_provisioning_profile_embedded(bundle)

    def test_the_installer_is_built_from_the_app_and_signed_with_the_selected_identity(self):
        from pathlib import Path
        name = 'Mac Installer Distribution: Fixture (ABCDE12345)'
        command = installer_command(Path('/b/Hahakigami Lite.app'), Path('/b/pkg/Hahakigami Lite.pkg'), name)
        self.assertEqual(command, ['/usr/bin/productbuild', '--component', '/b/Hahakigami Lite.app', '/Applications',
                                   '--sign', name, '/b/pkg/Hahakigami Lite.pkg'])

    def test_the_package_must_be_signed_by_a_mac_installer_distribution_certificate(self):
        signed = ('Package "Hahakigami Lite.pkg":\n   Status: signed by a developer certificate issued by Apple for distribution\n'
                  '   Certificate Chain:\n    1. Mac Installer Distribution: Fixture (TEAM)\n       SHA256 Fingerprint: AA BB\n'
                  '    2. Apple Worldwide Developer Relations Certification Authority\n')
        verify_package_signature(signed)
        for changed in [signed.replace('Mac Installer Distribution', 'Developer ID Installer'),
                        signed.replace('1. Mac Installer Distribution', '2. Mac Installer Distribution').replace('2. Apple Worldwide', '1. Apple Worldwide'),
                        'Package "x.pkg":\n   Status: no signature\n', '']:
            with self.assertRaises(ValueError): verify_package_signature(changed)

    def test_an_apple_distribution_app_is_checked_without_the_hardened_runtime_or_a_developer_id(self):
        metadata = 'Identifier=app.hahakigami.lite\nAuthority=Apple Distribution: Fixture (TEAM)\nTeamIdentifier=TEAM\nSealed Resources version=2\ndesignated => identifier "app.hahakigami.lite" and anchor apple generic and certificate leaf[subject.CN] = "Apple Distribution: Fixture (TEAM)" and certificate 1[field.1.2.840.113635.100.6.2.1]\n'
        verify_metadata(metadata, 'app.hahakigami.lite', False, APPLE_DISTRIBUTION)
        for changed in [metadata.replace('Authority=Apple Distribution', 'Authority=Apple Development'), metadata.replace('app.hahakigami.lite', 'app.hahakigami.desktop')]:
            with self.assertRaises(ValueError): verify_metadata(changed, 'app.hahakigami.lite', False, APPLE_DISTRIBUTION)


TEAM = 'ABCDE12345'
STORE_IDENTIFIERS = (f'{TEAM}.app.hahakigami.lite', TEAM)
PROFILE = {'Entitlements': {'com.apple.application-identifier': f'{TEAM}.app.hahakigami.lite',
                            'com.apple.developer.team-identifier': TEAM}, 'TeamIdentifier': [TEAM]}


class StoreSigningTests(unittest.TestCase):
    def test_the_selected_identity_is_found_by_its_full_name(self):
        output = listing(('B', f'Apple Distribution: Fixture ({TEAM})'), ('C', f'Mac Installer Distribution: Fixture ({TEAM})'))
        self.assertEqual(identity_names(output, INSTALLER), [('C' * 40, f'Mac Installer Distribution: Fixture ({TEAM})')])
        self.assertEqual(identity_name(output, 'c' * 40, INSTALLER), f'Mac Installer Distribution: Fixture ({TEAM})')
        with self.assertRaises(ValueError): identity_name(output, 'D' * 40, INSTALLER)

    def test_two_valid_identities_with_one_name_cannot_be_chosen_by_name(self):
        name = f'Mac Installer Distribution: Fixture ({TEAM})'
        output = listing(('C', name), ('D', name), ('E', f'Mac Installer Distribution: Other ({TEAM})'))
        with self.assertRaisesRegex(ValueError, 'More than one.*Fixture'): identity_name(output, 'C' * 40, INSTALLER)
        self.assertEqual(identity_name(output, 'E' * 40, INSTALLER), f'Mac Installer Distribution: Other ({TEAM})')
        # The same certificate listed twice (two keychains) is still one identity.
        self.assertEqual(identity_name(listing(('C', name), ('C', name)), 'C' * 40, INSTALLER), name)

    def test_the_team_is_read_from_the_identity_name(self):
        self.assertEqual(identity_team(f'Apple Distribution: Fixture Co ({TEAM})'), TEAM)
        for name in ['Apple Distribution: Fixture', f'Apple Distribution: Fixture ({TEAM}) extra', 'Apple Distribution: Fixture (abc)']:
            with self.assertRaises(ValueError): identity_team(name)

    def test_the_profile_must_be_for_the_lite_app_and_the_signing_team(self):
        self.assertEqual(store_identifiers(PROFILE, TEAM, 'app.hahakigami.lite'), STORE_IDENTIFIERS)
        with self.assertRaises(ValueError): store_identifiers(PROFILE, TEAM, 'app.hahakigami.desktop')
        with self.assertRaises(ValueError): store_identifiers(PROFILE, 'ZZZZZ99999', 'app.hahakigami.lite')
        other = {**PROFILE, 'Entitlements': {**PROFILE['Entitlements'], 'com.apple.application-identifier': f'ZZZZZ99999.app.hahakigami.lite'}}
        with self.assertRaises(ValueError): store_identifiers(other, TEAM, 'app.hahakigami.lite')
        wildcard = {**PROFILE, 'Entitlements': {**PROFILE['Entitlements'], 'com.apple.application-identifier': f'{TEAM}.*'}}
        with self.assertRaises(ValueError): store_identifiers(wildcard, TEAM, 'app.hahakigami.lite')
        wrong_team = {**PROFILE, 'Entitlements': {**PROFILE['Entitlements'], 'com.apple.developer.team-identifier': 'ZZZZZ99999'}}
        with self.assertRaises(ValueError): store_identifiers(wrong_team, TEAM, 'app.hahakigami.lite')
        with self.assertRaises(ValueError): store_identifiers({}, TEAM, 'app.hahakigami.lite')

    def test_the_store_entitlements_add_the_two_identifiers_and_leave_the_shipped_file_alone(self):
        import plistlib
        before = LITE_ENTITLEMENTS.read_bytes()
        entitlements = plistlib.loads(store_entitlements(STORE_IDENTIFIERS))
        self.assertEqual(entitlements, {**{key: True for key in LITE_REQUIRED_ENTITLEMENTS},
                                        'com.apple.application-identifier': STORE_IDENTIFIERS[0],
                                        'com.apple.developer.team-identifier': TEAM})
        self.assertEqual(LITE_ENTITLEMENTS.read_bytes(), before)
        self.assertNotIn(b'application-identifier', before)

    def test_the_entitlements_file_is_handed_to_tauri_without_mutating_the_overrides(self):
        from pathlib import Path
        overrides = {'bundle': {'macOS': {'signingIdentity': 'B' * 40}}}
        result = with_entitlements(overrides, Path('/tmp/x/lite-store.entitlements'))
        self.assertEqual(result['bundle']['macOS'], {'signingIdentity': 'B' * 40, 'entitlements': '/tmp/x/lite-store.entitlements'})
        self.assertNotIn('entitlements', overrides['bundle']['macOS'])

    def test_a_store_signature_must_carry_both_identifiers(self):
        signed = LITE_PLIST.replace('</dict>', f'<key>com.apple.application-identifier</key><string>{STORE_IDENTIFIERS[0]}</string>'
                                               f'<key>com.apple.developer.team-identifier</key><string>{TEAM}</string></dict>')
        verify_entitlements(signed, STORE_IDENTIFIERS)
        # A development build does not need them, and a store build is not satisfied without them.
        verify_entitlements(LITE_PLIST)
        with self.assertRaises(ValueError): verify_entitlements(LITE_PLIST, STORE_IDENTIFIERS)
        for key in ('com.apple.application-identifier', 'com.apple.developer.team-identifier'):
            with self.assertRaisesRegex(ValueError, key):
                verify_entitlements(signed.replace(f'<key>{key}</key>', '<key>x</key>'), STORE_IDENTIFIERS)
        with self.assertRaises(ValueError): verify_entitlements(signed.replace(TEAM + '.app', 'ZZZZZ99999.app'), STORE_IDENTIFIERS)
