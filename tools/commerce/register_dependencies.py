"""Admit dependency upstreams from installed package metadata and verified Git remotes."""
import concurrent.futures,email,json,pathlib,re,subprocess,sys,tomllib
ROOT=pathlib.Path(__file__).resolve().parents[2];sys.path.insert(0,str(ROOT/'.venv/research'))
import census
def main():
 host=next(line.split(': ',1)[1] for line in subprocess.check_output(['rustc','-vV'],text=True).splitlines() if line.startswith('host: '))
 metadata=json.loads(subprocess.check_output(['cargo','metadata','--format-version','1','--offline','--filter-platform',host],cwd=ROOT))
 local={p['id']:p for p in metadata['packages'] if p['source'] is None}
 wanted={d['name'] for p in local.values() for d in p['dependencies'] if d['source']}
 candidates=[]
 for p in metadata['packages']:
  if p['name'] in wanted and p['source']:
   candidates.append({'ecosystem':'Cargo','package':p['name'],'version':p['version'],'candidate_url':p['repository'],'license':p['license'] or 'UNVERIFIED','scope':'RUNTIME','metadata_path':p['manifest_path'],'clone_status':'NOT_CLONED','role':'FOUNDATIONAL_DEPENDENCY'})
 for folder,scope in [('apps/web','BUILD'),('tests/commerce','TEST')]:
  m=json.loads((ROOT/folder/'package.json').read_text())
  for name in m.get('devDependencies',{}):
   p=json.loads((ROOT/folder/'node_modules'/name/'package.json').read_text());r=p['repository'];url=r['url'] if isinstance(r,dict) else r
   candidates.append({'ecosystem':'Npm','package':name,'version':p['version'],'candidate_url':url.removeprefix('git+'),'license':p.get('license','UNVERIFIED'),'scope':scope,'metadata_path':str(ROOT/folder/'node_modules'/name/'package.json'),'clone_status':'NOT_CLONED','role':'DEVELOPMENT_DEPENDENCY'})
 for name in ['tree-sitter','tree-sitter-language-pack','jsonschema','lxml','html-text','w3lib','lxml_html_clean']:
  file=next((ROOT/'.venv/research').glob(name.replace('-','_')+'-*.dist-info/METADATA'));p=email.message_from_string(file.read_text(encoding='utf-8'));links=p.get_all('Project-URL',[])
  link=next((s.split(', ',1)[1] for s in links if s.lower().startswith(('source,','repository,'))),None)
  if link is None:link=next((s.split(', ',1)[1] for s in links if s.lower().startswith('homepage,') and 'github.com' in s),p.get('Home-page'))
  lic=p.get('License-Expression') or p.get('License') or ('MIT' if any('MIT License' in s for s in p.get_all('Classifier',[])) else 'UNVERIFIED')
  candidates.append({'ecosystem':'Python','package':name,'version':p['Version'],'candidate_url':link,'license':lic,'scope':'BUILD','metadata_path':str(file),'clone_status':'NOT_CLONED','role':'RESEARCH_TOOL_DEPENDENCY'})
 def resolve(p):
  try:
   url=p['candidate_url'];assert url
   remote=census.git('ls-remote','--symref',url,'HEAD',timeout=90);assert re.search('[0-9a-f]{40}\\s+HEAD',remote)
   p.update(repository_url=url.removesuffix('.git').rstrip('/'),remote_status='VERIFIED_REMOTE',resolution_output=remote,resolved_at=census.now(),commit_sha=None,notes='Package metadata and remote verified; upstream source not cloned or semantically censused; no absorption claim.')
  except Exception as e:p.update(repository_url=None,remote_status='UNRESOLVED',error=str(e))
  return p
 with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:records=list(pool.map(resolve,candidates))
 census.dump(ROOT/'research/commerce/dependencies.json',{'schema_version':1,'packages':records,'runtime_packages':sum(p['scope']=='RUNTIME' for p in records),'native_dependency_extinction':False})
 assert all(p['remote_status']=='VERIFIED_REMOTE' for p in records), 'Unresolved dependencies retained; cannot declare them registered'
 # One canonical donor per upstream; package participation is explicit, no invented capability.
 groups={}
 for p in records:groups.setdefault(p['repository_url'],[]).append(p)
 declaration=ROOT/'.ynventa/declared/donors.rs';text=declaration.read_text(encoding="utf-8").rstrip();assert text.endswith(']');text=text[:-1]
 # Idempotent update of support records, retaining the reviewed commerce donor declarations.
 text='\n'.join(line for line in text.splitlines() if not line.startswith('Donor { key: "dependency--'))+'\n'
 for url,items in sorted(groups.items()):
  key='dependency--'+re.sub('[^a-z0-9]+','-',url.lower().removeprefix('https://')).strip('-')
  packages='&['+','.join('Package { ecosystem: Ecosystem::'+p['ecosystem']+', name: '+json.dumps(p['package'])+' }' for p in items)+']'
  text+='Donor { key: '+json.dumps(key)+', name: '+json.dumps(items[0]['package'])+', origin: '+json.dumps(url)+', license: '+json.dumps(items[0]['license'])+', claimed: DonorState::Registered, exception: Exception::None, packages: '+packages+', source_paths: &[], capabilities: &[], cutover: None, provenance: &["research/commerce/dependencies.json"] },\n'
 declaration.write_text(text+']\n',encoding='utf-8',newline='\n');print('Verified dependency packages:',len(records),'unique upstreams:',len(groups),'runtime packages:',sum(p['scope']=='RUNTIME' for p in records))
if __name__=='__main__':main()
