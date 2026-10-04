"""Research-only donor acquisition and conservative structural census; never runtime."""
from __future__ import annotations
import argparse, ast, collections, datetime, hashlib, json, pathlib, re, subprocess
from concurrent.futures import ThreadPoolExecutor

ROOT = pathlib.Path(__file__).resolve().parents[2]
YN = ROOT / 'research/commerce'
SEEDS = ['mrkooblu/semrush-mcp', 'purahmanian/keepa-mcp',
 'purahmanian/junglescout-mcp', 'purahmanian/google-trends-mcp',
 'amzscout-corp/amzscout-skill-mcp', 'coaxon/amazon-mcp',
 'aws-samples/sample-amazon-spapi-mcp-server', 'microsoft/playwright-mcp',
 'firecrawl/firecrawl-mcp-server', 'apify/apify-mcp-server']
LANG = {'.py':'python','.js':'javascript','.mjs':'javascript','.cjs':'javascript','.jsx':'javascript','.ts':'typescript',
 '.tsx':'tsx','.rs':'rust','.go':'go','.java':'java','.cs':'csharp',
 '.sh':'bash','.c':'c','.h':'c','.cpp':'cpp','.rb':'ruby','.php':'php','.css':'css','.html':'html','.scss':'scss',
 '.mts':'typescript','.pyi':'python','.ex':'elixir','.exs':'elixir','.jsm':'javascript','.bash':'bash',
 '.ps1':'powershell','.m':'objc','.graphql':'graphql','.sql':'sql'}
MANIFESTS = {'package.json','package-lock.json','pnpm-lock.yaml','yarn.lock',
 'Cargo.toml','Cargo.lock','pyproject.toml','poetry.lock','uv.lock','requirements.txt',
 'go.mod','go.sum','pom.xml','build.gradle','Dockerfile','Makefile'}
FACETS = {'network-behavior':r'https?://|fetch\(|requests\.|httpx|axios',
 'auth':r'(?i)authorization|api_key|apikey|access_token|oauth|bearer',
 'rate-limits':r'(?i)rate.?limit|quota|429|throttl',
 'errors':r'(?i)raise |throw |except |catch\s*\(|isError',
 'retry-semantics':r'(?i)retry|backoff|retries',
 'pagination':r'(?i)pagination|next.?token|cursor|page_size|offset',
 'storage':r'(?i)sqlite|redis|localStorage|writeFile|open\(',
 'process-model':r'(?i)subprocess|spawn\(|exec\(|stdio|StdioServerTransport',
 'protocols':r'(?i)StreamableHTTP|SSEServerTransport|FastMCP|McpServer|jsonrpc'}
def safe_excerpt(line):
 for pattern in [r"https://hooks\.slack\.com/services/[^\s\"'<>\\)]+",r'AKIA[0-9A-Z]{16}',r'(?<![A-Za-z0-9])gh[pousr]_[A-Za-z0-9]{25,}',r'(?<![A-Za-z0-9])sk-(?:proj-)?[A-Za-z0-9_-]{25,}',r'-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----']:
  line=re.sub(pattern,'[REDACTED_CREDENTIAL_LIKE_LITERAL]',line)
 return line[:400]

def now(): return datetime.datetime.now(datetime.timezone.utc).isoformat()
def git(*args, cwd=None, timeout=300):
 p = subprocess.run(['git','-c','core.longpaths=true','-c','http.version=HTTP/1.1',*args],cwd=cwd,capture_output=True,timeout=timeout)
 if p.returncode: raise RuntimeError(p.stderr.decode('utf-8','replace')[-3000:])
 return p.stdout.decode('utf-8','replace').strip()
def dump(path, value):
 path.parent.mkdir(parents=True,exist_ok=True)
 path.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
def jsonl(path, rows):
 path.write_text(''.join(json.dumps(x,ensure_ascii=False)+'\n' for x in rows),encoding='utf-8')

def classify(path, data):
 parts = path.lower().split('/'); name = parts[-1]; suffix = pathlib.Path(name).suffix
 if any(p in {'vendor','vendored','node_modules','third_party','third-party'} for p in parts): return 'VENDORED'
 if any(p in {'dist','generated','__generated__'} for p in parts) or name.endswith(('.min.js','.map')): return 'GENERATED'
 if b'\0' in data: return 'BINARY'
 if any(p in {'fixtures','fixture','__fixtures__','snapshots','__snapshots__'} for p in parts): return 'FIXTURE'
 if name.startswith(('test_','test.')) or '.test.' in name or '.spec.' in name or name.endswith('_test.go') or any(p in {'test','tests','__tests__'} for p in parts): return 'TEST'
 if name in {x.lower() for x in MANIFESTS} or name.startswith(('docker-compose','requirements','dockerfile')) or '.github' in parts or '.husky' in parts: return 'BUILD'
 if suffix in LANG: return 'FIRST_PARTY_SOURCE'
 if data.startswith(b'#!') or name.endswith(('_bash_completion','_zsh_completion')): return 'FIRST_PARTY_SOURCE'
 if suffix in {'.pxd','.pyx','.idl','.pch','.fish'}: return 'FIRST_PARTY_SOURCE'
 if name in {'gemfile','rakefile','vagrantfile','pkgbuild','portfile','gradlew'}: return 'BUILD'
 if suffix in {'.gemspec','.kts','.nuspec','.mn','.pbxproj','.xcscheme','.vcxproj','.filters','.user','.diff','.bat','.tpl','.jinja2'}: return 'BUILD'
 if name in {'cname','version','build_number','expected_builds','procfile'} or suffix in {'.dist','.pro','.webmanifest','.types'}: return 'CONFIG'
 if name in {'authors','news'} or suffix in {'.license','.cff'}: return 'DOC'
 if suffix in {'.excalidraw','.xib','.rtf','.jp2'}: return 'ASSET'
 if 'schema' in name and suffix in {'.json','.yaml','.yml'}: return 'SCHEMA'
 if suffix in {'.md','.mdx','.rst','.txt','.toc','.todo','.1'} or name.startswith(('license','copying','notice')): return 'DOC'
 if suffix in {'.mustache','.tmpl','.csproj','.sln','.build','.xcconfig','.gradle'}: return 'BUILD'
 if suffix in {'.list','.jsonc','.typed','.properties','.http','.example'}: return 'CONFIG'
 if suffix in {'.po','.pot','.csv','.tsv','.pem','.crt'}: return 'ASSET'
 if suffix in {'.astro','.ipynb','.ps1','.m','.graphql'}: return 'FIRST_PARTY_SOURCE'
 if suffix in {'.json','.toml','.yaml','.yml','.ini','.cfg','.conf','.plist','.service','.in','.xml','.lock'} or name.startswith('.') or name.endswith(('rc','ignore')): return 'CONFIG'
 if suffix=='.patch': return 'BUILD'
 if suffix in {'.png','.jpg','.jpeg','.gif','.svg','.ico','.webp','.pdf','.woff','.woff2','.mp4','.zip'}: return 'ASSET'
 if suffix in {'.css','.scss','.html','.svelte','.vue','.sql'}: return 'FIRST_PARTY_SOURCE'
 return 'UNKNOWN'

def source_language(path, content):
 language=LANG.get(pathlib.Path(path).suffix)
 if language is None:
  first=content.split(b'\n',1)[0].decode('utf-8','replace')
  if first.startswith('#!'):
   if re.search(r'\bpython[0-9.]*\b',first):language='python'
   elif re.search(r'\b(?:bash|sh|zsh)\b',first):language='bash'
  if path.endswith(('_bash_completion','_zsh_completion')):language='bash'
 return language

def parse_source(path, content):
 language=source_language(path,content); symbols=[]; imports=[]
 if language is None: return 'PARSE_UNKNOWN',symbols,imports
 try:
  from tree_sitter_language_pack import get_parser
  tree=get_parser(language).parse(content)
  def walk(node):
   if node.type in {'function_definition','function_declaration','method_definition','class_definition','class_declaration','function_item','struct_item','interface_declaration','type_alias_declaration','enum_declaration'}:
    n=node.child_by_field_name('name')
    if n: symbols.append({'source_path':path,'symbol':n.text.decode('utf-8','replace'),'kind':node.type,'line_start':node.start_point.row+1,'line_end':node.end_point.row+1})
   if node.type in {'import_statement','import_from_statement','use_declaration','import_declaration','import_spec'}:
    imports.append({'source_path':path,'line':node.start_point.row+1,'text':node.text.decode('utf-8','replace')})
   for c in node.children: walk(c)
  walk(tree.root_node)
  return ('PARSE_UNKNOWN' if tree.root_node.has_error else 'PARSED'),symbols,imports
 except Exception as e: return 'PARSE_UNKNOWN',symbols,[{'parse_error':str(e)}]

def census(record, checkout):
 out=YN/'donors/census'/record['donor_id']; out.mkdir(parents=True,exist_ok=True)
 tracked=git('ls-files','--stage','-z',cwd=checkout).split('\0')
 rows=[]; symbols=[]; imports=[]; deps=[]; licenses=[]; tests=[]; fixtures=[]
 facets={k:[] for k in FACETS}; submodules=[]; entrypoints=[]
 for item in tracked:
  if not item: continue
  metadata,path=item.split('\t',1); mode,blob,_=metadata.split()
  if mode=='160000':
   submodules.append({'path':path,'commit_sha':blob,'status':'SUBMODULE_CENSUS_PENDING'}); continue
  # Read locked Git blobs, never dereference repository symlinks or execute donor code.
  content=subprocess.check_output(['git','cat-file','blob',blob],cwd=checkout)
  kind=classify(path,content); lang=LANG.get(pathlib.Path(path).suffix)
  row={'path':path,'classification':kind,'language':lang,'git_mode':mode,'blob_hash':blob,'sha256':hashlib.sha256(content).hexdigest(),'bytes':len(content)}
  if kind in {'FIRST_PARTY_SOURCE','TEST'}:
   row['parse_status'],sy,im=parse_source(path,content); symbols+=sy; imports+=im
  text=content.decode('utf-8','replace')
  if kind not in {'VENDORED','GENERATED','BINARY','ASSET'}:
   for facet,pattern in FACETS.items():
    for no,line in enumerate(text.splitlines(),1):
     if re.search(pattern,line): facets[facet].append({'source_path':path,'line_start':no,'line_end':no,'blob_hash':blob,'evidence_type':'LEXICAL_CANDIDATE','excerpt':safe_excerpt(line)})
  if pathlib.Path(path).name in MANIFESTS or pathlib.Path(path).name.startswith('requirements'):
   dep={'source_path':path,'blob_hash':blob,'status':'INVENTORIED','roles':[]}
   try:
    if path.endswith('package.json'):
     m=json.loads(text); dep['manifest']=m
     for key,role in [('dependencies','RUNTIME'),('devDependencies','DEVELOPMENT'),('optionalDependencies','OPTIONAL'),('peerDependencies','PEER')]:
      for name,version in m.get(key,{}).items(): dep['roles'].append({'name':name,'version':version,'role':role})
     if m.get('bin'): entrypoints.append({'source_path':path,'bin':m['bin'],'scripts':m.get('scripts',{})})
     dep['status']='PARSED'
    elif path.endswith('.toml'):
     import tomllib
     dep['manifest']=tomllib.loads(text); dep['status']='PARSED'
    elif path.endswith('.json'): dep['manifest']=json.loads(text); dep['status']='PARSED'
   except Exception as e: dep['status']='PARSE_UNKNOWN'; dep['error']=str(e)
   deps.append(dep)
  if pathlib.Path(path).name.lower().startswith(('license','copying','notice')):
   licenses.append({'source_path':path,'sha256':row['sha256'],'blob_hash':blob,'text':text,'license':'UNVERIFIED'})
  if kind=='TEST': tests.append(row)
  if kind=='FIXTURE': fixtures.append(row)
  rows.append(row)
 counts=collections.Counter(x['classification'] for x in rows)
 source=[x for x in rows if x['classification']=='FIRST_PARTY_SOURCE']
 unknown=[x for x in source if x.get('parse_status')!='PARSED']
 summary={'donor_id':record['donor_id'],'commit_sha':record['commit_sha'],'status':'CENSUS_PARTIAL',
 'total_files':len(rows),'classified_files':len(rows)-counts['UNKNOWN'],'classification_counts':dict(counts),
 'first_party_source_files':len(source),'source_parsed':len(source)-len(unknown),'parse_unknown':len(unknown),
 'unknown_files':counts['UNKNOWN'],'tests':len(tests),'fixtures':len(fixtures),'symbols':len(symbols),
 'semantic_review':'PENDING','submodules':submodules,
 'limitations':['Structural parsing is not semantic behavior verification.','Lexical matches are candidates, never verified capabilities.','Non-JSON/TOML manifests and lockfiles require semantic dependency review.','API surfaces and external service behavior require source-backed manual review.']}
 record.update(tree_status='TREE_CENSUSED' if counts['UNKNOWN']==0 and not submodules else 'PARTIAL',source_census_status='STRUCTURAL_PARSED' if not unknown else 'PARTIAL',dependency_census_status='PARTIAL',test_census_status='INVENTORIED',protocol_census_status='PARTIAL',capability_census_status='PARTIAL',file_count=len(rows),primary_languages=dict(collections.Counter(x['language'] for x in rows if x['language'])),license_paths=[x['source_path'] for x in licenses],license_hashes=[x['sha256'] for x in licenses],repository_size=sum(x['bytes'] for x in rows))
 dump(out/'identity.json',record); dump(out/'tree.json',{'files':[x['path'] for x in rows],'submodules':submodules})
 jsonl(out/'files.jsonl',rows); jsonl(out/'symbols.jsonl',symbols); jsonl(out/'evidence.jsonl',[])
 dump(out/'languages.json',record['primary_languages']); dump(out/'dependencies.json',deps)
 dump(out/'dependency-graph.json',{'status':'PARTIAL','imports':imports,'call_edges':[]})
 dump(out/'entrypoints.json',{'status':'PARTIAL','candidates':entrypoints})
 dump(out/'capabilities.json',[]); dump(out/'mcp-surface.json',{'status':'UNVERIFIED','tools':[]})
 dump(out/'external-services.json',{'status':'PARTIAL','candidate_index':'network-behavior.json','candidate_count':len(facets['network-behavior'])})
 for name,items in facets.items(): dump(out/(name+'.json'),{'status':'CANDIDATE_EVIDENCE','candidates':items})
 dump(out/'tests.json',{'status':'INVENTORIED','files':tests,'executed':False}); dump(out/'fixtures.json',fixtures)
 dump(out/'build.json',{'status':'PARTIAL','files':[x for x in rows if x['classification']=='BUILD']})
 dump(out/'license.json',{'status':'UNVERIFIED','files':licenses})
 dump(out/'risks.json',{'status':'PARTIAL','issues':summary['limitations'],'parse_unknown':[x['path'] for x in unknown]})
 dump(out/'summary.json',summary)
 return record

def acquire(identity):
 donor_id=identity.replace('/','--'); proposed='https://github.com/'+identity+'.git'
 record={'schema_version':1,'donor_id':donor_id,'candidate_identifier':'github:'+identity,'name':identity.split('/')[-1],
 'repository_url':None,'resolved_remote_url':None,'default_branch':None,'commit_sha':None,'commit_time':None,'retrieved_at':now(),
 'remote_status':'UNRESOLVED','clone_status':'NOT_CLONED','donor_type':'UNCLASSIFIED','license':'UNVERIFIED',
 'license_paths':[],'license_hashes':[],'primary_languages':{},'repository_size':None,'file_count':0,
 'tree_status':'NOT_STARTED','source_census_status':'NOT_STARTED','dependency_census_status':'NOT_STARTED','test_census_status':'NOT_STARTED','protocol_census_status':'NOT_STARTED','capability_census_status':'NOT_STARTED',
 'external_services':[],'capabilities_total':0,'capabilities_verified':0,'capabilities_documented_only':0,'capabilities_unknown':0,
 'absorption_status':'NOT_STARTED','oracle_status':'NOT_STARTED','runtime_dependency':False,'extinction_status':'NOT_ELIGIBLE','notes':[]}
 try:
  remote=git('ls-remote','--symref',proposed,'HEAD',timeout=90)
  branch=re.search(r'ref: refs/heads/(.+)\s+HEAD',remote); head=re.search(r'([0-9a-f]{40})\s+HEAD',remote)
  if not branch or not head: raise RuntimeError('Remote has no resolvable default branch and HEAD')
  record.update(repository_url=proposed.removesuffix('.git'),resolved_remote_url=proposed,default_branch=branch[1],remote_status='VERIFIED_REMOTE',remote_resolution={'mechanism':'git ls-remote --symref','output':remote,'resolved_at':now()})
  checkout=YN/'donors/checkouts'/donor_id
  if not checkout.exists(): git('clone','--recurse-submodules',proposed,str(checkout),timeout=1800)
  if git('rev-parse','--is-shallow-repository',cwd=checkout)!='false': raise RuntimeError('Shallow clone rejected')
  if git('remote','get-url','origin',cwd=checkout)!=proposed: raise RuntimeError('Existing checkout origin mismatch')
  record.update(clone_status='FULL_CLONE',commit_sha=git('rev-parse','HEAD',cwd=checkout),commit_time=git('show','-s','--format=%cI','HEAD',cwd=checkout),clone_method='git clone --recurse-submodules (no depth/filter/sparse)',history_commits=int(git('rev-list','--count','HEAD',cwd=checkout)))
  record=census(record,checkout)
 except Exception as e: record['notes'].append(str(e))
 dump(YN/'donors/census'/donor_id/'identity.json',record)
 print(donor_id,record['remote_status'],record['clone_status'],record['file_count'],flush=True)
 return record

def main():
 p=argparse.ArgumentParser(); p.add_argument('--jobs',type=int,default=3); p.add_argument('--only',action='append'); args=p.parse_args()
 seeds=args.only or SEEDS
 with ThreadPoolExecutor(max_workers=args.jobs) as pool: records=list(pool.map(acquire,seeds))
 dump(YN/'donors/registry.json',{'schema_version':1,'generated_at':now(),'donors':records})
 summaries=[]
 for r in records:
  file=YN/'donors/census'/r['donor_id']/'summary.json'
  if file.exists(): summaries.append(json.loads(file.read_text(encoding='utf-8')))
 metrics={'donor_candidates':len(records),'remote_verified':sum(r['remote_status']=='VERIFIED_REMOTE' for r in records),'unresolved':sum(r['remote_status']=='UNRESOLVED' for r in records),'full_clones':sum(r['clone_status']=='FULL_CLONE' for r in records),
 **{k:sum(s[k] for s in summaries) for k in ['total_files','classified_files','first_party_source_files','source_parsed','parse_unknown','unknown_files','tests','fixtures']},
 'census_complete':0,'capabilities_verified':0,'native_absorbed':0,'oracle_verified':0,'extinct':0,'runtime_donor_dependencies':0}
 dump(YN/'metrics.json',metrics)
 print(json.dumps(metrics,indent=2))
if __name__=='__main__': main()
