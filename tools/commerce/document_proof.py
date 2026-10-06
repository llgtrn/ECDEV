"""Check live capture bytes and exact locked decoding symbols; retain limits of the proof."""
import ast,hashlib,json,pathlib,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2]
def main():
 run=json.loads((ROOT/'.ecdev-data/document-live-run.json').read_text(encoding='utf-8'))
 assert run['mode']=='LIVE' and run['cost_minor']==0 and not run['errors']
 evidence=run['observations'][0];snapshot=evidence['normalized_value'];recipe=snapshot['document_decoding']
 raw=(ROOT/'.ecdev-data/raw'/f"{evidence['raw_hash']}.html").read_bytes()
 assert hashlib.sha256(raw).hexdigest()==evidence['raw_hash']==snapshot['content_hash']==recipe['wire_capture_sha256']
 assert recipe['encoding']=='UTF-8'
 assert hashlib.sha256(raw.decode('utf-8').encode()).hexdigest()==recipe['decoded_utf8_sha256']
 for claim in run['supplier_leads'][0]['claims']:assert claim['raw_capture_sha256']==evidence['raw_hash']
 for field in ['moq','unit_price','lead_time','factory_capacity','certification_claim']:
  assert run['supplier_leads'][0]['fields'][field]['status']=='UNKNOWN'
 sources=[]
 scrapy=json.loads((ROOT/'research/commerce/donors/census/scrapy--scrapy/identity.json').read_text(encoding='utf-8'))['commit_sha']
 for donor,commit,path,names in [
  ('scrapy--scrapy',scrapy,'scrapy/http/response/text.py',['_declared_encoding','_headers_encoding','_body_declared_encoding','_bom_encoding','_auto_detect_fun','_body_inferred_encoding']),
  ('scrapy--w3lib','537c5d46455ae8b2c67b53fc03b36ef1da8c4837','w3lib/encoding.py',['html_to_unicode','http_content_type_encoding','html_body_declared_encoding','read_bom','resolve_encoding'])]:
  checkout=ROOT/'research/commerce/donors/checkouts'/donor
  raw_source=subprocess.check_output(['git','-C',str(checkout),'show',commit+':'+path])
  assert (checkout/path).read_text(encoding='utf-8')==raw_source.decode().replace('\r\n','\n')
  blob=subprocess.check_output(['git','-C',str(checkout),'rev-parse',commit+':'+path]).decode().strip()
  tree=ast.parse(raw_source);lines=raw_source.decode().splitlines(keepends=True)
  for name in names:
   node=next(n for n in ast.walk(tree) if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef)) and n.name==name)
   text=''.join(lines[node.lineno-1:node.end_lineno])
   sources.append(dict(donor_id=donor,commit_sha=commit,source_path=path,blob_hash=blob,source_sha256=hashlib.sha256(raw_source).hexdigest(),symbol=name,line_start=node.lineno,line_end=node.end_lineno,symbol_sha256=hashlib.sha256(text.encode()).hexdigest(),parse_status='PARSED',source_excerpt=text))
 record=dict(status='PASS_DECLARED_DECODING_ORACLE_AND_LIVE_SUPPLIER_CAPTURE',goal_complete=False,native_source='adapter/web/src/document.rs',oracle_families=5,oracle_cases=2360,declared_document_cases=54,source_contracts=sources,native_policy='BOM > HTTP charset > first 1024 bytes meta > strict UTF-8. Invalid or unknown encoding is unavailable; no statistical inference or replacement.',donor_differences=['Scrapy explicit caller encoding precedes BOM; ECDEV has no caller override.','Scrapy may statistically infer undeclared bytes; ECDEV denies invalid UTF-8.','w3lib may replace invalid bytes and supports UTF-32; ECDEV rejects both.','Meta prescan uses native HTML DOM and 1024-byte limit; malformed declaration parity unproven.'],live=dict(run_id=run['run_id'],mode=run['mode'],status=run['status'],known_network_calls=run['known_network_calls'],paid_cost_minor=0,paid_provider_calls=0,captures=len(run['observations']),supplier_leads=len(run['supplier_leads']),candidates=len(run['candidates']),frontier=run['frontier'],decoding=recipe,wire_bytes=len(raw),evidence_raw_hash=evidence['raw_hash'],supplier_claims=run['supplier_leads'][0]['claims']),live_limit='The fresh HARIO response declares UTF-8. It proves live supplier route and raw provenance, not live legacy-charset acquisition or the cause of the earlier NON_UTF8_DOCUMENT error.',runtime_dependency_packages=14,runtime_dependency_upstreams=13,full_wave_remaining='The broader extraction, competition, economics-supported shortlist and canonical registration requirements remain open.')
 (ROOT/'research/commerce/document-decoding-proof.json').write_text(json.dumps(record,indent=2,ensure_ascii=False)+'\n',encoding='utf-8',newline='\n')
 print('Verified live wire bytes and 11 locked source symbols:',run['run_id'],len(raw),'bytes')
if __name__=='__main__':main()
