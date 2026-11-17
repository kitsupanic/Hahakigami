#!/usr/bin/env python3
"""Build a pinned OpenDisk release-engine harness without its persistent cache."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import subprocess

ROOT = Path(__file__).resolve().parents[1]
REF = "f7551cf21a5c35a963c607401ba5705ec0869b64"
SOURCE_FILES = [
    "Services/Scanning/ScanEngine.swift",
    "Services/Scanning/TraversalScanner.swift",
    "Services/Scanning/FileTree.swift",
    "Services/Scanning/ScanMetrics.swift",
    "Services/Scanning/DiskScanning.swift",
    "Services/Scanning/IncrementalUpdater.swift",
    "Services/Scanning/SystemInterop/FSEventsChangeJournal.swift",
    "Services/Scanning/SystemInterop/VolumeAttributes.swift",
    "Services/Scanning/SystemInterop/BulkDirectoryReader.swift",
    "Utilities/PathHelpers.swift",
    "Models/ScanProgress.swift",
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, help="Use an existing checkout at the pinned commit")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("OpenDisk engine comparison requires macOS")
    cache = ROOT / "benchmark-results" / ".cache"
    cache.mkdir(parents=True, exist_ok=True)
    source = args.source or cache / "OpenDisk"
    if not source.exists():
        subprocess.run(["git", "clone", "https://github.com/137137137/OpenDisk.git", str(source)], check=True, timeout=900)
        subprocess.run(["git", "-C", str(source), "checkout", "--detach", REF], check=True, timeout=900)
    actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True, timeout=30).strip()
    if actual != REF:
        parser.error(f"Expected OpenDisk {REF}; checkout is {actual}")
    if subprocess.check_output(["git", "-C", str(source), "status", "--porcelain"], text=True, timeout=30).strip():
        parser.error("Reference checkout must be unmodified")
    binary = cache / "opendisk-benchmark"
    files = [str(source / "OpenDisk" / name) for name in SOURCE_FILES]
    arch = platform.machine()
    subprocess.run([
        "swiftc", "-O", "-swift-version", "6", "-parse-as-library",
        "-target", f"{arch}-apple-macos15.0", "-o", str(binary), *files,
        str(ROOT / "scripts" / "OpenDiskNoCache.swift"),
        str(ROOT / "scripts" / "OpenDiskBenchmark.swift"),
    ], check=True, timeout=900)
    tracked_files = [Path(p) for p in files]
    (cache / "opendisk-benchmark-build.json").write_text(json.dumps({"source_ref": REF,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "source_sha256": {str(p.relative_to(source)): hashlib.sha256(p.read_bytes()).hexdigest() for p in tracked_files},
        "adapter_sha256": hashlib.sha256((ROOT / "scripts" / "OpenDiskBenchmark.swift").read_bytes()).hexdigest(),
        "cache_shim_sha256": hashlib.sha256((ROOT / "scripts" / "OpenDiskNoCache.swift").read_bytes()).hexdigest()}, indent=2) + "\n")
    print(binary)


if __name__ == "__main__":
    main()
