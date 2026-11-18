#!/usr/bin/env python3
"""Alternate baseline/candidate full release-engine scans; no app acceptance claim."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import platform
import statistics
import subprocess
import time
from benchmark import ROOT, command_output, file_hash

COVERAGE_KEYS = ('entries', 'directories', 'errors', 'allocated_bytes')


def summarize(runs, rounds=3):
    scopes = {}
    for scope in dict.fromkeys(r['scope'] for r in runs):
        selected = [r for r in runs if r['scope'] == scope]
        warm = [r for r in selected if r['phase'] == 'warm']
        medians = {}
        for variant in ('baseline', 'candidate'):
            elapsed = [r['elapsed_ms'] for r in warm if r['variant'] == variant and r.get('success')]
            medians[variant] = statistics.median(elapsed) if elapsed else None
        pairs = []
        for number in range(1, rounds + 1):
            pair = [r for r in warm if r['round'] == number]
            valid = len(pair) == 2 and {r['variant'] for r in pair} == {'baseline', 'candidate'}
            valid = valid and all(r.get('success') and r.get('state') == 'completed' and r.get('errors') == 0 for r in pair)
            matching = valid and all(pair[0].get(k) is not None and pair[0].get(k) == pair[1].get(k) for k in COVERAGE_KEYS)
            pairs.append({'round': number, 'complete_error_free_pair': bool(valid), 'matching_aggregate_coverage': bool(matching)})
        ratio = medians['candidate'] / medians['baseline'] if medians['candidate'] is not None and medians['baseline'] else None
        matching = len(warm) == rounds * 2 and all(p['matching_aggregate_coverage'] for p in pairs)
        scopes[scope] = {
            'warm_median_ms': medians, 'candidate_over_baseline_ratio': ratio,
            'pair_checks': pairs, 'matching_aggregate_coverage': matching,
            'raw_speed_improvement': ratio is not None and ratio < 1,
            'equivalent_aggregate_speed_gate': matching and ratio is not None and ratio < 1,
            'initial_runs': [r for r in selected if r['phase'] == 'initial'],
            'max_peak_rss_bytes': {v: max((r['peak_rss_bytes'] for r in selected if r['variant'] == v and r.get('peak_rss_bytes') is not None), default=None) for v in ('baseline', 'candidate')},
            'path_set_coverage_verified': False, 'app_level_comparison': False, 'acceptance_met': False,
        }
    return scopes


def manifest_record(path):
    if path is None:
        return None
    contents = path.read_text()
    return {'path': str(path.resolve()), 'sha256': file_hash(path), 'contents': json.loads(contents)}


def requested_workers(args, variant):
    override = getattr(args, f'{variant}_workers', None)
    return args.workers if override is None else override


def run(variant, binary, fingerprint, scope, path, phase, number, args):
    workers = requested_workers(args, variant)
    record = {'variant': variant, 'scope': scope, 'path': path, 'phase': phase, 'round': number, 'requested_workers': workers}
    print(f'{scope} {phase} {number}: {variant} starting', flush=True)
    started = time.monotonic()
    try:
        record['binary_sha256_before'] = file_hash(binary)
        if record['binary_sha256_before'] != fingerprint:
            raise ValueError('Binary changed before scan; measurement rejected')
        result = subprocess.run([str(binary), path, str(workers), 'bulk'], capture_output=True, text=True, timeout=args.timeout)
        record.update(exit_code=result.returncode, stderr=result.stderr[-4000:])
        record['binary_sha256_after'] = file_hash(binary)
        if record['binary_sha256_after'] != fingerprint:
            raise ValueError('Binary changed during scan; measurement rejected')
        if result.returncode != 0:
            raise ValueError(f'Scanner exited {result.returncode}')
        raw = json.loads(result.stdout)
        record['raw'] = raw
        record['state'] = str(raw.get('state', '')).lower()
        for key in (*COVERAGE_KEYS, 'elapsed_ms'):
            record[key] = raw['stats'].get(key)
        record['peak_rss_bytes'] = raw.get('peak_rss_native_units')
        if raw.get('workers') != workers or str(raw.get('backend', '')).lower() != 'bulk':
            raise ValueError('Unexpected scanner backend or worker count')
        if record['state'] != 'completed':
            raise ValueError(f"Unexpected scan state: {record['state']}")
        if any(not isinstance(record[k], (int, float)) or isinstance(record[k], bool) or record[k] < 0 for k in (*COVERAGE_KEYS, 'elapsed_ms')):
            raise ValueError('Missing or invalid scanner statistics')
        record['success'] = True
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
        record.update(success=False, error=str(error))
    record['wall_ms'] = (time.monotonic() - started) * 1000
    print(f"{scope} {phase} {number}: {variant} elapsed={record.get('elapsed_ms', '?')}ms entries={record.get('entries', '?')} errors={record.get('errors', '?')} success={record['success']}", flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline-bin', type=Path, required=True)
    parser.add_argument('--candidate-bin', type=Path, required=True)
    parser.add_argument('--baseline-source-manifest', type=Path, required=True, help='JSON source fingerprint map captured with baseline build')
    parser.add_argument('--candidate-source-manifest', type=Path, help='JSON source fingerprint map captured with candidate build')
    parser.add_argument('--scope', action='append', metavar='NAME=PATH', help='Default: home and startup')
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--workers', type=int, default=8)
    parser.add_argument('--baseline-workers', type=int, help='Override shared workers for baseline')
    parser.add_argument('--candidate-workers', type=int, help='Override shared workers for candidate')
    parser.add_argument('--timeout', type=int, default=600)
    parser.add_argument('--output', type=Path, default=ROOT / 'benchmark-results/speed-trial.json')
    args = parser.parse_args()
    if platform.system() != 'Darwin':
        parser.error('macOS required; scanner native RSS units are bytes on macOS')
    if min(args.rounds, args.workers, args.timeout, requested_workers(args, 'baseline'), requested_workers(args, 'candidate')) < 1:
        parser.error('rounds, workers, and timeout must be positive')
    scopes = [value.split('=', 1) for value in (args.scope or [f'home={Path.home()}', 'startup=/'])]
    if any(len(s) != 2 or not s[0] or not Path(s[1]).is_absolute() for s in scopes) or len({s[0] for s in scopes}) != len(scopes):
        parser.error('Use unique --scope NAME=/absolute/path')
    binaries = {v: getattr(args, f'{v}_bin').resolve() for v in ('baseline', 'candidate')}
    fingerprints = {v: file_hash(p) for v, p in binaries.items()}
    report = {
        'recorded_at_utc': datetime.now(timezone.utc).isoformat(),
        'host': {'os': command_output('sw_vers', '-productVersion'), 'architecture': platform.machine(), 'hardware': command_output('sysctl', '-n', 'hw.model')},
        'binaries': {v: {'path': str(p), 'sha256': fingerprints[v]} for v, p in binaries.items()},
        'source_manifests': {v: manifest_record(getattr(args, f'{v}_source_manifest')) for v in binaries},
        'workers': args.workers, 'variant_workers': {v: requested_workers(args, v) for v in binaries}, 'backend': 'bulk', 'rounds': args.rounds,
        'permissions': 'Inherited from same shell; tooling does not change Full Disk Access or permissions.',
        'cache_policy': 'Fresh processes, full scans, no incremental reuse. OS metadata caches are not flushed.',
        'limitations': ['Engine comparison only, not app launch/UI or OpenDisk acceptance.', 'Matching aggregates do not prove identical path coverage.', 'Live files may change between runs.', 'Source manifests are supplied snapshots, not proof of binary provenance.'],
        'runs': [], 'scopes': {}, 'app_level_comparison': False, 'acceptance_met': False,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    def save():
        report['scopes'] = summarize(report['runs'], args.rounds)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
    save()
    for scope, path in scopes:
        for phase, number in [('initial', 0)] + [('warm', n) for n in range(1, args.rounds + 1)]:
            order = ('baseline', 'candidate') if number % 2 == 0 else ('candidate', 'baseline')
            for variant in order:
                report['runs'].append(run(variant, binaries[variant], fingerprints[variant], scope, path, phase, number, args))
                save()
    print(json.dumps({scope: {key: value for key, value in summary.items() if key != 'initial_runs'} for scope, summary in report['scopes'].items()}, indent=2), flush=True)


if __name__ == '__main__':
    main()
