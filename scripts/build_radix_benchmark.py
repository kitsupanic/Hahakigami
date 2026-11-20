#!/usr/bin/env python3
"""Build pinned unmodified Radix sources plus a separate release-engine adapter."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import re
import subprocess
from refs import REFS, write_swift_ref

ROOT = Path(__file__).resolve().parents[1]
REF = REFS["Radix"]


def run(command, **kwargs):
    return subprocess.run(command, check=True, timeout=900, **kwargs)


def source_files(source):
    manifest = (source / "Package.swift").read_text()
    section = manifest.split("sources: [", 1)[1].split("],", 1)[0]
    files = re.findall(r'"([^"]+\.swift)"', section)
    if not files:
        raise ValueError("Pinned manifest has no source list")
    return [source / "Radix" / name for name in files]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("Radix engine comparison requires macOS")
    cache = ROOT / "benchmark-results" / ".cache"
    cache.mkdir(parents=True, exist_ok=True)
    source = args.source or cache / "Radix"
    if not source.exists():
        run(["git", "clone", "https://github.com/colinvkim/Radix.git", str(source)])
        run(["git", "-C", str(source), "checkout", "--detach", REF])
    actual = run(["git", "-C", str(source), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    if actual != REF or run(["git", "-C", str(source), "status", "--porcelain"], capture_output=True, text=True).stdout.strip():
        parser.error("Reference checkout must be clean and at pinned commit " + REF)
    files = source_files(source)
    adapter = ROOT / "scripts" / "RadixBenchmark.swift"
    binary = cache / "radix-benchmark"
    run(["swiftc", "-O", "-swift-version", "6", "-default-isolation", "MainActor",
         "-enable-upcoming-feature", "InferIsolatedConformances", "-enable-upcoming-feature", "NonisolatedNonsendingByDefault",
         "-parse-as-library", "-target", f"{platform.machine()}-apple-macos14.0", "-o", str(binary),
         *map(str, files), str(adapter), str(write_swift_ref(cache, "Radix"))])
    (cache / "radix-benchmark-build.json").write_text(json.dumps({"source_ref": REF,
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "source_sha256": {str(p.relative_to(source)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files},
        "adapter_sha256": hashlib.sha256(adapter.read_bytes()).hexdigest()}, indent=2) + "\n")
    print(binary)


if __name__ == "__main__":
    main()
