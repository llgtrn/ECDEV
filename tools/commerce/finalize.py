"""Bind reviewed behavior, repository ownership and verifiable proofs to canonical declarations."""
import json,pathlib,re,sys
ROOT=pathlib.Path(__file__).resolve().parents[2];sys.path.insert(0,str(ROOT/'.venv/research'))
import census
def main():
 r=ROOT/'.ynventa/declared/repository.rs';text=r.read_text(encoding='utf-8')
 if 'key: "commerce.keepa"' not in text:
  node='Node { key: "commerce.keepa", kind: NodeKind::Adapter, concept: Concept::Subsystem, name: "Keepa boundary normalization", path: "adapter/keepa", canonical_path: "adapter/keepa", lifecycle: NodeLifecycle::Active, provides: &["commerce.keepa-current-value"], requires: &[], reuses: &[], inputs: &["finite integer JSON"], outputs: &["optional current value"], lineage: &["purahmanian--keepa-mcp"] },\n'
  text=text.replace('], edges:',node+'], edges:');r.write_text(text,encoding='utf-8')
 d=ROOT/'.ynventa/declared/donors.rs';text=d.read_text(encoding='utf-8')
 pattern=r'(key: "keepa.resolve-current"[^}]*?)replacement: None, maps_to: None'
 text,n=re.subn(pattern,r'\1replacement: Some("commerce.keepa"), maps_to: Some("capability/commerce.keepa-current-value")',text);assert n==1 or 'replacement: Some("commerce.keepa")' in text
 # Add the parity proof only to this specified capability.
 start=text.index('Capability { key: "keepa.resolve-current"');end=text.index(' }',start)
 cap=text[start:end].replace('proofs: &[]','proofs: &[Proof { kind: ProofKind::Parity, locator: "adapter/keepa/src/lib.rs::donor_oracle_integer_json_contract" }]')
 text=text[:start]+cap+text[end:];d.write_text(text,encoding='utf-8')
 entries=[
 ('commerce.economics','Algorithm','commerce',['commerce.unit-economics'],'Deterministic arithmetic in currency minor units; all fees supplied assumptions',['domain/commerce/src/economics.rs'],[('Regression','domain/commerce/src/economics.rs::known_unit_economics'),('Regression','domain/commerce/src/economics.rs::extreme_inputs_return_errors_without_panicking')]),
 ('commerce.planning','Algorithm','commerce',['commerce.intent-planning'],'Provider-independent capability DAG; no observations when providers unavailable',['domain/commerce/src/planner.rs'],[]),
 ('commerce.storage','Storage','commerce',['commerce.run-storage'],'SQLite WAL transactions; persisted modes; replay performs zero network calls',['domain/commerce/src/service.rs'],[('Regression','domain/commerce/src/service.rs::durable_runs_replay_without_network')]),
 ('commerce.server','Protocol','commerce.server',['commerce.mcp','commerce.http','commerce.events'],'One shared engine; official MCP SDK; local HTTP; durable event stream',['apps/server/src/lib.rs'],[]),
 ('commerce.observability','Runtime','commerce.web',['commerce.observability'],'HTTP/events projections without duplicate formulas',['apps/web/src/app.ts'],[]),
 ('commerce.donor-census','Parser','tools.commerce',['commerce.donor-census'],'Full clone; locked Git blobs; every tracked file classified; unknown parsing stays partial',['tools/commerce/census.py'],[]),
 ('commerce.keepa-current','Algorithm','commerce.keepa',['commerce.keepa-current-value'],'Only reviewed finite integer JSON contract; not a live provider or complete donor absorption',['adapter/keepa/src/lib.rs'],[('Parity','adapter/keepa/src/lib.rs::donor_oracle_integer_json_contract')])]
 tech='&[\n'
 for key,kind,node,caps,purpose,sources,proofs in entries:
  q=json.dumps;strings=lambda a:'&['+','.join(q(s) for s in a)+']'
  proof='&['+','.join('Proof { kind: ProofKind::'+kind+', locator: '+q(loc)+' }' for kind,loc in proofs)+']'
  tech+='Technology { key: '+q(key)+', name: '+q(key)+', kind: TechnologyKind::'+kind+', claimed: TechnologyLifecycle::Experimental, purpose: '+q(purpose)+', implements: '+strings(caps)+', node: '+q(node)+', sources: '+strings(sources)+', invariants: &[], proofs: '+proof+', lineage: &[], relations: &[], norl: NorlRelevance::Unresolved, claims: &[] },\n'
 tech+=']\n';(ROOT/'.ynventa/declared/technologies.rs').write_text(tech,encoding='utf-8')
 registry=json.loads((ROOT/'research/commerce/donors/registry.json').read_text(encoding='utf-8'))
 for donor in registry['donors']:donor['repository']=donor['repository_url']
 census.dump(ROOT/'research/commerce/donors/registry.json',registry)
 out=ROOT/'research/commerce/donors/census/purahmanian--keepa-mcp';caps=json.loads((out/'capabilities.json').read_text(encoding='utf-8'))
 cap=next(c for c in caps if c['capability_id']=='keepa.resolve-current')
 cap.update(native_implementation={'source_path':'adapter/keepa/src/lib.rs','symbol':'resolve_current','status':'RUST_REPLACEMENT','canonical_technology_state':'EXPERIMENTAL'},oracle_status='508_INTEGER_JSON_CASES_MATCHED',runtime_donor_source_dependency=False,absorption_status='EXPERIMENTAL_REPLACEMENT')
 census.dump(out/'capabilities.json',caps);census.dump(ROOT/'research/commerce/capabilities.json',caps)
 census.dump(ROOT/'research/commerce/oracle.json',{'capability':'keepa.resolve-current','cases':508,'matched':508,'mismatched':0,'contract':'Finite integer JSON values only','fixture':'adapter/keepa/tests/fixtures/keepa-current-oracle.json','proof':'adapter/keepa/src/lib.rs::donor_oracle_integer_json_contract','binary_evidence':'Canonical ynventa prove binds proof and replacement tree','provider_live_verification':'UNAVAILABLE','native_absorption':False,'donor_extinction':False})
 print('Canonical ownership, seven experimental technologies, and content-bound proof declarations written')
if __name__=='__main__':main()
