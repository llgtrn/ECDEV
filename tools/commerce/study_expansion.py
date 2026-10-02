"""Narrow source-reviewed contracts, canonical donor registration and native capability graph."""
import json,pathlib,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'.venv/research'))
import census
REVIEWS=[
 ('apify--crawlee','crawl.deduplicate','packages/core/src/storages/request_queue.ts','addRequest',229,279,'uniqueKey request cache avoids duplicate backend writes; backend state and deferred transactions exist. ECDEV implements only bounded run-local normalized-URL deduplication.','commerce.research'),
 ('scrapinghub--extruct','extract.structured_data','extruct/_extruct.py','extract',22,84,'Syntax validation, configurable error handling and HTML tree parsing precede extraction. ECDEV implements strict JSON-LD only; microdata/RDFa and malformed-JS repair remain unknown/unimplemented.','commerce.web-research'),
 ('tkem--cachetools','cache.ttl','src/cachetools/__init__.py','TTLCache',451,501,'Monotonic expiration bounds cache lookup; expired keys behave as missing. ECDEV uses persistent wall-clock TTL and HTTP conditional revalidation, not donor LRU parity.','commerce.research'),
 ('dgtlmoon--changedetection.io','monitor.change_detection','changedetectionio/diff/__init__.py','render_diff',424,474,'Two snapshots and configured text/word policies produce a diff. ECDEV compares typed product price/stock/seller fields only, not generic text diff parity.','commerce.research'),
 ('encode--httpx','fetch.redirect','httpx/_client.py','_build_redirect_request',475,493,'Redirect method, URL, headers and streams are reconstructed separately. ECDEV owns bounded manual GET redirects and validates each target; session/auth transfer is absent.','commerce.web-research'),
 ('amzn--selling-partner-api-models','marketplace.catalog','models/catalog-items-api-model/catalogItems_2022-04-01.json','ItemSummary.itemName',1020,1040,'Official model examples scope summaries by marketplaceId with itemName/brand. ECDEV selects the supplied marketplace summary and leaves price, inventory and sales unknown. Live SP-API IO is not implemented.','commerce')]
def main():
 reg=json.loads((ROOT/'research/commerce/donors/registry.json').read_text(encoding='utf-8'));records={d['donor_id']:d for d in reg['donors']};graph=[]
 for donor,cap,path,symbol,start,end,contract,node in REVIEWS:
  r=records[donor];checkout=ROOT/'research/commerce/donors/checkouts'/donor;blob=census.git('rev-parse','HEAD:'+path,cwd=checkout)
  content=census.git('show','HEAD:'+path,cwd=checkout);assert symbol.split('.')[-1] in content
  row={'capability':cap,'status':'STUDIED','donor_id':donor,'commit_sha':r['commit_sha'],'source_path':path,'blob_hash':blob,'symbol':symbol,'line_start':start,'line_end':end,'behavior_contract':contract,'native_node':node,'oracle_parity':'NOT_CLAIMED','native_lifecycle':'EXPERIMENTAL','license':r['license']}
  if row['native_node']=='commerce.research':row['native_node']='commerce'
  graph.append(row)
 census.dump(ROOT/'research/commerce/expansion-capability-graph.json',{'schema_version':1,'capabilities':graph,'edges':[{'from':r['donor_id'],'relation':'SOURCE_REVIEWED_FOR','to':r['capability'],'evidence':r} for r in graph],'all_other_hypotheses':'CANDIDATE_SOURCE_INVENTORY_NOT_SEMANTIC_VERIFICATION'})
 d=ROOT/'.ynventa/declared/donors.rs';text=d.read_text(encoding='utf-8').rstrip();assert text.endswith(']');text=text[:-1]
 for r in reg['donors']:
  if 'key: '+json.dumps(r['donor_id']) in text or r['remote_status']!='VERIFIED_REMOTE':continue
  caps=[g for g in graph if g['donor_id']==r['donor_id']]
  decl='&['+','.join('Capability { key: '+json.dumps(g['capability'])+', required: true, spec: "research/commerce/expansion-capability-graph.json", replacement: Some('+json.dumps(g['native_node'])+'), maps_to: Some('+json.dumps('capability/'+g['capability'])+'), norl: NorlRelevance::Unresolved, proofs: &[] }' for g in caps)+']'
  # Partial semantic reviews cannot promote a whole donor to Censused.
  text+='Donor { key: '+json.dumps(r['donor_id'])+', name: '+json.dumps(r['name'])+', origin: '+json.dumps(r['repository_url'])+', license: '+json.dumps(r['license'])+', claimed: DonorState::Discovered, exception: Exception::None, packages: &[], source_paths: &[], capabilities: '+decl+', cutover: None, provenance: &["research/commerce/donors/census/'+r['donor_id']+'/identity.json"] },\n'
 d.write_text(text+']\n',encoding='utf-8')
 d=ROOT/'.ynventa/declared/repository.rs';text=d.read_text(encoding='utf-8')
 text=text.replace('provides: &["commerce.unit-economics", "commerce.intent-planning", "commerce.run-storage"]','provides: &["commerce.unit-economics", "commerce.intent-planning", "commerce.run-storage", "marketplace.catalog"]')
 if 'key: "commerce.web-research"' not in text:
  text=text.replace('], edges:', 'Node { key: "commerce.web-research", kind: NodeKind::Adapter, concept: Concept::Subsystem, name: "Native public web research", path: "adapter/web", canonical_path: "adapter/web", lifecycle: NodeLifecycle::Active, provides: &["fetch.redirect", "extract.structured_data", "fetch.http", "extract.product"], requires: &[], reuses: &[], inputs: &[], outputs: &[], lineage: &[] },\nNode { key: "commerce.research", kind: NodeKind::Domain, concept: Concept::Subsystem, name: "Cost-aware research", path: "domain/commerce/src/research.rs", canonical_path: "domain/commerce/src/research.rs", lifecycle: NodeLifecycle::Active, provides: &["crawl.deduplicate", "cache.ttl", "monitor.change_detection", "research.market"], requires: &[], reuses: &[], inputs: &[], outputs: &[], lineage: &[] },\n], edges:',1)
  # No nested physical owner: research remains a capability of the existing commerce node.
  start=text.index('Node { key: "commerce.research"');end=text.index('\n], edges:',start);text=text[:start]+text[end:]
  text=text.replace('"marketplace.catalog"]','"marketplace.catalog", "crawl.deduplicate", "cache.ttl", "monitor.change_detection", "research.market"]',1)
  text=text.replace('], edges: &[','], edges: &[Edge { from: "commerce.server", to: "commerce.web-research", kind: EdgeKind::DependsOn, scope: Scope::Runtime }, Edge { from: "commerce.web-research", to: "commerce", kind: EdgeKind::DependsOn, scope: Scope::Runtime },',1)
 d.write_text(text,encoding='utf-8')
 # Resolve the research capability owner to the existing canonical node.
 d=ROOT/'.ynventa/declared/donors.rs';d.write_text(d.read_text(encoding='utf-8').replace('Some("commerce.research")','Some("commerce")'),encoding='utf-8')
 d=ROOT/'.ynventa/declared/technologies.rs';text=d.read_text(encoding='utf-8').rstrip()
 for key,node,path,provides,proof in [('commerce.public-research','commerce.web-research','adapter/web/src/lib.rs',['fetch.http','fetch.redirect','extract.structured_data','extract.product'],'adapter/web/src/lib.rs::extraction_keeps_unknowns_and_minor_units'),('commerce.zero-paid-research','commerce','domain/commerce/src/research.rs',['research.market','crawl.deduplicate','cache.ttl','monitor.change_detection'],'apps/server/src/lib.rs::zero_paid_research_e2e'),('commerce.marketplace-ontology','commerce','domain/commerce/src/marketplace.rs',['marketplace.catalog'],'domain/commerce/src/marketplace.rs::amazon_mapping_selects_market_and_keeps_unknowns')]:
  if 'key: '+json.dumps(key) not in text:
   text=text[:-1]+'Technology { key: '+json.dumps(key)+', name: '+json.dumps(key)+', kind: TechnologyKind::Runtime, claimed: TechnologyLifecycle::Experimental, purpose: "Native subset with explicit unknowns; no full donor parity or extinction", implements: &['+','.join(json.dumps(p) for p in provides)+'], node: '+json.dumps(node)+', sources: &['+json.dumps(path)+'], invariants: &[], proofs: &[Proof { kind: ProofKind::Regression, locator: '+json.dumps(proof)+' }], lineage: &[], relations: &[], norl: NorlRelevance::Unresolved, claims: &[] },\n]'
 d.write_text(text+'\n',encoding='utf-8');print('Source-reviewed narrow contracts:',len(graph))
if __name__=='__main__':main()
