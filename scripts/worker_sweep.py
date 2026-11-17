#!/usr/bin/env python3
"""Measure selected Rust bulk worker counts using rotated order and fresh processes."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import statistics
import subprocess
from benchmark import file_hash, scanner_fingerprints
from speed_trial import manifest_record

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--counts", type=int, nargs="+", default=[1, 2, 4, 8], help="Unique worker counts from 1 through 16")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/examples/scan")
    parser.add_argument("--output", type=Path, default=ROOT / "benchmark-results/workers.json")
    parser.add_argument("--source-manifest", type=Path, help="JSON source fingerprint snapshot captured with this binary build")
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error("At least one round is required")
    if len(set(args.counts)) != len(args.counts) or any(count < 1 or count > 16 for count in args.counts):
        parser.error("--counts must contain unique counts from 1 through 16")
    fingerprint = file_hash(args.binary)
    supplied_source = manifest_record(args.source_manifest)
    source = None if supplied_source else scanner_fingerprints()
    runs = []
    workers = args.counts
    for number in range(args.rounds):
        order = workers[number % len(workers):] + workers[:number % len(workers)]
        for count in order:
            before = file_hash(args.binary)
            if before != fingerprint:
                raise ValueError("Binary changed before scan; measurement rejected")
            raw = json.loads(subprocess.check_output([str(args.binary), str(args.path), str(count), "bulk"], text=True, timeout=600))
            after = file_hash(args.binary)
            if after != fingerprint:
                raise ValueError("Binary changed during scan; measurement rejected")
            if raw.get("workers") != count or str(raw.get("backend", "")).lower() != "bulk":
                raise ValueError("Unexpected backend or worker count; measurement rejected")
            runs.append({"round": number + 1, "workers": count, "binary_sha256_before": before, "binary_sha256_after": after, "raw": raw})
            print(f"round {number+1} workers {count}: {raw['stats']['elapsed_ms']}ms", flush=True)
    medians = {str(count): statistics.median(run["raw"]["stats"]["elapsed_ms"] for run in runs if run["workers"] == count) for count in workers}
    coverage = {(run["raw"]["stats"]["entries"], run["raw"]["stats"]["allocated_bytes"], run["raw"]["stats"]["errors"]) for run in runs}
    completed = all(str(run["raw"]["state"]).lower() == "completed" for run in runs)
    fastest = min(workers, key=lambda count: medians[str(count)]) if completed and len(coverage) == 1 else None
    report = {"recorded_at_utc": datetime.now(timezone.utc).isoformat(), "path": str(args.path),
              "rounds": args.rounds, "backend": "bulk", "binary_sha256": fingerprint,
              "source_manifest": supplied_source, "production_source_sha256": source,
              "source_provenance": "Supplied build snapshot; not proof of binary provenance." if supplied_source else "Fallback current production source fingerprints; may differ from the measured binary and are not proof of binary provenance.", "counts": workers, "ordering": "rotated configured counts; OS cache not flushed",
              "runs": runs, "median_ms": medians, "coverage_totals_stable": len(coverage) == 1,
              "fastest_workers_if_coverage_stable": fastest}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"median_ms": medians, "fastest_workers_if_coverage_stable": fastest}, indent=2))


if __name__ == "__main__":
    main()
