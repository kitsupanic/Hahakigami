#!/usr/bin/env python3
"""Frozen full-scan comparison; allocation-model differences are not path loss."""
import argparse
import json
import math
from pathlib import Path
import statistics

from coverage_compare import compare
from benchmark import file_hash
from performance_suite import fingerprints, run, schedule


def normalize(result, layer):
    result = dict(result)
    raw = result.get('raw', {})
    if layer == 'native':
        stats = raw.get('stats', {})
        for key in ('elapsed_ms', 'entries', 'allocated_bytes', 'errors'):
            result[key] = stats.get(key)
        result['peak_rss_bytes'] = raw.get('peak_rss_native_units')
        result['workers'] = raw.get('workers')
        result['coverage_complete'] = not stats.get('incomplete', True) and stats.get('excluded', 0) == 0
    result['accounting'] = raw.get('accounting')
    elapsed = result.get('elapsed_ms')
    if not isinstance(elapsed, (int, float)) or not math.isfinite(elapsed) or elapsed <= 0:
        result.update(success=False, error='Missing positive measured scan timer')
    return result


def raw_coverage(left, right):
    result = compare(left, right)
    result['raw_fields_match'] = bool(result['complete'] and not result['unknown_fields']
        and not any(result['missing_paths'].values()) and not any(result['mismatches'].values()))
    result['interpretation'] = ('Native paths, identities, and raw own sizes only; attributed totals '
        'may change intentionally. This does not establish independent app path parity.')
    return result


def summarize(rows):
    summaries = []
    for scope, layer in sorted({(row['scope'], row['layer']) for row in rows}):
        entry = {'scope': scope, 'layer': layer}
        for role in sorted({row['role'] for row in rows}):
            warm = [row for row in rows if row['scope'] == scope and row['layer'] == layer
                and row['role'] == role and row['phase'] == 'warm']
            if len(warm) != 5 or not all(row.get('success') for row in warm):
                entry[role] = {'complete': False, 'successful_samples': sum(bool(row.get('success')) for row in warm)}
                continue
            entry[role] = {'complete': True, 'elapsed_ms': statistics.median(row['elapsed_ms'] for row in warm),
                'elapsed_samples_ms': [row['elapsed_ms'] for row in warm],
                'entries': [row['entries'] for row in warm], 'allocated_bytes': [row['allocated_bytes'] for row in warm],
                'accounting': [row.get('accounting') for row in warm]}
            memory = 'sampled_peak_aggregate_rss_bytes' if layer == 'app' else 'peak_rss_bytes'
            values = [row.get(memory) for row in warm]
            entry[role]['rss_bytes'] = statistics.median(values) if all(type(v) is int and v > 0 for v in values) else None
        if all(entry[role]['complete'] for role in ('baseline', 'candidate')):
            entry['time_ratio'] = entry['candidate']['elapsed_ms'] / entry['baseline']['elapsed_ms']
        if entry.get('reference', {}).get('complete') and entry['candidate']['complete']:
            entry['reference_time_ratio'] = entry['candidate']['elapsed_ms'] / entry['reference']['elapsed_ms']
        summaries.append(entry)
    return summaries


def validate(config, output):
    roles = {e['role'] for e in config['executables']}
    if roles not in ({'baseline', 'candidate'}, {'baseline', 'candidate', 'reference'}):
        raise ValueError('Baseline/candidate required, with optional original reference')
    if {(e['role'], e['layer']) for e in config['executables']} != {
        (role, layer) for role in roles for layer in ('native', 'app')
    } or len(config['executables']) != 2 * len(roles):
        raise ValueError('Exactly one executable per role and native/app layer required')
    if len(config['exporters']) != len(roles) or {e['role'] for e in config['exporters']} != roles:
        raise ValueError('One native exporter per role required')
    for root in config['scopes'].values():
        scope = Path(root)
        if not scope.is_absolute() or not scope.is_dir() or output.resolve().is_relative_to(scope.resolve()):
            raise ValueError('Existing absolute scopes required; output must be outside every scope')


def execute(config, output):
    validate(config, output)
    output.mkdir(parents=True, exist_ok=False)
    executables = config['executables']
    frozen = executables + config['exporters']
    helpers = [Path(__file__).resolve(), Path(__file__).with_name('performance_suite.py').resolve(),
        Path(__file__).with_name('coverage_compare.py').resolve()]
    def hashes():
        return fingerprints(frozen) | {str(path): file_hash(path) for path in helpers}
    before = hashes()
    report = {'config': config, 'complete': False, 'hashes_before': before, 'runs': [], 'exports': [],
        'limitations': ['Warm OS caches; no cache purge.', 'Five warm samples plus separate initial observations; no timed retries or exclusions.',
            'Native engine and app CPU-draw timers have distinct boundaries.', 'RSS is not physical footprint.',
            'Clone-aware totals intentionally differ; raw coverage is assessed separately.']}
    def save():
        (output / 'report.json').write_text(json.dumps(report, indent=2))
    save()
    coverage = {}
    for phase in ('before', 'after'):
        for scope, root in config['scopes'].items():
            for exporter in config['exporters']:
                tag = f'{phase}-{scope}-{exporter["role"]}'
                state = output / 'state' / tag; state.mkdir(parents=True)
                path = output / f'{tag}.ndjson'
                request = {'schema_version': 2, 'root': root, 'scope': scope, 'mode': 'full', 'state_dir': str(state), 'coverage_output': str(path)}
                result = run(exporter, request, 600)
                report['exports'].append({'phase': phase, 'scope': scope, 'role': exporter['role'], 'result': result})
                if result.get('success') and path.exists():
                    coverage[(phase, scope, exporter['role'])] = path
                save()
        if phase == 'before':
            for scope, root in config['scopes'].items():
                for layer in ('native', 'app'):
                    cells = [e for e in executables if e['layer'] == layer]
                    for sample_phase, number, ordered in schedule(cells, 5):
                        for executable in ordered:
                            tag = f'{scope}-{layer}-{executable["role"]}-{sample_phase}-{number}'
                            state = output / 'state' / tag; state.mkdir(parents=True)
                            request = {'schema_version': 2, 'root': root, 'scope': scope, 'mode': 'full', 'state_dir': str(state), 'coverage_output': None}
                            result = normalize(run(executable, request, 600), layer)
                            report['runs'].append({'scope': scope, 'layer': layer, 'role': executable['role'], 'phase': sample_phase, 'round': number, **result})
                            save(); print(tag, result.get('success'), result.get('elapsed_ms'), flush=True)
    report['coverage'] = {}
    for scope in config['scopes']:
        roles = sorted({e['role'] for e in executables})
        pairs = [(f'{role}_repeat', ('before', scope, role), ('after', scope, role)) for role in roles]
        pairs += [(f'{phase}_{role}_overlap', (phase, scope, role), (phase, scope, 'candidate'))
            for role in roles if role != 'candidate' for phase in ('before', 'after')]
        for label, a, b in pairs:
            report['coverage'][f'{scope}/{label}'] = raw_coverage(coverage[a], coverage[b]) if a in coverage and b in coverage else {'raw_fields_match': False, 'error': 'Missing successful export'}
    report.update(hashes_after=hashes(), complete=True,
        summaries=summarize(report['runs']))
    report['fingerprints_stable'] = report['hashes_before'] == report['hashes_after']
    save()
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    execute(json.loads(args.config.read_text()), args.output)
