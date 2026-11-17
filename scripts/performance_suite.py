#!/usr/bin/env python3
"""Explicit, isolated performance runs with conservative coverage and acceptance gates.

Config: executables [{id,role,layer,path,args,protocol,source_paths}], scopes
{name:absolute_path}, modes [full,...], rounds (default 5), timeout_seconds.
config protocol receives --config JSON containing root/mode/state_dir and optional
coverage_output. Adapter args use {root}, {mode}, {state_dir}, {coverage_output}.
Children report JSON metrics; app reports additionally require rendered:true.
Optional acceptance_policy fixes the compact cohort: required_scopes fixture/github,
memory_scopes github, and incremental_scopes equal to required_scopes.
It requires both layers, all four modes, initial runs, actual reuse, cross-layer
coverage, <=1.05 timing, <=0.8 full RSS and <=0.65 retained-capacity ratios.
"""
import argparse
import ctypes
from datetime import datetime, timezone
import hashlib
import json
import math
import os
import platform
import re
from pathlib import Path
import signal
import statistics
import subprocess
import tempfile
import time
from coverage_compare import compare


def digest(path):
    value = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def fingerprints(executables):
    return {str(path): digest(path) for executable in executables for path in [executable['path'], *executable.get('source_paths', [])]}


def schedule(executables, rounds=5):
    if rounds < 1:
        raise ValueError('At least one warm round required')
    yield 'initial', 0, list(executables)
    for number in range(1, rounds + 1):
        offset = number % len(executables)
        yield 'warm', number, list(executables[offset:]) + list(executables[:offset])


def process_snapshot():
    """RSS and birth identity from ps; only descendant trees of owned PID used."""
    output = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,rss=,lstart='], text=True)
    result = {}
    for line in output.splitlines():
        columns = line.split(None, 3)
        if len(columns) == 4:
            result[int(columns[0])] = (int(columns[1]), int(columns[2]) * 1024, columns[3])
    return result


def owned_rss(snapshot, pid, birth, identities):
    if pid not in snapshot or snapshot[pid][2] != birth:
        return 0
    selected = {pid}
    changed = True
    while changed:
        changed = False
        for child, (parent, _, identity) in snapshot.items():
            if parent in selected and child not in selected:
                if child in identities and identities[child] != identity:
                    continue
                identities[child] = identity
                selected.add(child)
                changed = True
    return sum(snapshot[child][1] for child in selected)


def host_conditions():
    """Public NSProcessInfo observations, never a privacy-grant or protected-path probe."""
    if platform.system() != 'Darwin':
        return {'available': False, 'reason': 'NSProcessInfo requires macOS'}
    try:
        ctypes.CDLL('/System/Library/Frameworks/Foundation.framework/Foundation')
        objc = ctypes.CDLL('/usr/lib/libobjc.A.dylib')
        objc.objc_getClass.argtypes = [ctypes.c_char_p]
        objc.objc_getClass.restype = ctypes.c_void_p
        objc.sel_registerName.argtypes = [ctypes.c_char_p]
        objc.sel_registerName.restype = ctypes.c_void_p
        pointer = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p)(('objc_msgSend', objc))
        integer = ctypes.CFUNCTYPE(ctypes.c_long, ctypes.c_void_p, ctypes.c_void_p)(('objc_msgSend', objc))
        boolean = ctypes.CFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)(('objc_msgSend', objc))
        process = pointer(objc.objc_getClass(b'NSProcessInfo'), objc.sel_registerName(b'processInfo'))
        if not process:
            raise ValueError('NSProcessInfo unavailable')
        thermal = integer(process, objc.sel_registerName(b'thermalState'))
        return {'available': True, 'low_power_mode': boolean(process, objc.sel_registerName(b'isLowPowerModeEnabled')), 'thermal_state': {0:'nominal', 1:'fair', 2:'serious', 3:'critical'}.get(thermal, 'unknown'), 'source':'public NSProcessInfo at process boundary'}
    except (OSError, ValueError, AttributeError) as error:
        return {'available': False, 'reason': str(error)}


def command(executable, request, directory):
    if executable.get('protocol', 'config') == 'config':
        config_path = Path(directory) / 'request.json'
        config_path.write_text(json.dumps(request))
        return [executable['path'], *executable.get('args', []), '--config', str(config_path)]
    return [executable['path'], *[arg.format(**request) for arg in executable.get('args', ['{root}'])]]


def _run(executable, request, timeout):
    started = time.monotonic()
    record = {'success': False, 'memory_sampling_interval_ms': 100, 'memory_measure': 'sampled aggregate RSS; excludes unrelated helpers', 'host_conditions_before': host_conditions()}
    with tempfile.TemporaryDirectory(prefix='performance-command-') as directory:
        with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
            process = subprocess.Popen(command(executable, request, directory), stdout=stdout, stderr=stderr, start_new_session=True, cwd=directory, env={**os.environ, 'LLVM_PROFILE_FILE':str(Path(directory)/'profile-%p.profraw')})
            birth = None
            identities = {}
            peak = 0
            sampling_error = None
            while process.poll() is None:
                try:
                    snapshot = process_snapshot()
                    if birth is None and process.pid in snapshot:
                        birth = snapshot[process.pid][2]
                    peak = max(peak, owned_rss(snapshot, process.pid, birth, identities))
                except (OSError, subprocess.SubprocessError, ValueError) as error:
                    sampling_error = str(error)
                if time.monotonic() - started > timeout:
                    # Signal only our process group; no app-name or broad process matching.
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                    record['error'] = 'timeout'
                    break
                time.sleep(0.1)
            stdout.seek(0); stderr.seek(0)
            try:
                raw = json.loads(stdout.read())
                if not isinstance(raw, dict):
                    raise ValueError('Child report must be a JSON object')
                record.update(raw=raw, exit_code=process.returncode)
                record['success'] = record.get('error') != 'timeout' and process.returncode == 0 and raw.get('success', True) is True and str(raw.get('state', 'completed')).lower() == 'completed'
                for key in ('elapsed_ms', 'entries', 'logical_bytes', 'allocated_bytes', 'errors', 'peak_rss_bytes', 'workers', 'workers_measurement', 'rendered', 'render_completion', 'preparation', 'launch_ms', 'cache_mode', 'reused_entries', 'rescanned_directories', 'requested_mode', 'phases', 'power', 'thermal', 'scheduling', 'coverage_limits', 'coverage_limitations', 'accounting_policy', 'mount_policy', 'coverage_complete', 'fallback', 'measurements', 'storage', 'scratch', 'query_storage', 'intervened'):
                    record[key] = raw.get(key)
                # Optional candidate contract catches stale linked scanner dependencies.
                if executable.get('expect_exact_path_capacity') and request.get('mode') == 'full':
                    storage = raw.get('storage')
                    paths = storage.get('paths') if isinstance(storage, dict) else None
                    paths = paths if isinstance(paths, dict) else {}
                    length, capacity = paths.get('length_bytes'), paths.get('capacity_bytes')
                    record['source_contract_verified'] = type(length) is int and length > 0 and type(capacity) is int and length == capacity
                    if not record['source_contract_verified']:
                        record.update(success=False, error='Candidate path-capacity source contract not present in executable')
                if executable.get('expect_compact_tree'):
                    record['source_contract_verified'] = compact_storage_contract(raw.get('storage'))
                    if not record['source_contract_verified']:
                        record.update(success=False, error='Compact-tree source contract not present or malformed')
                if executable['layer'] == 'app' and raw.get('rendered') is not True:
                    record.update(success=False, error='No rendered completion callback')
                if executable.get('expect_canvas_geometry'):
                    record['canvas_geometry_verified'] = rendered_canvas_contract(raw.get('canvas_evidence'))
                    if not record['canvas_geometry_verified']:
                        record.update(success=False, error='Canvas geometry does not establish the expected rendered map')
            except (ValueError, TypeError) as error:
                record['error'] = str(error)
            record['stderr_tail'] = stderr.read()[-4000:].decode('utf-8', 'replace')
            record['sampled_peak_aggregate_rss_bytes'] = peak if birth and not sampling_error else None
            record['memory_sampling_error'] = sampling_error
            record['wall_ms'] = (time.monotonic() - started) * 1000
            record['host_conditions_after'] = host_conditions()
    return record


def run(executable, request, timeout):
    try:
        return _run(executable, request, timeout)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        return {'success': False, 'error': str(error), 'sampled_peak_aggregate_rss_bytes': None}


def validate_policy_shape(policy):
    required = {'required_scopes', 'memory_scopes', 'incremental_scopes'}
    if not isinstance(policy, dict) or set(policy) != required:
        raise ValueError('Acceptance policy requires exactly required_scopes, memory_scopes, incremental_scopes')
    for key in required:
        values = policy[key]
        if not isinstance(values, list) or not values or any(not isinstance(v, str) for v in values) or len(set(values)) != len(values):
            raise ValueError('Acceptance policy scope lists must be nonempty and unique')
    scopes = set(policy['required_scopes'])
    if scopes != {'fixture', 'github'} or set(policy['memory_scopes']) != {'github'} or set(policy['incremental_scopes']) != scopes:
        raise ValueError('Compact acceptance requires fixture/github scopes and github memory gates')


def acceptance_policy(config):
    """Explicit compact-tree cohort; omitted policy preserves legacy gates."""
    policy = config.get('acceptance_policy')
    if policy is None:
        return None
    validate_policy_shape(policy)
    scopes = set(policy['required_scopes'])
    if not scopes <= set(config['scopes']) or not {'full', 'same_process', 'persisted', 'changed'} <= set(config.get('modes', [])):
        raise ValueError('Required acceptance scopes and modes must be configured')
    pairs = {(e['role'], e['layer']) for e in config['executables']}
    if not {(role, layer) for role in ('baseline', 'candidate') for layer in ('native', 'app')} <= pairs:
        raise ValueError('Compact acceptance requires baseline/candidate in native/app layers')
    if config.get('rounds', 5) < 5:
        raise ValueError('Compact acceptance requires at least five warm rounds')
    return policy


def rendered_canvas_contract(value):
    """Reject collapsed/low-resolution positive maps; empty maps release backing."""
    if not isinstance(value, dict):
        return False
    count = value.get('expected_positive_nodes')
    width, height = value.get('backing_width'), value.get('backing_height')
    css_width, css_height, dpr = (value.get(k) for k in ('css_width', 'css_height', 'device_pixel_ratio'))
    if type(count) is not int or count < 0 or any(type(v) is not int or v < 0 for v in (width, height)):
        return False
    if any(type(v) not in (int, float) or not math.isfinite(v) or v < 0 for v in (css_width, css_height, dpr)) or dpr <= 0:
        return False
    if count == 0:
        return width == height == 0
    expected_width, expected_height = css_width * dpr, css_height * dpr
    if not math.isfinite(expected_width) or not math.isfinite(expected_height):
        return False
    return (min(width, height, css_width, css_height) > 0
        and abs(width - math.trunc(expected_width)) <= 1
        and abs(height - math.trunc(expected_height)) <= 1)


def compact_storage_contract(storage):
    """Reject stale layouts and malformed accounting; capacity is not RSS."""
    if not isinstance(storage, dict) or storage.get('schema_version') != 2 or storage.get('layout') != 'compact_native_v1':
        return False
    size = storage.get('node_size_bytes')
    if type(size) is not int or not 0 < size <= 80:
        return False
    buffers = {name: storage.get(name) for name in ('records', 'native_names', 'children', 'path_overrides', 'display_name_overrides', 'root', 'issues', 'workers')}
    geometry = storage.get('record_storage')
    if 'record_storage' in storage and geometry != 'chunked_4096_v1':
        return False
    if geometry == 'chunked_4096_v1':
        buffers['record_chunks'] = storage.get('record_chunks')
    elif 'record_chunks' in storage:
        return False
    for name in ('clones', 'clone_attributions', 'accounting'):
        if name in storage:
            buffers[name] = storage[name]
    total = 0
    lengths = 0
    for buffer in buffers.values():
        if not isinstance(buffer, dict):
            return False
        length, capacity = buffer.get('length_bytes'), buffer.get('capacity_bytes')
        if type(length) is not int or type(capacity) is not int or not 0 <= length <= capacity:
            return False
        total += capacity
        lengths += length
    return type(storage.get('retained_length_bytes')) is int and storage['retained_length_bytes'] == lengths and type(storage.get('retained_capacity_bytes')) is int and storage['retained_capacity_bytes'] == total


def gates(runs, coverage, hashes_stable, rounds=5, modes=('full',), policy=None):
    def finite_positive(value):
        return type(value) in (int, float) and math.isfinite(value) and value > 0
    results = {}
    if policy is not None:
        try:
            validate_policy_shape(policy)
        except ValueError as error:
            return {'acceptance_met': False, 'gates': {'policy': {'status': 'inconclusive', 'reasons': [str(error)]}}}
    if policy and not {'full', 'same_process', 'persisted', 'changed'} <= set(modes):
        return {'acceptance_met': False, 'gates': {'policy': {'status': 'inconclusive', 'reasons': ['required modes omitted']}}}
    for layer in ('native', 'app'):
        for scope in (policy['required_scopes'] if policy else ('home', 'startup', 'fixture')):
            for mode in modes:
                if mode == 'changed' and scope != 'fixture':
                    continue
                key = f'{layer}/{scope}/{mode}'
                selected = [r for r in runs if r['layer'] == layer and r['scope'] == scope and r['mode'] == mode and r['phase'] == 'warm']
                pairs = {role: [r for r in selected if r['role'] == role] for role in ('baseline', 'candidate')}
                memory = 'sampled_peak_aggregate_rss_bytes' if layer == 'app' else 'peak_rss_bytes'
                reason = []
                if rounds < 5:
                    reason.append('fewer than five warm rounds')
                if not hashes_stable:
                    reason.append('source or binary changed')
                if not coverage.get(f'{layer}/{scope}', {}).get('equivalent') or (policy and not coverage.get(f'{layer}/{scope}', {}).get('compact_cohort_verified')):
                    reason.append('complete stable path and accounting coverage unverified')
                for role, values in pairs.items():
                    initial = [r for r in runs if r.get('layer') == layer and r.get('scope') == scope and r.get('mode') == mode and r.get('phase') == 'initial' and r.get('role') == role]
                    if policy and (len(initial) != 1 or initial[0].get('round') != 0 or not initial[0].get('success') or initial[0].get('intervened') or initial[0].get('coverage_complete') is not True or not finite_positive(initial[0].get('elapsed_ms')) or not finite_positive(initial[0].get(memory)) or initial[0].get('requested_mode') != mode or initial[0].get('cache_mode') not in ('full', 'incremental') or (mode == 'full' and initial[0].get('cache_mode') != 'full') or (layer == 'app' and initial[0].get('rendered') is not True) or (mode == 'persisted' and initial[0].get('preparation_success') is not True)):
                        reason.append(f'{role} separate initial run missing or invalid')
                    if policy and scope in policy['incremental_scopes'] and mode in ('same_process', 'persisted') and any(r.get('cache_mode') != 'incremental' or not finite_positive(r.get('reused_entries')) for r in values + initial):
                        reason.append(f'{role} actual incremental reuse unverified')
                    if len(values) != rounds or {r['round'] for r in values} != set(range(1, rounds + 1)) or any(not r.get('success') or r.get('intervened') or r.get('coverage_complete') is not True or not finite_positive(r.get('elapsed_ms')) or not finite_positive(r.get(memory)) or r.get('requested_mode') != mode or r.get('cache_mode') not in ('full', 'incremental') or (mode == 'full' and r.get('cache_mode') != 'full') or (layer == 'app' and r.get('rendered') is not True) or (mode == 'persisted' and r.get('preparation_success') is not True) for r in values):
                        reason.append(f'{role} measurements incomplete')
                if policy:
                    evidence = {}
                    valid_scheduling = True
                    for role in ('baseline', 'candidate'):
                        cohort = [r for r in runs if r.get('layer') == layer and r.get('scope') == scope and r.get('mode') == mode and r.get('role') == role and r.get('phase') in ('initial', 'warm')]
                        evidence[role] = {}
                        for row in cohort:
                            scheduling = row.get('scheduling')
                            workers = row.get('workers')
                            valid_scheduling &= type(workers) is int and workers > 0 and isinstance(scheduling, dict) and bool(scheduling)
                            try:
                                encoded = json.dumps(scheduling, sort_keys=True, allow_nan=False)
                            except (ValueError, TypeError):
                                encoded = None
                                valid_scheduling = False
                            evidence[role][(row['phase'], row['round'])] = (workers, encoded)
                    if not valid_scheduling or evidence['baseline'] != evidence['candidate']:
                        reason.append('unchanged per-round worker scheduling unverified')
                    if mode == 'full' and scope in policy['memory_scopes']:
                        capacities = {role: [r.get('storage', {}).get('retained_capacity_bytes') if isinstance(r.get('storage'), dict) else None for r in values] for role, values in pairs.items()}
                        if any(not finite_positive(v) for values in capacities.values() for v in values):
                            reason.append('retained snapshot capacity unverified')
                if reason:
                    results[key] = {'status': 'inconclusive', 'reasons': reason}
                    continue
                medians = {role: {metric: statistics.median(r[metric] for r in values) for metric in ('elapsed_ms', memory)} for role, values in pairs.items()}
                time_ratio = medians['candidate']['elapsed_ms'] / medians['baseline']['elapsed_ms']
                memory_ratio = medians['candidate'][memory] / medians['baseline'][memory]
                passed = time_ratio <= 1.05 and (mode != 'full' or scope not in (policy['memory_scopes'] if policy else ('home', 'startup')) or memory_ratio <= 0.8)
                capacity_ratio = None
                if policy and mode == 'full' and scope in policy['memory_scopes']:
                    capacity_ratio = statistics.median(capacities['candidate']) / statistics.median(capacities['baseline'])
                    passed &= capacity_ratio <= 0.65
                results[key] = {'incremental_benefit_verified': mode != 'full' and all(r.get('cache_mode') == 'incremental' and finite_positive(r.get('reused_entries')) for r in selected), 'retained_capacity_ratio': capacity_ratio, 'status': 'pass' if passed else 'fail', 'time_ratio': time_ratio, 'memory_ratio': memory_ratio, 'medians': medians, 'actual_cache_modes': {role: sorted({str(r.get('cache_mode', 'unreported')) for r in values}) for role, values in pairs.items()}, 'fallbacks': {role: [r.get('fallback') for r in values if r.get('fallback')] for role, values in pairs.items()}, 'phase_units': 'microseconds; child reported', 'phase_timings': {role: [r.get('phases') for r in values] for role, values in pairs.items()}}
    return {'acceptance_met': bool(results) and all(r['status'] == 'pass' for r in results.values()), 'gates': results}


def validate(config):
    executables = config['executables']
    acceptance_policy(config)
    if 'full' not in config.get('modes', ['full']):
        raise ValueError('Full scans required for acceptance')
    if not executables or len({e['id'] for e in executables}) != len(executables):
        raise ValueError('Unique executable IDs required')
    for item in executables:
        if not re.fullmatch(r'[A-Za-z0-9_-]+', item['id']):
            raise ValueError('Executable ID must be a simple state-directory label')
        if not Path(item['path']).is_absolute() or not Path(item['path']).is_file():
            raise ValueError('Executable must be an existing absolute file')
        if item['role'] not in ('baseline', 'candidate', 'reference') or item['layer'] not in ('native', 'app'):
            raise ValueError('Unknown role or layer')
    pairs = [(e['role'], e['layer']) for e in executables if e['role'] != 'reference']
    if len(set(pairs)) != len(pairs):
        raise ValueError('Only one baseline/candidate per layer allowed')
    if any(not re.fullmatch(r'[A-Za-z0-9_-]+', name) for name in config['scopes']):
        raise ValueError('Scope must be a simple state-directory label')
    if type(config.get('rounds', 5)) is not int or not 1 <= config.get('rounds', 5) <= 20:
        raise ValueError('Warm rounds must be within 1..20')
    timeout = config.get('timeout_seconds', 600)
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or not 1 <= timeout <= 3600:
        raise ValueError('Timeout must be finite within 1..3600 seconds')
    if any(mode not in ('full', 'same_process', 'persisted', 'changed') for mode in config.get('modes', ['full'])):
        raise ValueError('Unknown requested measurement mode')
    if any(not Path(path).is_absolute() for path in config['scopes'].values()):
        raise ValueError('Absolute scope paths required')
    if 'changed' in config.get('modes', []) and 'fixture' not in config['scopes']:
        raise ValueError('Changed scans require an owned fixture')


def execute(config, checkpoint=None):
    validate(config)
    executables = config['executables']
    rounds = config.get('rounds', 5)
    policy = acceptance_policy(config)
    modes = config.get('modes', ['full'])
    measurement_modes = config.get('measurement_modes', modes)
    if not measurement_modes or any(mode not in modes for mode in measurement_modes):
        raise ValueError('Measured modes must be a nonempty subset of requested acceptance modes')
    coverage_scopes = config.get('coverage_scopes', list(config['scopes']))
    if any(scope not in config['scopes'] for scope in coverage_scopes):
        raise ValueError('Coverage scopes must be configured measurement scopes')
    before = fingerprints(executables)
    report = {'schema_version': 2, 'recorded_at_utc': datetime.now(timezone.utc).isoformat(), 'config': config, 'runner_sha256': digest(Path(__file__)), 'complete': False, 'acceptance_met': False, 'hashes_before': before, 'runs': [], 'coverage': {}, 'limitations': ['Live scopes may change between scans.', 'Shared or reparented helpers are not attributed.', 'No privacy grants or cache flushing performed.']}
    if set(coverage_scopes) != set(config['scopes']):
        report['limitations'].append('Coverage omitted for selected scopes; their acceptance gates remain inconclusive.')
    with tempfile.TemporaryDirectory(prefix='hahakigami-performance-state-') as temporary_path:
        temporary = Path(temporary_path).resolve()
        for scope, root in config['scopes'].items():
            for mode in measurement_modes:
                if mode == 'changed' and scope != 'fixture':
                    continue
                for phase, number, order in schedule(executables, rounds):
                    for executable in order:
                        if mode not in executable.get('modes', modes):
                            continue
                        state = Path(temporary) / executable['id'] / scope / mode
                        if mode == 'full':
                            state = state / f'full-{phase}-{number}'
                        state.mkdir(parents=True, exist_ok=True)
                        request = {'schema_version': 2, 'root': root, 'scope': scope, 'mode': mode, 'state_dir': str(state), 'coverage_output': None}
                        prepared = None
                        if mode == 'persisted':
                            seed_request = {**request, 'mode': executable.get('seed_mode', 'seed')}
                            prepared = run(executable, seed_request, config.get('timeout_seconds', 600))
                        record = run(executable, request, config.get('timeout_seconds', 600)) if prepared is None or prepared.get('success') else {'success':False, 'error':'Separate seed process failed'}
                        if prepared is not None:
                            record['preparation_success'] = prepared.get('success') is True
                            record['preparation'] = prepared
                        record.update(id=executable['id'], role=executable['role'], layer=executable['layer'], scope=scope, mode=mode, phase=phase, round=number)
                        report['runs'].append(record)
                        if checkpoint:
                            checkpoint(report)
                        print(f"{scope} {mode} {phase} {number} {executable['id']}: success={record.get('success')} elapsed_ms={record.get('elapsed_ms')} rss={record.get('peak_rss_bytes')}", flush=True)
        # Coverage never overlaps measured scans. Each side repeated to detect drift.
        for layer in ('native', 'app'):
            selected = {e['role']: e for e in executables if e['layer'] == layer and e['role'] in ('baseline', 'candidate')}
            if len(selected) != 2:
                continue
            for scope, root in config['scopes'].items():
                if scope not in coverage_scopes:
                    continue
                outputs = {}
                successful = True
                for role, executable in selected.items():
                    outputs[role] = []
                    for repeat in range(2):
                        output = Path(temporary) / f'{layer}-{scope}-{role}-{repeat}.ndjson'
                        request = {'schema_version': 2, 'root': root, 'scope': scope, 'mode': 'full', 'state_dir': str(Path(temporary) / 'coverage-state' / executable['id']), 'coverage_output': str(output)}
                        Path(request['state_dir']).mkdir(parents=True, exist_ok=True)
                        result = run(executable, request, config.get('timeout_seconds', 600))
                        successful &= result['success'] and output.is_file()
                        outputs[role].append(output)
                try:
                    comparison = compare(outputs['baseline'][0], outputs['candidate'][0]) if successful else {'equivalent': False, 'limitations': ['coverage export failed']}
                    if successful:
                        # Incomplete exports cannot establish stable complete coverage. Preserve
                        # the primary mismatch report without redundant failed-equivalence work.
                        stable = comparison.get('complete', False) and all(compare(paths[0], paths[1])['equivalent'] for paths in outputs.values())
                        comparison['stable'] = stable
                        comparison['stability_status'] = 'verified' if stable else ('not_verified_incomplete_exports' if not comparison.get('complete') else 'not_verified_drift_or_mismatch')
                        comparison['equivalent'] &= stable
                    report['coverage'][f'{layer}/{scope}'] = comparison
                    if config.get('export_external_coverage', False):
                        report.setdefault('external_coverage', {})
                        for reference in [e for e in executables if e['layer'] == layer and e['role'] == 'reference']:
                            output = Path(temporary) / f'external-{reference["id"]}-{scope}.ndjson'
                            request = {'schema_version':2,'root':root,'scope':scope,'mode':'full','state_dir':str(Path(temporary)/'external-coverage-state'/reference['id']), 'coverage_output':str(output)}
                            Path(request['state_dir']).mkdir(parents=True,exist_ok=True)
                            exporter = {**reference}
                            if exporter.get('protocol') == 'adapter':
                                exporter['args'] = [*exporter.get('args',['{root}']), '--coverage-output','{coverage_output}']
                            exported = run(exporter,request,config.get('timeout_seconds',600))
                            key=f'{layer}/{scope}/{reference["id"]}'
                            report['external_coverage'][key] = compare(outputs['baseline'][0],output) if exported.get('success') and output.is_file() else {'equivalent':False,'error':'External export failed','run':exported}

                except (ValueError, OSError) as error:
                    report['coverage'][f'{layer}/{scope}'] = {'equivalent': False, 'error': str(error)}
                if checkpoint:
                    checkpoint(report)
        if policy:
            # Each exported mode is compared to a common full-scan oracle across
            # both layers and roles, twice. Never certify a fallback as reuse.
            for scope in policy['required_scopes']:
                if scope not in coverage_scopes:
                    continue
                exports = []
                successful = True
                for executable in executables:
                    if executable['role'] not in ('baseline', 'candidate'):
                        continue
                    export_modes = ['full'] + (['same_process', 'persisted'] if scope in policy['incremental_scopes'] else [])
                    for mode in export_modes:
                        for repeat in range(2):
                            output = temporary / f"compact-{executable['id']}-{scope}-{mode}-{repeat}.ndjson"
                            state = temporary / 'compact-coverage-state' / executable['id'] / scope / mode / str(repeat)
                            state.mkdir(parents=True, exist_ok=True)
                            request = {'schema_version': 2, 'root': config['scopes'][scope], 'scope': scope, 'mode': mode, 'state_dir': str(state), 'coverage_output': str(output)}
                            prepared = run(executable, {**request, 'mode': executable.get('seed_mode', 'seed'), 'coverage_output': None}, config.get('timeout_seconds', 600)) if mode == 'persisted' else None
                            result = run(executable, request, config.get('timeout_seconds', 600)) if prepared is None or prepared.get('success') else {'success': False}
                            valid = result.get('success') is True and not result.get('intervened') and output.is_file() and result.get('coverage_complete') is True and result.get('requested_mode') == mode
                            valid &= result.get('cache_mode') == ('full' if mode == 'full' else 'incremental')
                            if mode != 'full':
                                reuse = result.get('reused_entries')
                                valid &= type(reuse) in (int, float) and math.isfinite(reuse) and reuse > 0
                            successful &= valid
                            exports.append(output)
                try:
                    verified = successful and bool(exports) and all(compare(exports[0], output).get('equivalent') is True for output in exports)
                except (ValueError, OSError):
                    verified = False
                for layer in ('native', 'app'):
                    entry = report['coverage'].setdefault(f'{layer}/{scope}', {'equivalent': False})
                    entry['compact_cohort_verified'] = verified
                    entry['compact_export_count'] = len(exports)
                    entry['equivalent'] &= verified
    report.setdefault('external_coverage', {})
    for name, paths in config.get('external_coverage', {}).items():
        try:
            report['external_coverage'][name] = compare(paths['left'], paths['right'])
        except (ValueError, OSError) as error:
            report['external_coverage'][name] = {'equivalent': False, 'error': str(error)}
    report['hashes_after'] = fingerprints(executables)
    report.update(gates(report['runs'], report['coverage'], before == report['hashes_after'], rounds, modes, policy))
    report['complete'] = True
    return report


def write_report(path, report):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + '.tmp')
    temporary.write_text(json.dumps(report, indent=2) + '\n')
    temporary.replace(path)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = execute(json.loads(args.config.read_text()), lambda report: write_report(args.output, report))
    write_report(args.output, report)
    print(json.dumps({'acceptance_met': report['acceptance_met'], 'gates': report['gates']}, indent=2))
