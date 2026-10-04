"""Bind bounded social implementations to existing canonical owners, not whole donors."""
import json,re,subprocess
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2];BASE=ROOT/'research/commerce'
def read(p):return json.loads(p.read_text(encoding='utf-8'))
def dump(p,v):p.write_text(json.dumps(v,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
def quoted(s):return json.dumps(s,ensure_ascii=False)
def main():
 graph=read(BASE/'social-capability-graph.json');registry=read(BASE/'donors/registry.json');records={d['donor_id']:d for d in registry['donors']}
 declarations=ROOT/'.ynventa/declared';path=declarations/'donors.rs';text=path.read_text(encoding='utf-8').rstrip();assert text.endswith(']');text=text[:-1]
 for donor in sorted({c['donor_id'] for c in graph['capabilities']}):
  record=records[donor];caps=[]
  text=re.sub(r'(Donor \{ key: '+re.escape(quoted(donor))+r'.*?license: )"[^"]*"',lambda m:m[1]+quoted(record['license']),text)
  for c in [c for c in graph['capabilities'] if c['donor_id']==donor]:
   native='social.rank-exposure' if c['symbol']=='calculate_news_weight' else 'social.thresholds' if c['symbol']=='evaluate_thresholds' else None
   c['native_replacement_status']='BOUNDED_NATIVE_ORACLE' if native else 'SOURCE_CONTRACT_REVIEWED_FULL_BEHAVIOR_UNREPLACED'
   c['native_capability']=native;c['oracle_status']='560_CASES_TWO_FAMILIES_NATIVE_TEST' if native else 'NOT_CLAIMED'
   c['native_implementation']='domain/commerce/src/social.rs' if native else None
   proof='&[Proof { kind: ProofKind::Parity, locator: "domain/commerce/src/social.rs::independent_social_donor_oracles" }]' if native else '&[]'
   caps.append('Capability { key: '+quoted(c['capability'])+', required: true, spec: "research/commerce/social-capability-graph.json", replacement: '+('Some("commerce")' if native else 'None')+', maps_to: '+('Some('+quoted('capability/'+native)+')' if native else 'None')+', norl: NorlRelevance::Unresolved, proofs: '+proof+' }')
  # Explicit unresolved obligation prevents reviewed slices from becoming whole-donor extinction.
  caps.append('Capability { key: '+quoted('social.'+donor.split('--')[1].lower()+'.remaining-whole-donor')+', required: true, spec: "research/commerce/social-capability-graph.json", replacement: None, maps_to: None, norl: NorlRelevance::Unresolved, proofs: &[] }')
  if 'key: '+quoted(donor) not in text:
   text+='Donor { key: '+quoted(donor)+', name: '+quoted(record['name'])+', origin: '+quoted(record['repository_url'])+', license: '+quoted(record['license'])+', claimed: DonorState::Registered, exception: Exception::None, packages: &[], source_paths: &[], capabilities: &['+','.join(caps)+'], cutover: None, provenance: &["research/commerce/donors/census/'+donor+'/identity.json", "research/commerce/social-license-review.json"] },\n'
 path.write_text(text+']\n',encoding='utf-8');dump(BASE/'social-capability-graph.json',graph)
 techs=[
 ('commerce.social-domain','commerce',['social.model','social.separation','social.link'],['domain/commerce/src/social.rs','domain/commerce/src/social/runtime.rs'],['domain/commerce/tests/social.rs::simulated_and_untrusted_states_cannot_be_observations','domain/commerce/tests/social.rs::entity_identity_is_conservative','apps/server/src/lib.rs::social_snapshot_mcp_projection_and_scenario_isolation']),
 ('commerce.social-trends','commerce',['social.snapshot','social.score','social.cluster','social.rank-exposure','social.thresholds'],['domain/commerce/src/social.rs'],['domain/commerce/src/social.rs::independent_social_donor_oracles','domain/commerce/tests/social.rs::temporal_windows_velocity_acceleration_transparency','domain/commerce/tests/social.rs::clustering_keeps_evidence_and_unknown_timestamps']),
 ('commerce.social-watch','commerce',['social.watch'],['domain/commerce/src/social/runtime.rs'],['domain/commerce/tests/social.rs::watch_and_simulation_boundary_persist_after_restart','domain/commerce/tests/social.rs::watch_policy_rejects_noise_failures_and_incomplete_disappearance']),
 ('commerce.social-public','commerce.web-research',['social.query'],['adapter/web/src/social.rs'],['adapter/web/src/social.rs::public_social_unknown_and_zero_provenance','adapter/web/src/social.rs::dates_and_hostile_payloads'])]
 path=declarations/'technologies.rs';text=path.read_text(encoding='utf-8').rstrip();assert text.endswith(']');text=text[:-1]
 for key,node,caps,sources,proofs in techs:
  if 'key: '+quoted(key) in text:continue
  text+='Technology { key: '+quoted(key)+', name: '+quoted(key)+', kind: TechnologyKind::Algorithm, claimed: TechnologyLifecycle::Experimental, purpose: "Independent bounded social evidence with explicit unknowns, fixture/live/cache scope and no full donor or demand forecast claim", implements: &['+','.join(map(quoted,caps))+'], node: '+quoted(node)+', sources: &['+','.join(map(quoted,sources))+'], invariants: &[], proofs: &['+','.join('Proof { kind: ProofKind::'+('Parity' if 'independent_social_donor_oracles' in p else 'Regression')+', locator: '+quoted(p)+' }' for p in proofs)+'], lineage: &[], relations: &[], norl: NorlRelevance::Unresolved, claims: &[] },\n'
 path.write_text(text+']\n',encoding='utf-8')
 path=declarations/'repository.rs';text=path.read_text(encoding='utf-8')
 for node in ['commerce','commerce.web-research']:
  pattern=r'(Node \{ key: '+re.escape(quoted(node))+r'.*?provides: &\[)(.*?)(\])';match=re.search(pattern,text);assert match
  caps=sorted({cap for _,n,cs,_,_ in techs if n==node for cap in cs});existing=match[2]
  addition=', '+', '.join(quoted(c) for c in caps if quoted(c) not in existing)
  text=text[:match.start(2)]+existing+addition+text[match.end(2):]
 path.write_text(text,encoding='utf-8')
 # Dependency manifests stay donor-only. Inventory exact direct requirements; locks/builds remain unresolved.
 reviews=[]
 for donor in sorted({c['donor_id'] for c in graph['capabilities']}):
  folder=BASE/'donors/census'/donor;files=[json.loads(l) for l in (folder/'files.jsonl').read_text(encoding='utf-8').splitlines()]
  dependencies=read(folder/'dependencies.json');requirements=[]
  for dep in dependencies:
   p=dep['source_path'];manifest=dep.get('manifest',{})
   for group in ['dependencies','devDependencies','peerDependencies','optionalDependencies']:
    for name,version in manifest.get(group,{}).items():requirements.append({'source_path':p,'name':name,'requirement':version,'role':group})
   for line in manifest.get('project',{}).get('dependencies',[]):requirements.append({'source_path':p,'requirement':line,'role':'DONOR_PYTHON_RUNTIME'})
   if p.endswith('requirements.txt'):
    raw=subprocess.check_output(['git','-C',str(BASE/'donors/checkouts'/donor),'show',records[donor]['commit_sha']+':'+p]).decode('utf-8')
    for i,line in enumerate(raw.splitlines()):
     if line.strip() and not line.lstrip().startswith('#'):requirements.append({'source_path':p,'line':i+1,'requirement':line.strip(),'role':'DONOR_REQUIREMENT_EXPRESSION_NOT_ECDEV_DEPENDENCY'})
  reviews.append({'donor_id':donor,'manifest_records':len(dependencies),'direct_requirement_expressions':requirements,'parse_unknown':[{'path':f['path'],'status':f.get('parse_status'),'reason':'Unsupported language or structural parser failure; source remains inventoried, not silently parsed'} for f in files if f['classification']=='FIRST_PARTY_SOURCE' and f.get('parse_status')!='PARSED'],'runtime_ecdev_dependencies_added':0,'remaining':'Transitive lock semantics, native runtime replacement and external services remain unresolved; manifest classification is not license clearance.'})
 dump(BASE/'social-dependency-review.json',{'reviews':reviews})
 print('Registered social donors: 7; source contracts: 43; native bounded replacements: 2; canonical technologies: 4')
if __name__=='__main__':main()
