// Hahakigami benchmark driver. Calls the pinned OpenDisk engine without modifying
// its traversal, rollup, metrics, concurrency or partial snapshot behavior.
import Darwin
import Foundation

@main
struct Benchmark {
    static func main() async throws {
        let arguments = CommandLine.arguments
        guard arguments.count >= 2 else {
            fputs("usage: opendisk-benchmark PATH [engine|traversal|system-data]\n", stderr)
            exit(2)
        }
        let path = arguments[1]
        let mode = arguments.count > 2 && !arguments[2].hasPrefix("--") ? arguments[2] : "engine"
        let optionStart = mode == "engine" && (arguments.count <= 2 || arguments[2].hasPrefix("--")) ? 2 : 3
        var coverage: String?
        if arguments.count > optionStart {
            guard arguments.count == optionStart + 2, arguments[optionStart] == "--coverage-output" else { exit(2) }
            coverage = arguments[optionStart + 1]
        }
        let start = DispatchTime.now().uptimeNanoseconds
        let tree: FileTree
        let unreadable: Int
        if mode == "engine" {
            let result = await ScanEngine().scan(path: path, onEvent: { _ in })
            tree = result.tree
            unreadable = result.unreadableDirectories
        } else if mode == "traversal" || mode == "system-data" {
            // Diagnostic scope: ScanEngine's System/Data allowed-device policy,
            // without its additional sibling-volume traversal/composition.
            // The original TraversalScanner and BulkDirectoryReader remain unchanged.
            var devices: Set<dev_t>? = nil
            if mode == "system-data" {
                guard path == "/", let system = VolumeAttributes.deviceID(ofPath: "/"),
                      let data = VolumeAttributes.deviceID(ofPath: "/System/Volumes/Data") else {
                    fputs("system-data mode requires the startup root and System/Data device IDs\n", stderr)
                    exit(2)
                }
                devices = [system, data]
            }
            let metrics = ScanMetrics()
            var scanned = await TraversalScanner.scan(
                path: path, rootName: path, allowedDevices: devices,
                workerCount: VolumeAttributes.isVolumeRoot(path)
                    ? TraversalScanner.volumeWorkerCount : TraversalScanner.subtreeWorkerCount,
                metrics: metrics, isCancelled: { false }
            )
            scanned.rollUpDirectorySizes()
            tree = scanned
            unreadable = metrics.unreadableDirectories
        } else {
            fputs("unknown benchmark mode\n", stderr)
            exit(2)
        }
        let elapsedMS = Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000
        var usage = rusage()
        getrusage(RUSAGE_SELF, &usage)
        let reachability = tree.reachabilityBitmap()
        let output: [String: Any] = [
            "schema_version": 2, "source_ref": "f7551cf21a5c35a963c607401ba5705ec0869b64",
            "complete": unreadable == 0,
            "coverage_limitations": ["Scanner retains String names; invalid UTF-8 original bytes unavailable.", "Only directory/file kinds and hard-link identities retained; symlink kind unavailable.", "Allocated sizes deduplicate hard links but not APFS clones."],
            "comparator": "OpenDisk", "mode": mode, "cache": "disabled_no_io_shim",
            "elapsed_ms": elapsedMS,
            "entries": reachability.filter { $0 }.count,
            "allocated_bytes": tree.size(of: FileTree.rootID),
            "logical_bytes": NSNull(), "errors": unreadable,
            "peak_rss_bytes": usage.ru_maxrss,
            "stored_nodes": tree.nodeCount,
            "workers_measurement": "configured_traversal_limit_not_observed_active_workers",
            "workers": VolumeAttributes.isVolumeRoot(path)
                ? TraversalScanner.volumeWorkerCount : TraversalScanner.subtreeWorkerCount
        ]
        if let coverage { try export(tree, root: path, unreadable: unreadable, to: coverage) }
        let data = try JSONSerialization.data(withJSONObject: output, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    }
    static func export(_ tree: FileTree, root: String, unreadable: Int, to path: String) throws {
        guard FileManager.default.createFile(atPath: path, contents: nil) else { throw CocoaError(.fileWriteUnknown) }
        let handle = try FileHandle(forWritingTo: URL(fileURLWithPath: path))
        defer { try? handle.close() }
        func write(_ row: [String: Any]) throws {
            try handle.write(contentsOf: JSONSerialization.data(withJSONObject: row, options: [.sortedKeys]))
            try handle.write(contentsOf: Data([10]))
        }
        try write(["type": "header", "schema_version": 1, "complete": false, "node_count": tree.reachabilityBitmap().filter { $0 }.count,
                   "native_path_fidelity": "unavailable_swift_string", "root": root,
                   "limitations": ["invalid UTF-8 bytes unavailable", "symlink kind unavailable", "unreadable directories: \(unreadable)"]])
        // Traverse links with a depth-first iterator; no second tree/path table is retained.
        var stack: [FileTree.NodeID] = [FileTree.rootID]
        let prefix = root == "/" ? "/" : root + "/"
        var exported = 0
        while let id = stack.popLast() {
            exported += 1
            let full = tree.path(of: id)
            let relative = id == FileTree.rootID ? "" : (full.hasPrefix(prefix) ? String(full.dropFirst(prefix.count)) : full)
            var identity: Any = NSNull()
            if let key = tree.hardLinkKey(of: id) { identity = ["device": key.device, "inode": key.fileID] }
            try write(["type": "node", "path_base64": Data(relative.utf8).base64EncodedString(),
                       "kind": tree.isDirectory(id) ? "directory" : "file", "logical_bytes": NSNull(),
                       "allocated_bytes": tree.size(of: id), "size_scope": tree.isDirectory(id) ? "aggregate" : "own",
                       "identity": identity, "summary": false])
            stack.append(contentsOf: tree.children(of: id))
        }
        try write(["type": "footer", "schema_version": 1, "complete": true, "node_count": exported])
    }

}
