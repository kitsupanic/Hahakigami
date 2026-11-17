#!/usr/bin/env python3
"""Compare real engines on a disposable deterministic metadata fixture."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import tempfile
from benchmark import ROOT, file_hash, scanner_fingerprints


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "benchmark-results/fixture.json")
    args = parser.parse_args()
    cache = ROOT / "benchmark-results" / ".cache"
    cache.mkdir(parents=True, exist_ok=True)
    binaries = {"hahakigami": ROOT / "target/release/examples/scan", "opendisk": cache / "opendisk-benchmark"}
    fingerprints = {app: file_hash(binary) for app, binary in binaries.items()}
    cases = {}
    # Only this script's newly-created test directories are removed on exit.
    with tempfile.TemporaryDirectory(prefix="inventory-fixture-", dir=cache) as temporary:
        fixture = Path(temporary)
        for case in ["regular", "symlinks", "hardlinks", "unusual_names", "sparse"]:
            directory = fixture / case
            directory.mkdir()
            if case == "regular":
                (directory / "empty").touch()
                (directory / "nested").mkdir()
                (directory / "nested" / "allocated.bin").write_bytes(b"x" * 8192)
            elif case == "symlinks":
                (directory / "ordinary").touch()
                (directory / "link").symlink_to("ordinary")
                (directory / "loop").symlink_to(".")
                (directory / "broken").symlink_to("missing")
            elif case == "hardlinks":
                (directory / "original").write_bytes(b"x" * 8192)
                os.link(directory / "original", directory / "alias")
                (directory / "nested").mkdir()
                os.link(directory / "original", directory / "nested" / "alias-again")
            elif case == "unusual_names":
                for name in ["日本語.dat", "a\nb", "with space", "emoji-🌿"]:
                    (directory / name).touch()
            else:
                with (directory / "sparse").open("wb") as file:
                    file.truncate(1024 * 1024 * 1024)
            raw = {}
            for app, binary in binaries.items():
                command = [str(binary), str(directory)]
                command += ["8", "bulk"] if app == "hahakigami" else ["engine"]
                raw[app] = json.loads(subprocess.check_output(command, text=True, timeout=60))
            rust, reference = raw["hahakigami"]["stats"], raw["opendisk"]
            cases[case] = {"raw": raw, "entries_match": rust["entries"] == reference["entries"],
                           "allocated_match": rust["allocated_bytes"] == reference["allocated_bytes"],
                           "issues_match": rust["errors"] == reference["errors"]}
    report = {"recorded_at_utc": datetime.now(timezone.utc).isoformat(),
              "description": "Both engines scan identical freshly-created disposable fixtures. File data is created for allocation tests, never read by either scanner.",
              "binary_sha256": fingerprints, "scanner_source_sha256": scanner_fingerprints(),
              "cases": cases, "all_fixture_totals_match": all(c["entries_match"] and c["allocated_match"] and c["issues_match"] for c in cases.values()),
              "acceptance_met": False}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({case: {key: value for key, value in result.items() if key != "raw"} for case, result in cases.items()}, indent=2))


if __name__ == "__main__":
    main()
