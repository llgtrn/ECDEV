"""Census of a dependency donor pinned to the version ECDEV resolved: structure, files, classification.

Usage: dependency_census.py census <donor_id> <repo url> <tag or commit> <license>
       dependency_census.py finalize <donor_id> <semantic matrix path>

`census` clones the pinned ref (untracked checkout), runs the structural census of census.py and
leaves semantic_review PENDING. `finalize` marks the review COMPLETE only when the matrix maps every
first-party source file to a reviewed prefix; reference parsers are retired, so files other than
Python stay PARSE_UNKNOWN and the status stays CENSUS_PARTIAL: the review is a reading, not a parse.
"""
import json, pathlib, sys
import census

YN = census.YN


def run_census(donor_id, url, ref, licence):
    checkout = YN / 'donors/checkouts' / donor_id
    if not checkout.exists():
        census.git('clone', '--no-checkout', url, str(checkout), timeout=1800)
        census.git('checkout', '--detach', ref, cwd=checkout)
    commit = census.git('rev-parse', 'HEAD', cwd=checkout)
    record = {'schema_version': 1, 'donor_id': donor_id, 'candidate_identifier': 'dependency:' + url.rstrip('/').removesuffix('.git'),
              'name': url.rstrip('/').split('/')[-1].removesuffix('.git'), 'repository_url': url.removesuffix('.git'),
              'resolved_remote_url': url, 'default_branch': None, 'commit_sha': commit,
              'commit_time': census.git('show', '-s', '--format=%cI', 'HEAD', cwd=checkout), 'retrieved_at': census.now(),
              'remote_status': 'VERIFIED_REMOTE', 'clone_status': 'PINNED_REF_CHECKOUT', 'donor_type': 'UNCLASSIFIED',
              'license': licence, 'license_paths': [], 'license_hashes': [], 'primary_languages': {}, 'repository_size': None,
              'file_count': 0, 'tree_status': 'NOT_STARTED', 'source_census_status': 'NOT_STARTED',
              'dependency_census_status': 'NOT_STARTED', 'test_census_status': 'NOT_STARTED', 'protocol_census_status': 'NOT_STARTED',
              'capability_census_status': 'NOT_STARTED', 'external_services': [], 'capabilities_total': 0, 'capabilities_verified': 0,
              'capabilities_documented_only': 0, 'capabilities_unknown': 0, 'absorption_status': 'NOT_STARTED',
              'oracle_status': 'NOT_STARTED', 'runtime_dependency': False, 'extinction_status': 'NOT_ELIGIBLE',
              'notes': ['pinned ref ' + ref], 'pinned_ref': ref}
    langs = {}
    for line in census.git('ls-files', cwd=checkout).split('\n'):
        lang = census.LANG.get(pathlib.Path(line).suffix)
        if lang:
            langs[lang] = langs.get(lang, 0) + 1
    record['primary_languages'] = langs
    record['file_count'] = len(census.git('ls-files', cwd=checkout).split('\n'))
    census.census(record, checkout)
    print(donor_id, commit, record['file_count'])


def finalize(donor_id, matrix_path):
    base = YN / 'donors/census' / donor_id
    summary = json.loads((base / 'summary.json').read_text(encoding='utf-8'))
    files = [json.loads(x) for x in (base / 'files.jsonl').read_text(encoding='utf-8').splitlines()]
    matrix = json.loads((census.ROOT / matrix_path).read_text(encoding='utf-8'))
    prefixes = [e['prefix'] for e in matrix['file_coverage']['entries']]
    hit = lambda p: any(p == x or (x.endswith('/') and p.startswith(x)) for x in prefixes)
    missing = [f['path'] for f in files if f['classification'] == 'FIRST_PARTY_SOURCE' and not hit(f['path'])]
    if missing:
        raise SystemExit('SEMANTIC_FILE_COVERAGE_INCOMPLETE ' + ', '.join(missing[:8]))
    summary.update(semantic_review='COMPLETE', semantic_matrix=matrix_path, parse_policy='REFERENCE_PARSERS_RETIRED_SOURCE_READ_BY_REVIEWER')
    census.dump(base / 'summary.json', summary)
    print(donor_id, 'COMPLETE', len([f for f in files if f['classification'] == 'FIRST_PARTY_SOURCE']), 'source files covered')


if __name__ == '__main__':
    cmd = sys.argv[1]
    if cmd == 'census':
        run_census(*sys.argv[2:6])
    elif cmd == 'finalize':
        finalize(*sys.argv[2:4])
