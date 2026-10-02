"""Locked source review records; donor execution/oracle parity is never inferred."""
import hashlib,json,pathlib,re,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2]
CHECKOUT=ROOT/'research/commerce/donors/checkouts/apify--crawlee'
COMMIT='438e3419626bd070f8984566bfb86ab9355f55d6'
def git(*args):
    return subprocess.check_output(['git','-C',str(CHECKOUT),*args])
def write(path,value):
    (ROOT/path).write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
contracts=[
 ('url.identity','packages/core/src/request.ts','static computeUniqueKey','URL normalization supplies default identity; extended method/payload identity and always-enqueue are separate policies. Native ECDEV has an explicit HTTP GET site policy; exact normalization parity is pending.'),
 ('crawl.frontier','packages/core/src/storages/request_queue.ts','async addRequest(','uniqueKey caches avoid duplicate additions; backend owns queue persistence. Deferred storage transactions remain outside ECDEV scope.'),
 ('crawl.frontier','packages/core/src/storages/request_queue.ts','async fetchNextRequest','backend fetch delegates request locking; in-progress requests are not fetched a second time.'),
 ('crawl.frontier','packages/core/src/storages/request_queue.ts','async markRequestAsHandled','handled state is delegated to backend and cached; completed requests remain deduplicated.'),
 ('crawl.frontier','packages/core/src/storages/request_queue.ts','async reclaimRequest','reclaim returns a request to queue; forefront is delegated to the backend.'),
 ('crawl.frontier','packages/fs-storage/src/resource-clients/request-queue.ts','async setExpectedRequestProcessingTimeSecs','adapter delegates processing timeout to native backend; this is not evidence of native backend internals.'),
 ('crawl.frontier','packages/fs-storage/src/resource-clients/request-queue.ts','async extendRequestProcessingTimeSecs','native lock extension returns whether the backend still holds the request.'),
 ('crawl.frontier','packages/fs-storage/src/resource-clients/request-queue.ts','async persistState','adapter flushes native persistence on teardown; crash semantics require independent backend review.'),
]
tests=[('test/core/storages/request_queue.test.ts','a reclaimed request is fetched again'),('packages/fs-storage/test/request-queue/reload-persistence.test.ts','requests added and persisted are restored')]
def main():
    assert git('rev-parse','HEAD').decode().strip()==COMMIT
    rows=[]
    for cap,path,symbol,contract in contracts:
        blob=git('show',COMMIT+':'+path);lines=blob.decode().splitlines();start=next(i for i,s in enumerate(lines) if symbol in s);end=min(len(lines),start+70)
        rows.append(dict(capability=cap,commit_sha=COMMIT,source_path=path,blob_hash=git('rev-parse',COMMIT+':'+path).decode().strip(),content_sha256=hashlib.sha256(blob).hexdigest(),symbol=symbol,line_start=start+1,line_end=end,reviewed_excerpt='\n'.join(lines[start:end]),contract=contract,status='SOURCE_REVIEWED',oracle_parity='PENDING_EXECUTION'))
    test_rows=[]
    for path,symbol in tests:
        blob=git('show',COMMIT+':'+path);assert symbol in blob.decode()
        test_rows.append(dict(source_path=path,commit_sha=COMMIT,blob_hash=git('rev-parse',COMMIT+':'+path).decode().strip(),symbol=symbol,execution='NOT_EXECUTED'))
    write('research/commerce/frontier-source-review.json',dict(donor='apify/crawlee',commit_sha=COMMIT,status='PARTIAL_SOURCE_REVIEW_NOT_FULL_ABSORPTION',contracts=rows,donor_tests=test_rows,native_node='commerce',native_source='domain/commerce/src/frontier.rs',native_tests='domain/commerce/tests/frontier.rs',remaining=['Donor native backend internals','Donor runtime oracle execution','Backpressure/priority parity','Cancellation and retry source census','Research-run integration']))
    p=ROOT/'.ynventa/declared/repository.rs';text=p.read_text(encoding='utf-8')
    if '"crawl.frontier"' not in text:
        text=text.replace('"commerce.unit-economics",','"crawl.frontier", "url.identity", "commerce.unit-economics",',1)
        p.write_text(text,encoding='utf-8')
    p=ROOT/'.ynventa/declared/technologies.rs';text=p.read_text(encoding='utf-8').rstrip()
    if 'key: "commerce.frontier"' not in text:
        names=['identity_preserves_product_parameters_and_normalizes_safe_variants','persistence_priority_handled_and_deduplication','retries_obey_backoff_retry_after_and_exhaustion','independent_workers_fenced_origin_global_limits_cancellation_and_deadline','depth_url_and_attempt_budgets_and_throttle_are_durable','deliberate_process_interruption_recovers_expired_lease']
        proofs=', '.join('Proof { kind: ProofKind::Regression, locator: '+json.dumps('domain/commerce/tests/frontier.rs::'+n)+' }' for n in names)
        text=text[:-1]+'Technology { key: "commerce.frontier", name: "Durable commerce URL frontier", kind: TechnologyKind::Runtime, claimed: TechnologyLifecycle::Experimental, purpose: "ECDEV-owned SQLite lease fencing, persistence, URL identity and scheduling; integration and donor oracle parity pending", implements: &["crawl.frontier", "url.identity"], node: "commerce", sources: &["domain/commerce/src/frontier.rs", "domain/commerce/tests/frontier.rs"], invariants: &[], proofs: &['+proofs+'], lineage: &[], relations: &[], norl: NorlRelevance::Unresolved, claims: &[] },\n]\n'
        p.write_text(text,encoding='utf-8')
if __name__=='__main__':main()
