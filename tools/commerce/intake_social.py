"""Exact-identity social donor intake; full Git census, never a runtime dependency."""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / '.venv/research'))
import census

DONORS = ['666ghj/BettaFish', '666ghj/MiroFish', 'NanmiCoder/MediaCrawler',
          'SANSAN0/TrendRadar', 'VladUZH/harken', 'Shiva-74/viral-trend-agent',
          't4niha/Trend-Finder']


def verify(identity):
    result = subprocess.run(['gh', 'api', 'repos/' + identity],
                            capture_output=True, timeout=90)
    row = {'requested_identity': identity, 'status': 'SOURCE_NOT_VERIFIED'}
    if result.returncode:
        row['reason'] = result.stderr.decode('utf-8', 'replace')[-1500:]
        return row
    data = json.loads(result.stdout)
    if data['full_name'].casefold() != identity.casefold() or data.get('private'):
        row['resolved_identity'] = data.get('full_name')
        row['reason'] = 'Exact owner/repository identity did not match a public repository'
        return row
    remote = census.git('ls-remote', '--symref', data['clone_url'], 'HEAD', timeout=90)
    import re
    branch = re.search(r'ref: refs/heads/(.+)\s+HEAD', remote)
    head = re.search(r'([0-9a-f]{40})\s+HEAD', remote)
    if not branch or not head or branch[1] != data['default_branch']:
        row['reason'] = 'Git remote HEAD/default branch not verified'
        return row
    row.update(status='VERIFIED_REMOTE', resolved_identity=data['full_name'],
               repository_url=data['html_url'], clone_url=data['clone_url'],
               default_branch=branch[1], remote_head_at_verification=head[1],
               git_remote_evidence=remote, verified_at=census.now(),
               github_license_hint=data.get('license'),
               license_gate='REQUIRES_LOCKED_LICENSE_SOURCE_REVIEW')
    return row


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--verify-only', action='store_true')
    args = parser.parse_args()
    registry_path = census.YN / 'donors/registry.json'
    initial = registry_path.read_bytes()
    with ThreadPoolExecutor(max_workers=3) as pool:
        identities = list(pool.map(verify, DONORS))
    report = {'schema_version': 1, 'verified_at': census.now(), 'donors': identities,
              'substitution_allowed': False, 'runtime_dependencies_added': 0}
    census.dump(census.YN / 'social-donor-intake.json', report)
    for row in identities:
        print(row['requested_identity'], row['status'],
              row.get('remote_head_at_verification', row.get('reason', '')), flush=True)
    if args.verify_only:
        return
    verified = [r for r in identities if r['status'] == 'VERIFIED_REMOTE']
    with ThreadPoolExecutor(max_workers=3) as pool:
        records = list(pool.map(census.acquire, [r['resolved_identity'] for r in verified]))
    evidence = {r['resolved_identity']: r for r in verified}
    for record in records:
        identity = record['candidate_identifier'].removeprefix('github:')
        record['social_identity_verification'] = evidence[identity]
        if record['commit_sha'] != evidence[identity]['remote_head_at_verification']:
            record['notes'].append('Remote moved during intake; exact cloned commit remains recorded, requires review')
        record['license_absorption_gate'] = 'PENDING_SOURCE_REVIEW_NO_CODE_COPIED'
        census.dump(census.YN / 'donors/census' / record['donor_id'] / 'identity.json', record)
    if hashlib.sha256(registry_path.read_bytes()).digest() != hashlib.sha256(initial).digest():
        raise RuntimeError('Concurrent registry edit: preserving existing registry; intake records remain on disk')
    registry = json.loads(initial)
    existing = {r['donor_id'] for r in registry['donors']}
    registry['donors'].extend(r for r in records if r['donor_id'] not in existing)
    registry['generated_at'] = census.now()
    census.dump(registry_path, registry)
    report['records'] = [{'donor_id': r['donor_id'], 'commit_sha': r['commit_sha'],
                          'clone_status': r['clone_status'], 'file_count': r['file_count']}
                         for r in records]
    census.dump(census.YN / 'social-donor-intake.json', report)


if __name__ == '__main__':
    main()
