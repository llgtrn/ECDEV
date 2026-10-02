"""Review exact AST symbols in the locked crawler and its pinned queue backend."""
import hashlib,json,pathlib,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'.venv/research'))
from tree_sitter_language_pack import get_parser
CRAWLEE=('apify--crawlee','438e3419626bd070f8984566bfb86ab9355f55d6')
BACKEND=('apify--crawlee-storage','ac0c602c15dc16b2783a23d3e7137fbae183a34e')
QUEUE='packages/core/src/storages/request_queue.ts'
BASIC='packages/basic-crawler/src/internals/basic-crawler.ts'
CONCURRENCY='packages/basic-crawler/src/internals/autoscaling/concurrency_system.ts'
THROTTLE='packages/basic-crawler/src/internals/throttling_request_manager.ts'
NATIVE='crawlee-storage/src/request_queue.rs'
REVIEWS=[
 (CRAWLEE,'url.identity','packages/core/src/request.ts','computeUniqueKey','Default and extended method/payload normalization are separate policies; the oracle uses explicit canonical GET uniqueKeys, not Request.computeUniqueKey parity.'),
 (CRAWLEE,'crawl.deduplicate',QUEUE,'addRequest','Core cache deduplicates additions and records handled state; deferred transactions and duplicate forefront promotion are outside native parity.'),
 (CRAWLEE,'crawl.frontier',QUEUE,'fetchNextRequest','Core delegates request locking to the selected storage backend.'),
 (CRAWLEE,'crawl.frontier',QUEUE,'markRequestAsHandled','Successful processing marks handled and updates the core cache; handled requests remain deduplicated.'),
 (CRAWLEE,'crawl.frontier',QUEUE,'reclaimRequest','Core passes request bookkeeping and forefront choice to the backend.'),
 (BACKEND,'crawl.frontier',NATIVE,'add_batch_of_requests','Indexes uniqueKey and maintains handled/pending counts and signed order; default duplicate behavior and new forefront insertion are compared.'),
 (BACKEND,'crawl.frontier',NATIVE,'ordered_candidate_keys','Forefront list is reverse/LIFO; normal candidates order by timestamp and insertion sequence. ECDEV maps forefront to positive priority ranks.'),
 (BACKEND,'crawl.frontier',NATIVE,'fetch_next_request','Disk orderNo is authoritative; future signed timestamps reserve active work. Expiration and shared reopen are compared.'),
 (BACKEND,'crawl.frontier',NATIVE,'mark_request_as_handled','Handled records retain identity and leave pending count; ECDEV additionally requires a current lease token.'),
 (BACKEND,'crawl.frontier',NATIVE,'reclaim_request','Reclaim restores an available ordering timestamp and regular/forefront sign. Native reclaim is atomic, token-fenced and retry-budget bounded.'),
 (BACKEND,'crawl.frontier',NATIVE,'set_expected_request_processing_time','Initial lock is three minutes and can only be increased; oracle advances beyond that minimum. ECDEV timeout remains explicitly configurable.'),
 (BACKEND,'crawl.frontier',NATIVE,'prolong_request_lock','Lock extension checks the held reservation and updates disk; ECDEV lock extension is not absorbed.'),
 (BACKEND,'crawl.frontier',NATIVE,'persist_state','State is persisted inline; shared reopen respects future-dated locks. Single-owner eager unlock is outside native comparison.'),
 (CRAWLEE,'crawl.retry',BASIC,'canRequestBeRetried','noRetry and NonRetryableError deny retry; RetryRequestError can override exhaustion; per-request limits override global counts. ECDEV forbids unlimited retries.'),
 (CRAWLEE,'crawl.retry',BASIC,'requestFunctionErrorHandler','Retry increments retryCount and reclaims; exhausted failure marks handled and reports failure. ECDEV keeps distinct FAILED; complete crawler error policy remains source-reviewed.'),
 (CRAWLEE,'crawl.cancellation',BASIC,'stop','Stops dispatch while run bookkeeping is maintained; ECDEV cancellation fences outstanding completions rather than claiming identical drain semantics.'),
 (CRAWLEE,'crawl.cancellation',BASIC,'teardown','Aborts the owned autoscaled pool and releases per-run resources; browser/session teardown is outside frontier scope.'),
 (CRAWLEE,'crawl.backpressure',CONCURRENCY,'hasCapacityForTask','Capacity depends on desired concurrency and request rate; ECDEV owns hard global/per-origin lease admission, not dynamic load autoscaling.'),
 (CRAWLEE,'crawl.backpressure',CONCURRENCY,'tryRegisterTaskStart','Checks capacity before incrementing active work; ECDEV admission is an immediate SQLite transaction.'),
 (CRAWLEE,'crawl.throttle',THROTTLE,'checkReadiness','Domain readiness/pacing are separate from core queue; ECDEV persists origin intervals and deadlines, not complete registrable-domain routing parity.'),
 (CRAWLEE,'crawl.throttle',THROTTLE,'#recordRateLimit','Rate-limit signals update throttle state; ECDEV persists Retry-After/backoff with narrower origin semantics.'),
]
def locked(donor,path):
 key,commit=donor;checkout=ROOT/'research/commerce/donors/checkouts'/key
 def git(*args):return subprocess.check_output(['git','-C',str(checkout),*args])
 assert git('rev-parse','HEAD').decode().strip()==commit
 blob=git('show',commit+':'+path)
 assert (checkout/path).read_text(encoding='utf-8')==blob.decode().replace('\r\n','\n')
 return blob,dict(donor_id=key,commit_sha=commit,source_path=path,blob_hash=git('rev-parse',commit+':'+path).decode().strip(),content_sha256=hashlib.sha256(blob).hexdigest())
def main():
 contracts=[]
 for donor,cap,path,symbol,contract in REVIEWS:
  blob,evidence=locked(donor,path);tree=get_parser('rust' if path.endswith('.rs') else 'typescript').parse(blob)
  parse_status="PARTIAL_UNSUPPORTED_SYNTAX" if tree.root_node.has_error else "PARSED"
  todo=[tree.root_node];matches=[]
  while todo:
   node=todo.pop();name=node.child_by_field_name('name')
   if node.type in ('method_definition','function_item') and name and blob[name.start_byte:name.end_byte].decode()==symbol:matches.append(node)
   todo.extend(node.named_children)
  assert len(matches)==1,(path,symbol,len(matches))
  node=matches[0];evidence.update(source_parse_status=parse_status,symbol_parse_status="PARTIAL_UNSUPPORTED_SYNTAX" if node.has_error else "PARSED",capability=cap,symbol=symbol,line_start=node.start_point.row+1,line_end=node.end_point.row+1,reviewed_excerpt=blob[node.start_byte:node.end_byte].decode(),contract=contract,status='SOURCE_REVIEWED',oracle_parity='BOUNDED_QUEUE_TRACES_ONLY' if cap=='crawl.frontier' else 'NOT_CLAIMED');contracts.append(evidence)
 fixture_path=ROOT/'domain/commerce/tests/fixtures/crawlee-frontier.json';fixture=json.loads(fixture_path.read_text(encoding='utf-8'))
 record=dict(donor='apify/crawlee',commit_sha=CRAWLEE[1],status='PARTIAL_SOURCE_REVIEW_WITH_BOUNDED_QUEUE_ORACLE',contracts=contracts,backend_dependency=fixture['backend'],oracle_fixture='domain/commerce/tests/fixtures/crawlee-frontier.json',oracle_fixture_sha256=hashlib.sha256(fixture_path.read_bytes()).hexdigest(),oracle_traces=len(fixture['cases']),oracle_operations=sum(len(c['steps']) for c in fixture['cases']),native_node='commerce',native_source='domain/commerce/src/frontier.rs',native_tests='domain/commerce/tests/frontier.rs',remaining=['Complete crawler error/retry override oracle','Dynamic autoscaling and registrable-domain pacing parity','Request.computeUniqueKey full normalization parity','Deferred transactions, duplicate forefront promotion and batch mutations','Single-owner eager unlock and lease extension APIs','Whole Crawlee donor semantic census and full absorption'])
 (ROOT/'research/commerce/frontier-source-review.json').write_text(json.dumps(record,indent=2)+'\n',encoding='utf-8',newline='\n');print('Locked AST source contracts:',len(contracts),'queue traces:',record['oracle_traces'])
if __name__=='__main__':main()
