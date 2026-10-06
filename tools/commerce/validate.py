"""Verify record schemas, immutable source evidence, and honest completion gates."""
import json,pathlib,sys,re,collections,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2]
LIFECYCLE=ROOT/('target/lifecycle-review.exe' if sys.platform=='win32' else 'target/lifecycle-review')
sys.path.insert(0,str(ROOT/'.venv/research'))
import jsonschema
import census
def read(p):return json.loads(p.read_text(encoding='utf-8'))

def validate_inventory(summary, files):
 counts=collections.Counter(f['classification'] for f in files)
 assert len(files)==summary['total_files'], 'CENSUS_TOTAL_MISMATCH'
 assert len({f['path'] for f in files})==len(files), 'CENSUS_DUPLICATE_PATH'
 assert dict(counts)==summary['classification_counts'], 'CENSUS_CLASSIFICATION_COUNTS_MISMATCH'
 assert len(files)-counts['UNKNOWN']==summary['classified_files'], 'CENSUS_CLASSIFIED_MISMATCH'
 assert counts['UNKNOWN']==summary['unknown_files'], 'CENSUS_UNKNOWN_MISMATCH'
 source=[f for f in files if f['classification']=='FIRST_PARTY_SOURCE']
 assert len(source)==summary['first_party_source_files'], 'CENSUS_SOURCE_MISMATCH'
 assert sum(f.get('parse_status')=='PARSED' for f in source)==summary['source_parsed'], 'CENSUS_PARSED_MISMATCH'
 assert sum(f.get('parse_status')!='PARSED' for f in source)==summary['parse_unknown'], 'CENSUS_PARSE_UNKNOWN_MISMATCH'
 assert counts['TEST']==summary['tests'], 'CENSUS_TEST_MISMATCH'
 assert counts['FIXTURE']==summary['fixtures'], 'CENSUS_FIXTURE_MISMATCH'

def validate_file_coverage(summary, files):
 """A complete census names its semantic matrix; a matrix with file_coverage must map every
 first-party source file to a reviewed prefix (first match wins)."""
 if 'semantic_matrix' not in summary: return
 matrix=read(ROOT/summary['semantic_matrix'])
 if 'file_coverage' not in matrix: return
 prefixes=[e['prefix'] for e in matrix['file_coverage']['entries']]
 hit=lambda p:any(p==x or (x.endswith('/') and p.startswith(x)) for x in prefixes)
 missing=[f['path'] for f in files if f['classification']=='FIRST_PARTY_SOURCE' and not hit(f['path'])]
 assert not missing, ('SEMANTIC_FILE_COVERAGE_INCOMPLETE',missing[:5])
 for sub in summary.get('submodules',[]):
  reviewed=matrix.get('submodule_coverage',{}).get(sub['path'])
  assert reviewed and reviewed['commit_sha']==sub['commit_sha'] and reviewed['files'], ('SUBMODULE_NOT_SEMANTICALLY_REVIEWED',sub['path'])

def inventory_negative_cases():
 path=ROOT/'research/commerce/donors/census/scrapinghub--price-parser'
 summary=read(path/'summary.json');files=[json.loads(x) for x in (path/'files.jsonl').read_text(encoding='utf-8').splitlines()]
 validate_inventory(summary,files)
 mutations=[dict(summary,classification_counts={**summary['classification_counts'],'UNKNOWN':1}),dict(summary,source_parsed=0),dict(summary,classified_files=0)]
 for changed in mutations:
  try:validate_inventory(changed,files)
  except AssertionError:continue
  raise AssertionError('CORRUPT_CENSUS_ACCEPTED')
 try:validate_inventory(summary,files+[files[0]])
 except AssertionError:pass
 else:raise AssertionError('DUPLICATE_CENSUS_ACCEPTED')
 print('PASS: valid locked price-parser census; stale classification counts, source parser count, classified count and duplicate path rejected')

def validate_lifecycle(d, summary, actual):
 assert actual is not None, 'DONOR_NOT_CANONICALLY_ASSESSED'
 if d['extinction_status']=='EXTINCT' or actual['extinct']:
  assert actual['extinct'] and actual['effective']=='EXTINCT', 'UNPROVEN_EXTINCTION_CLAIM'
  assert summary['status']=='CENSUS_COMPLETE' and summary['semantic_review']=='COMPLETE', 'EXTINCTION_WITH_INCOMPLETE_SEMANTIC_CENSUS'
  assert d['runtime_dependency'] is False, 'EXTINCT_DONOR_RUNTIME_DEPENDENCY'
 if d['absorption_status']=='NATIVE_ABSORBED':
  assert actual['effective'] in ('CUTOVER','EXTINCT') and summary['semantic_review']=='COMPLETE', 'UNPROVEN_WHOLE_ABSORPTION_CLAIM'
 if d['oracle_status']=='ORACLE_VERIFIED':
  required=[c for c in actual['capabilities'] if c['required']]
  assert required and all(c['parity_pass'] for c in required) and summary['semantic_review']=='COMPLETE', 'UNPROVEN_WHOLE_ORACLE_CLAIM'

def lifecycle_negative_cases():
 assessed={d['donor_id']:d for d in json.loads(subprocess.check_output([str(LIFECYCLE),str(ROOT)]))}
 donors={d['donor_id']:d for d in read(ROOT/'research/commerce/donors/registry.json')['donors']}
 summary=lambda k:read(ROOT/'research/commerce/donors/census'/k/'summary.json')
 # The extinct donor is accepted exactly as governance assessed it.
 validate_lifecycle(donors['scrapinghub--price-parser'],summary('scrapinghub--price-parser'),assessed['scrapinghub--price-parser'])
 extinct=assessed['scrapinghub--price-parser']
 for changed,census,evidence in [(donors['scrapinghub--price-parser'],summary('scrapinghub--price-parser'),dict(extinct,effective='CUTOVER',extinct=False)),(donors['scrapinghub--price-parser'],dict(summary('scrapinghub--price-parser'),status='CENSUS_PARTIAL'),extinct)]:
  try:validate_lifecycle(changed,census,evidence)
  except AssertionError:continue
  raise AssertionError('EXTINCTION_WITHOUT_EVIDENCE_OR_CENSUS_ACCEPTED')
 # A bounded, non-extinct donor rejects every unproven whole-donor claim.
 k='apify--crawlee';d=donors[k];actual=assessed[k];s=summary(k)
 validate_lifecycle(d,s,actual)
 for changed,evidence in [(dict(d,extinction_status='EXTINCT'),actual),(d,dict(actual,effective='EXTINCT',extinct=True)),(dict(d,absorption_status='NATIVE_ABSORBED'),actual),(dict(d,oracle_status='ORACLE_VERIFIED'),actual)]:
  try:validate_lifecycle(changed,s,evidence)
  except AssertionError:continue
  raise AssertionError('UNPROVEN_WHOLE_DONOR_CLAIM_ACCEPTED')
 print('PASS: assessed extinct donor accepted; regressed evidence and incomplete census rejected; unproven extinction, whole absorption and whole oracle claims of a bounded donor rejected')
def main():
 if '--inventory-negative-cases' in sys.argv:
  inventory_negative_cases();return
 if '--lifecycle-negative-cases' in sys.argv:
  lifecycle_negative_cases();return
 reg=read(ROOT/'research/commerce/donors/registry.json');schema=ROOT/'tools/commerce/schemas'
 authority=LIFECYCLE
 assert authority.is_file(), 'Build tools/commerce/lifecycle_review.rs against .ecdev before measuring donor lifecycle'
 assessed={d['donor_id']:d for d in json.loads(subprocess.check_output([str(authority),str(ROOT)]))}
 total=classified=source=parsed=unknown=tests=capabilities=0
 for d in reg['donors']:
  jsonschema.validate(d,read(schema/'donor.schema.json'))
  path=ROOT/'research/commerce/donors/census'/d['donor_id'];summary=read(path/'summary.json');jsonschema.validate(summary,read(schema/'census.schema.json'))
  files=[json.loads(x) for x in (path/'files.jsonl').read_text(encoding='utf-8').splitlines()]
  validate_inventory(summary,files)
  assert summary['status']=='CENSUS_PARTIAL' or (summary['unknown_files']==0 and summary['parse_unknown']==0 and summary['semantic_review']=='COMPLETE')
  if summary['status']=='CENSUS_COMPLETE': validate_file_coverage(summary,files)
  validate_lifecycle(d,summary,assessed.get(d['donor_id']))
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
 report['native_absorbed']=sum(d['absorption_status']=='NATIVE_ABSORBED' for d in reg['donors'])
 report['oracle_verified']=sum(d['oracle_status']=='ORACLE_VERIFIED' for d in reg['donors'])
 report['extinct']=sum(assessed[d['donor_id']]['extinct'] for d in reg['donors'])
 report['lifecycle_measurement']='CURRENT_CANONICAL_SOURCE_AND_FRESH_PROOFS'
 deps=read(ROOT/'research/commerce/dependencies.json')
 report['runtime_dependency_packages']=deps['runtime_packages']
 report['runtime_dependency_upstreams']=len({p['repository_url'] for p in deps['packages'] if p['scope']=='RUNTIME'})
 report['runtime_donor_dependencies']=report['runtime_dependency_upstreams']
 report['seed_runtime_dependencies']=0
 report['rust_replacement_capabilities']=sum(c.get('native_implementation') is not None for c in read(ROOT/'research/commerce/capabilities.json'))
 for capability in read(ROOT/'research/commerce/capabilities.json'):
  if capability.get('oracle_status')=='2937_PRICE_PUBLIC_API_CASES_MATCHED':
   import hashlib
   fixture=ROOT/capability['fixture'];oracle=read(fixture)
   assert hashlib.sha256(fixture.read_bytes()).hexdigest()==capability['fixture_sha256']
   assert len(oracle['cases'])==capability['oracle_cases']==2937
   assert oracle['commit_sha']==capability['commit_sha']==next(d['commit_sha'] for d in reg['donors'] if d['donor_id']=='scrapinghub--price-parser')
   assert oracle['oracle']=='UNMODIFIED_LOCKED_PRICE_PARSER_FULL_PUBLIC_PRICE_API' and oracle['whole_donor_parity'] is False
   public_report=read(ROOT/'research/commerce/price-public-api-oracle-report.json')
   assert public_report['status']=='PASS' and public_report['fixture_sha256']==capability['fixture_sha256'] and public_report['whole_donor_parity'] is False
   assert hashlib.sha256((ROOT/public_report['native_source']).read_bytes()).hexdigest()==public_report['native_source_sha256']
   assert hashlib.sha256((ROOT/'adapter/web/data/price-currency.json').read_bytes()).hexdigest()==public_report['data_sha256']
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
 for capability in read(ROOT/'research/commerce/capabilities.json'):
  if capability.get('oracle_status')=='54_DECLARED_DOCUMENT_CASES_MATCHED':
   import hashlib
   fixture=ROOT/capability['fixture'];oracle=read(fixture)
   assert hashlib.sha256(fixture.read_bytes()).hexdigest()==capability['fixture_sha256']
   assert len(oracle['cases'])==capability['oracle_cases']==54
   assert oracle['commit_sha']==capability['commit_sha']=='537c5d46455ae8b2c67b53fc03b36ef1da8c4837'
   assert oracle['oracle']=='UNMODIFIED_LOCKED_W3LIB_HTML_TO_UNICODE'
 for capability in read(ROOT/'research/commerce/capabilities.json'):
  if capability.get('oracle_status')=='1235_PRICE_NUMBER_CASES_MATCHED':
   import hashlib
   fixture=ROOT/capability['fixture'];oracle=read(fixture)
   assert hashlib.sha256(fixture.read_bytes()).hexdigest()==capability['fixture_sha256']
   assert len(oracle['cases'])==capability['oracle_cases']==1235
   assert oracle['commit_sha']==capability['commit_sha']==next(d['commit_sha'] for d in reg['donors'] if d['donor_id']=='scrapinghub--price-parser')
   assert oracle['oracle']=='UNMODIFIED_LOCKED_PRICE_PARSER_PARSE_NUMBER'
 report['oracle_compared_capabilities']=sum(c.get('oracle_status') in ('508_INTEGER_JSON_CASES_MATCHED','1620_ROBOTS_CASES_MATCHED','132_MICRODATA_CASES_MATCHED','46_QUEUE_TRACES_MATCHED','54_DECLARED_DOCUMENT_CASES_MATCHED','1235_PRICE_NUMBER_CASES_MATCHED','2937_PRICE_PUBLIC_API_CASES_MATCHED') for c in read(ROOT/'research/commerce/capabilities.json'))
 report['oracle_proven_native_capabilities']=report['oracle_compared_capabilities']
 report['oracle_cases_executed']=sum(c.get('oracle_cases',0) for c in read(ROOT/'research/commerce/capabilities.json') if c.get('oracle_status') in ('508_INTEGER_JSON_CASES_MATCHED','1620_ROBOTS_CASES_MATCHED','132_MICRODATA_CASES_MATCHED','46_QUEUE_TRACES_MATCHED','54_DECLARED_DOCUMENT_CASES_MATCHED','1235_PRICE_NUMBER_CASES_MATCHED','2937_PRICE_PUBLIC_API_CASES_MATCHED'))
 helper_report=ROOT/'research/commerce/price-decimal-helper-oracle-report.json'
 if helper_report.exists():
  import hashlib
  helper=read(helper_report); fixture=ROOT/helper['fixture']; oracle=read(fixture)
  assert helper['status']=='PASS' and helper['whole_donor_parity'] is False
  assert hashlib.sha256(fixture.read_bytes()).hexdigest()==helper['fixture_sha256']
  assert hashlib.sha256((ROOT/helper['native_source']).read_bytes()).hexdigest()==helper['native_source_sha256']
  assert oracle['oracle']=='UNMODIFIED_LOCKED_PRICE_PARSER_DECIMAL_SEPARATOR_HELPER'
  assert oracle['commit_sha']==helper['commit_sha']==next(d['commit_sha'] for d in reg['donors'] if d['donor_id']=='scrapinghub--price-parser')
  assert len(oracle['cases'])==helper['cases']==oracle['decimal_alphabets']*3*4*5+14
  assert oracle['decimal_alphabets']==helper['decimal_alphabets']==66
  report['oracle_cases_executed']+=helper['cases']
  # Supplemental direct-helper family, not a new replacement capability or whole donor.
  report['supplemental_helper_oracle_families']=1
 for filename,kind,count,families in [('price-literal-helper-oracle-report.json','UNMODIFIED_LOCKED_PRICE_PARSER_LITERAL_UNION_SEARCH',1576,1),('price-source-helpers-oracle-report.json','UNMODIFIED_LOCKED_PRICE_PARSER_SOURCE_HELPERS',9560,2)]:
  helper=read(ROOT/'research/commerce'/filename);fixture=ROOT/helper['fixture'];oracle=read(fixture)
  assert helper['status']=='PASS' and helper['whole_donor_parity'] is False
  assert oracle['oracle']==kind and len(oracle['cases'])==helper['cases']==count
  assert oracle['commit_sha']==helper['commit_sha']==next(d['commit_sha'] for d in reg['donors'] if d['donor_id']=='scrapinghub--price-parser')
  assert hashlib.sha256(fixture.read_bytes()).hexdigest()==helper['fixture_sha256']
  assert hashlib.sha256((ROOT/helper['native_source']).read_bytes()).hexdigest()==helper['native_source_sha256']
  assert hashlib.sha256((ROOT/'adapter/web/data/price-currency.json').read_bytes()).hexdigest()==helper['data_sha256']
  report['oracle_cases_executed']+=helper['cases'];report['supplemental_helper_oracle_families']+=families
 object_report=read(ROOT/'research/commerce/price-object-oracle-report.json');fixture=ROOT/object_report['fixture'];oracle=read(fixture)
 assert object_report['status']=='PASS' and object_report['whole_donor_parity'] is False
 assert oracle['oracle']=='UNMODIFIED_LOCKED_PRICE_PARSER_PRICE_OBJECT'
 assert len(oracle['construction'])==1083 and len(oracle['comparisons'])==36 and object_report['cases']==1119
 assert oracle['commit_sha']==object_report['commit_sha']==next(d['commit_sha'] for d in reg['donors'] if d['donor_id']=='scrapinghub--price-parser')
 assert hashlib.sha256(fixture.read_bytes()).hexdigest()==object_report['fixture_sha256']
 assert hashlib.sha256((ROOT/object_report['native_source']).read_bytes()).hexdigest()==object_report['native_source_sha256']
 assert hashlib.sha256((ROOT/'adapter/web/data/price-currency.json').read_bytes()).hexdigest()==object_report['data_sha256']
 assert oracle['unicode_version']==read(ROOT/'adapter/web/data/price-currency.json')['unicode_version']=='14.0.0'
 report['oracle_cases_executed']+=object_report['cases'];report['supplemental_helper_oracle_families']+=1
 scalar=read(ROOT/'research/commerce/price-scalar-oracle-report.json');fixture=ROOT/scalar['fixture'];oracle=read(fixture)
 assert scalar['status']=='PASS' and scalar['whole_donor_parity'] is False
 assert oracle['oracle']=='UNMODIFIED_LOCKED_PRICE_PARSER_TYPED_SCALAR' and len(oracle['cases'])==scalar['cases']==1134
 assert oracle['commit_sha']==scalar['commit_sha']==next(d['commit_sha'] for d in reg['donors'] if d['donor_id']=='scrapinghub--price-parser')
 assert hashlib.sha256(fixture.read_bytes()).hexdigest()==scalar['fixture_sha256']
 assert hashlib.sha256((ROOT/scalar['native_source']).read_bytes()).hexdigest()==scalar['native_source_sha256']
 assert hashlib.sha256((ROOT/'adapter/web/data/price-currency.json').read_bytes()).hexdigest()==scalar['data_sha256']
 assert oracle['unicode_version']==read(ROOT/'adapter/web/data/price-currency.json')['unicode_version']=='14.0.0'
 report['oracle_cases_executed']+=scalar['cases'];report['supplemental_helper_oracle_families']+=1
 summaries=[read(ROOT/'research/commerce/donors/census'/d['donor_id']/'summary.json') for d in reg['donors']]
 report['symbols_censused']=sum(s['symbols'] for s in summaries)
 report['unknown_files']=sum(s['unknown_files'] for s in summaries)
 graph=read(ROOT/'research/commerce/expansion-capability-graph.json')
 report['source_reviewed_contracts']=len(graph['capabilities']);report['mapped_capabilities']=len(graph['capabilities'])
 social_graph=read(ROOT/'research/commerce/social-capability-graph.json')
 report['source_reviewed_contracts']+=len(social_graph['capabilities']);report['mapped_capabilities']+=len(social_graph['capabilities'])
 social_oracle=read(ROOT/'research/commerce/social-oracle-report.json')
 import hashlib
 assert social_oracle['status']=='PASS';assert hashlib.sha256((ROOT/social_oracle['fixture']).read_bytes()).hexdigest()==social_oracle['fixture_sha256']
 report['oracle_compared_capabilities']+=social_oracle['families'];report['oracle_proven_native_capabilities']+=social_oracle['families'];report['oracle_cases_executed']+=social_oracle['cases'];report['rust_replacement_capabilities']+=social_oracle['families']
 memory_graph=ROOT/'research/commerce/memory-capability-graph.json'
 if memory_graph.exists():
  report['source_reviewed_contracts']+=len(read(memory_graph)['contracts']);report['mapped_capabilities']+=len(read(memory_graph)['contracts'])
  memory_oracle=read(ROOT/'research/commerce/memory-time-oracle-report.json')
  assert memory_oracle['status']=='PASS';assert hashlib.sha256((ROOT/memory_oracle['fixture']).read_bytes()).hexdigest()==memory_oracle['fixture_sha256']
  report['oracle_compared_capabilities']+=memory_oracle['families'];report['oracle_proven_native_capabilities']+=memory_oracle['families'];report['oracle_cases_executed']+=memory_oracle['cases'];report['rust_replacement_capabilities']+=1
 source=(ROOT/'domain/commerce/src/research.rs').read_text(encoding='utf-8');block=source.split('pub const ZERO_COST_STAGES',1)[1].split('];',1)[0]
 stages=re.findall(r'\("[^"]+",\s*(true|false)\)',block)
 assert stages
 report['zero_cost_stage_count']=sum(s=='true' for s in stages);report['research_stage_count']=len(stages);report['zero_cost_coverage_bps']=report['zero_cost_stage_count']*10000//len(stages)
 jsonschema.validate({k:v for k,v in report.items() if isinstance(v,int)},read(schema/'metrics.schema.json'))
 census.dump(ROOT/'research/commerce/metrics.json',report);print(json.dumps(report,indent=2))
if __name__=='__main__':main()
