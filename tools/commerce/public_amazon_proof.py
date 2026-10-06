"""Record actual public Amazon availability and separate provider identities."""
import hashlib
import json
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    run = json.loads((ROOT/'.ecdev-data/public-amazon-live.json').read_text(encoding='utf-8'))
    assert run['mode'] == 'LIVE' and run['cost_minor'] == 0
    assert any(c['provider'] == 'public-amazon' for c in run['provider_calls'])
    assert all(c['provider'] in ['public-amazon', 'native-web'] and c['actual_cost_minor'] == 0 for c in run['provider_calls'])
    selected = [c for c in run['candidates'] if c['state'] == 'SHORTLISTED']
    assert selected and all(c['economics_uncertainty']['profit_expected'] is None for c in selected)
    captures = []
    for evidence in run['observations']:
        assert evidence['mode'] == 'LIVE'
        raw = (ROOT/'.ecdev-data/raw'/(evidence['raw_hash']+'.html')).read_bytes()
        assert hashlib.sha256(raw).hexdigest() == evidence['raw_hash'] == evidence['normalized_value']['content_hash']
        captures.append(dict(provider=evidence['provider'],source=evidence['external_source'],requested_url=evidence['normalized_value']['requested_url'],final_url=evidence['normalized_value']['final_url'],raw_capture_sha256=evidence['raw_hash'],products=len(evidence['normalized_value']['products'])))
    with urllib.request.urlopen('http://127.0.0.1:8765/api/providers', timeout=30) as response:
        profiles = {p['id']:p for p in json.load(response)}
    for provider, layer in [('public-amazon','PUBLIC_AMAZON'),('amazon-sp-api','OFFICIAL_SP_API'),('keepa','KEEPA')]:
        assert profiles[provider]['source_layer'] == layer
    record = dict(status='PASS_PROVIDER_SEPARATION_AND_LIVE_PUBLIC_FALLBACK',goal_complete=False,parent_main='5a447d0095684c846ebc6213472afd51d94e9296',run_id=run['run_id'],mode=run['mode'],run_status=run['status'],providers={k:profiles[k] for k in ['public-amazon','amazon-sp-api','keepa']},provider_calls=run['provider_calls'],errors=run['errors'],frontier=run['frontier'],captures=captures,funnel=run['funnel'],known_network_calls=run['known_network_calls'],total_network_calls=run['network_calls'],paid_provider_calls=0,paid_cost_minor=0,public_amazon_product_captures=sum(c['products'] for c in captures if c['provider']=='public-amazon'),native_scope=['Robots-enforced public Amazon documents on JP/US hosts with scoped redirects.','Single-product detail URL ASIN assertion, offer/price/seller/availability/variation/category/brand/review projections; missing data remains unknown.','Native CAPTCHA-form regression proves blocking with a retailer fallback.','Cache keys include provider identity; recovered captures cannot be reassigned across providers.','Extraction and relative links use the final response URL; requested URL remains inspectable.'],limitations=['The live Amazon attempt returned HTTP 503. This proves unavailability, not a permanent block or successful public-product extraction.','Public ASIN and commerce mapping is fixture-regression verified; live Amazon product mapping was not observed.','SP-API adapter remains unimplemented and unavailable; credential status comes from the actual local registry. No official requests were made.','The failed Amazon acquisition did not expose a request count; total requests remain unknown.','Final-URL handling is implemented; an actual cross-URL redirect was not observed in this validation.','No full donor absorption or extinction claim follows from provider-boundary regressions.'])
    (ROOT/'research/commerce/public-amazon-proof.json').write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n',encoding='utf-8',newline='\n')
    print(json.dumps({k:record[k] for k in ['status','run_id','errors','funnel','known_network_calls','total_network_calls','paid_cost_minor','public_amazon_product_captures']},indent=2))


if __name__ == '__main__':
    main()
