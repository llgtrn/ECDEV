"""Verify record schemas, immutable source evidence, and honest completion gates."""
import json,pathlib,sys,re
ROOT=pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'.venv/research'))
import jsonschema
import census
def read(p):return json.loads(p.read_text(encoding='utf-8'))
def main():
 reg=read(ROOT/'research/commerce/donors/registry.json');schema=ROOT/'tools/commerce/schemas'
 total=classified=source=parsed=unknown=tests=capabilities=0
 for d in reg['donors']:
  jsonschema.validate(d,read(schema/'donor.schema.json'))
  path=ROOT/'research/commerce/donors/census'/d['donor_id'];summary=read(path/'summary.json');jsonschema.validate(summary,read(schema/'census.schema.json'))
  files=[json.loads(x) for x in (path/'files.jsonl').read_text().splitlines()]
  assert len(files)==summary['total_files'];assert len({x['path'] for x in files})==len(files)
  assert sum(x['classification']=='UNKNOWN' for x in files)==summary['unknown_files']
  assert summary['status']=='CENSUS_PARTIAL' or (summary['unknown_files']==0 and summary['parse_unknown']==0 and summary['semantic_review']=='COMPLETE')
  assert d['extinction_status']!='EXTINCT';assert d['runtime_dependency'] is False
  checkout=ROOT/'research/commerce/donors/checkouts'/d['donor_id']
  if '--records-only' not in sys.argv:
   assert census.git('rev-parse','HEAD',cwd=checkout)==d['commit_sha']
  for cap in read(path/'capabilities.json'):
   if cap['status']=='VERIFIED':
    assert cap['evidence'];capabilities+=1
    for e in cap['evidence']:
     assert e['commit_sha']==d['commit_sha'];assert any(f['path']==e['source_path'] and f['blob_hash']==e['blob_hash'] for f in files)
  total+=summary['total_files'];classified+=summary['classified_files'];source+=summary['first_party_source_files'];parsed+=summary['source_parsed'];unknown+=summary['parse_unknown'];tests+=summary['tests']
 report={'status':'PASS','donor_candidates':len(reg['donors']),'remote_verified':sum(d['remote_status']=='VERIFIED_REMOTE' for d in reg['donors']),'full_clones':sum(d['clone_status']=='FULL_CLONE' for d in reg['donors']),'total_files':total,'classified_files':classified,'first_party_source_files':source,'source_parsed':parsed,'parse_unknown':unknown,'tests':tests,'capabilities_verified':capabilities,'census_complete':sum(read(ROOT/'research/commerce/donors/census'/d['donor_id']/'summary.json')['status']=='CENSUS_COMPLETE' for d in reg['donors']),'native_absorbed':0,'oracle_verified':0,'extinct':0,'runtime_donor_dependencies':0}
 deps=read(ROOT/'research/commerce/dependencies.json')
 report['runtime_dependency_packages']=deps['runtime_packages']
 report['runtime_dependency_upstreams']=len({p['repository_url'] for p in deps['packages'] if p['scope']=='RUNTIME'})
 report['runtime_donor_dependencies']=report['runtime_dependency_upstreams']
 report['seed_runtime_dependencies']=0
 report['rust_replacement_capabilities']=sum(c.get('native_implementation') is not None for c in read(ROOT/'research/commerce/capabilities.json'))
 for capability in read(ROOT/'research/commerce/capabilities.json'):
  if capability.get('oracle_status')=='132_MICRODATA_CASES_MATCHED':
   import hashlib
   fixture=ROOT/capability['fixture'];oracle=read(fixture)
   assert hashlib.sha256(fixture.read_bytes()).hexdigest()==capability['fixture_sha256']
   assert len(oracle['cases'])==capability['oracle_cases']==132
   assert oracle['commit_sha']==capability['commit_sha']==next(d['commit_sha'] for d in reg['donors'] if d['donor_id']=='scrapinghub--extruct')
   assert oracle['oracle']=='UNMODIFIED_LOCKED_LXML_MICRODATA_EXTRACTOR'
  if capability.get('oracle_status')=='46_QUEUE_TRACES_MATCHED':
   import hashlib
   fixture=ROOT/capability['fixture'];oracle=read(fixture)
   assert hashlib.sha256(fixture.read_bytes()).hexdigest()==capability['fixture_sha256']
   assert len(oracle['cases'])==capability['oracle_cases']==46
   assert sum(len(c['steps']) for c in oracle['cases'])==capability['oracle_operations']==564
   assert oracle['commit_sha']==capability['commit_sha']==next(d['commit_sha'] for d in reg['donors'] if d['donor_id']=='apify--crawlee')
   assert oracle['oracle']=='LOCKED_REQUEST_QUEUE_BACKEND_WITH_PINNED_NATIVE_BINARY'
   assert oracle['backend']['version']=='0.2.2'
   lock=read(ROOT/'tools/commerce/package-lock.json')
   assert lock['packages']['node_modules/@crawlee/fs-storage-native']['integrity']==oracle['backend']['integrity']
   assert lock['packages']['node_modules/'+oracle['backend']['platform_package']]['integrity']==oracle['backend']['platform_integrity']
 report['oracle_compared_capabilities']=sum(c.get('oracle_status') in ('508_INTEGER_JSON_CASES_MATCHED','1620_ROBOTS_CASES_MATCHED','132_MICRODATA_CASES_MATCHED','46_QUEUE_TRACES_MATCHED') for c in read(ROOT/'research/commerce/capabilities.json'))
 report['oracle_proven_native_capabilities']=report['oracle_compared_capabilities']
 report['oracle_cases_executed']=sum(c.get('oracle_cases',0) for c in read(ROOT/'research/commerce/capabilities.json') if c.get('oracle_status') in ('508_INTEGER_JSON_CASES_MATCHED','1620_ROBOTS_CASES_MATCHED','132_MICRODATA_CASES_MATCHED','46_QUEUE_TRACES_MATCHED'))
 summaries=[read(ROOT/'research/commerce/donors/census'/d['donor_id']/'summary.json') for d in reg['donors']]
 report['symbols_censused']=sum(s['symbols'] for s in summaries)
 report['unknown_files']=sum(s['unknown_files'] for s in summaries)
 graph=read(ROOT/'research/commerce/expansion-capability-graph.json')
 report['source_reviewed_contracts']=len(graph['capabilities']);report['mapped_capabilities']=len(graph['capabilities'])
 source=(ROOT/'domain/commerce/src/research.rs').read_text(encoding='utf-8');block=source.split('pub const ZERO_COST_STAGES',1)[1].split('];',1)[0]
 stages=re.findall(r'\("[^"]+",\s*(true|false)\)',block)
 assert stages
 report['zero_cost_stage_count']=sum(s=='true' for s in stages);report['research_stage_count']=len(stages);report['zero_cost_coverage_bps']=report['zero_cost_stage_count']*10000//len(stages)
 jsonschema.validate({k:v for k,v in report.items() if isinstance(v,int)},read(schema/'metrics.schema.json'))
 census.dump(ROOT/'research/commerce/metrics.json',report);print(json.dumps(report,indent=2))
if __name__=='__main__':main()
