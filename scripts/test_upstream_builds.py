import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


apps = load("build_upstream_apps")
radix = load("build_radix_benchmark")


class UpstreamBuildTests(unittest.TestCase):
    def test_changed_anchor_fails_instead_of_unchecked_patch(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source.swift"
            path.write_text("anchor anchor")
            with self.assertRaises(ValueError):
                apps.replace_once(path, "anchor", "changed")
            self.assertEqual(path.read_text(), "anchor anchor")

    def test_manifest_sources_are_explicit_not_directory_glob(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Package.swift").write_text('sources: ["Models/A.swift", "Services/B.swift"], resources: ["wrong.swift"]')
            self.assertEqual(radix.source_files(root), [root / "Radix/Models/A.swift", root / "Radix/Services/B.swift"])

    def test_dirty_upstream_rejected_before_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with mock.patch.object(apps, "checked", side_effect=[mock.Mock(stdout=apps.REFS["Radix"]), mock.Mock(stdout=" M scanner.swift")]):
                with self.assertRaises(ValueError):
                    apps.stage("Radix", root, root, root)
            self.assertEqual(list(root.iterdir()), [])

    def test_persisted_mode_rejected_when_scanner_has_no_persistent_cache(self):
        self.assertIn('["full"].contains(mode)', apps.bootstrap_source("Radix"))
        self.assertIn('["full", "persisted", "seed"].contains(mode)', apps.bootstrap_source("OpenDisk"))

    def test_native_build_subprocess_is_bounded(self):
        with mock.patch.object(radix.subprocess, "run") as run:
            radix.run(["swiftc", "-O"])
            self.assertEqual(run.call_args.kwargs["timeout"], 900)
            self.assertTrue(run.call_args.kwargs["check"])

    @unittest.skipUnless((ROOT / "benchmark-results/.cache/Radix/.git").exists(), "Optional pinned source audit requires cached upstream source")
    def test_staged_radix_preserves_scanner_and_removes_privacy_probe_entrypoints(self):
        source = ROOT / "benchmark-results/.cache/Radix"
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            staged = apps.stage("Radix", source, output, output / "state")
            scanner = "Radix/Services/ScanEngine.swift"
            self.assertEqual((source / scanner).read_bytes(), (staged / scanner).read_bytes())
            integration = (staged / "Radix/Services/SystemIntegration.swift").read_text()
            self.assertIn(".unknown // Benchmark: do not probe", integration)
            self.assertIn("static func primeFullDiskAccessListEntry() {}", integration)
            bootstrap = (staged / "Radix/BenchmarkBootstrap.swift").read_text()
            self.assertIn('"final_results_canvas_draw_cpu_only"', bootstrap)
            self.assertNotIn("BenchmarkDrawingView", bootstrap)
            for kind in ("Sunburst", "Treemap"):
                view = (staged / f"Radix/Features/Visualization/{kind}ChartView.swift").read_text()
                self.assertIn("chartModel.layoutReadiness.renderedLayoutID == layoutRequestID", view)
                canvas = (staged / f"Radix/Features/Visualization/{kind}ChartCanvases.swift").read_text()
                self.assertIn("BenchmarkRun.chartDrew", canvas)
            self.assertNotIn('"swiftui_model_published_then_main_queue_turn"', bootstrap)
            self.assertTrue((output / "Radix-instrumentation.patch").is_file())
            with self.assertRaises(ValueError):
                apps.stage("Radix", source, output, output / "state")

    @unittest.skipUnless((ROOT / "benchmark-results/.cache/OpenDisk/.git").exists(), "Optional pinned source audit requires cached upstream source")
    def test_staged_opendisk_removes_probe_and_only_changes_cache_location(self):
        source = ROOT / "benchmark-results/.cache/OpenDisk"
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            staged = apps.stage("OpenDisk", source, output, output / "state")
            scanner = "OpenDisk/Services/Scanning/ScanEngine.swift"
            self.assertEqual((source / scanner).read_bytes(), (staged / scanner).read_bytes())
            analyzer = (staged / "OpenDisk/Services/DiskAnalyzer.swift").read_text()
            self.assertNotIn("!FullDiskAccess.isGranted", analyzer)
            app = (staged / "OpenDisk/App/OpenDiskApp.swift").read_text()
            self.assertNotIn(".onAppear(perform: checkFullDiskAccessAtStartup)", app)
            cache = (staged / "OpenDisk/Services/Scanning/ScanCache.swift").read_text()
            self.assertNotIn('if BenchmarkRun.mode == "full" { return nil }', cache)
            self.assertIn('BenchmarkRun.stateRoot', cache)
            rings = (staged / "OpenDisk/Views/Charts/RingsChartView.swift").read_text()
            self.assertIn("benchmarkFinal && layoutSourceRoot == root", rings)
            self.assertIn("BenchmarkRun.chartDrew", rings)
            bootstrap = (staged / "OpenDisk/BenchmarkBootstrap.swift").read_text()
            self.assertIn("header.capturedAt >= scanStartedAt", bootstrap)


if __name__ == "__main__":
    unittest.main()
