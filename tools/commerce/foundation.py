"""Generate ECDEV declarations and schemas, without modifying canonical protocol code."""
import hashlib,json,pathlib,re,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'.venv/research'))
import census

def write(path,value):
 census.dump(ROOT/path,value)
def schema(properties,required=None):
 return {'$schema':'https://json-schema.org/draft/2020-12/schema','type':'object','properties':properties,'required':required or list(properties),'additionalProperties':False}
def field(type_,**kwargs): return {'type':type_,**kwargs}
def main():
 # Re-census the immutable local clones, no network or donor execution.
 registry=json.loads((ROOT/'research/commerce/donors/registry.json').read_text())
 for record in registry['donors']:
  census.census(record,ROOT/'research/commerce/donors/checkouts'/record['donor_id'])
  license_text='\n'.join(x['text'] for x in json.loads((ROOT/'research/commerce/donors/census'/record['donor_id']/'license.json').read_text())['files'])
  if 'Permission is hereby granted, free of charge' in license_text: record['license']='MIT'
  elif 'Apache License' in license_text and 'Version 2.0' in license_text: record['license']='Apache-2.0'
  record['donor_type']='TYPE_C' if record['donor_id'].startswith('amzscout-corp') else 'TYPE_A'
  record['notes'].append('Repository is a research donor; hosted live backend remains external. No native absorption claimed.')
  census.dump(ROOT/'research/commerce/donors/census'/record['donor_id']/'identity.json',record)
 write('research/commerce/donors/registry.json',registry)
 write('research/commerce/capabilities.json',[])
 money=['selling_price','product_cost','packaging','sample_amortization','freight','insurance','duty','import_tax','brokerage','prep','label','fulfillment_fee','referral_bps','storage_fee','payment_cost','ppc','returns','fx_cost','reserve','units','fixed_launch_cost','lead_time_days','inventory_days','payout_days','supplier_credit_days']
 scenario={k:field('integer',minimum=0) for k in money};scenario['selling_price']['minimum']=1;scenario['units']['minimum']=1;scenario['referral_bps']['maximum']=10000
 scenario.update(currency=field('string',pattern='^[A-Z]{3}$'),fulfillment=field('string',enum=['FBA','FBM','EPROLO_DROPSHIP','THREE_PL','BULK_IMPORT','FACTORY_DIRECT']))
 write('tools/commerce/schemas/scenario.schema.json',schema(scenario,['currency','fulfillment','selling_price','product_cost','fulfillment_fee','referral_bps','units']))
 intent={k:field('integer',minimum=1) for k in ['capital','min_price','max_price','max_weight_g','max_inventory_per_sku']}
 intent.update(market=field('string',minLength=1),currency=field('string',pattern='^[A-Z]{3}$'),minimum_margin_bps=field('integer',minimum=0,maximum=10000),positive_trend=field('boolean'),exclude_regulated=field('boolean'))
 write('tools/commerce/schemas/intent.schema.json',schema(intent,[k for k in intent if k not in ['positive_trend','exclude_regulated']]))
 nullable=['repository_url','resolved_remote_url','default_branch','commit_sha','commit_time']
 donor={k:field(['string','null']) for k in nullable}
 donor.update({k:field('string') for k in ['donor_id','candidate_identifier','remote_status','clone_status','donor_type','license','absorption_status','oracle_status','extinction_status']})
 donor['runtime_dependency']=field('boolean')
 ds=schema(donor);ds['additionalProperties']=True
 ds['allOf']=[{'if':{'properties':{'remote_status':{'const':'VERIFIED_REMOTE'}}},'then':{'properties':{'repository_url':field('string',pattern='^https://'),'resolved_remote_url':field('string')}}}]
 write('tools/commerce/schemas/donor.schema.json',ds)
 counts=['total_files','classified_files','first_party_source_files','source_parsed','parse_unknown','unknown_files','tests','fixtures','symbols']
 cs=schema({'donor_id':field('string'),'commit_sha':field('string',pattern='^[0-9a-f]{40}$'),'status':field('string',enum=['CENSUS_PARTIAL','CENSUS_COMPLETE']),**{k:field('integer',minimum=0) for k in counts}});cs['additionalProperties']=True
 write('tools/commerce/schemas/census.schema.json',cs)
 ev=schema({**{k:field('string',minLength=1) for k in ['id','source_type','provider','external_source','market','timestamp','retrieved_at','unit','run_id']},'mode':field('string',enum=['LIVE','CACHED','REPLAY','FIXTURE','SIMULATED','INFERRED']),'raw_hash':field('string',pattern='^[0-9a-fA-F]{64}$'),'query':{},'normalized_value':{},'currency':field(['string','null']),'confidence':field('number',minimum=0,maximum=1),'freshness_seconds':field('integer',minimum=0),'cost_minor':field(['integer','null'],minimum=0)})
 write('tools/commerce/schemas/evidence.schema.json',ev)
 write('tools/commerce/schemas/metrics.schema.json',{'$schema':'https://json-schema.org/draft/2020-12/schema','type':'object','additionalProperties':field('integer',minimum=0)})
 # Canonical graph ownership, role layering and semantic capabilities.
 nodes=[('ynventa.ecdev','Ynventa','Subsystem','.ynventa',[]),('commerce','Domain','Subsystem','domain/commerce',['commerce.unit-economics','commerce.intent-planning','commerce.run-storage']),('commerce.server','Application','Service','apps/server',['commerce.mcp','commerce.http','commerce.events']),('commerce.web','Application','Interface','apps/web',['commerce.observability']),('tools.commerce','Tool','Subsystem','tools/commerce',['commerce.donor-census']),('research.commerce','Research','Dataset','research/commerce',[]),('commerce.tests','Test','Evaluation','tests/commerce',[])]
 def q(x): return json.dumps(x)
 def strings(values): return '&['+', '.join(q(x) for x in values)+']'
 decl='Repository { system: "chronica", shard: "ecdev", name: "ECDEV", origin: "llgtrn/ECDEV", nodes: &[\n'
 for key,kind,concept,path,caps in nodes:
  decl+=f'Node {{ key: {q(key)}, kind: NodeKind::{kind}, concept: Concept::{concept}, name: {q(key)}, path: {q(path)}, canonical_path: {q(path)}, lifecycle: NodeLifecycle::Active, provides: {strings(caps)}, requires: &[], reuses: &[], inputs: &[], outputs: &[], lineage: &[] }},\n'
 decl+='], edges: &[Edge { from: "commerce.server", to: "commerce", kind: EdgeKind::DependsOn, scope: Scope::Runtime }, Edge { from: "commerce.web", to: "commerce", kind: EdgeKind::Calls, scope: Scope::Runtime }] }\n'
 (ROOT/'.ynventa/declared/repository.rs').write_text(decl)
 donors='&[\n'
 for r in registry['donors']:
  # DISCOVERED until semantic census and source-backed behavior mapping exist.
  donors+=f'Donor {{ key: {q(r["donor_id"])}, name: {q(r["name"])}, origin: {q(r["repository_url"] or "")}, license: {q(r["license"])}, claimed: DonorState::Discovered, exception: Exception::None, packages: &[], source_paths: &[], capabilities: &[], cutover: None, provenance: &[{q("research/commerce/donors/census/"+r["donor_id"]+"/identity.json")}] }},\n'
 donors+=']\n';(ROOT/'.ynventa/declared/donors.rs').write_text(donors)
 write('research/commerce/protocol-origin.json',{'repository_url':'https://github.com/llgtrn/.Ynventa-','commit_sha':census.git('rev-parse','HEAD',cwd=ROOT/'research/commerce/donors/checkouts/llgtrn--Ynventa'),'role':'CANONICAL_PROTOCOL_REFERENCE','method':'full git clone --recurse-submodules; canonical migrate scaffold','shard_registration':'ECDEV_NOT_IN_CANONICAL_CLOSED_SHARD_SET','copied_subsystem':'BYTE_IDENTICAL_REQUIRED','evidence_paths':['.ynventa/src/protocol.rs','.ynventa/src/repository/mod.rs','.ynventa/src/declare/decl.rs']})
 print('Generated schemas, recensus, and canonical ECDEV declarations')
if __name__=='__main__':main()
