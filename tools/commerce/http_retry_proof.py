"""Validate fresh HTTP failure accounting separately from synthetic retry timing."""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    run = json.loads((ROOT/'.ecdev-data/http-retry-live.json').read_text(encoding='utf-8'))
    assert run['mode'] == 'LIVE' and run['cost_minor'] == 0
    calls = run['provider_calls']
    assert all(c['provider'] in ['native-web', 'public-amazon'] and c['actual_cost_minor'] == 0 for c in calls)
    http_failures = [c['acquisition_failure'] for c in calls if c.get('acquisition_failure') and c['acquisition_failure']['http_status'] is not None]
    assert http_failures and all(f['request_count'] > 0 for f in http_failures)
    assert all(f['reason'] == f"HTTP_STATUS_{f['http_status']}" for f in http_failures)
    known = sum(c['request_count'] or 0 for c in calls)
    assert run['known_network_calls'] == known
    assert run['network_calls'] == (None if any(c['request_count'] is None for c in calls) else known)
    captures = []
    for observation in run['observations']:
        assert observation['mode'] == 'LIVE'
        digest = observation['raw_hash']
        assert hashlib.sha256((ROOT/'.ecdev-data/raw'/(digest+'.html')).read_bytes()).hexdigest() == digest
        captures.append({'source':observation['external_source'],'sha256':digest,'provider':observation['provider']})
    record = {'status':'PASS_FRESH_HTTP_RESPONSE_ACCOUNTING_AND_PUBLIC_FALLBACK','goal_complete':False,'parent_main':'a27e192a55b47973e1350aff5a45a42d6e0b9376','run_id':run['run_id'],'mode':run['mode'],'run_status':run['status'],'frontier':run['frontier'],'funnel':run['funnel'],'errors':run['errors'],'provider_calls':calls,'captures':captures,'known_network_calls':known,'total_network_calls':run['network_calls'],'paid_provider_calls':0,'paid_cost_minor':0,'live_retry_after_header_observed':any(f['retry_after_header'] is not None for f in http_failures),'timing_regressions':['Delay-seconds, IMF-fixdate, RFC850 and asctime; invalid calendar dates remain unknown.','Synthetic 429 and 503 responses reach persisted retry scheduling and request accounting.','Origin cooldown survives reopen; other origins remain available; stale workers cannot modify it.','Completing a stale-cache fallback can preserve origin cooldown atomically.'],'standard_reference':'https://www.rfc-editor.org/rfc/rfc9110.html#name-retry-after','limits':['Live response accounting is verified; no Retry-After header was observed in the captured live failure.','Retry timing is regression verified and is not presented as an observed live server delay.','Origin cooldown is scoped to the frontier run and original request origin; cross-run and redirected-origin coordination remain unproven.','Redirect-specific Retry-After deferral remains pending.','No full HTTP, Crawlee or extraction donor absorption is claimed.']}
    (ROOT/'research/commerce/http-retry-proof.json').write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n',encoding='utf-8',newline='\n')
    print(json.dumps({k:record[k] for k in ['status','run_id','funnel','known_network_calls','total_network_calls','paid_cost_minor','live_retry_after_header_observed']},indent=2))


if __name__ == '__main__':
    main()
