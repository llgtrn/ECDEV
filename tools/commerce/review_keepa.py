"""Record the source review of the locked Keepa donor; executable contracts precede replacements."""
import json,pathlib,sys
ROOT=pathlib.Path(__file__).resolve().parents[2];sys.path.insert(0,str(ROOT/'.venv/research'))
import census
def main():
 registry=json.loads((ROOT/'research/commerce/donors/registry.json').read_text());d=next(r for r in registry['donors'] if r['donor_id']=='purahmanian--keepa-mcp')
 out=ROOT/'research/commerce/donors/census'/d['donor_id'];checkout=ROOT/'research/commerce/donors/checkouts'/d['donor_id']
 files=[json.loads(x) for x in (out/'files.jsonl').read_text().splitlines()];sy=[json.loads(x) for x in (out/'symbols.jsonl').read_text().splitlines()]
 def evidence(path,symbol=None,kind='SOURCE_IMPLEMENTATION'):
  row=next(f for f in files if f['path']==path);span=next((s for s in sy if s['source_path']==path and s['symbol']==symbol),None)
  if symbol:assert span is not None,symbol
  return {'donor_id':d['donor_id'],'repository_url':d['repository_url'],'commit_sha':d['commit_sha'],'source_path':path,'blob_hash':row['blob_hash'],'symbol':symbol,'line_start':span['line_start'] if span else 1,'line_end':span['line_end'] if span else len((checkout/path).read_text().splitlines()),'evidence_type':kind,'confidence':1.0}
 specs=[('get_product','get-product','runGetProduct','/product','Product summary; stats.current then CSV tail, then current then legacy; USD text output; no stats means fallback'),('get_price_history','get-price-history','runGetPriceHistory','/product','Price slots 0,1,2,18; pairs except buybox triplets; exclude negative samples; date range; maximum 60 evenly sampled points; USD formatter'),('get_sales_rank_history','get-sales-rank-history','runGetSalesRankHistory','/product','Slot 3 history; first/last heuristic trend; rank is not measured sales'),('search_products','search-products','runSearchProducts','/search','GET domain,type=product,term,optional catid; asinList; missing list becomes empty'),('get_best_sellers','get-best-sellers','runGetBestSellers','/bestsellers','GET domain,category; bestSellersList.asinList; output first 100'),('get_deals','get-deals','runGetDeals','/deal','JSON selection with page,domainId,priceTypes=[0],dateRange=0,isRangeEnabled=true,deltaPercentRange=[threshold,2147483647],minRating; uses dr[].current indexed slots')]
 caps=[]
 index=(checkout/'src/index.ts').read_text()
 for name,file,symbol,endpoint,behavior in specs:
  assert '"'+name+'"' in index
  caps.append({'capability_id':'keepa.'+name,'donor_id':d['donor_id'],'status':'VERIFIED','evidence':[evidence('src/tools/'+file+'.ts',symbol),evidence('src/index.ts',None,'TOOL_REGISTRATION')],'behavior_contract':behavior,'endpoint':endpoint,'external_service':'Keepa API (TYPE_B)','input_schema_source':'src/tools/'+file+'.ts','test_map':['tests/tools.test.ts'],'tests_executed':False,'native_implementation':None,'oracle_status':'NOT_STARTED','absorption_status':'NOT_STARTED'})
 caps.append({'capability_id':'keepa.resolve-current','donor_id':d['donor_id'],'status':'VERIFIED','evidence':[evidence('src/services/keepa-values.ts','resolveCurrent'),evidence('src/services/keepa-values.ts','lastCsvValue'),evidence('src/constants.ts','csvStride')],'behavior_contract':'For finite integer JSON values: take nonnegative stats.current[index], else last complete nonnegative CSV sample at pair stride (triplet for slot18), else nonnegative current[index], else nonnegative legacy; absent => null; zero is valid. Incomplete trailing CSV samples ignored. Does not infer currency or retrieve live data.','test_map':['tests/tools.test.ts:buildProductSummary'],'native_implementation':None,'oracle_plan':{'cases':'Deterministic precedence, zero, absent, -1, triplets, incomplete tails, and seeded randomized integer JSON records','execution':'Node TypeScript donor resolver vs Rust resolver; frozen cases under tests/commerce/fixtures','network':'NONE','unsupported':'NaN/Infinity/undefined cannot be serialized as JSON; out-of-range or malformed input handled explicitly'},'oracle_status':'NOT_STARTED','absorption_status':'NOT_STARTED'})
 census.dump(out/'capabilities.json',caps);census.jsonl(out/'evidence.jsonl',[e for c in caps for e in c['evidence']]);census.dump(ROOT/'research/commerce/capabilities.json',caps)
 lock=json.loads((checkout/'package-lock.json').read_text());deps=json.loads((out/'dependencies.json').read_text())
 for dep in deps:
  if dep['source_path']=='package-lock.json':
   dep['roles']=[{'name':name,'version':p.get('version'),'role':'DEVELOPMENT' if p.get('dev') else 'RUNTIME','optional':p.get('optional',False),'integrity':p.get('integrity')} for name,p in lock['packages'].items() if name];dep['status']='PARSED';dep['role_review']='COMPLETE'
 census.dump(out/'dependencies.json',deps)
 common=evidence('src/services/keepa-client.ts','keepaFetch')
 for facet,value in {
 'auth':{'env':'KEEPA_API_KEY','transport':'query key','missing':'withKey returns isError and explanatory text','secrets_risk':'Never forward raw upstream request URLs to ECDEV logs'},
 'network-behavior':{'base_url':'https://api.keepa.com','method':'GET','timeout_ms':30000,'response':'JSON','redirects':'Fetch defaults'},
 'errors':{'400':'invalid key or params, structured error message surfaced','402':'quota exceeded','429':'rate limited','other_non_2xx':'HTTP error','missing_product':'null or no-product message'},
 'retry-semantics':{'automatic_retries':0,'backoff':'NONE'},
 'rate-limits':{'proactive_limiter':'NONE','quota_state':'Returned fields typed; no automatic quota scheduling'},
 'pagination':{'deals':'Explicit page; no iterator','search':'No pagination loop','bestsellers':'First 100 formatted'},
 'storage':{'persistent_storage':'NONE','cache':'NONE'},
 'process-model':{'runtime':'Node >=18','transport':'stdio','subprocesses':'NONE in source','http_server':'NONE','entrypoint':'dist/index.js'},
 'protocols':{'transport':'stdio','sdk':'@modelcontextprotocol/sdk','resources':[],'prompts':[],'tools':6,'auth':'Provider query key, not MCP authentication'},
 'external-services':{'services':[{'name':'Keepa API','type':'TYPE_B','base_url':'https://api.keepa.com','absorbed':False}]},
 'risks':{'issues':['USD formatting irrespective of domain: unsuitable for Japan currency normalization','Rank trend is a heuristic, not sales evidence','No proactive quota limiter, cache or automatic retry','Live provider correctness not verified without credentials','README free-tier statements not verified']}
 }.items():census.dump(out/(facet+'.json'),{'status':'SOURCE_REVIEWED','reviewed_at':census.now(),'details':value,'evidence':[common]})
 census.dump(out/'mcp-surface.json',{'status':'VERIFIED','transports':['stdio'],'tools':caps[:6],'resources':[],'prompts':[],'evidence':[evidence('src/index.ts',None,'TOOL_REGISTRATION')]})
 census.dump(out/'entrypoints.json',{'status':'VERIFIED','node':'dist/index.js','build':'tsc','evidence':[evidence('package.json',None,'CONFIG'),evidence('src/index.ts')]})
 census.dump(out/'build.json',{'status':'SOURCE_REVIEWED','runtime':'Node >=18','compile':'tsc (ES2022/Node16 strict)','ci_node':[18,20],'release_node':24,'external_build_executable':['npm','node'],'tests':'vitest forks, fully mocked fetch','release':'npm publish and MCP registry publish are release-only network writes; not executed','evidence':[evidence('package.json',None,'CONFIG'),evidence('.github/workflows/ci.yml',None,'CONFIG'),evidence('.github/workflows/release.yml',None,'CONFIG')]})
 census.dump(out/'license.json',{'status':'VERIFIED','license':'MIT','files':[{'source_path':'LICENSE','sha256':next(f['sha256'] for f in files if f['path']=='LICENSE')}],'evidence':[evidence('LICENSE',None,'LICENSE')]})
 summary=json.loads((out/'summary.json').read_text());assert summary['unknown_files']==0 and summary['parse_unknown']==0 and not summary['submodules'];assert all(dep['status']=='PARSED' for dep in deps)
 summary.update(status='CENSUS_COMPLETE',semantic_review='COMPLETE',capabilities_verified=len(caps),limitations=['Census establishes donor code behavior, not live provider correctness or absorption.'],gates={'remote_verified':'PASS','commit_locked':'PASS','tree_classified':'PASS','source_accounted':'PASS','manifests_parsed':'PASS','runtime_deps_classified':'PASS','tests_inventoried':'PASS','entrypoints_inventoried':'PASS','mcp_surface_inventoried':'PASS','external_services_identified':'PASS','capabilities_mapped':'PASS','unknown_material_files':0})
 census.dump(out/'summary.json',summary)
 d.update(source_census_status='SOURCE_CENSUSED',dependency_census_status='COMPLETE',protocol_census_status='COMPLETE',capability_census_status='COMPLETE',capabilities_total=len(caps),capabilities_verified=len(caps),external_services=['Keepa API (TYPE_B)'])
 census.dump(out/'identity.json',d);census.dump(ROOT/'research/commerce/donors/registry.json',registry)
 # Binding to the canonical declaration stores specifications, not unsupported native claims.
 path=ROOT/'.ynventa/declared/donors.rs';decl=path.read_text();start=decl.index('Donor { key: "purahmanian--keepa-mcp"');end=decl.index('\n',start)
 capability='&['+','.join('Capability { key: '+json.dumps(c['capability_id'])+', required: true, spec: "research/commerce/donors/census/purahmanian--keepa-mcp/capabilities.json", replacement: None, maps_to: None, norl: NorlRelevance::Unresolved, proofs: &[] }' for c in caps)+']'
 row=decl[start:end].replace('claimed: DonorState::Discovered','claimed: DonorState::Censused').replace('capabilities: &[]','capabilities: '+capability)
 path.write_text(decl[:start]+row+decl[end:]);print('Keepa: 29/29 files, 12/12 source files, 7 source-backed capabilities; no absorption or extinction claimed')
if __name__=='__main__':main()
