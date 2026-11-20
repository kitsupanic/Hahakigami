#!/usr/bin/env python3
"""Stage/build isolated instrumented upstream UI apps. Never launch an app.

Native scanner files remain byte-identical except OpenDisk's ScanCache storage
location. Changes to DiskAnalyzer remove a privacy probe, not scanner coverage.
The AppKit draw callback confirms CPU drawing after model publication, not GPU presentation.
"""
import argparse
import importlib.util
import difflib
import hashlib
import json
import plistlib
from pathlib import Path
import shutil
import subprocess
from refs import REFS

ROOT = Path(__file__).resolve().parents[1]


def checked(command, **kwargs):
    return subprocess.run(command, check=True, timeout=1800, **kwargs)


def replaced(text, old, new, where, once=True):
    if text.count(old) != 1 if once else old not in text:
        raise ValueError(f"Pinned instrumentation anchor changed in {where}: {old[:60]!r}")
    return text.replace(old, new)


def replace_once(path, old, new):
    path.write_text(replaced(path.read_text(), old, new, path))


BOOTSTRAP = r'''
// Benchmark-only persistence and timing. No HOME override or privacy probe.
import Darwin
import Foundation
import AppKit
import SwiftUI

nonisolated final class BenchmarkDefaults: UserDefaults {
    nonisolated(unsafe) static let shared = BenchmarkDefaults()
    private let lock = NSRecursiveLock()
    private var values: [String: Any] = [:]
    private let file: URL
    override init?(suiteName: String?) { fatalError("Use shared") }
    init() {
        file = URL(fileURLWithPath: BenchmarkRun.stateRoot).appendingPathComponent("preferences.plist")
        super.init(suiteName: "benchmark-in-memory-unused")!
        if let data = try? Data(contentsOf: file), let loaded = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any] { values = loaded }
    }
    override func object(forKey key: String) -> Any? { lock.lock(); defer { lock.unlock() }; return values[key] }
    override func set(_ value: Any?, forKey key: String) {
        lock.lock(); defer { lock.unlock() }; values[key] = value
        if let data = try? PropertyListSerialization.data(fromPropertyList: values, format: .binary, options: 0) { try? data.write(to: file, options: .atomic) }
    }
    override func removeObject(forKey key: String) { set(nil, forKey: key) }
    override func bool(forKey key: String) -> Bool { (object(forKey: key) as? NSNumber)?.boolValue ?? false }
    override func integer(forKey key: String) -> Int { (object(forKey: key) as? NSNumber)?.intValue ?? 0 }
    override func string(forKey key: String) -> String? { object(forKey: key) as? String }
    override func data(forKey key: String) -> Data? { object(forKey: key) as? Data }
    override func array(forKey key: String) -> [Any]? { object(forKey: key) as? [Any] }
    override func dictionary(forKey key: String) -> [String: Any]? { object(forKey: key) as? [String: Any] }
    override func stringArray(forKey key: String) -> [String]? { object(forKey: key) as? [String] }
}

nonisolated enum BenchmarkRun {
    static func fail(_ message: String) -> Never { fputs(message + "\n", stderr); exit(2) }
    static let launch = DispatchTime.now().uptimeNanoseconds
    static let config: [String: String] = {
        let args = CommandLine.arguments
        guard let index = args.firstIndex(of: "--config") else { return [:] }
        guard args.indices.contains(index + 1),
              let data = try? Data(contentsOf: URL(fileURLWithPath: args[index + 1])),
              let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              value["schema_version"] as? Int == 2 else { fail("Invalid benchmark config") }
        return value.compactMapValues { $0 as? String }
    }()
    static var mode: String {
        let mode = config["mode"] ?? "full"
        guard ["full", "persisted", "seed"].contains(mode) else { fail("Unsupported upstream UI mode") }
        return mode
    }
    static var root: String {
        guard let path = config["root"] ?? ProcessInfo.processInfo.environment["HAHAKIGAMI_BENCHMARK_ROOT"], path.hasPrefix("/") else { fail("Missing absolute benchmark scan root") }
        return path
    }
    static var stateRoot: String {
        if let path = config["state_dir"], path.hasPrefix("/") { return path }
        guard let path = ProcessInfo.processInfo.environment["HAHAKIGAMI_BENCHMARK_STATE_ROOT"], path.hasPrefix("/") else { fail("Missing absolute disposable benchmark state root") }
        return path
    }
    @MainActor static var start: UInt64 = 0
    @MainActor static var launchMS: Double = 0
    @MainActor static var finished = false
    @MainActor static var scanStartedAt = Date.distantPast
    @MainActor static var finalContentID: UUID?
    @MainActor static var armedContentID: UUID?
    @MainActor static var armedVersion: Int?
    @MainActor static var pending: ((UInt64) -> Void)?
    @MainActor static func prepare() {
        _ = mode; _ = root; _ = stateRoot
        UserDefaults.standard.register(defaults: ["ApplePersistenceIgnoreState": true, "NSQuitAlwaysKeepsWindows": false])
    }
    @MainActor static func begin() {
        guard start == 0 else { return }
        let launched = launch
        start = DispatchTime.now().uptimeNanoseconds
        scanStartedAt = Date()
        launchMS = Double(start - launched) / 1_000_000
    }
    @MainActor static func arm(contentID: UUID, version: Int) {
        guard contentID == finalContentID else { return }
        armedContentID = contentID; armedVersion = version
    }
    @MainActor static func chartDrew(contentID: UUID?, version: Int, timestamp: UInt64) {
        guard let completion = pending else { return }
        if let expected = finalContentID {
            guard contentID == expected, armedContentID == expected, armedVersion == version else { return }
        }
        pending = nil
        completion(timestamp)
    }
    @MainActor static func waitForCache() async -> (Bool, Double, String?) {
        return (true, 0, nil) // CACHE_WAIT_IMPLEMENTATION
    }
    @MainActor static func complete(entries: Int, allocated: Int64, errors: Int, contentID: UUID? = nil) {
        guard start != 0, !finished else { return }
        finished = true
        finalContentID = contentID
        // Only the completed results Canvas may trigger this callback.
        pending = { drawTimestamp in
          Task { @MainActor in
            let (published, waitMS, cacheError) = await waitForCache()
            var usage = rusage(); getrusage(RUSAGE_SELF, &usage)
            let elapsed = Double(drawTimestamp - start) / 1_000_000
            let report: [String: Any] = ["schema_version": 2, "mode": mode, "cache": "production_enabled_private_state", "elapsed_ms": elapsed, "draw_elapsed_ms": elapsed,
                "launch_ms": launchMS, "launch_clock_origin": "app_initializer_entry_not_process_exec", "entries": entries, "allocated_bytes": allocated,
                "errors": errors, "peak_rss_bytes": usage.ru_maxrss, "complete": errors == 0 && published,
                "cache_publication_complete": published, "cache_publication_wait_ms": waitMS, "cache_publication_error": cacheError.map { $0 as Any } ?? NSNull(),
                "rendered": true, "render_barrier": "final_results_canvas_draw_cpu_only",
                "coverage_limitations": ["Instrumented source build, not shipped release", "CPU draw callback observed; GPU presentation is not measured", "Native path fidelity and comparator accounting limitations apply"],
                "memory_scope": "owned_app_process_including_postdraw_cache_publication_external_sampler_required"]
            if let data = try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]) {
                print(String(decoding: data, as: UTF8.self)); fflush(stdout)
                try? data.write(to: URL(fileURLWithPath: stateRoot).appendingPathComponent("report.json"), options: .atomic)
            }
            DispatchQueue.main.async { NSApplication.shared.terminate(nil) }
          }
        }
    }
}
'''


def bootstrap_source(name):
    text = BOOTSTRAP.replace('"schema_version": 2, "mode": mode', '"schema_version": 2, "comparator": "' + name + '", "source_ref": "' + REFS[name] + '", "workers": NSNull(), "mode": mode')
    if name == "Radix":
        text = text.replace('"cache": "production_enabled_private_state"', '"cache": "none_no_persistent_scan_cache"')
        text = text.replace('["full", "persisted", "seed"].contains(mode)', '["full"].contains(mode)')
    else:
        text = text.replace('return (true, 0, nil) // CACHE_WAIT_IMPLEMENTATION', '''let waitStart = DispatchTime.now().uptimeNanoseconds
        let deadline = waitStart + 30_000_000_000
        while DispatchTime.now().uptimeNanoseconds < deadline {
            if let (header, _) = ScanCache.peek(forRoot: root), header.capturedAt >= scanStartedAt {
                return (true, Double(DispatchTime.now().uptimeNanoseconds - waitStart) / 1_000_000, nil)
            }
            try? await Task.sleep(for: .milliseconds(50))
        }
        return (false, Double(DispatchTime.now().uptimeNanoseconds - waitStart) / 1_000_000, "normal background cache publication not observed within 30 seconds")''')
    return text


def instrument_results_canvases(app, name):
    if name == "Radix":
        for kind in ("Sunburst", "Treemap"):
            directory = app / "Features" / "Visualization"
            canvas = directory / f"{kind}ChartCanvases.swift"
            text = canvas.read_text()
            end = text.index("\n}\n") + 3
            base, rest = text[:end], text[end:]
            base = replaced(base, "    let renderVersion: Int", "    let renderVersion: Int\n    let benchmarkContentID: UUID", canvas)
            base = replaced(base, "lhs.renderVersion == rhs.renderVersion", "lhs.renderVersion == rhs.renderVersion && lhs.benchmarkContentID == rhs.benchmarkContentID", canvas)
            old = "            }\n        }\n    }\n}"
            callback = """            }
            let drawnAt = DispatchTime.now().uptimeNanoseconds
            let contentID = benchmarkContentID
            let version = renderVersion
            Task { @MainActor in BenchmarkRun.chartDrew(contentID: contentID, version: version, timestamp: drawnAt) }
        }
    }
}"""
            if base.count(old) != 1:
                raise ValueError("Pinned Canvas drawing anchor changed")
            canvas.write_text("import Dispatch\n" + base.replace(old, callback) + rest)
            view = directory / f"{kind}ChartView.swift"
            text = view.read_text()
            text = replaced(text, "renderVersion: chartModel.renderedLayoutVersion,", "renderVersion: chartModel.renderedLayoutVersion,\n                    benchmarkContentID: treeStore.contentID,", view)
            marker = f"private struct {kind}RenderedChartLayer: View {{"
            offset = text.index(marker)
            before, layer = text[:offset], text[offset:]
            layer = replaced(layer, "    let renderVersion: Int", "    let renderVersion: Int\n    let benchmarkContentID: UUID", view)
            layer = replaced(layer, "renderVersion: renderVersion", "renderVersion: renderVersion,\n                benchmarkContentID: benchmarkContentID", view)
            # Sunburst's memberwise call had no comma; Treemap's already did.
            text = before + layer
            old = "                    layoutID: layoutRequestID\n                )\n            }"
            new = """                    layoutID: layoutRequestID
                )
                if chartModel.layoutReadiness.renderedLayoutID == layoutRequestID {
                    BenchmarkRun.arm(contentID: treeStore.contentID, version: chartModel.renderedLayoutVersion)
                }
            }"""
            if text.count(old) != 1:
                raise ValueError("Pinned layout completion anchor changed")
            view.write_text(text.replace(old, new))
    else:
        chart = app / "Views" / "Charts" / "RingsChartView.swift"
        text = chart.read_text()
        text = replaced(text, "    let root: ChartItem", "    let root: ChartItem\n    var benchmarkFinal = false", chart)
        text = replaced(text, "    @State private var layout: RingsChartLayout.Layout?", "    @State private var layout: RingsChartLayout.Layout?\n    @State private var layoutSourceRoot: ChartItem?", chart)
        text = replaced(text, "StaticChartLayer(layout: layout)", "StaticChartLayer(layout: layout, benchmarkFinal: benchmarkFinal && layoutSourceRoot == root)", chart)
        text = replaced(text, "layout = RingsChartLayout.layout(root: root, in: geometry.size, freeBytes: freeBytes)", "layout = RingsChartLayout.layout(root: root, in: geometry.size, freeBytes: freeBytes)\n                layoutSourceRoot = root", chart, once=False)
        offset = text.index("private struct StaticChartLayer: View, Equatable {")
        before, layer = text[:offset], text[offset:]
        layer = replaced(layer, "    let layout: RingsChartLayout.Layout", "    let layout: RingsChartLayout.Layout\n    let benchmarkFinal: Bool", chart)
        old = "            }\n        }\n    }\n}"
        end = layer.index("\n}\n") + 3
        base, rest = layer[:end], layer[end:]
        callback = """            }
            if benchmarkFinal {
                let drawnAt = DispatchTime.now().uptimeNanoseconds
                Task { @MainActor in BenchmarkRun.chartDrew(contentID: nil, version: 0, timestamp: drawnAt) }
            }
        }
    }
}"""
        if base.count(old) != 1:
            raise ValueError("Pinned rings Canvas drawing anchor changed")
        chart.write_text("import Dispatch\n" + before + base.replace(old, callback) + rest)
        replace_once(app / "Views" / "Analysis" / "DiskAnalysisView.swift", "root: chartRoot,", "root: chartRoot,\n                        benchmarkFinal: !analyzer.isScanning,")


def stage(name, source, output, state):
    actual = checked(["git", "-C", str(source), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    dirty = checked(["git", "-C", str(source), "status", "--porcelain"], capture_output=True, text=True).stdout.strip()
    if actual != REFS[name] or dirty:
        raise ValueError(f"{name} reference must be clean and pinned")
    staged = output / f"{name}-source"
    if staged.exists():
        raise ValueError(f"Refusing to overwrite prior source evidence: {staged}")
    shutil.copytree(source, staged, ignore=shutil.ignore_patterns(".git", ".build"))
    app = staged / name
    original = {str(p.relative_to(staged)): p.read_bytes() for p in app.rglob("*.swift")}
    # All preferences use a file-backed isolated defaults object, including AppStorage.
    for p in app.rglob("*.swift"):
        text = p.read_text().replace("UserDefaults.standard", "BenchmarkDefaults.shared")
        text = text.replace("UserDefaults = .standard", "UserDefaults = BenchmarkDefaults.shared")
        import re
        text = re.sub(r'@AppStorage\(([^\n]*)\)', r'@AppStorage(\1, store: BenchmarkDefaults.shared)', text)
        p.write_text(text)
    (app / "BenchmarkBootstrap.swift").write_text(bootstrap_source(name))
    if name == "Radix":
        replace_once(app / "Services" / "SystemIntegration.swift", "nonisolated static func fullDiskAccessStatus() -> FullDiskAccessStatus {\n        probeFullDiskAccess()\n    }", "nonisolated static func fullDiskAccessStatus() -> FullDiskAccessStatus {\n        .unknown // Benchmark: do not probe protected paths or TCC databases.\n    }")
        replace_once(app / "Services" / "SystemIntegration.swift", "static func primeFullDiskAccessListEntry() {\n        _ = probeFullDiskAccess()\n    }", "static func primeFullDiskAccessListEntry() {}")
        replace_once(app / "RadixApp.swift", "@StateObject private var appModel = AppModel()", "@StateObject private var appModel: AppModel")
        replace_once(app / "RadixApp.swift", "    init() {", "    init() {\n        _ = BenchmarkRun.launch\n        BenchmarkRun.prepare()\n        BenchmarkDefaults.shared.set(true, forKey: \"didCompleteOnboarding\")\n        _appModel = StateObject(wrappedValue: AppModel())")
        replace_once(app / "RadixApp.swift", "startingUpdater: true", "startingUpdater: false")
        replace_once(app / "RadixApp.swift", ".environmentObject(appModel)\n                .frame", """.environmentObject(appModel)
                .onAppear {
                    BenchmarkRun.begin()
                    appModel.startScan(ScanTarget(url: URL(fileURLWithPath: BenchmarkRun.root)))
                }
                .onReceive(appModel.scanState.$snapshot) { snapshot in
                    if let snapshot, snapshot.isComplete {
                        BenchmarkRun.complete(entries: snapshot.treeStore.nodeCount, allocated: snapshot.root.allocatedSize, errors: snapshot.scanWarnings.count, contentID: snapshot.treeStore.contentID)
                    }
                }
                .frame""")
        # There are two environmentObject anchors; replace_once intentionally checks.
    else:
        replace_once(app / "App" / "OpenDiskApp.swift", "    init() {", "    init() {\n        _ = BenchmarkRun.launch\n        BenchmarkRun.prepare()")
        replace_once(app / "App" / "SoftwareUpdater.swift", "startingUpdater: true", "startingUpdater: false")
        replace_once(app / "App" / "OpenDiskApp.swift", ".onAppear(perform: checkFullDiskAccessAtStartup)", "")
        content = app / "Views" / "Main" / "ContentView.swift"
        text = content.read_text(); start = text.index("            DevicePickerView("); end = text.index("\n        }\n        .environment", start)
        text = text[:start] + '            DiskAnalysisView(rootPath: BenchmarkRun.root, rootName: "Benchmark")' + text[end:]; content.write_text(text)
        analyzer = app / "Services" / "DiskAnalyzer.swift"
        text = analyzer.read_text(); start = text.index('        if !ScanAccess.isSandboxed && path == "/" && !FullDiskAccess.isGranted {'); end = text.index('\n        needsFullDiskAccess = false', start)
        analyzer.write_text(text[:start] + text[end:])
        replace_once(app / "Services" / "Scanning" / "ScanCache.swift", """guard let caches = FileManager.default.urls(
            for: .cachesDirectory, in: .userDomainMask
        ).first else { return nil }""", 'let caches = URL(fileURLWithPath: BenchmarkRun.stateRoot).appendingPathComponent("cache", isDirectory: true)')
        view = app / "Views" / "Analysis" / "DiskAnalysisView.swift"
        replace_once(view, "scanAccess.beginAccess(toPath: rootPath)\n            Task { await analyzer.scanDirectory(rootPath) }", "scanAccess.beginAccess(toPath: rootPath)\n            Task { BenchmarkRun.begin(); await analyzer.scanDirectory(rootPath) }")
        replace_once(view, "if !isScanning { Task { await refreshVolumeCapacity() } }", """if !isScanning {
                Task { await refreshVolumeCapacity() }
                BenchmarkRun.complete(entries: analyzer.itemsScanned, allocated: analyzer.totalDiskScannedBytes, errors: analyzer.unreadableDirectories)
            }""")
    instrument_results_canvases(app, name)
    diff = []
    for p in app.rglob("*.swift"):
        relative = str(p.relative_to(staged)); before = original.get(relative, b"").decode()
        diff.extend(difflib.unified_diff(before.splitlines(True), p.read_text().splitlines(True), fromfile="a/" + relative, tofile="b/" + relative))
    (output / f"{name}-instrumentation.patch").write_text("".join(diff))
    (output / f"{name}-source-evidence.json").write_text(json.dumps({"source_ref": actual,
        "state_root": str(state), "supported_modes": ["full"] if name == "Radix" else ["full", "persisted", "seed"], "original_source_sha256": {p: hashlib.sha256(b).hexdigest() for p,b in original.items()},
        "instrumented_source_sha256": {str(p.relative_to(staged)): hashlib.sha256(p.read_bytes()).hexdigest() for p in app.rglob("*.swift")},
        "limitations": ["CPU drawing callback is observed; GPU frame is not", "Only full/persisted-cache auto-run is instrumented; same-process UI rescans require separate driver", "No privacy probes or grants changed"]}, indent=2) + "\n")
    shutil.copy2(source / "LICENSE", output / f"{name}-LICENSE.txt")
    return staged


def signing_helpers():
    spec = importlib.util.spec_from_file_location("hahakigami_build_macos", ROOT / "scripts" / "build_macos.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def development_identity(requested=None):
    helpers = signing_helpers()
    result = checked(["/usr/bin/security", "find-identity", "-v", "-p", "codesigning"], capture_output=True, text=True)
    return helpers.select_identity(helpers.development_identities(result.stdout), requested)


def sign_bundle(bundle, name, identity):
    checked(["codesign", "--force", "--deep", "--sign", identity, str(bundle)])
    checked(["codesign", "--verify", "--deep", "--strict", str(bundle)])
    result = checked(["codesign", "--display", "--verbose=4", "--requirements", "-", str(bundle)], capture_output=True, text=True)
    metadata = result.stdout + result.stderr
    signing_helpers().verify_metadata(metadata, "local.hahakigami.benchmark." + name.lower(), False)
    return metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-root", type=Path, required=True)
    parser.add_argument("--output-root", type=Path, required=True)
    parser.add_argument("--app", choices=["Radix", "OpenDisk", "all"], default="all")
    parser.add_argument("--stage-only", action="store_true")
    parser.add_argument("--identity", help="Existing Apple Development SHA-1; defaults to the single valid identity")
    args = parser.parse_args()
    for path in (args.state_root, args.output_root):
        if not path.is_absolute() or path == Path("/") or path == Path.home(): parser.error("Use absolute disposable state/output directories")
        path.mkdir(parents=True, exist_ok=True)
    identity = None if args.stage_only else development_identity(args.identity)
    for name in REFS if args.app == "all" else [args.app]:
        state = args.state_root / name; state.mkdir(exist_ok=True)
        staged = stage(name, ROOT / "benchmark-results" / ".cache" / name, args.output_root, state)
        if not args.stage_only:
            derived = args.output_root / f"{name}-derived"
            checked(["xcodebuild", "-project", str(staged / f"{name}.xcodeproj"), "-scheme", name,
                "-configuration", "Release", "-derivedDataPath", str(derived), "build",
                "CODE_SIGNING_ALLOWED=NO", f"PRODUCT_BUNDLE_IDENTIFIER=local.hahakigami.benchmark.{name.lower()}"])
            bundle = derived / "Build" / "Products" / "Release" / f"{name}.app"
            metadata = sign_bundle(bundle, name, identity)
            (args.output_root / f"{name}-signing-metadata.txt").write_text(metadata)
            info = plistlib.loads((bundle / "Contents/Info.plist").read_bytes())
            executable = bundle / "Contents/MacOS" / info["CFBundleExecutable"]
            (args.output_root / f"{name}-app-evidence.json").write_text(json.dumps({
                "source_ref": REFS[name], "bundle": str(bundle), "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
                "instrumentation_patch_sha256": hashlib.sha256((args.output_root / f"{name}-instrumentation.patch").read_bytes()).hexdigest(),
                "signing": "isolated_apple_development_instrumented_upstream_build", "state_root": str(state)}, indent=2) + "\n")
            print(bundle)
        print(f"Launch only an owned instance with HAHAKIGAMI_BENCHMARK_STATE_ROOT={state} and HAHAKIGAMI_BENCHMARK_ROOT=<scope>.")


if __name__ == "__main__":
    main()
