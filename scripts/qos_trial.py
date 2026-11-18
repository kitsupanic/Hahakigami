#!/usr/bin/env python3
"""Rotated same-binary QoS diagnostics; no app acceptance or core-placement claim."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import platform
import statistics
import subprocess
import time
from benchmark import ROOT, file_hash, scanner_fingerprints
from worker_diagnostics_trial import analysis

MODES = ('automatic', 'userInitiated', 'background')
COVERAGE = ('entries', 'directories', 'errors', 'allocated_bytes')


def rotated_order(number, modes=MODES):
    offset = number % len(modes)
    return modes[offset:] + modes[:offset]


def validate(raw, workers, mode):
    if str(raw.get('state', '')).lower() != 'completed':
        raise ValueError('Scan did not complete')
    if raw.get('workers') != workers or str(raw.get('backend', '')).lower() != 'bulk':
        raise ValueError('Unexpected workers or backend')
    if raw.get('scan_qos') != mode:
        raise ValueError('Missing or unexpected requested scan QoS')
    diagnostic = analysis(raw)
    if not diagnostic['accounting_valid']:
        raise ValueError('Worker accounting mismatch')
    for row in raw['worker_stats']:
        if 'qosError' not in row or row['qosError'] is not None:
            raise ValueError('Missing QoS application result or native QoS failure')
        if mode != 'automatic' and row.get('qos') != mode:
            raise ValueError('Explicit QoS class was not observed')
    return diagnostic


def summarize(runs, modes=MODES):
    medians = {}
    for mode in modes:
        values = [r['raw']['stats']['elapsed_ms'] for r in runs if r['mode'] == mode and r['phase'] == 'warm' and r.get('success')]
        medians[mode] = statistics.median(values) if values else None
    auto = medians.get('automatic')
    warm = [r for r in runs if r['phase'] == 'warm' and r['mode'] in modes]
    coverage = {tuple(r['raw']['stats'].get(key) for key in COVERAGE) for r in warm if r.get('success')}
    counts = {mode: sum(r['mode'] == mode for r in warm) for mode in modes}
    comparable = bool(warm) and len(set(counts.values())) == 1 and all(r.get('success') for r in warm) and len(coverage) == 1 and all(medians[m] is not None for m in modes)
    return {'warm_median_ms': medians,
            'percent_elapsed_change_vs_automatic': {m: (medians[m] / auto - 1) * 100 if medians[m] is not None and auto else None for m in modes},
            'warm_aggregate_coverage_stable': comparable,
            'initial_runs': [r for r in runs if r['phase'] == 'initial' and r['mode'] in modes],
            'acceptance_met': False, 'app_level_comparison': False}


def run(binary, fingerprint, path, workers, mode, number, timeout):
    record = {'mode': mode, 'workers': workers, 'round': number + 1, 'phase': 'initial' if number == 0 else 'warm'}
    started = time.monotonic()
    try:
        record['binary_sha256_before'] = file_hash(binary)
        if record['binary_sha256_before'] != fingerprint:
            raise ValueError('Binary changed before run; evidence rejected')
        result = subprocess.run([str(binary), str(path), str(workers), 'bulk', '--qos', mode], capture_output=True, text=True, timeout=timeout)
        record.update(exit_code=result.returncode, stderr=result.stderr[-4000:])
        record['binary_sha256_after'] = file_hash(binary)
        if record['binary_sha256_after'] != fingerprint:
            raise ValueError('Binary changed during run; evidence rejected')
        if result.returncode != 0:
            raise ValueError(f'Scan exited {result.returncode}')
        record['raw'] = json.loads(result.stdout)
        record['analysis'] = validate(record['raw'], workers, mode)
        record['success'] = True
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
        record.update(success=False, error=str(error))
    record['wall_ms'] = (time.monotonic() - started) * 1000
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/examples/scan')
    parser.add_argument('--workers', type=int, default=7)
    parser.add_argument('--modes', nargs='+', choices=MODES, default=list(MODES), help='Unique subset of QoS modes')
    parser.add_argument('--path', type=Path, default=Path('/'))
    parser.add_argument('--rounds', type=int, default=3, help='Initial rotation plus subsequent warm rotations')
    parser.add_argument('--timeout', type=int, default=600)
    parser.add_argument('--output', type=Path, default=ROOT / 'benchmark-results/qos-trial.json')
    args = parser.parse_args()
    if not args.path.is_absolute() or not 1 <= args.workers <= 16 or min(args.rounds, args.timeout) < 1:
        parser.error('Use absolute path, workers 1 through 16, and positive rounds/timeout')
    if len(set(args.modes)) != len(args.modes):
        parser.error('--modes must be unique')
    binary = args.binary.resolve()
    fingerprint = file_hash(binary)
    report = {'recorded_at_utc': datetime.now(timezone.utc).isoformat(), 'host': {'os': platform.platform(), 'architecture': platform.machine()},
              'binary': str(binary), 'binary_sha256': fingerprint, 'current_production_source_sha256': scanner_fingerprints(),
              'source_provenance': 'Current source fingerprints are not proof of binary provenance.', 'path': str(args.path), 'workers': args.workers,
              'rounds': args.rounds, 'backend': 'bulk', 'modes': args.modes,
              'permissions': 'Inherited unchanged; no permission grants, power settings or cache changes.',
              'cache_policy': 'Fresh processes/full scans; OS metadata caches are not flushed.',
              'limitations': ['Exploratory instrumented engine comparison, not app/OpenDisk acceptance.', 'QoS samples do not imply P/E core placement.', 'Live filesystems may drift; matching aggregates do not prove identical paths.', 'Initial rotation is excluded from warm medians.'],
              'runs': [], 'summary': {}, 'acceptance_met': False, 'app_level_comparison': False}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def save():
        report['summary'] = summarize(report['runs'], args.modes)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    save()
    for number in range(args.rounds):
        for mode in rotated_order(number, args.modes):
            print(f'round {number + 1}: {mode} starting', flush=True)
            record = run(binary, fingerprint, args.path, args.workers, mode, number, args.timeout)
            report['runs'].append(record)
            save()
            print(f"round {number + 1}: {mode} success={record['success']} elapsed={record.get('raw', {}).get('stats', {}).get('elapsed_ms', '?')}ms", flush=True)
    print(json.dumps({k: v for k, v in report['summary'].items() if k != 'initial_runs'}, indent=2), flush=True)


if __name__ == '__main__':
    main()
