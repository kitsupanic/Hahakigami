#!/usr/bin/env python3
"""Check hardware Auto through native commands using disposable preferences."""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import subprocess
import time
from benchmark import ROOT, file_hash, scanner_fingerprints


def verify(raw):
    scan, settings = raw['scan'], raw['settings']
    if scan['state'] != 'completed' or scan['workerMode'] != 'auto' or scan['workerSource'] != 'hardware':
        raise ValueError('Expected a completed hardware Auto scan')
    ceiling = 8 if settings['autoScope'] == 'volume' else 5
    if not 1 <= scan['workers'] <= min(settings['maxWorkers'], ceiling):
        raise ValueError('Auto worker count exceeds its scope or CPU cap')
    if raw['capturedWorkers'] != scan['workers']:
        raise ValueError('Running scan did not preserve its worker snapshot')
    automatic = raw['automaticAfterLegacyReload']
    if automatic['workerMode'] != 'auto' or automatic['qos'] != 'automatic' or automatic['resolvedWorkers'] != automatic['defaultWorkers']:
        raise ValueError('Retired preferences overrode production Auto')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--path', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'benchmark-results/workers-policy.json')
    parser.add_argument('--timeout', type=int, default=240)
    args = parser.parse_args()
    if not args.path.is_absolute() or args.timeout < 1:
        parser.error('Use an absolute path and positive timeout')
    binary, scope = args.binary.resolve(strict=True), args.path.resolve(strict=True)
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    raw_output = output.with_suffix('.native.json')
    fingerprint = file_hash(binary)
    env = os.environ.copy()
    env['HAHAKIGAMI_WORKERS_PATH'], env['HAHAKIGAMI_WORKERS_REPORT'] = str(scope), str(raw_output)
    report = {'recordedAtUtc': datetime.now(timezone.utc).isoformat(), 'binarySha256': fingerprint,
              'scope': str(scope), 'currentScannerSources': scanner_fingerprints(),
              'currentNativeSources': {str(p.relative_to(ROOT)): file_hash(p) for p in sorted((ROOT / 'src-tauri/src').glob('*.rs'))},
              'permissions': 'Inherited unchanged; no user preferences or macOS grants changed.',
              'method': 'One full hardware Auto scan through native test commands; production Auto check in disposable preferences.',
              'limitations': ['No alternating performance comparison or proof of the fastest count.', 'No P/E placement or energy trace.', 'Not a graphical app or OpenDisk acceptance run.'], 'acceptanceMet': False}
    raw_output.unlink(missing_ok=True)  # stale data must not satisfy a renamed or skipped test
    started = time.monotonic()
    result = subprocess.run([str(binary), '--ignored', '--exact', 'tests::automatic_workers_probe', '--nocapture', '--test-threads=1'], env=env, text=True, capture_output=True, timeout=args.timeout)
    report.update(wallSeconds=time.monotonic()-started, exitCode=result.returncode, binarySha256After=file_hash(binary))
    if fingerprint != report['binarySha256After']:
        raise ValueError('Executable changed; reject evidence')
    if not result.returncode and '1 passed' not in result.stdout:
        report['failure'] = 'Probe test did not run (renamed or filtered out); no evidence recorded.\n' + result.stdout[-2000:]
        result.returncode = report['exitCode'] = 1
    elif result.returncode:
        report['failure'] = (result.stderr + result.stdout)[-4000:]
    else:
        report['native'] = json.loads(raw_output.read_text())
        verify(report['native'])
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k:v for k,v in report.items() if k not in ('native','currentNativeSources','currentScannerSources')}, indent=2))
    if result.returncode:
        raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
