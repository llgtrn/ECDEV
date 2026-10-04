"""Locked-source memory contract review; does not load a donor runtime."""
from __future__ import annotations
import ast, collections, hashlib, json, pathlib, subprocess, tomllib
import census

CONTRACTS = {
 'getzep--zep': [
  ('ingestion/src/zep_ingest/types.py','Episode','Validated episode text/type/source timestamp/metadata; source provenance survives SDK projection.'),
  ('ingestion/src/zep_ingest/types.py','Destination','Exactly one graph_id or user_id; no silent destination fallback.'),
  ('ingestion/src/zep_ingest/types.py','to_graph_add_kwargs','Omit unset optional fields; forwards to managed Zep graph API, not local persistence.'),
  ('ingestion/src/zep_ingest/_validation.py','check_timestamp','Absent timestamp allowed; supplied value must parse and have an explicit timezone.'),
  ('ingestion/src/zep_ingest/triples.py','FactTriple','Named fact endpoints, optional UUID identities, source-validity timestamps and metadata; SDK owns fact creation.'),
  ('ingestion/src/zep_ingest/submitters/sequential.py','_is_retryable','Unsent transport failures/429 retried; ambiguous writes/5xx require explicit idempotency permission.'),
  ('ingestion/src/zep_ingest/submitters/sequential.py','_retry_after_seconds','Seconds or aware HTTP date; nonfinite/unparseable delays unavailable; elapsed delays zero.'),
  ('ingestion/src/zep_ingest/pipeline.py','Pipeline','Preview runs loader/transforms without SDK writes; run validates replay then uses selected submitter. Whole orchestration replacement not claimed.'),
  ('ingestion/src/zep_ingest/transforms/canonicalizer.py','AliasCanonicalizer','Explicit alias rewrite/annotation with ambiguity/risky-word guards; must never silently establish commerce identity.'),
 ],
 'getzep--graphiti': [
  ('graphiti_core/utils/datetime_utils.py','ensure_utc','Normalize explicit-zone datetimes to UTC; donor assumes UTC for naive inputs, which ECDEV intentionally rejects.'),
  ('graphiti_core/utils/datetime_utils.py','convert_datetimes_to_strings','Recursively normalize UTC ISO strings before lexicographic database comparisons; tuple becomes list.'),
  ('graphiti_core/edges.py','EntityEdge','Fact, episode provenance, created/valid/invalid/expired times and embedding; model fields do not themselves prove factual truth.'),
  ('graphiti_core/search/search_filters.py','SearchFilters','Separate valid/invalid/created/expired filters and labels; storage/query construction is external, not native ECDEV.'),
  ('graphiti_core/search/search_config.py','EdgeSearchMethod','Cosine, BM25 and BFS retrieval choices; independent publisher or demand inference is not implied.'),
  ('graphiti_core/search/search_config.py','EdgeReranker','RRF/distance/episode-mention/MMR/cross-encoder choices; native learned/calibrated ranking not claimed.'),
 ]
}

def locked(checkout, commit, path):
 return subprocess.check_output(['git','-C',str(checkout),'show',commit+':'+path])

def main():
 registry_path=census.YN/'donors/registry.json'
 registry=json.loads(registry_path.read_text(encoding='utf-8'))
 graph=[]; reviews={}
 for record in registry['donors']:
  key=record['donor_id']
  if key not in CONTRACTS: continue
  base=census.YN/'donors/census'/key; checkout=census.YN/'donors/checkouts'/key; commit=record['commit_sha']
  files=[json.loads(x) for x in (base/'files.jsonl').read_text(encoding='utf-8').splitlines()]
  classification_evidence=[]
  for row in files:
   path=row['path']
   if not (path.endswith(('.jsonl','.eml')) or pathlib.Path(path).name.startswith('Makefile') or pathlib.Path(path).name in {'go.work','go.work.sum'}): continue
   raw=locked(checkout,commit,path)
   assert hashlib.sha256(raw).hexdigest()==row['sha256']
   if path.endswith(('.jsonl','.eml')): kind='FIXTURE'; reason='Pinned example/email/benchmark records; material data, not executable source.'
   elif pathlib.Path(path).name.startswith('Makefile'): kind='BUILD'; reason='Pinned Makefile compilation targets.'
   elif pathlib.Path(path).name in {'go.work','go.work.sum'}: kind='BUILD'; reason='Go workspace/module checksum metadata.'
   else: continue
   row['classification']=kind; classification_evidence.append({'path':path,'classification':kind,'sha256':row['sha256'],'basis':reason})
  census.jsonl(base/'files.jsonl',files)
  summary=json.loads((base/'summary.json').read_text(encoding='utf-8'))
  counts=collections.Counter(row['classification'] for row in files)
  summary.update(classification_counts=dict(counts),classified_files=len(files)-counts['UNKNOWN'],unknown_files=counts['UNKNOWN'],fixtures=counts['FIXTURE'],semantic_review='BOUNDED_SOURCE_CONTRACT_REVIEW_WHOLE_DONOR_INCOMPLETE')
  summary['classification_evidence']=classification_evidence
  census.dump(base/'summary.json',summary)
  record['tree_status']='TREE_CENSUSED' if not counts['UNKNOWN'] and not summary['submodules'] else 'PARTIAL'
  licenses=[]
  for row in files:
   if not pathlib.Path(row['path']).name.lower().startswith(('license','copying','notice')): continue
   raw=locked(checkout,commit,row['path']); header=raw[:300].decode('utf-8','replace')
   terms='Apache-2.0' if 'Apache' in header and '2.0' in header else 'MIT' if 'MIT License' in header else 'UNKNOWN'
   licenses.append({'path':row['path'],'blob_hash':row['blob_hash'],'sha256':hashlib.sha256(raw).hexdigest(),'terms':terms,'basis':'LOCKED_GIT_BLOB'})
  root_license=next(row for row in licenses if row['path']=='LICENSE')
  record['license']=root_license['terms'];record['license_absorption_gate']='SOURCE_STUDY_ONLY_NO_PRODUCTION_COPY'
  contracts=[]
  for path,name,behavior in CONTRACTS[key]:
   raw=locked(checkout,commit,path); tree=ast.parse(raw)
   node=next(n for n in tree.body if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef,ast.ClassDef)) and n.name==name)
   inventory=next(f for f in files if f['path']==path)
   cap={'capability':key+'.'+name,'donor_id':key,'commit_sha':commit,'source_path':path,'source_symbol':name,'line_start':node.lineno,'line_end':node.end_lineno,'source_sha256':hashlib.sha256(raw).hexdigest(),'source_blob_hash':inventory['blob_hash'],'behavior_contract':behavior,'review_status':'SOURCE_BACKED_BOUNDED_CONTRACT_STUDY','native_owner':'commerce.web-research' if name=='ensure_utc' else None,'native_implementation':'adapter/web/src/social.rs::published' if name=='ensure_utc' else None,'production_callers':['adapter/web/src/social.rs::normalize (Bluesky and JSON Feed publication timestamps)'] if name=='ensure_utc' else [],'oracle':'adapter/web/src/social.rs::graphiti_timezone_oracle' if name=='ensure_utc' else None,'runtime_dependency':False,'whole_donor_replacement':False}
   contracts.append(cap);graph.append(cap)
  manifests=[]
  for dep in json.loads((base/'dependencies.json').read_text(encoding='utf-8')):
   path=dep['source_path'];raw=locked(checkout,commit,path); entry={'source_path':path,'source_sha256':hashlib.sha256(raw).hexdigest(),'inventory_status':dep['status'],'dependency_semantics':'LOCKFILE_TRANSITIVE_CLOSURE_NOT_FULLY_REVIEWED'}
   if path.endswith('.toml'):
    data=tomllib.loads(raw.decode()); entry['direct_requirements']=data.get('project',{}).get('dependencies',[]);entry['optional_requirements']=data.get('project',{}).get('optional-dependencies',{})
   elif pathlib.Path(path).name.startswith('requirements'):
    entry['direct_requirements']=[line.strip() for line in raw.decode().splitlines() if line.strip() and not line.lstrip().startswith('#')]
   elif path.endswith('package.json'):
    data=json.loads(raw);entry['direct_requirements']=data.get('dependencies',{});entry['development_requirements']=data.get('devDependencies',{})
   manifests.append(entry)
  census.dump(base/'memory-modules-api-inventory.json',{'modules':[{'source_path':f['path'],'language':f['language'],'parse_status':f.get('parse_status')} for f in files if f['classification']=='FIRST_PARTY_SOURCE'],'symbols_inventory':'symbols.jsonl','protocol_candidates':'protocols.json','api_review':'research/commerce/'+('zep-capability-review.json' if key.endswith('--zep') else 'graphiti-capability-review.json'),'source_backed_contracts':contracts,'whole_semantic_coverage':'INCOMPLETE'})
  review={'donor_id':key,'exact_pinned_commit':commit,'retrieved_at':record['retrieved_at'],'license':root_license['terms'],'license_files':licenses,'full_clone':record['clone_status'],'submodules':record.get('submodule_status',''),'tracked_files':len(files),'source_parsed':summary['source_parsed'],'source_parse_unknown':summary['parse_unknown'],'source_contracts':contracts,'dependency_inventory':manifests,'runtime_dependencies_added':0,'native_absorption_candidates':['Explicit-zone UTC normalization at the commerce acquisition boundary'],'keep_external_candidates':['Managed SDK graph calls','Generic temporal graph storage/retrieval','LLM extraction/embedding and graph drivers'],'rejected_capabilities':['Silent rewriting of observed product identities through aliases','Social-to-sales/demand inference','Generic agent memory duplicated inside ECDEV','Paid or authenticated calls without credentials/permission'],'unknowns':['Whole-donor semantic/API replacement coverage','Transitive dependency license closure','Compatibility between legacy Graphiti service image 0.3 and separately pinned current Graphiti source','Managed Zep Cloud implementation is not present or independently verifiable from SDK forwarding code'],'overlap_with_chronica':'General temporal graph/memory orchestration is outside commerce-domain ownership; cross-shard interface needs upstream architectural review. No generic ECDEV memory backend was introduced.','overlap_with_norl':'Canonical organism module reserves cognition/backends to norl; domain evidence may feed it, but ECDEV cannot claim its cognition capabilities.','recommended_ownership':'Commerce source-time/provenance validation belongs to existing commerce.web-research adapter; generic memory/cognition remains outside this shard.','whole_donor_absorbed':False}
  if key.endswith('--zep'):
   review.update(actual_repository_purpose='Current Zep Cloud examples/integrations, a real ingestion SDK package, benchmarks, plus legacy CE Go server; not solely examples and not current Cloud server implementation.',runtime_vs_integration='ingestion/integrations invoke zep_cloud; legacy/src holds a server and PostgreSQL persistence with HTTP calls to a Graphiti service.',persistence_design='Legacy Bun/PostgreSQL models for sessions/messages/users; current ingestion delegates persistence to managed graph SDK.',temporal_model='Source-created episode timestamps and valid/invalid fact fields in SDK/legacy Graphiti DTOs; legacy database created/updated/deleted audit times are distinct.',graph_model='Current managed graph SDK forwarding; legacy Graphiti HTTP service DTOs and explicit docker dependency.',retrieval_model='Legacy session/memory APIs plus external Graphiti facts/search; Cloud benchmark uses SDK context retrieval.',ingestion_model='Loader -> transforms -> validated Episode -> Sequential/Batch SDK submitter; preview has no API calls.',external_services=['Zep Cloud SDK','Legacy PostgreSQL/pgvector','Legacy Graphiti service','Neo4j and OpenAI in legacy deployment','Optional Anthropic/OpenAI contextualizers'],local_vs_managed='Legacy CE can be deployed locally with external database/Graphiti/model dependencies; current Cloud integrations require managed-service access.')
   dependency_paths=['legacy/src/lib/graphiti/service_ce.go','legacy/docker-compose.ce.yaml','README.md']
   evidence=[]
   for path in dependency_paths:
    raw=locked(checkout,commit,path);evidence.append({'source_path':path,'commit_sha':commit,'sha256':hashlib.sha256(raw).hexdigest(),'blob_hash':subprocess.check_output(['git','-C',str(checkout),'rev-parse',commit+':'+path]).decode().strip()})
   census.dump(census.YN/'zep-dependency-graph.json',{'source_proven_relation':True,'edges':[{'from':'ECDEV commerce evidence continuity','to':'Zep episode/fact/provenance contract','decision':'STUDY_BOUNDARY_ONLY'},{'from':'Zep legacy CE graph memory','to':'getzep/graphiti','decision':'SEPARATE_OSS_DONOR_INTAKE','evidence':evidence},{'from':'ECDEV explicit-zone publication timestamp','to':'Graphiti ensure_utc','decision':'INDEPENDENT_NATIVE_SUBSET_WITH_ORACLE'},{'from':'Zep modern SDK integrations','to':'Zep Cloud','decision':'MANAGED_SERVICE_NOT_NATIVE'}],'legacy_service_version':'zepai/graphiti:0.3','current_graphiti_compatibility':'UNKNOWN_NOT_ASSUMED','runtime_dependencies_added':0})
  else:
   review.update(actual_repository_purpose='Local OSS temporal graph building/retrieval library plus server/MCP/examples; not Zep Cloud server.',runtime_vs_integration='graphiti_core contains the implementation; server and mcp_server are separate entrypoints within the donor.',persistence_design='Graph-driver abstraction with Neo4j default and optional FalkorDB/Neptune/Kuzu integrations; no donor database started.',temporal_model='EntityEdge distinguishes created_at, valid_at, invalid_at, expired_at, episode reference_time; UTC normalization is independent from truth classification.',graph_model='Episodes, entities, fact edges and community graph, with attributed episode IDs and entity resolution workflows.',retrieval_model='Cosine/BM25/BFS methods and RRF/MMR/distance/cross-encoder rerank choices; not calibrated commerce demand.',ingestion_model='Async episode extraction/resolution/persistence orchestrated through models, LLM/embedder clients and graph drivers.',external_services=['Configured graph database','OpenAI default model/embedding client','Optional Anthropic/Groq/Google/other models','Posthog telemetry dependency'],local_vs_managed='OSS core can run against selected local/remote graph and model services; Zep Cloud is not required by the chosen pure UTC helper.')
  name='zep' if key.endswith('--zep') else 'graphiti';census.dump(census.YN/(name+'-capability-review.json'),review);reviews[key]=review
  census.dump(base/'identity.json',record)
 census.dump(registry_path,registry)
 census.dump(census.YN/'memory-capability-graph.json',{'contracts':graph,'whole_donors_absorbed':0,'runtime_dependencies_added':0})
 print('Memory source contracts',len(graph),'donors',len(reviews))

if __name__=='__main__': main()
