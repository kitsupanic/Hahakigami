#!/usr/bin/env python3
"""Measure selected Rust bulk worker counts using rotated order and fresh processes."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import statistics
import subprocess
from benchmark import file_hash, rotated, scanner_fingerprints
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
    if args.rounds < 2:
        parser.error("At least two rounds are required (the first is a cold initial round)")
    if len(set(args.counts)) != len(args.counts) or any(count < 1 or count > 16 for count in args.counts):
        parser.error("--counts must contain unique counts from 1 through 16")
    fingerprint = file_hash(args.binary)
    supplied_source = manifest_record(args.source_manifest)
    source = None if supplied_source else scanner_fingerprints()
    runs = []
    workers = args.counts
    report = {"recorded_at_utc": datetime.now(timezone.utc).isoformat(), "path": str(args.path),
              "rounds": args.rounds, "backend": "bulk", "binary_sha256": fingerprint,
              "source_manifest": supplied_source, "production_source_sha256": source,
              "source_provenance": "Supplied build snapshot; not proof of binary provenance." if supplied_source else "Fallback current production source fingerprints; may differ from the measured binary and are not proof of binary provenance.", "counts": workers, "ordering": "rotated configured counts; OS cache not flushed; round 1 is the cold initial rotation, excluded from medians",
              "runs": runs}

    def save():
        warm = [run for run in runs if run["phase"] == "warm" and run["success"]]
        ok = [run for run in runs if run["success"]]
        medians = {str(count): statistics.median(run["raw"]["stats"]["elapsed_ms"] for run in warm if run["workers"] == count)
                   for count in workers if any(run["workers"] == count for run in warm)}
        coverage = {(run["raw"]["stats"]["entries"], run["raw"]["stats"]["allocated_bytes"], run["raw"]["stats"]["errors"]) for run in ok}
        complete = len(ok) == len(runs) and all(str(run["raw"]["state"]).lower() == "completed" for run in ok)
        stable = len(coverage) == 1
        report.update(median_ms=medians, coverage_totals_stable=stable,
                      fastest_workers_if_coverage_stable=min(medians, key=medians.get) if complete and stable and len(medians) == len(workers) else None)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")

    for number in range(args.rounds):
        for count in rotated(workers, number):
            before = file_hash(args.binary)
            if before != fingerprint:
                raise ValueError("Binary changed before scan; measurement rejected")
            record = {"round": number + 1, "phase": "initial" if number == 0 else "warm", "workers": count, "binary_sha256_before": before}
            try:
                raw = json.loads(subprocess.check_output([str(args.binary), str(args.path), str(count), "bulk"], text=True, timeout=600))
                raw["state"], [raw["stats"][key] for key in ("elapsed_ms", "entries", "allocated_bytes", "errors")]  # fail on a malformed report
                record.update(success=True, raw=raw)
            except (OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError, AttributeError) as error:
                record.update(success=False, error=str(error))
            record["binary_sha256_after"] = file_hash(args.binary)
            if record["binary_sha256_after"] != fingerprint:
                raise ValueError("Binary changed during scan; measurement rejected")
            if record["success"] and (raw.get("workers") != count or str(raw.get("backend", "")).lower() != "bulk"):
                raise ValueError("Unexpected backend or worker count; measurement rejected")
            runs.append(record)
            save()
            print(f"round {number+1} workers {count}: {record['raw']['stats']['elapsed_ms'] if record['success'] else 'failed: ' + record['error']}", flush=True)
    print(json.dumps({"median_ms": report["median_ms"], "fastest_workers_if_coverage_stable": report["fastest_workers_if_coverage_stable"]}, indent=2))


if __name__ == "__main__":
    main()
