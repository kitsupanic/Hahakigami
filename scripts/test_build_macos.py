import unittest
from build_macos import development_identities, select_identity, local_environment, verify_metadata, prepare_benchmark_output, benchmark_windows, benchmark_frontend_command

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
        metadata = 'Identifier=app.hahakigami.desktop\nAuthority=Apple Development: Fixture\nTeamIdentifier=TEAM\nInfo.plist entries=10\nSealed Resources version=2\ndesignated => identifier "app.hahakigami.desktop" and anchor apple generic\n'
        verify_metadata(metadata, 'app.hahakigami.desktop', False)
        verify_metadata(metadata.replace('designated => ', '# designated => '), 'app.hahakigami.desktop', False)
        for changed in [metadata.replace('app.hahakigami.desktop', 'other'), metadata.replace('Info.plist entries=10', 'Info.plist=not bound'), metadata.replace('Sealed Resources version=2', 'Sealed Resources=none'), metadata.replace('anchor apple generic', 'cdhash H"123"'), metadata.replace('Authority=Apple Development: Fixture', 'Signature=adhoc'), metadata.replace('TeamIdentifier=TEAM\n', '')]:
            with self.assertRaises(ValueError): verify_metadata(changed, 'app.hahakigami.desktop', False)

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
