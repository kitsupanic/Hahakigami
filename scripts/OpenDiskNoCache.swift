// Benchmark-only ScanCache API shim. Header and Entry signatures adapted from
// OpenDisk (https://github.com/137137137/OpenDisk), MIT, Copyright (c) 2026 137137137.
// See scripts/OPENDISK-LICENSE.txt. No user cache is read or written.
import Foundation

enum ScanCache {
    struct Header: Sendable {
        let eventID: UInt64
        let capturedAt: Date
        let fullScanSeconds: TimeInterval
    }
    struct Entry { let tree: FileTree; let header: Header }
    static func peek(forRoot rootPath: String) -> (header: Header, fileBytes: Int)? { nil }
    static func load(forRoot rootPath: String) -> Entry? { nil }
    static func save(tree: FileTree, forRoot rootPath: String, header: Header) {}
}
