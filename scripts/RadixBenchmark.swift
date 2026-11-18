// Calls pinned Radix ScanEngine with untouched, normal ScanOptions defaults.
import Darwin
import Foundation

@main
struct Benchmark {
    static func main() async throws {
        guard CommandLine.arguments.count >= 2 else {
            fputs("usage: radix-benchmark PATH [--coverage-output FILE]\n", stderr)
            exit(2)
        }
        let root = CommandLine.arguments[1]
        var coverage: String?
        if CommandLine.arguments.count > 2 {
            guard CommandLine.arguments.count == 4, CommandLine.arguments[2] == "--coverage-output" else { exit(2) }
            coverage = CommandLine.arguments[3]
        }
        let options = ScanOptions(includeHiddenFiles: AppScanPreferences.defaults.showHiddenFiles)
        let target = ScanTarget(url: URL(fileURLWithPath: root))
        let start = DispatchTime.now().uptimeNanoseconds
        var snapshot: ScanSnapshot?
        for try await event in ScanEngine().scan(target: target, options: options) {
            if case .finished(let result) = event { snapshot = result }
        }
        let elapsed = Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000
        var usage = rusage()
        getrusage(RUSAGE_SELF, &usage)
        guard let snapshot else { throw CocoaError(.fileReadUnknown) }
        var summarized = 0
        snapshot.treeStore.forEachIndexedNodeID { id in
            if let node = snapshot.treeStore.node(id: id), node.isAutoSummarized || (node.isPackage && snapshot.treeStore.children(of: id).isEmpty) { summarized += 1 }
        }
        let report: [String: Any] = [
            "schema_version": 2, "source_ref": "df1235bfb05acb7a678718166491bcc27817d95d",
            "comparator": "Radix", "mode": "engine", "cache": "none",
            "elapsed_ms": elapsed, "peak_rss_bytes": usage.ru_maxrss,
            "entries": snapshot.treeStore.nodeCount,
            "allocated_bytes": snapshot.root.allocatedSize,
            "errors": snapshot.scanWarnings.count, "complete": snapshot.isComplete,
            "workers_measurement": "configured_traversal_limit_not_observed_active_workers",
            "workers": ScanEngine.shallowRelistWorkerLimit(for: options),
            "summarized_directories": summarized,
            "coverage_limitations": ["Normal app defaults include hidden files and summarize packages/eligible directories.", "Allocation accounting deduplicates APFS clones; differs from OpenDisk/Hahakigami.", "Paths are Foundation URL representations; original invalid UTF-8 native bytes are unavailable."]
        ]
        // Timing and process peak RSS captured before optional export allocations/I/O.
        if let coverage { try export(snapshot, to: coverage, summarized: summarized) }
        let data = try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    }

    static func export(_ snapshot: ScanSnapshot, to path: String, summarized: Int) throws {
        guard FileManager.default.createFile(atPath: path, contents: nil) else { throw CocoaError(.fileWriteUnknown) }
        let handle = try FileHandle(forWritingTo: URL(fileURLWithPath: path))
        defer { try? handle.close() }
        func write(_ row: [String: Any]) throws {
            try handle.write(contentsOf: JSONSerialization.data(withJSONObject: row, options: [.sortedKeys]))
            try handle.write(contentsOf: Data([10]))
        }
        try write(["type": "header", "schema_version": 1, "complete": false, "node_count": snapshot.treeStore.nodeCount,
                   "native_path_fidelity": "unavailable_foundation_url", "root": snapshot.target.url.path,
                   "limitations": ["package/eligible directories summarized", "summarized directories: \(summarized)", "invalid UTF-8 source bytes unavailable"]])
        let root = snapshot.target.url.path
        let prefix = root == "/" ? "/" : root + "/"
        try snapshot.treeStore.forEachIndexedNodeID { id in
            guard let node = snapshot.treeStore.node(id: id) else { return }
            let full = node.url.path
            let relative = full == root ? "" : (full.hasPrefix(prefix) ? String(full.dropFirst(prefix.count)) : full)
            var identity: Any = NSNull()
            if let value = node.fileIdentity {
                switch value {
                case .fileSystem(let device, let inode, _): identity = ["device": device, "inode": inode]
                case .resourceIdentifier(let bytes): identity = ["resource_identifier_b64": bytes.base64EncodedString()]
                }
            }
            try write(["type": "node", "path_base64": Data(relative.utf8).base64EncodedString(),
                       "kind": node.isSynthetic ? "synthetic" : (node.isSymbolicLink ? "symlink" : (node.isDirectory ? "directory" : "file")),
                       "allocated_bytes": node.allocatedSize,
                       "size_scope": node.isDirectory ? "aggregate" : "own", "identity": identity,
                       "summary": node.isAutoSummarized || (node.isPackage && snapshot.treeStore.children(of: id).isEmpty)])
        }
        try write(["type": "footer", "schema_version": 1, "complete": true, "node_count": snapshot.treeStore.nodeCount])
    }
}
