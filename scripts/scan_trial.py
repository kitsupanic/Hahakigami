#!/usr/bin/env python3
"""Fingerprint and time one Rust release scan; not an acceptance comparison."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import subprocess
import time
from benchmark import ROOT, file_hash, scanner_fingerprints


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/examples/scan")
    parser.add_argument("--output", type=Path, default=ROOT / "benchmark-results/trial.json")
    args = parser.parse_args()
    started = datetime.now(timezone.utc).isoformat()
    fingerprint = file_hash(args.binary)
    source = scanner_fingerprints()
    print(f"Single release-engine trial: {args.path}, {args.workers} workers", flush=True)
    t = time.monotonic()
    raw = json.loads(subprocess.check_output([str(args.binary), str(args.path), str(args.workers), "bulk"], text=True, timeout=600))
    result = {"recorded_at_utc": started, "measurement": "single warm engine trial, not acceptance comparison",
              "binary_sha256": fingerprint, "scanner_source_sha256": source,
              "wall_ms": (time.monotonic() - t) * 1000, "raw": raw}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
