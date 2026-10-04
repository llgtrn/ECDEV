"""Review pinned social source contracts; no donor code enters production."""
from __future__ import annotations
import ast
import hashlib
import json
from collections import Counter
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'research/commerce'
# Explicit contracts from inspected implementation, not README capability claims.
REVIEWS = {
 '666ghj--BettaFish': [
 ('QueryEngine/agent.py','research','Report structure, paragraph research, final report and optional save; LLM orchestration remains external.'),
 ('QueryEngine/agent.py','_initial_search_and_summary','Search-tool/date validation, bounded results with URLs/dates, search history then LLM summary; native owns captures and state, never treats summary as observed.'),
 ('InsightEngine/agent.py','_cluster_and_sample_results','Embedding/KMeans representative sampling with hotness sorting; exception falls back to prefix. Native token clustering is a distinct bounded method.'),
 ('InsightEngine/agent.py','_deduplicate_results','URL or first 100 text characters identifies results. Native uses platform/native ID; text similarity cannot establish product identity.'),
 ('InsightEngine/agent.py','_perform_sentiment_analysis','Preserves platform, author, URL and publish time; model failure returns no analysis, not neutral or demand.'),
 ('InsightEngine/agent.py','execute_search_tool','Database search dispatch and keyword optimization are distinct from live acquisition; credentials and private DB are unavailable.'),
 ('ReportEngine/agent.py','generate_report','Normalize engine reports, template/layout/word planning, LLM chapter assembly into document IR and HTML; native exposes evidence-linked deterministic JSON, no equivalent full report parity.'),
 ('MediaEngine/agent.py','research','Media search/summary workflow with external provider; no paid provider execution in native wave.')],
 '666ghj--MiroFish': [
 ('backend/app/services/simulation_manager.py','create_simulation','Project/graph seed creates persisted CREATED simulation with two platform switches; never an observed provider.'),
 ('backend/app/services/simulation_manager.py','prepare_simulation','Graph entities and generated profiles/config prepare a simulation; external graph and LLM runtimes remain absent.'),
 ('backend/app/services/oasis_profile_generator.py','to_reddit_format','Persona profile maps to simulation-specific actor fields; synthetic identity is not an observed author.'),
 ('backend/app/services/oasis_profile_generator.py','to_twitter_format','Persona exported for simulated Twitter; platform labels do not establish live platform coverage.'),
 ('backend/app/services/simulation_runner.py','add_action','Actions update run and round summaries; action counts are simulated interactions, excluded from trend observations.'),
 ('backend/app/services/simulation_config_generator.py','generate_config','Generate temporal/environment/agent simulation configuration with LLM; no validated future demand forecast.')],
 'NanmiCoder--MediaCrawler': [
 ('base/base_crawler.py','store_content','Abstract post persistence contract; donor availability does not authorize commercial native crawling.'),
 ('base/base_crawler.py','store_comment','Abstract comment persistence, distinct from post records.'),
 ('base/base_crawler.py','store_creator','Abstract creator storage; actual XHS implementation intentionally omits creator personal profiles.'),
 ('media_platform/xhs/core.py','search','Keyword/page loop, bounded note detail and comment retrieval with session client; no auth/session/bypass behavior ported.'),
 ('media_platform/xhs/core.py','get_comments','Semaphore, crawl interval, max comment count and callback; native API path implements bounded pages without this restricted code.'),
 ('media_platform/xhs/core.py','get_creators_and_notes','Creator URL/token parsing, access restriction handling, note list and comments; donor auth requirements are not ECDEV permission.'),
 ('media_platform/xhs/core.py','download_media','Optional media download exists in donor; native preserves references only, never fetches social media.'),
 ('store/xhs/__init__.py','update_xhs_note','Maps note, tags, media and counts; native retains absent engagement as unknown, excludes credential query tokens.'),
 ('store/xhs/__init__.py','update_xhs_note_comment','Comment/parent IDs and counters; donor zero defaults deliberately not used for missing native metrics.'),
 ('store/xhs/__init__.py','save_creator','Current implementation returns without saving creator personal profile; not an active donor creator-persistence capability.')],
 'sansan0--TrendRadar': [
 ('trendradar/core/analyzer.py','calculate_news_weight','Explicit weighted rank exposure, bounded frequency and top-rank ratio; bounded numeric oracle, not demand or growth.'),
 ('trendradar/core/frequency.py','_word_matches','Substring or regex keyword matching; native Unicode term matching has separate scope, no regex parity.'),
 ('trendradar/crawler/rss/parser.py','_parse_json_feed','JSON Feed item dispatch, empty list handling, invalid JSON failure; native supports bounded JSON Feed independent parser.'),
 ('trendradar/crawler/rss/fetcher.py','fetch_feed','HTTP timeout/errors, per-feed cap and preserved published time; native public transport has DNS pinning and bounded redirect validation.')],
 'VladUZH--harken': [
 ('src/harken/sources/hackernews.py','fetch_page','Public Algolia search, keyword filter, opaque timestamp pagination; native missing timestamps remain unknown rather than now.'),
 ('src/harken/sources/bluesky.py','fetch_page','Public Bluesky search with q/limit/latest/cursor/since; preserves record text, author and likes.'),
 ('src/harken/sources/rss.py','fetch','Configured feed fetching and keyword filtering; native supports JSON Feed and explicitly reports unsupported XML RSS.'),
 ('src/harken/thresholds.py','evaluate_thresholds','Minimum samples/baseline gate volume multiplier and sentiment deterioration; independent numeric trigger oracle.'),
 ('src/harken/store.py','alert_metrics','Current window and complete preceding windows separate counts and sentiment; native snapshots disclose actual capture/published windows.'),
 ('src/harken/pipeline.py','track','Fetch, analyze and store with source failures isolated; native partial failures remain visible and suppress disappearance alerts.')],
 'Shiva-74--viral-trend-agent': [
 ('intelligence/virality_scorer.py','_get_upvote_velocity','Latest two snapshots estimate per-hour growth; single snapshot age estimate is not native observed velocity.'),
 ('intelligence/virality_scorer.py','_get_comment_velocity','Snapshot delta divided by bounded elapsed time; native rejects zero/nonmonotonic timestamps instead of inventing elapsed time.'),
 ('intelligence/virality_scorer.py','_get_cross_platform_bonus','Cluster platform counter bonus; native separates publisher/source/platform independence.'),
 ('intelligence/virality_scorer.py','_get_recency_score','Heuristic exponential half-life decay; native independently implements mathematical decay, no unlicensed donor execution/copy.'),
 ('predictor/viral_predictor.py','_verdict','Probability-labelled verdict is heuristic, not a validated demand forecast; no prediction parity claimed.')],
 't4niha--Trend-Finder': [
 ('support/preprocessing/text_cleaner.py','clean_text','URL/markdown/whitespace normalization before embeddings; native retains original text/provenance and normalizes derived terms only.'),
 ('support/clustering/weekly_hdbscan.py','cluster_embeddings','HDBSCAN labels/noise/probability from embeddings; native deterministic token overlap is not equivalent semantic clustering.'),
 ('support/clustering/weekly_hdbscan.py','score_clusters','Explicit score/comment/upvote-ratio aggregate weights; incompatible native platform counters never mixed into an observed total.'),
 ('support/clustering/weekly_hdbscan.py','select_top_posts','Ranks cluster representativeness and engagement separately before weighted selection; native scores expose components.')]
}

def dump(path, value):
 path.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')

def main():
 regpath=BASE/'donors/registry.json'; registry=json.loads(regpath.read_text(encoding='utf-8'))
 graph=[]; licenses=[]
 for record in registry['donors']:
  donor=record['donor_id']
  if donor not in REVIEWS: continue
  folder=BASE/'donors/census'/donor; checkout=BASE/'donors/checkouts'/donor
  rows=[json.loads(s) for s in (folder/'files.jsonl').read_text(encoding='utf-8').splitlines()]
  inventory={r['path']:r for r in rows}
  def source(path):
   return subprocess.check_output(['git','-C',str(checkout),'show',record['commit_sha']+':'+path]).decode('utf-8')
  for row in rows:
   if row['classification']=='UNKNOWN':
    content=source(row['path'])
    if donor=='666ghj--BettaFish' and row['path'].startswith('logs/') and row['path'].endswith('_example.log'):
     row.update(classification='FIXTURE', classification_evidence='Pinned example engine execution log; no executable source', review_sha256=hashlib.sha256(content.encode()).hexdigest())
    elif donor=='sansan0--TrendRadar' and row['path'] in ('version_configs','version_mcp'):
     assert len(content)<1000
     row.update(classification='CONFIG',classification_evidence='Pinned plain version marker',review_sha256=hashlib.sha256(content.encode()).hexdigest())
  (folder/'files.jsonl').write_text(''.join(json.dumps(r,ensure_ascii=False)+'\n' for r in rows),encoding='utf-8')
  summary=json.loads((folder/'summary.json').read_text(encoding='utf-8')); counts=Counter(r['classification'] for r in rows)
  summary.update(classification_counts=dict(counts),classified_files=len(rows)-counts['UNKNOWN'],unknown_files=counts['UNKNOWN'],fixtures=counts['FIXTURE'],semantic_review='PARTIAL_SOURCE_CONTRACTS_REVIEWED_NOT_WHOLE_DONOR')
  dump(folder/'summary.json',summary)
  license_inventory=json.loads((folder/'license.json').read_text(encoding='utf-8'))
  actual=[]
  for item in license_inventory['files']:
   p=item['source_path']; text=source(p)
   if not any(x in Path(p).name.upper() for x in ('LICENSE','COPYING','NOTICE')) or p.endswith(('.tsx','.json')):continue
   header=text.lstrip()[:200]
   label=('NON-COMMERCIAL-LEARNING-1.1' if header.startswith('NON-COMMERCIAL LEARNING LICENSE') else 'AGPL-3.0' if header.startswith('GNU AFFERO GENERAL PUBLIC LICENSE') else ('GPL-2.0' if 'Version 2,' in header else 'GPL-3.0') if header.startswith('GNU GENERAL PUBLIC LICENSE') else 'MIT' if header.startswith('MIT License') else 'OFL-1.1' if 'SIL OPEN FONT LICENSE' in header else 'ISC' if 'ISC License' in header else 'UNKNOWN')

   actual.append({'path':p,'sha256':inventory[p]['sha256'],'blob_hash':inventory[p]['blob_hash'],'terms_detected':label,'additional_noncommercial_terms_present':'NON-COMMERCIAL LEARNING LICENSE' in text and not header.startswith('NON-COMMERCIAL LEARNING LICENSE')})
  root=next((r['terms_detected'] for r in actual if r['path']=='LICENSE'),'UNKNOWN')
  gate='BLOCKED_NO_LICENSE_PERMISSION' if root=='UNKNOWN' else 'BLOCKED_COMMERCIAL_ABSORPTION' if root.startswith('NON-COMMERCIAL') else 'MIXED_TERMS_NO_PRODUCTION_ABSORPTION' if any(r.get('additional_noncommercial_terms_present') for r in actual) else 'STRONG_COPYLEFT_NO_PRODUCTION_COPY' if root in ('GPL-2.0','GPL-3.0','AGPL-3.0') else 'INDEPENDENT_CONTRACT_IMPLEMENTATION'
  review={'donor_id':donor,'commit_sha':record['commit_sha'],'root_terms':root,'files':actual,'license_absorption_gate':gate,'code_copied_to_production':False,'runtime_dependency':False,'scope':'Source term identification, not legal clearance. Header/subdirectory/vendor terms remain applicable; no donor runtime or network service linked. AGPL network obligations require review if modified covered code is ever served.','absence_of_COPYING_NOTICE':[p for p in inventory if 'COPYING' in Path(p).name.upper() or 'NOTICE' in Path(p).name.upper()]}
  licenses.append(review);record.update(license=root,license_absorption_gate=gate)
  tests=json.loads((folder/'tests.json').read_text(encoding='utf-8'))
  for index,(path,symbol,contract) in enumerate(REVIEWS[donor]):
   text=source(path); tree=ast.parse(text)
   selected=[n for n in ast.walk(tree) if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef)) and n.name==symbol]
   assert selected,(donor,path,symbol)
   n=selected[0]; cap='social.'+donor.split('--')[1].lower()+'.'+symbol.lstrip('_')
   graph.append({'capability':cap,'donor_id':donor,'commit_sha':record['commit_sha'],'source_path':path,'blob_hash':inventory[path]['blob_hash'],'sha256':inventory[path]['sha256'],'symbol':symbol,'line_start':n.lineno,'line_end':n.end_lineno,'behavior_contract':contract,'status':'STUDIED','native_node':'commerce','oracle_status':'NOT_CLAIMED','native_replacement_status':'CONTRACT_ONLY_PENDING_IMPLEMENTATION','test_inventory':'research/commerce/donors/census/'+donor+'/tests.json','donor_test_count':summary['tests'],'mandatory_whole_donor_obligation':True})
  record['capabilities_verified']=len(REVIEWS[donor]);record['social_source_review']='PARTIAL_CONTRACTS_NOT_FULL_ABSORPTION'
  dump(folder/'identity.json',record)
 dump(regpath,registry)
 dump(BASE/'social-license-review.json',{'schema_version':1,'reviews':licenses,'production_donor_copies':0})
 dump(BASE/'social-capability-graph.json',{'schema_version':1,'capabilities':graph,'whole_donors_absorbed':0,'native_runtime_donor_dependencies':0,'remaining':'All unmapped source behavior remains an unresolved whole-donor obligation; reviewed primitives never imply extinction.'})
 print('Reviewed contracts:',len(graph),'donors:',len(licenses))

if __name__=='__main__': main()
