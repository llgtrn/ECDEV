"""Verify the actual persisted live research chain; never inject candidates."""
import hashlib
import json
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class Scripts(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=False)
        self.scripts = []
        self.active = False

    def handle_starttag(self, tag, attrs):
        if tag == 'script' and dict(attrs).get('type') == 'application/ld+json':
            self.active = True
            self.scripts.append('')

    def handle_endtag(self, tag):
        if tag == 'script':
            self.active = False

    def handle_data(self, data):
        if self.active:
            self.scripts[-1] += data


def pointer(value, path):
    for part in path.split('/')[1:]:
        part = part.replace('~1', '/').replace('~0', '~')
        value = value[int(part)] if isinstance(value, list) else value[part]
    return value


def main():
    run = json.loads((ROOT / '.ynventa/materialized/live-shortlist.json').read_text(encoding='utf-8'))
    assert run['mode'] == 'LIVE' and run['cost_minor'] == 0
    assert run['known_network_calls'] > 0
    assert all(call['provider'] == 'native-web' and call['actual_cost_minor'] == 0 for call in run['provider_calls'])
    selected = [c for c in run['candidates'] if c['state'] == 'SHORTLISTED']
    assert selected and len(selected) == run['funnel']['shortlisted']
    captures, documents = [], {}
    recognized = 0
    for observation in run['observations']:
        assert observation['mode'] == 'LIVE'
        raw = (ROOT / '.ynventa/materialized/raw' / (observation['raw_hash'] + '.html')).read_bytes()
        snapshot = observation['normalized_value']
        assert hashlib.sha256(raw).hexdigest() == observation['raw_hash'] == snapshot['content_hash']
        scripts = Scripts()
        scripts.feed(raw.decode('utf-8'))
        documents[observation['external_source']] = scripts.scripts
        role = snapshot['page_metadata']['classification']['role']
        recognized += role == 'PRODUCT'
        captures.append(dict(page=observation['external_source'],raw_capture_sha256=observation['raw_hash'],role=role,products=len(snapshot['products'])))
    witnesses = []
    for candidate in selected:
        assert candidate['decision']['purpose'] == 'FURTHER_RESEARCH'
        assert candidate['decision']['missing_selection_evidence'] == []
        assert len(candidate['decision']['listing_origins']) >= 2
        assert candidate['product']['price_minor'] > 0
        assert candidate['economics_uncertainty']['profit_expected'] is None
        assert 'product_cost' in candidate['economics_uncertainty']['unknown_costs']
        identifiers = []
        for row in candidate['resolution']['observations']:
            for field in ['gtin', 'ean', 'upc', 'gtin8', 'gtin14']:
                for evidence in row['product']['fields'].get(field, {}).get('evidence', []):
                    if evidence['source'] == 'JSON_LD':
                        source = json.loads(documents[evidence['page']][evidence['script_index']])
                        assert pointer(source, evidence['json_pointer']) == evidence['value']
                        identifiers.append(evidence)
        assert len(identifiers) >= 2
        witnesses.append(dict(entity_id=candidate['entity_id'],resolution=candidate['resolution'],decision=candidate['decision'],competition=candidate['competition_evidence'],demand=candidate['demand_evidence'],economics_uncertainty=candidate['economics_uncertainty'],identifier_witnesses=identifiers))
    request = urllib.request.Request('http://127.0.0.1:8765/api/tools/ecdev.runs.replay',json.dumps({'run_id':run['run_id']}).encode(),{'Content-Type':'application/json'})
    with urllib.request.urlopen(request, timeout=30) as response:
        replay = json.load(response)
    assert replay['mode'] == 'REPLAY' and replay['network_calls'] == 0
    assert replay['funnel'] == run['funnel']
    assert replay['candidates'] == run['candidates']
    discovered = sum(run['frontier']['states'].values())
    record = dict(status='PASS_LIVE_PUBLIC_RESEARCH_CHAIN_TO_FURTHER_RESEARCH_SHORTLIST',goal_complete=False,parent_main='6b6175107eb625e6ad38385627fad73d2d3bfa8d',run_id=run['run_id'],mode=run['mode'],run_status=run['status'],discovered_urls=discovered,captures=captures,pages_captured=len(captures),product_pages_recognized=recognized,candidates_normalized=len(run['candidates']),funnel=run['funnel'],frontier=run['frontier'],known_network_calls=run['known_network_calls'],paid_provider_calls=0,paid_cost_minor=0,paid_providers=run['paid_providers'],completeness=run['completeness'],errors=run['errors'],observed_sample=run['observed_sample'],shortlist_witnesses=witnesses,replay=dict(run_id=replay['run_id'],network_calls=0,funnel_preserved=True,candidates_preserved=True),targets=dict(discovered_100=discovered>=100,product_pages_30=recognized>=30,candidates_20=len(run['candidates'])>=20),limitations=['Shortlist purpose is further research; profitability, demand forecast, supplier terms and regulatory validation remain incomplete.','Captured origin diversity does not verify independent publishers or total market coverage.','No donor lifecycle promotion follows from these native regressions.','Full donor contract absorption and canonical upstream registration still require work.'])
    (ROOT/'research/commerce/live-shortlist-proof.json').write_text(json.dumps(record,ensure_ascii=False,indent=2)+'\n',encoding='utf-8',newline='\n')
    print(json.dumps({k:record[k] for k in ['status','run_id','discovered_urls','pages_captured','product_pages_recognized','candidates_normalized','funnel','known_network_calls','paid_cost_minor','targets']},indent=2))


if __name__ == '__main__':
    main()
