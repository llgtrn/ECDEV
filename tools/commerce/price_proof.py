"""Register bounded price normalization and inspect live page enrichment honestly."""
import ast,hashlib,json,pathlib,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[2];COMMIT='64e213a46a40473ba4f8aa3b249917fdc64d8a16'
def read(path):return json.loads((ROOT/path).read_text(encoding='utf-8'))
def dump(path,value):(ROOT/path).write_text(json.dumps(value,indent=2,ensure_ascii=False)+'\n',encoding='utf-8',newline='\n')
def register():
 fixture='adapter/web/tests/fixtures/price-number.json';oracle=read(fixture);assert len(oracle['cases'])==1235
 records=read('research/commerce/capabilities.json');records=[r for r in records if r['capability_id']!='extract.price-number']
 records.append(dict(capability_id='extract.price-number',donor_id='scrapinghub--price-parser',status='VERIFIED',evidence=oracle['source_evidence'],native_implementation='adapter/web/src/price.rs',native_status='EXPERIMENTAL_BOUNDED_REPLACEMENT',oracle_status='1235_PRICE_NUMBER_CASES_MATCHED',oracle_cases=1235,commit_sha=COMMIT,fixture=fixture,fixture_sha256=hashlib.sha256((ROOT/fixture).read_bytes()).hexdigest(),license='BSD-3-Clause',scope=oracle['scope'],runtime_donor_source_dependency=False,remaining=['Full currency symbol and first-price text selection APIs','Nonfinite and non-ASCII numeric alphabets','Unbounded scientific exponents intentionally unavailable','Full donor semantic census and absorption']))
 dump('research/commerce/capabilities.json',records)
 for path in ['research/commerce/donors/registry.json','research/commerce/donors/census/scrapinghub--price-parser/identity.json']:
  record=read(path)
  target=next(d for d in record['donors'] if d['donor_id']=='scrapinghub--price-parser') if 'donors' in record else record
  target['license']='BSD-3-Clause';target['license_review']='LOCKED_THREE_CONDITION_BSD_NOTICE_RETAINED'
  dump(path,record)
 p=ROOT/'.ynventa/declared/donors.rs';lines=p.read_text(encoding='utf-8').splitlines()
 for i,line in enumerate(lines):
  if 'key: "scrapinghub--price-parser"' in line:
   line=line.replace('license: "UNVERIFIED"','license: "BSD-3-Clause"')
   line=line.replace('capabilities: &[]','capabilities: &[Capability { key: "extract.price-number", required: false, spec: "research/commerce/price-page-proof.json", replacement: Some("commerce.web-research"), maps_to: Some("capability/extract.price-number"), norl: NorlRelevance::Unresolved, proofs: &[Proof { kind: ProofKind::Parity, locator: "adapter/web/src/price.rs::locked_price_parser_numeric_oracle" }] }]')
   line=line.replace('key: "extract.price-number", name: "Bounded exact formatted decimal normalization", mode: CapabilityMode::Replace, required: false, native: Some("commerce.web-research")','key: "extract.price-number", required: false, spec: "research/commerce/price-page-proof.json", replacement: Some("commerce.web-research"), maps_to: Some("capability/extract.price-number"), norl: NorlRelevance::Unresolved').replace(' }], routes: &[] }]',' }] }]')
   lines[i]=line
 p.write_text('\n'.join(lines)+'\n',encoding='utf-8',newline='\n')
 p=ROOT/'.ynventa/declared/repository.rs';text=p.read_text(encoding='utf-8')
 if '"extract.price-number"' not in text:text=text.replace('provides: &["extract.declared-document"','provides: &["extract.price-number", "extract.page-product", "extract.declared-document"')
 p.write_text(text,encoding='utf-8',newline='\n')
 p=ROOT/'.ynventa/declared/technologies.rs';text=p.read_text(encoding='utf-8').rstrip();assert text.endswith(']')
 if 'commerce.price-number' not in text:
  entries=[]
  for key,source,cap,proofs,donor,purpose in [
   ('commerce.price-number','adapter/web/src/price.rs','extract.price-number',['locked_price_parser_numeric_oracle','precision_and_ambiguous_currency_do_not_invent_money'],'scrapinghub--price-parser','1235 locked donor numeric cases; exact decimal output and bounded conversion; symbol recognition and text selection remain unabsorbed'),
   ('commerce.page-product','adapter/web/src/page_product.rs','extract.page-product',['standalone_meta_and_cross_format_conflicts_are_explicit','ambiguous_page_association_and_article_do_not_create_prices','breadcrumb_and_shipping_fields_resolve_to_source_assertions'],None,'Single-product page metadata and standalone explicit OpenGraph products; cross-format conflicts and unknown supplier facts preserved')]:
   proof='&['+','.join('Proof { kind: ProofKind::'+('Parity' if 'oracle' in name else 'Regression')+', locator: '+json.dumps(source+'::'+name)+' }' for name in proofs)+']'
   entries.append('Technology { key: '+json.dumps(key)+', name: '+json.dumps(key)+', kind: TechnologyKind::Parser, claimed: TechnologyLifecycle::Experimental, purpose: '+json.dumps(purpose)+', implements: &['+json.dumps(cap)+'], node: "commerce.web-research", sources: &['+json.dumps(source)+'], invariants: &[], proofs: '+proof+', lineage: '+('&['+json.dumps(donor)+']' if donor else '&[]')+', relations: &[], norl: NorlRelevance::Unresolved, claims: &[] },')
  text=text[:-1]+'\n'.join(entries)+'\n]'
 p.write_text(text+'\n',encoding='utf-8',newline='\n')
 print('Registered numeric oracle and three page-evidence regressions')
def proof():
 oracle=read('adapter/web/tests/fixtures/price-number.json');source=ROOT/'research/commerce/donors/checkouts/scrapinghub--price-parser'
 raw=subprocess.check_output(['git','-C',str(source),'show',COMMIT+':price_parser/parser.py']);tree=ast.parse(raw);lines=raw.decode().splitlines(keepends=True);contracts=[]
 for name in ['fromstring','extract_currency_symbol','extract_price_text','get_decimal_separator','parse_number','or_regex']:
  node=next(n for n in ast.walk(tree) if isinstance(n,ast.FunctionDef) and n.name==name)
  excerpt=''.join(lines[node.lineno-1:node.end_lineno]);contracts.append(dict(commit_sha=COMMIT,source_path='price_parser/parser.py',symbol=name,line_start=node.lineno,line_end=node.end_lineno,symbol_sha256=hashlib.sha256(excerpt.encode()).hexdigest(),source_excerpt=excerpt,parse_status='PARSED'))
 sys.path.insert(0,str(ROOT/'.venv/research'))
 from tree_sitter_language_pack import get_parser
 for donor,path,symbol,language,contract in [
  ('mozilla--readability','Readability.js','_getArticleMetadata','javascript','JSON-LD title/description precede HTML metadata; fields have ordered fallback. ECDEV retains conflicts instead of a preferred field truth; article-author, prose extraction and broad metadata fallback parity are not claimed.'),
  ('firecrawl--firecrawl','apps/api/src/scraper/scrapeURL/lib/extractMetadata.ts','extractMetadata','typescript','First delegates to firecrawl-rs, then Cheerio metadata projection; title and individual OpenGraph fields are separate. Only this wrapper/fallback is reviewed; delegated native implementation and complete Firecrawl scraping are unabsorbed.')]:
  identity=read('research/commerce/donors/census/'+donor+'/identity.json');checkout=ROOT/'research/commerce/donors/checkouts'/donor;commit=identity['commit_sha']
  data=subprocess.check_output(['git','-C',str(checkout),'show',commit+':'+path]);assert data.decode().replace('\r\n','\n')==(checkout/path).read_text(encoding='utf-8')
  parsed=get_parser(language).parse(data);todo=[parsed.root_node];matches=[]
  while todo:
   node=todo.pop();name=node.child_by_field_name('name')
   if node.type in ('method_definition','function_declaration') and name and data[name.start_byte:name.end_byte].decode()==symbol:matches.append(node)
   todo.extend(node.named_children)
  assert len(matches)==1,(donor,symbol,len(matches));node=matches[0]
  contracts.append(dict(donor_id=donor,commit_sha=commit,source_path=path,blob_hash=subprocess.check_output(['git','-C',str(checkout),'rev-parse',commit+':'+path]).decode().strip(),source_sha256=hashlib.sha256(data).hexdigest(),symbol=symbol,line_start=node.start_point.row+1,line_end=node.end_point.row+1,symbol_sha256=hashlib.sha256(data[node.start_byte:node.end_byte]).hexdigest(),parse_status='PARSED' if not node.has_error else 'PARTIAL_UNSUPPORTED_SYNTAX',behavior_contract=contract))
 record=dict(status='PASS_BOUNDED_PRICE_NUMBER_PARITY',goal_complete=False,source_evidence=oracle['source_evidence'],source_contracts=contracts,numeric_cases=1235,unique_numeric_inputs=len({(c['input'],c['decimal_separator']) for c in oracle['cases']}),native_source='adapter/web/src/price.rs',page_enrichment_source='adapter/web/src/page_product.rs',scope=oracle['scope'],native_limits='4096-byte numeric input and exponent magnitude, 8192-byte output; finite ASCII decimal syntax only. No binary floating point. No currency inferred from ambiguous symbols.',whole_donor_absorption=False)
 live=ROOT/'.ynventa/materialized/page-product-live.json'
 if live.exists():
  run=json.loads(live.read_text(encoding='utf-8'));assert run['mode']=='LIVE' and run['cost_minor']==0
  captures=[]
  for observation in run['observations']:
   snapshot=observation['normalized_value'];wire=(ROOT/'.ynventa/materialized/raw'/f"{observation['raw_hash']}.html").read_bytes();assert hashlib.sha256(wire).hexdigest()==snapshot['content_hash']==observation['raw_hash']
   captures.append(dict(source=observation['external_source'],content_hash=observation['raw_hash'],products=len(snapshot['products']),product_fields=[dict(title=p['title'],price=p['fields']['price_minor'],canonical_url=p['fields']['canonical_url'],breadcrumbs=p['fields']['breadcrumbs'],original_price=p['fields']['original_price'],shipping_text=p['fields']['shipping_text']) for p in snapshot['products']]))
  record['live']=dict(run_id=run['run_id'],mode=run['mode'],status=run['status'],network_calls=run['network_calls'],known_network_calls=run['known_network_calls'],cost_minor=0,frontier=run['frontier'],funnel=run['funnel'],errors=run['errors'],captures=captures)
 dump('research/commerce/price-page-proof.json',record);print('Recorded bounded source contracts and live page evidence')
if __name__=='__main__':register() if '--register' in sys.argv else proof()
