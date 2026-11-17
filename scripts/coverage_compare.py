#!/usr/bin/env python3
"""Bounded raw-path comparison; incomplete/truncated exports never prove equivalence."""
import base64
from contextlib import closing
import json
import sqlite3
import tempfile
from pathlib import Path


def compare(left, right):
    with tempfile.TemporaryDirectory(prefix='coverage-') as directory:
        with closing(sqlite3.connect(str(Path(directory) / 'paths.sqlite'))) as db:
            db.execute('PRAGMA cache_size=-8192')
            headers, issues, branches = [], [], []
            for side, path in enumerate((left, right)):
                db.execute(f'CREATE TABLE p{side}(path BLOB PRIMARY KEY, value TEXT)')
                header, footer, count, line_number = None, None, 0, 0
                excluded = {'count':0, 'path_samples_base64':[]}
                with open(path, encoding='utf-8') as stream:
                    while True:
                        line = stream.readline(1024 * 1024 + 1)
                        if not line:
                            break
                        line_number += 1
                        if len(line) > 1024 * 1024:
                            raise ValueError('Coverage record exceeds line budget')
                        row = json.loads(line)
                        if not isinstance(row, dict):
                            raise ValueError('Coverage record must be an object')
                        if footer is not None:
                            raise ValueError('Records follow coverage footer')
                        if row.get('type') == 'header':
                            if header is not None or line_number != 1 or row.get('schema_version') != 1:
                                raise ValueError('Exactly one supported leading header required')
                            header = row
                        elif row.get('type') == 'node':
                            if header is None:
                                raise ValueError('Missing coverage header')
                            raw = base64.b64decode(row['path_base64'], validate=True)
                            if raw.startswith(b'/') or b'\0' in raw or any(p in (b'.', b'..') for p in raw.split(b'/')):
                                raise ValueError('Coverage path must be normalized and root-relative')
                            if row.get('excluded') is True:
                                excluded['count'] += 1
                                if len(excluded['path_samples_base64']) < 20:
                                    excluded['path_samples_base64'].append(base64.b64encode(raw).decode())
                            fields = {key: row.get(key) for key in ('kind', 'logical_bytes', 'allocated_bytes', 'identity', 'summary')}
                            fields['size_scope'] = row.get('size_scope', header.get('size_scope'))
                            for key in ('logical_bytes', 'allocated_bytes'):
                                if fields[key] is not None and (type(fields[key]) is not int or fields[key] < 0):
                                    raise ValueError('Invalid recorded size')
                            try:
                                db.execute(f'INSERT INTO p{side} VALUES (?,?)', (raw, json.dumps(fields, sort_keys=True)))
                            except sqlite3.IntegrityError as error:
                                raise ValueError('Duplicate native path') from error
                            count += 1
                        elif row.get('type') == 'footer':
                            footer = row
                        else:
                            raise ValueError('Unknown coverage record')
                if header is None:
                    raise ValueError('Missing coverage header')
                root = db.execute(f'SELECT value FROM p{side} WHERE path=?', (b'',)).fetchone()
                if root is None or json.loads(root[0])['kind'] != 'directory':
                    issues.append(f'export {side}: directory root missing')
                declared = header.get('node_count')
                if type(declared) is not int or declared <= 0 or declared != count:
                    issues.append(f'export {side}: declared node count mismatch')
                if footer is None or footer.get('schema_version') != 1 or footer.get('complete') is not True or type(footer.get('node_count')) is not int or footer['node_count'] != count:
                    issues.append(f'export {side}: completed footer missing or count mismatch')
                headers.append(header)
                branches.append(excluded)
                if excluded['count'] or header.get('issue_count', 0) or header.get('excluded_count', 0):
                    issues.append(f'export {side}: recorded scan issues or excluded branches')
                db.commit()
            counts, samples = {}, {}
            for side in range(2):
                query = f'SELECT a.path FROM p{side} a LEFT JOIN p{1-side} b USING(path) WHERE b.path IS NULL'
                counts[f'only_{side}'] = db.execute(f'SELECT COUNT(*) FROM ({query})').fetchone()[0]
                samples[f'only_{side}'] = [base64.b64encode(row[0]).decode() for row in db.execute(query + ' LIMIT 20')]
            mismatches = {key: 0 for key in ('kind', 'logical_bytes', 'allocated_bytes', 'identity', 'summary', 'size_scope')}
            unknown, mismatch_samples = 0, []
            for raw, a, b in db.execute('SELECT a.path,a.value,b.value FROM p0 a JOIN p1 b USING(path)'):
                a, b = json.loads(a), json.loads(b)
                changed = []
                for key in mismatches:
                    if key == 'identity' and (a[key] is None or b[key] is None):
                        continue
                    if key in ('logical_bytes', 'allocated_bytes', 'kind', 'size_scope') and (a[key] is None or b[key] is None):
                        unknown += 1
                    if a[key] != b[key]:
                        mismatches[key] += 1
                        changed.append(key)
                if changed and len(mismatch_samples) < 20:
                    mismatch_samples.append({'path_base64':base64.b64encode(raw).decode(), 'fields':changed})
                if a['summary'] or b['summary']:
                    if 'summary boundary' not in issues:
                        issues.append('summary boundary')
            complete = all(h.get('complete') is True and not h.get('limitations') for h in headers) and not issues
            compatible_policy = all(headers[0].get(key) and headers[0].get(key) == headers[1].get(key) for key in ('accounting_policy', 'mount_policy', 'size_scope'))
            equivalent = bool(complete and compatible_policy and not unknown and not any(counts.values()) and not any(mismatches.values()))
            return {'schema_version':1, 'complete':complete, 'equivalent':equivalent, 'policy_compatible':bool(compatible_policy), 'excluded_branches':branches, 'missing_paths':counts, 'missing_path_samples_base64':samples, 'mismatches':mismatches, 'mismatch_samples':mismatch_samples, 'unknown_fields':unknown, 'headers':headers, 'limitations':issues+[x for h in headers for x in h.get('limitations', [])]}


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('left'); parser.add_argument('right')
    args = parser.parse_args()
    print(json.dumps(compare(args.left, args.right), indent=2))
