#!/usr/bin/env python3
"""Alternate full release-engine scans; preserve raw measurements and honest gates."""
import argparse
from datetime import datetime, timezone
import json
import hashlib
from pathlib import Path
import platform
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
OPENDISK_REF = "f7551cf21a5c35a963c607401ba5705ec0869b64"


def summarize(runs):
    scopes = {}
    for scope in dict.fromkeys(run["scope"] for run in runs):
        selected = [r for r in runs if r["scope"] == scope]
        warm = [r for r in selected if r["phase"] == "warm"]
        rust = [r for r in warm if r["app"] == "hahakigami"]
        reference = [r for r in warm if r["app"] == "opendisk"]
        comparable = bool(rust and reference) and len(rust) == len(reference)
        for round_number in {r["round"] for r in warm}:
            pair = [r for r in warm if r["round"] == round_number]
            comparable &= len(pair) == 2 and all(r.get("success") for r in pair)
            if len(pair) == 2:
                comparable &= pair[0].get("entries") == pair[1].get("entries")
                comparable &= pair[0].get("allocated_bytes") == pair[1].get("allocated_bytes")
                comparable &= pair[0].get("errors") == pair[1].get("errors")
        valid_rust = [r["elapsed_ms"] for r in rust if r.get("success")]
        valid_reference = [r["elapsed_ms"] for r in reference if r.get("success")]
        rust_median = statistics.median(valid_rust) if valid_rust else None
        reference_median = statistics.median(valid_reference) if valid_reference else None
        ratio = rust_median / reference_median if rust_median is not None and reference_median else None
        scopes[scope] = {
            "hahakigami_warm_median_ms": rust_median,
            "opendisk_warm_median_ms": reference_median,
            "hahakigami_over_opendisk_ratio": ratio,
            "matching_aggregate_coverage": comparable,
            "raw_speed_target_met": ratio is not None and ratio <= 1,
            "equivalent_coverage_speed_gate": comparable and ratio is not None and ratio <= 1,
            # Matching aggregates are necessary but not proof of equivalent path sets.
            "path_set_coverage_verified": False,
            "app_level_comparison_completed": False,
            "acceptance_met": False,
        }
    return scopes


def run(app, scope, path, phase, number, args):
    binary = args.rust_bin if app == "hahakigami" else args.opendisk_bin
    command = [str(binary), str(path)]
    command += [str(args.workers), "bulk"] if app == "hahakigami" else [args.opendisk_mode]
    print(f"{scope} {phase} {number}: {app} starting", flush=True)
    started = time.monotonic()
    record = {"app": app, "scope": scope, "path": str(path), "phase": phase, "round": number}
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=args.timeout)
        record["wall_ms"] = (time.monotonic() - started) * 1000
        record["exit_code"] = result.returncode
        record["success"] = result.returncode == 0
        if not record["success"]:
            record["error"] = result.stderr[-4000:]
        else:
            raw = json.loads(result.stdout.strip())
            record["raw"] = raw
            stats = raw["stats"] if app == "hahakigami" else raw
            for key in ["entries", "allocated_bytes", "logical_bytes", "errors", "elapsed_ms"]:
                record[key] = stats.get(key)
            record["peak_rss_bytes"] = raw.get("peak_rss_native_units") if app == "hahakigami" else raw["peak_rss_bytes"]
            if app == "hahakigami" and str(raw.get("state", "")).lower() != "completed":
                record["success"] = False
                record["error"] = f"Unexpected scan state: {raw.get('state')}"
    except (subprocess.TimeoutExpired, OSError, ValueError) as error:
        record.update(success=False, error=str(error), wall_ms=(time.monotonic() - started) * 1000)
    print(f"{scope} {phase} {number}: {app} "
          f"elapsed={record.get('elapsed_ms', '?')}ms entries={record.get('entries', '?')} "
          f"errors={record.get('errors', '?')} success={record['success']}", flush=True)
    return record


def command_output(*command):
    try:
        return subprocess.check_output(command, text=True, stderr=subprocess.DEVNULL).strip()
    except (OSError, subprocess.CalledProcessError):
        return None


def file_hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def scanner_fingerprints():
    files = sorted((ROOT / "crates/disk-scan/src").rglob("*.rs"))
    files += [ROOT / "crates/disk-scan/Cargo.toml", ROOT / "Cargo.lock"]
    return {str(path.relative_to(ROOT)): file_hash(path) for path in files}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scope", action="append", metavar="NAME=PATH", help="Default: home and startup")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--timeout", type=int, default=600, help="Maximum seconds per process")
    parser.add_argument("--opendisk-mode", choices=["engine", "traversal", "system-data"], default="engine")
    parser.add_argument("--rust-bin", type=Path, default=ROOT / "target/release/examples/scan")
    parser.add_argument("--opendisk-bin", type=Path, default=ROOT / "benchmark-results/.cache/opendisk-benchmark")
    parser.add_argument("--output", type=Path, default=ROOT / "benchmark-results/comparison.json")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("macOS is required; ru_maxrss is reported in bytes here")
    if args.rounds < 1:
        parser.error("At least one warm round is required")
    scopes = [value.split("=", 1) for value in (args.scope or [f"home={Path.home()}", "startup=/"])]
    if any(len(value) != 2 for value in scopes):
        parser.error("Use --scope NAME=PATH")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = {
        "recorded_at_utc": datetime.now(timezone.utc).isoformat(),
        "host": {"os": command_output("sw_vers", "-productVersion"), "architecture": platform.machine(),
                 "hardware": command_output("sysctl", "-n", "hw.model"),
                 "cpus": command_output("sysctl", "-n", "hw.logicalcpu"),
                 "memory_bytes": command_output("sysctl", "-n", "hw.memsize")},
        "rust_version": command_output("rustc", "--version"),
        "swift_version": command_output("swiftc", "--version"),
        "hahakigami_git_revision": command_output("git", "-C", str(ROOT), "rev-parse", "HEAD"),
        "hahakigami_worktree_modified": bool(command_output("git", "-C", str(ROOT), "status", "--porcelain")),
        "opendisk_ref": OPENDISK_REF, "opendisk_mode": args.opendisk_mode,
        "workers": args.workers, "rounds": args.rounds,
        "binary_sha256": {"hahakigami": file_hash(args.rust_bin), "opendisk": file_hash(args.opendisk_bin)},
        "scanner_source_sha256": scanner_fingerprints(),
        "permissions": "Inherited from the same shell; Full Disk Access not granted or changed by tooling.",
        "cache_policy": "Fresh processes and full scans; OpenDisk ScanCache shim always returns nil and save is a no-op. OS metadata caches are not flushed.",
        "limitations": ["Release-engine harness comparison, not an OpenDisk.app UI benchmark.",
                         "Matching entries/allocated totals/errors is necessary, not sufficient, to verify equivalent path coverage.",
                         "OpenDisk root ScanEngine merges additional sibling system volumes; Hahakigami excludes unrelated mounts.",
                         "Live filesystems may change between scans; no snapshot or cache purge is performed."],
        "runs": [], "scopes": {}, "acceptance_met": False,
    }
    def save():
        report["scopes"] = summarize(report["runs"])
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    save()
    for scope, path in scopes:
        for app in ["hahakigami", "opendisk"]:
            report["runs"].append(run(app, scope, path, "initial", 0, args))
        for number in range(1, args.rounds + 1):
            order = ["opendisk", "hahakigami"] if number % 2 else ["hahakigami", "opendisk"]
            for app in order:
                report["runs"].append(run(app, scope, path, "warm", number, args))
    save()
    print(json.dumps(report["scopes"], indent=2), flush=True)


if __name__ == "__main__":
    main()
