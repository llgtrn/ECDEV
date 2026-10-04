type Data = Record<string, unknown>;
const content=document.querySelector<HTMLElement>('#content')!;
const titles=['Overview','Research','Candidates','Provider calls','Budget','Evidence graph','Monitoring','Social / Trends','Ynventa','Providers','Product inspection','Runs','Economics','Opportunity plan','System'];
let selected='Overview';
let renderVersion=0;
const el=(tag:string,text?:string)=>{const n=document.createElement(tag);if(text!==undefined)n.textContent=text;return n;};
const panel=(title:string)=>{const p=el('div');p.className='panel';p.append(el('h2',title));content.append(p);return p;};
const pretty=(data:unknown)=>el('pre',JSON.stringify(data,null,2));
async function request(path:string,body?:unknown):Promise<any>{const r=await fetch(path,body===undefined?undefined:{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});const d=await r.json();if(!r.ok)throw Error(d.error||`HTTP ${r.status}`);return d;}
function table(headers:string[],rows:Node[][]){const t=el('table');const head=el('tr');headers.forEach(h=>head.append(el('th',h)));t.append(head);rows.forEach(row=>{const tr=el('tr');row.forEach(cell=>{const td=el('td');td.append(cell);tr.append(td);});t.append(tr);});return t;}
function button(text:string,fn:()=>void){const b=el('button',text);b.onclick=fn;return b;}
function researchDetail(r:any){
 const p=panel(`${r.mode} | ${r.status}`);
 const cache=r.cache_metrics;
 p.append(el('p',`Requests: ${r.network_calls??'UNKNOWN'} total; ${r.known_network_calls??'UNKNOWN'} known. Paid cost: ${r.cost_minor??'UNKNOWN'} minor units.`));
 p.append(el('p',cache?.hit_ratio_bps===null||cache?.hit_ratio_bps===undefined?'Cache hit ratio: UNKNOWN (empty, unclassified or legacy ledger)':`Cache hit ratio: ${(cache.hit_ratio_bps/100).toFixed(2)}% (${cache.hit_count} / ${cache.acquisition_denominator} logical acquisitions, including failed attempts; ${cache.evidence_mode})`));
 p.append(el('h3','Frontier origins'),el('p','Counts describe enqueued URLs in this run. Different origins do not prove independent publishers.'));
 const origins=r.frontier?.origins??[];
 if(origins.length)p.append(table(['Origin','URLs','Discovered','Pending','Leased','Handled','Retryable','Failed','Cancelled'],origins.map((o:any)=>[o.origin,o.url_count,o.states.DISCOVERED,o.states.PENDING,o.states.LEASED,o.states.HANDLED,o.states.RETRYABLE,o.states.FAILED,o.states.CANCELLED].map(v=>el('span',String(v))))));
 else p.append(el('p','No frontier origin counts were recorded.'));
 p.append(el('h3','Captured origins'),pretty(r.observed_sample?.origins_observed??[]));
 p.append(pretty({frontier:r.frontier,crawl_run_id:r.crawl_run_id,funnel:r.funnel,cache_metrics:r.cache_metrics,provider_calls:r.provider_calls,paid_providers:r.paid_providers,cost_minor:r.cost_minor,network_calls:r.network_calls,coverage:r.coverage,completeness:r.completeness,observed_sample:r.observed_sample,candidates:r.candidates,supplier_leads:r.supplier_leads,next_actions:r.next_actions,executed_information_gain_plan:r.executed_information_gain_plan,errors:r.errors}));
}
function candidateDetail(c:any){
 const p=panel(String(c.product.title??c.id));
 p.append(el('p',`${c.state} | Identity: ${c.entity_id??'UNRESOLVED'}`));
 const fields=Object.entries(c.product.fields??{}) as [string,any][];
 p.append(table(['Field','Status','Value','Source evidence'],fields.map(([name,f])=>[
  el('code',name),el('span',String(f.status)),el('span',f.value===null?'UNKNOWN':JSON.stringify(f.value)),
  button(`${f.evidence?.length??0} observations`,()=>panel(name+' evidence').append(pretty(f.evidence??[])))
 ])));
 const listings=c.competition_evidence?.listing_observations??[];
 if(listings.length){
  const value=(f:any)=>f?.value===null||f?.value===undefined?String(f?.status??'UNKNOWN'):`${JSON.stringify(f.value)} (${f.status})`;
  p.append(el('h3','Captured listing evidence'),el('p','Metadata support describes this captured sample. Publisher independence, content accuracy and total marketplace coverage remain unverified.'),table(['Page','Availability','Rating / reviews','Shipping claim','Declared variants','Supported fields','Evidence'],listings.map((l:any)=>[
   el('span',String(l.page)),el('span',value(l.fields.availability)),el('span',`${value(l.fields.rating)} / ${value(l.fields.review_count)}`),el('span',value(l.fields.shipping_text)),
   el('span',l.variation_depth?.declared_member_count===null?'UNKNOWN':`${l.variation_depth.declared_member_count} declared members; ${l.variation_depth.embedded_structure_depth} embedded levels`),
   el('span',`${l.listing_completeness.supported_field_count} / ${l.listing_completeness.field_denominator}`),button('Inspect listing',()=>panel('Captured listing evidence').append(pretty(l)))
  ])));
 }
 p.append(el('h3','Identity observations and conflicts'),pretty(c.resolution??{status:'LEGACY_RECORD_WITHOUT_RESOLUTION'}),el('h3','Assessment'),pretty({decision:c.decision,economics:c.economics,economics_uncertainty:c.economics_uncertainty,competition_evidence:c.competition_evidence,demand_evidence:c.demand_evidence,unknowns:c.unknowns,rejection_reasons:c.rejection_reasons,evidence_ids:c.evidence_ids}));
}
async function render(){
 const version=++renderVersion;
 const api=async(path:string,body?:unknown)=>{const result=await request(path,body);if(version!==renderVersion)throw Error('Obsolete view');return result;};
 const showError=(e:unknown)=>{if(version===renderVersion)displayError(e);};
 document.querySelector('#title')!.textContent=selected;
 document.querySelectorAll('nav button').forEach(b=>b.classList.toggle('active',b.textContent===selected));
 content.replaceChildren();
 try {
 if(selected==='Overview'){
  const d=await api('/api/status');const cards=el('div');cards.className='cards';
  for(const [label,key] of [['Donor candidates','donor_candidates'],['Remote verified','remote_verified'],['Full clones','full_clones'],['Parsed source files','source_parsed'],['Parse unknown','parse_unknown'],['Source-backed capabilities','capabilities_verified']]){const c=el('div');c.className='card';c.append(el('small',label),el('strong',String(d.metrics[key])));cards.append(c);}content.append(cards);
  panel('Live public research').append(el('p',String(d.live_e2e)),el('p',d.live_e2e==='CAPTURED_ZERO_PAID_RESEARCH_SHORTLIST'?'Recorded captures support a shortlist for further research. Supplier terms, demand and profitability still require validation.':'A live shortlist with available capture evidence has not been recorded.'),pretty(d.live_research));
  panel('Foundation state').append(el('p',`${d.metrics.classified_files} / ${d.metrics.total_files} donor tree files classified. ${d.metrics.source_parsed} / ${d.metrics.first_party_source_files} first-party source files structurally parsed. Semantic census remains partial.`),el('p',`Fully absorbed donors: ${d.metrics.native_absorbed} | Oracle-compared primitives: ${d.metrics.oracle_compared_capabilities} | Extinct donors: ${d.metrics.extinct} | Runtime dependency upstreams: ${d.metrics.runtime_donor_dependencies}`),el('p','Protocol subsystem copied from the canonical repository. ECDEV shard registration is missing upstream.'));
 }else if(selected==='Research'){
  const p=panel('Native/public research | paid providers optional');p.append(el('p','Enter seed pages for bounded public-source research. The demonstration uses explicitly labeled HTML fixtures, zero paid calls, and supplied economics assumptions. Demand, supplier capacity and sales remain unknown.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify(await api('/api/research/example'),null,2);p.append(button('Use live JP catalog',()=>{input.value=JSON.stringify({market:'PUBLIC_WEB',query:'JP ecommerce: household and coffee catalog',sources:[{url:'https://shop.hariocorp.co.jp/collections/all'}],max_pages:100,max_depth:3,max_urls:2500,deadline_seconds:480,min_price_minor:3000,max_price_minor:6000},null,2);}),input,button('Run research',async()=>{try{const r=await api('/api/tools/ecdev.research.run',JSON.parse(input.value));researchDetail(r);}catch(e){showError(e);}}));
 }else if(selected==='Candidates'){
  const d=await api('/api/tools/ecdev.research.candidates',{});panel('Candidate funnel | rejections retained').append(table(['Title','State','Observed price','Reason','Unknown evidence','Detail'],d.map((c:any)=>[el('span',String(c.product.title)),el('span',c.state),el('span',String(c.product.price_minor??'UNKNOWN')),el('span',c.rejection_reasons.join(', ')||c.survival_reason),el('span',c.unknowns.join(', ')),button('Inspect',()=>candidateDetail(c))])));
 }else if(selected==='Provider calls'){
  const d=await api('/api/tools/ecdev.provider.calls',{});panel('Provider accounting | unknown quota stays unknown').append(table(['Provider','Status','Requests','Actual cost','Cache hit','Quota after','Details'],d.map((c:any)=>[el('span',c.provider),el('span',c.status),el('span',String(c.request_count??'UNKNOWN')),el('span',String(c.actual_cost_minor??'UNKNOWN')),el('span',String(c.cache_hit)),el('span',String(c.quota_after??'UNKNOWN')),button('Inspect',()=>panel(c.id).append(pretty(c)))])));
 }else if(selected==='Budget'){
  const d=await api('/api/tools/ecdev.provider.budget',{});panel('Current research budget').append(el('p',`Monthly remaining: ${d.month_remaining_minor} ${d.policy.currency} minor units. Paid providers are denied when any required ceiling is zero.`),el('p',`$0 research coverage: ${(d.zero_cost_coverage_bps/100).toFixed(2)}% across ${d.research_stage_count} declared stages; this measures executable stages, not marketplace accuracy.`),pretty(d));
 }else if(selected==='Evidence graph'){
  const d=await api('/api/tools/ecdev.evidence.graph',{});const rows=d.edges.map((e:any)=>[e.from,e.relation,e.to,e.mode].map(v=>el('span',String(v))));panel('Persisted evidence graph').append(table(['From','Relation','To','Mode'],rows));panel('Entities').append(pretty(d.entities));
 }else if(selected==='Monitoring'){
  const form=panel('Scheduled public watches');form.append(el('p','Watch up to five public URLs with fresh bounded captures. Unknown or blocked captures preserve the previous baseline. No external notifications are sent.'));
  const watchInput=el('textarea') as HTMLTextAreaElement;watchInput.value=JSON.stringify({market:'PUBLIC_WEB',query:'Product watch',targets:['https://shop.hariocorp.co.jp/collections/all'],interval_seconds:3600,enabled:true},null,2);
  form.append(watchInput,button('Save watch',async()=>{try{panel('Saved watch').append(pretty(await api('/api/tools/ecdev.monitor.create',JSON.parse(watchInput.value))));}catch(e){showError(e);}}));
  const watches=await api('/api/tools/ecdev.monitor.status',{});
  panel('Persisted schedules').append(table(['Watch','Status','Interval seconds','Next due','History','Control'],watches.map((w:any)=>[
   el('code',w.watch_id),el('span',w.status),el('span',String(w.schedule.interval_seconds)),el('span',new Date(w.schedule.next_due*1000).toLocaleString()),
   button('Inspect snapshots and changes',async()=>{try{panel(w.watch_id).append(pretty(await api('/api/tools/ecdev.monitor.status',{watch_id:w.watch_id})));}catch(e){showError(e);}}),
   button(w.status==='ACTIVE'?'Disable':'Enable',async()=>{try{await api('/api/tools/ecdev.monitor.create',{watch_id:w.watch_id,enabled:w.status!=='ACTIVE',market:w.request.market,query:w.request.query,targets:w.request.sources.map((s:any)=>s.url),interval_seconds:w.schedule.interval_seconds});await render();}catch(e){showError(e);}})
  ])));
  const p=panel('Compare captured commerce snapshots');p.append(el('p','Compare prices, availability and sellers from two saved research runs. Unknown fields remain unknown. This comparison makes no network requests.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify({before_run_id:'',after_run_id:''},null,2);p.append(input,button('Compare snapshots',async()=>{try{panel('Snapshot changes').append(pretty(await api('/api/tools/ecdev.monitor.compare',JSON.parse(input.value))));}catch(e){showError(e);}}));
 }else if(selected==='Social / Trends'){
  const form=panel('Public social research');form.append(el('p','OBSERVED source posts and DERIVED sampled trends remain separate from ESTIMATED, SIMULATED, UNKNOWN and CONFLICT. Fixture, cache and live captures have separate series. Social evidence alone cannot shortlist a product.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify({query:'matcha',sources:[{platform:'HACKER_NEWS'},{platform:'BLUESKY'}],request_budget:4,window_seconds:604800},null,2);
  const detail=(s:any)=>{const p=panel(`${s.capture_mode} | ${s.state} | ${s.query}`);p.append(el('p',`Captured mentions: ${s.mention_count}. Platforms: ${s.platform_count}. Source URLs: ${s.unique_sources}. Publishers identified: ${s.publisher_count}; independence UNKNOWN.`),el('p','Velocity measures change in a bounded retrieved sample. Platform coverage and real demand remain UNKNOWN.'),table(['Component','State','Value','Weight','Window / denominator'],s.score.components.map((c:any)=>[c.name,c.metric.state,c.metric.value??'UNKNOWN',c.weight,`${c.metric.window_seconds}s / ${JSON.stringify(c.metric.denominator)}`].map(v=>el('span',String(v))))),el('h3','Temporal series and source evidence'),pretty({velocity:s.velocity,acceleration:s.acceleration,persistence:s.persistence,novelty:s.novelty,time_decay:s.time_decay,observed_source_evidence:s.observed_source_evidence??s.captured_posts,engagement:s.engagement_observations,sentiment:s.sentiment,entity_links:s.entity_links,commerce_links:s.commerce_links,clusters:s.clusters,unknowns:s.unknowns,conflicts:s.conflicts,provider_failures:s.provider_failures,budget_usage:s.budget_usage,evidence_ids:s.evidence_ids}));};
  form.append(input,button('Discover trends',async()=>{try{detail(await api('/api/tools/ecdev.trend.discover',JSON.parse(input.value)));}catch(e){showError(e);}}));
  const d=await api('/api/tools/ecdev.trend.inspect',{});
  panel('Frozen trend snapshots and time series').append(table(['Query','Mode','State','Captured','Mentions','Platforms','Score coverage','Detail'],d.snapshots.map((s:any)=>[el('span',s.query),el('span',s.capture_mode),el('span',s.state),el('span',new Date(s.captured_at*1000).toISOString()),el('span',String(s.mention_count)),el('span',String(s.platform_count)),el('span',`${(Number(s.score.known_weight_coverage)*100).toFixed(1)}%`),button('Inspect evidence',async()=>{try{detail(await api('/api/tools/ecdev.trend.explain',{snapshot_id:s.snapshot_id}));}catch(e){showError(e);}})])));
  const watch=panel('Persisted trend watches');const wi=el('textarea') as HTMLTextAreaElement;wi.value=JSON.stringify({query:'matcha',research:{query:'matcha',sources:[{platform:'HACKER_NEWS'}],request_budget:2,window_seconds:86400},triggers:['TOPIC_MENTION_GROWTH','CROSS_PLATFORM_APPEARANCE','NEW_SOURCE_APPEARANCE'],interval_seconds:3600,minimum_mentions:3,threshold:1,enabled:false},null,2);watch.append(el('p','Nine local trigger types use complete acquisition, minimum samples and fenced leases. This example is disabled until enabled explicitly.'),wi,button('Save trend watch',async()=>{try{panel('Saved trend watch').append(pretty(await api('/api/tools/ecdev.trend.watch',JSON.parse(wi.value))));}catch(e){showError(e);}}),pretty(d.watches));
  panel('Simulation boundary and donor provenance').append(pretty(d.simulation),el('p','MiroFish scenarios are SIMULATED and contribute zero observed metrics. Simulation execution is currently unavailable.'),el('p',`Contracts: ${d.donor_provenance}. License review: ${d.license_review}. No full donor extinction claimed.`));
 }else if(selected==='Ynventa'){
  const d=await api('/api/donors');const rows=d.donors.map((r:any)=>{const a=el('a',r.name) as HTMLAnchorElement;a.href=String(r.repository_url);a.target='_blank';a.rel='noopener';const inspect=button('Inspect',async()=>{try{const s=await api('/api/donors/'+encodeURIComponent(r.donor_id));const p=panel(r.name);p.append(pretty({identity:r,census:s}));}catch(e){showError(e);}});return[a,el('code',String(r.commit_sha)),el('span',r.donor_type),el('span',r.tree_status),el('span',r.source_census_status),el('span',r.capability_census_status),el('span',String(r.lifecycle_review?.assessment?.effective??'UNKNOWN')+' (recorded review)'),inspect];});
  panel('Donor census').append(table(['Repository','Locked HEAD','Type','Tree','Source','Capabilities','Absorption','Detail'],rows));
 }else if(selected==='Providers'){
  const d=await api('/api/providers');panel('External data boundaries').append(table(['Provider','Status','Authentication','Native client','Reason'],d.map((p:any)=>[p.id,p.status,p.auth_state,p.adapter_state,p.reason].map(v=>el('span',v)))));
 }else if(selected==='Product inspection'){
  const p=panel('Inspect one product');p.append(el('p','Paid inspection is denied by the default zero budget. Explicit positive ceilings and KEEPA_API_KEY are required for one request. Live behavior remains unverified; no retries.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify({market:'AMAZON_JP',asin:'B08N5WRWNW'},null,2);p.append(input,button('Request product inspection',async()=>{try{const result=await api('/api/tools/ecdev.product.analyze',JSON.parse(input.value));panel(String(result.mode)).append(pretty(result));}catch(e){showError(e);}}));
 }else if(selected==='Runs'){
  const d=await api('/api/runs');if(!d.length){panel('Run history').append(el('p','No persisted runs. Submit an opportunity plan or economics simulation.'));return;}
  const rows=d.map((r:any)=>[el('code',r.run_id),el('span',r.mode),el('span',r.status),el('span',String(r.cost_minor)),button('Inspect',()=>panel(r.run_id).append(pretty(r))),button('Replay',async()=>{try{const v=await api('/api/tools/ecdev.runs.replay',{run_id:r.run_id});panel('REPLAY | no network').append(pretty(v));}catch(e){showError(e);}})]);
  panel('Durable runs').append(table(['Run','Mode','Status','Cost (minor units)','Detail','Replay'],rows));
 }else if(selected==='Economics'||selected==='Opportunity plan'){
  const economics=selected==='Economics';const p=panel(economics?'SIMULATED | supplied assumptions':'PLAN_ONLY | Amazon Japan');
  p.append(el('p',economics?'Costs and fees are supplied assumptions in one currency minor units. No live marketplace fee lookup.':'This creates a persisted capability DAG. This plan does not execute acquisition. Use Research for bounded public discovery; unavailable providers remain explicit.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify(economics?{currency:'JPY',fulfillment:'FBA',selling_price:4000,product_cost:800,freight:200,fulfillment_fee:500,referral_bps:1500,ppc:400,units:100,fixed_launch_cost:10000}:{market:'AMAZON_JP',currency:'JPY',capital:300000,min_price:3000,max_price:6000,max_weight_g:700,minimum_margin_bps:2500,max_inventory_per_sku:150000,positive_trend:true,exclude_regulated:true},null,2);p.append(input);
  p.append(button(economics?'Simulate economics':'Create plan',async()=>{try{const result=await api('/api/tools/'+(economics?'ecdev.economics.simulate':'ecdev.opportunity.search'),JSON.parse(input.value));panel(String(result.mode)).append(pretty(result));}catch(e){showError(e);}}));
 }else{panel('System introspection').append(pretty(await api('/api/status')));panel('Implemented tools').append(pretty(await api('/api/tools')));}
 document.querySelector('#connection')!.textContent='Connected | localhost';
 }catch(e){if(version!==renderVersion)return;showError(e);document.querySelector('#connection')!.textContent='Connection error';}
}
function displayError(e:unknown){const n=el('p',String(e));n.className='error';content.append(n);}
titles.forEach(title=>document.querySelector('#nav')!.append(button(title,()=>{selected=title;void render();})));
const events=new EventSource('/events');events.addEventListener('run',()=>{if(selected==='Runs')void render();});
void render();
