#!/usr/bin/env python3
"""Sample only a newly-spawned scanner process; profiled timing is not benchmark evidence."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import platform
import resource
import subprocess
import time
from benchmark import ROOT, file_hash, scanner_fingerprints


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--seconds", type=int, default=5)
    parser.add_argument("--output", type=Path, default=ROOT / "benchmark-results/profile.json")
    parser.add_argument("--sample-file", type=Path, default=ROOT / "benchmark-results/profile.txt")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("macOS sample is required")
    if args.seconds < 1 or args.seconds > 30:
        parser.error("Sample duration must be 1–30 seconds")
    binary = ROOT / "target/release/examples/scan"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.sample_file.parent.mkdir(parents=True, exist_ok=True)
    fingerprint = file_hash(binary)
    source = scanner_fingerprints()
    inherited_limit = resource.getrlimit(resource.RLIMIT_NOFILE)
    recorded = datetime.now(timezone.utc).isoformat()
    process = subprocess.Popen([str(binary), str(args.path), str(args.workers), "bulk"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    print(f"Sampling own scanner PID {process.pid} for {args.seconds} seconds", flush=True)
    try:
        # Skip startup, then sample this exact process; no name-based attachment.
        time.sleep(1)
        sampled = subprocess.run(["/usr/bin/sample", str(process.pid), str(args.seconds), "1", "-mayDie", "-file", str(args.sample_file)], capture_output=True, text=True, timeout=args.seconds + 30)
        stdout, stderr = process.communicate(timeout=600)
    except BaseException:
        process.terminate()
        process.wait(timeout=10)
        raise
    report = {"recorded_at_utc": recorded, "description": "Owned scanner process only; no sudo or privacy changes. Profiled elapsed time is excluded from speed comparisons.",
              "binary_sha256": fingerprint, "scanner_source_sha256": source, "pid": process.pid,
              "inherited_parent_rlimit_nofile": {"soft": inherited_limit[0], "hard": "unlimited" if inherited_limit[1] == resource.RLIM_INFINITY else inherited_limit[1], "source": "Profiler parent immediately before spawning scanner; child inherits this limit."},
              "sampling_seconds": args.seconds, "sampling_interval_ms": 1,
              "sample_file": str(args.sample_file.relative_to(ROOT)) if args.sample_file.is_relative_to(ROOT) else str(args.sample_file),
              "sample_exit_code": sampled.returncode, "sample_stdout": sampled.stdout, "sample_stderr": sampled.stderr,
              "scan_exit_code": process.returncode, "scan_stderr": stderr,
              "raw_profiled_scan": json.loads(stdout) if process.returncode == 0 else None,
              "timing_valid_for_benchmark": False, "acceptance_met": False}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"sample_exit_code": sampled.returncode, "scan_exit_code": process.returncode, "timing_valid_for_benchmark": False}, indent=2))


if __name__ == "__main__":
    main()
