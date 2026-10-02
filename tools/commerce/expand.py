"""Full source intake without resetting existing reviewed seed records."""
import concurrent.futures,json,pathlib,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'.venv/research'))
import census
HYPOTHESES={
 'firecrawl/firecrawl':['crawl.scheduler','extract.product'],
 'apify/crawlee':['crawl.frontier','crawl.domain_budget'],
 'microsoft/playwright':['fetch.browser','browser.session'],
 'browser-use/browser-use':['browser.task','browser.observation'],
 'scrapy/scrapy':['crawl.scheduler','crawl.retry','crawl.deduplicate'],
 'dgtlmoon/changedetection.io':['monitor.snapshot','monitor.change_detection'],
 'amzn/selling-partner-api-models':['marketplace.catalog','marketplace.offer','marketplace.fee'],
 'amzn/selling-partner-api-docs':['marketplace.protocol'],
 'mozilla/readability':['extract.readability'],
 'scrapinghub/extruct':['extract.structured_data','extract.json_ld'],
 'scrapinghub/price-parser':['extract.price'],
 'seomoz/reppy':['crawl.robots'],
 'httpie/cli':['fetch.http'],
 'encode/httpx':['fetch.http','fetch.redirect'],
 'tkem/cachetools':['cache.ttl','cache.eviction']}
def main():
 path=ROOT/'research/commerce/donors/registry.json';reg=json.loads(path.read_text(encoding='utf-8'));old={r['donor_id']:r for r in reg['donors']}
 new=[r for r in HYPOTHESES if old.get(r.replace('/','--'),{}).get('clone_status')!='FULL_CLONE']
 census.dump(ROOT/'research/commerce/expansion-intake.json',{'before':len(old),'requested':list(HYPOTHESES),'hypotheses':HYPOTHESES,'hypothesis_status':'UNVERIFIED_UNTIL_SOURCE_REVIEW'})
 with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
  for record in pool.map(census.acquire,new):
   record['repository']=record['repository_url'];record['capability_hypotheses']=HYPOTHESES[record['candidate_identifier'].removeprefix('github:')]
   old[record['donor_id']]=record
   census.dump(path,{'schema_version':1,'generated_at':census.now(),'donors':list(old.values())})
 print('Donors after intake:',len(old),flush=True)
if __name__=='__main__':main()
