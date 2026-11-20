#!/usr/bin/env python3
"""Collect rotated worker diagnostics; phase wall time is not CPU utilization."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import platform
import subprocess
import time
from benchmark import ROOT, file_hash, rotated, scanner_fingerprints

PHASES = ('metadataUs', 'storeWaitUs', 'commitUs', 'progressUs')


def analysis(raw):
    rows = raw.get('worker_stats', [])
    stats = raw.get('stats', {})
    totals = {key: sum(row.get(key, 0) for row in rows) for key in ('entries', 'directories', 'errors', 'elapsedUs', 'busyUs', 'queueWaitUs', *PHASES)}
    cpu_available = bool(rows) and all(row.get('cpuUs') is not None for row in rows)
    cpu_us = sum(row['cpuUs'] for row in rows) if cpu_available else None
    elapsed_us = stats.get('elapsed_ms', 0) * 1000
    entry_match = bool(rows) and totals['entries'] == stats.get('entries', 0) - 1
    error_match = bool(rows) and totals['errors'] == stats.get('errors')
    worker_match = bool(rows) and len(rows) == raw.get('workers') and {row.get('id') for row in rows} == set(range(1, len(rows) + 1))
    open_counts_available = bool(rows) and all('relativeOpens' in row and 'absoluteReopens' in row for row in rows)
    relative_opens = sum(row['relativeOpens'] for row in rows) if open_counts_available else None
    absolute_reopens = sum(row['absoluteReopens'] for row in rows) if open_counts_available else None
    return {
        'worker_totals': totals, 'total_worker_cpu_us': cpu_us,
        'average_cpu_threads': cpu_us / elapsed_us if cpu_us is not None and elapsed_us > 0 else None,
        'phase_wall_fraction_of_worker_busy': {phase: totals[phase] / totals['busyUs'] if totals['busyUs'] > 0 else None for phase in PHASES},
        'accounted_processing_us': sum(totals[phase] for phase in PHASES),
        'finalization_us': raw.get('finalization_us'),
        'entries_excluding_root_match': entry_match, 'errors_match': error_match,
        'worker_count_and_unique_ids_match': worker_match,
        'relative_open_attempts': relative_opens, 'absolute_reopen_attempts': absolute_reopens,
        'descriptor_limit': raw.get('descriptor_limit'), 'descriptor_peak': raw.get('descriptor_peak'),
        'accounting_valid': entry_match and error_match and worker_match,
        'observed_qos_classes': sorted({row.get('qos', 'unavailable') for row in rows}),
        'interpretation': 'Phase fractions divide summed phase wall time by summed worker busy wall time; they are not CPU utilization. Queue wait is outside busy processing. Worker directory tasks need not equal snapshot directories. QoS samples do not identify P/E core assignment.',
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/examples/scan')
    parser.add_argument('--path', type=Path, default=Path('/'))
    parser.add_argument('--counts', type=int, nargs='+', default=[5, 7, 14])
    parser.add_argument('--rounds', type=int, default=1)
    parser.add_argument('--timeout', type=int, default=600)
    parser.add_argument('--output', type=Path, default=ROOT / 'benchmark-results/worker-diagnostics.json')
    args = parser.parse_args()
    if args.rounds < 1 or args.timeout < 1 or len(set(args.counts)) != len(args.counts) or any(n < 1 or n > 16 for n in args.counts):
        parser.error('Use positive rounds/timeout and unique counts from 1 through 16')
    if not args.path.is_absolute():
        parser.error('--path must be absolute')
    binary = args.binary.resolve()
    fingerprint = file_hash(binary)
    report = {'recorded_at_utc': datetime.now(timezone.utc).isoformat(), 'host': {'os': platform.platform(), 'architecture': platform.machine()},
              'binary': str(binary), 'binary_sha256': fingerprint, 'current_production_source_sha256': scanner_fingerprints(),
              'source_provenance': 'Current production source snapshot, not proof of measured binary provenance.',
              'path': str(args.path), 'counts': args.counts, 'rounds': args.rounds, 'backend': 'bulk',
              'permissions': 'Inherited from same shell; tooling does not change permissions or Full Disk Access.',
              'cache_policy': 'Fresh process/full scan; OS metadata caches are not flushed.',
              'timing_classification': 'Exploratory diagnostic samples; first rotation retained as initial; subsequent rotations are warm.' if args.rounds == 1 else 'Rotated exploratory diagnostics; initial rotation separated from warm rotations.',
              'limitations': ['Instrumentation overhead is not measured here.', 'No core affinity or P/E assignment is observed.', 'Live filesystems can change between scans.', 'Phase wall time is not CPU utilization.'],
              'runs': [], 'acceptance_met': False, 'app_level_comparison': False}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def save():
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    save()
    for round_number in range(args.rounds):
        order = rotated(args.counts, round_number)
        for count in order:
            record = {'round': round_number + 1, 'phase': 'initial' if round_number == 0 else 'warm', 'workers': count}
            started = time.monotonic()
            print(f'round {round_number + 1} workers {count}: starting', flush=True)
            try:
                record['binary_sha256_before'] = file_hash(binary)
                if record['binary_sha256_before'] != fingerprint:
                    raise ValueError('Binary changed before scan; evidence rejected')
                result = subprocess.run([str(binary), str(args.path), str(count), 'bulk'], capture_output=True, text=True, timeout=args.timeout)
                record.update(exit_code=result.returncode, stderr=result.stderr[-4000:])
                record['binary_sha256_after'] = file_hash(binary)
                if record['binary_sha256_after'] != fingerprint:
                    raise ValueError('Binary changed during scan; evidence rejected')
                if result.returncode:
                    raise ValueError(f'Scan process exited {result.returncode}')
                raw = json.loads(result.stdout)
                record['raw'] = raw
                if raw.get('workers') != count or str(raw.get('backend', '')).lower() != 'bulk' or str(raw.get('state', '')).lower() != 'completed':
                    raise ValueError('Unexpected workers, backend or scan state')
                record['analysis'] = analysis(raw)
                if not record['analysis']['accounting_valid']:
                    raise ValueError('Worker entry/error/count accounting mismatch')
                record['success'] = True
            except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
                record.update(success=False, error=str(error))
            record['wall_ms'] = (time.monotonic() - started) * 1000
            report['runs'].append(record)
            save()
            print(f"round {round_number + 1} workers {count}: success={record['success']} elapsed={record.get('raw', {}).get('stats', {}).get('elapsed_ms', '?')}ms avg_cpu_threads={record.get('analysis', {}).get('average_cpu_threads')}", flush=True)


if __name__ == '__main__':
    main()
