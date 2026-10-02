type Data = Record<string, unknown>;
const content=document.querySelector<HTMLElement>('#content')!;
const titles=['Overview','Research','Candidates','Provider calls','Budget','Evidence graph','Monitoring','Ynventa','Providers','Product inspection','Runs','Economics','Opportunity plan','System'];
let selected='Overview';
const el=(tag:string,text?:string)=>{const n=document.createElement(tag);if(text!==undefined)n.textContent=text;return n;};
const panel=(title:string)=>{const p=el('div');p.className='panel';p.append(el('h2',title));content.append(p);return p;};
const pretty=(data:unknown)=>el('pre',JSON.stringify(data,null,2));
async function api(path:string,body?:unknown):Promise<any>{const r=await fetch(path,body===undefined?undefined:{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});const d=await r.json();if(!r.ok)throw Error(d.error||`HTTP ${r.status}`);return d;}
function table(headers:string[],rows:Node[][]){const t=el('table');const head=el('tr');headers.forEach(h=>head.append(el('th',h)));t.append(head);rows.forEach(row=>{const tr=el('tr');row.forEach(cell=>{const td=el('td');td.append(cell);tr.append(td);});t.append(tr);});return t;}
function button(text:string,fn:()=>void){const b=el('button',text);b.onclick=fn;return b;}
async function render(){
 document.querySelector('#title')!.textContent=selected;
 document.querySelectorAll('nav button').forEach(b=>b.classList.toggle('active',b.textContent===selected));
 content.replaceChildren();
 try {
 if(selected==='Overview'){
  const d=await api('/api/status');const cards=el('div');cards.className='cards';
  for(const [label,key] of [['Donor candidates','donor_candidates'],['Remote verified','remote_verified'],['Full clones','full_clones'],['Parsed source files','source_parsed'],['Parse unknown','parse_unknown'],['Source-backed capabilities','capabilities_verified']]){const c=el('div');c.className='card';c.append(el('small',label),el('strong',String(d.metrics[key])));cards.append(c);}content.append(cards);
  panel('Foundation state').append(el('p',`${d.metrics.classified_files} / ${d.metrics.total_files} donor tree files classified. ${d.metrics.source_parsed} / ${d.metrics.first_party_source_files} first-party source files structurally parsed. Semantic census remains partial.`),el('p',`Fully absorbed donors: ${d.metrics.native_absorbed} · Oracle-compared primitives: ${d.metrics.oracle_compared_capabilities} · Extinct donors: ${d.metrics.extinct} · Runtime dependency upstreams: ${d.metrics.runtime_donor_dependencies}`),el('p','Protocol subsystem copied from the canonical repository. ECDEV shard registration is missing upstream.'));
 }else if(selected==='Research'){
  const p=panel('Native/public research · paid providers optional');p.append(el('p','Enter seed pages for bounded public-source research. The demonstration uses explicitly labeled HTML fixtures, zero paid calls, and supplied economics assumptions. Demand, supplier capacity and sales remain unknown.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify(await api('/api/research/example'),null,2);p.append(input,button('Run research',async()=>{try{const r=await api('/api/tools/ecdev.research.run',JSON.parse(input.value));panel(`${r.mode} · ${r.status}`).append(pretty({funnel:r.funnel,paid_providers:r.paid_providers,cost_minor:r.cost_minor,network_calls:r.network_calls,coverage:r.coverage,candidates:r.candidates,errors:r.errors}));}catch(e){showError(e);}}));
 }else if(selected==='Candidates'){
  const d=await api('/api/tools/ecdev.research.candidates',{});panel('Candidate funnel · rejections retained').append(table(['Title','State','Observed price','Reason','Unknown evidence','Detail'],d.map((c:any)=>[el('span',String(c.product.title)),el('span',c.state),el('span',String(c.product.price_minor??'UNKNOWN')),el('span',c.rejection_reasons.join(', ')||c.survival_reason),el('span',c.unknowns.join(', ')),button('Inspect',()=>panel(c.id).append(pretty(c)))])));
 }else if(selected==='Provider calls'){
  const d=await api('/api/tools/ecdev.provider.calls',{});panel('Provider accounting · unknown quota stays unknown').append(table(['Provider','Status','Requests','Actual cost','Cache hit','Quota after','Details'],d.map((c:any)=>[el('span',c.provider),el('span',c.status),el('span',String(c.request_count??'UNKNOWN')),el('span',String(c.actual_cost_minor??'UNKNOWN')),el('span',String(c.cache_hit)),el('span',String(c.quota_after??'UNKNOWN')),button('Inspect',()=>panel(c.id).append(pretty(c)))])));
 }else if(selected==='Budget'){
  const d=await api('/api/tools/ecdev.provider.budget',{});panel('Current research budget').append(el('p',`Monthly remaining: ${d.month_remaining_minor} ${d.policy.currency} minor units. Paid providers are denied when any required ceiling is zero.`),el('p',`$0 research coverage: ${(d.zero_cost_coverage_bps/100).toFixed(2)}% across ${d.research_stage_count} declared stages; this measures executable stages, not marketplace accuracy.`),pretty(d));
 }else if(selected==='Evidence graph'){
  const d=await api('/api/tools/ecdev.evidence.graph',{});const rows=d.edges.map((e:any)=>[e.from,e.relation,e.to,e.mode].map(v=>el('span',String(v))));panel('Persisted evidence graph').append(table(['From','Relation','To','Mode'],rows));panel('Entities').append(pretty(d.entities));
 }else if(selected==='Monitoring'){
  const p=panel('Compare captured commerce snapshots');p.append(el('p','Compare prices, availability and sellers from two saved research runs. Unknown fields remain unknown. This comparison makes no network requests.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify({before_run_id:'',after_run_id:''},null,2);p.append(input,button('Compare snapshots',async()=>{try{panel('Snapshot changes').append(pretty(await api('/api/tools/ecdev.monitor.compare',JSON.parse(input.value))));}catch(e){showError(e);}}));
 }else if(selected==='Ynventa'){
  const d=await api('/api/donors');const rows=d.donors.map((r:any)=>{const a=el('a',r.name) as HTMLAnchorElement;a.href=String(r.repository_url);a.target='_blank';a.rel='noopener';const inspect=button('Inspect',async()=>{try{const s=await api('/api/donors/'+encodeURIComponent(r.donor_id));const p=panel(r.name);p.append(pretty({identity:r,census:s}));}catch(e){showError(e);}});return[a,el('code',String(r.commit_sha)),el('span',r.donor_type),el('span',r.tree_status),el('span',r.source_census_status),el('span',r.capability_census_status),el('span','NOT_STARTED'),inspect];});
  panel('Donor census').append(table(['Repository','Locked HEAD','Type','Tree','Source','Capabilities','Absorption','Detail'],rows));
 }else if(selected==='Providers'){
  const d=await api('/api/providers');panel('External data boundaries').append(table(['Provider','Status','Authentication','Native client','Reason'],d.map((p:any)=>[p.id,p.status,p.auth_state,p.adapter_state,p.reason].map(v=>el('span',v)))));
 }else if(selected==='Product inspection'){
  const p=panel('Inspect one product');p.append(el('p','Paid inspection is denied by the default zero budget. Explicit positive ceilings and KEEPA_API_KEY are required for one request. Live behavior remains unverified; no retries.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify({market:'AMAZON_JP',asin:'B08N5WRWNW'},null,2);p.append(input,button('Request product inspection',async()=>{try{const result=await api('/api/tools/ecdev.product.analyze',JSON.parse(input.value));panel(String(result.mode)).append(pretty(result));}catch(e){showError(e);}}));
 }else if(selected==='Runs'){
  const d=await api('/api/runs');if(!d.length){panel('Run history').append(el('p','No persisted runs. Submit an opportunity plan or economics simulation.'));return;}
  const rows=d.map((r:any)=>[el('code',r.run_id),el('span',r.mode),el('span',r.status),el('span',String(r.cost_minor)),button('Inspect',()=>panel(r.run_id).append(pretty(r))),button('Replay',async()=>{try{const v=await api('/api/tools/ecdev.runs.replay',{run_id:r.run_id});panel('REPLAY · no network').append(pretty(v));}catch(e){showError(e);}})]);
  panel('Durable runs').append(table(['Run','Mode','Status','Cost (minor units)','Detail','Replay'],rows));
 }else if(selected==='Economics'||selected==='Opportunity plan'){
  const economics=selected==='Economics';const p=panel(economics?'SIMULATED · supplied assumptions':'PLAN_ONLY · Amazon Japan');
  p.append(el('p',economics?'Costs and fees are supplied assumptions in one currency’s minor units. No live marketplace fee lookup.':'This creates a persisted capability DAG. Live acquisition remains UNAVAILABLE until providers are implemented and verified.'));
  const input=el('textarea') as HTMLTextAreaElement;input.value=JSON.stringify(economics?{currency:'JPY',fulfillment:'FBA',selling_price:4000,product_cost:800,freight:200,fulfillment_fee:500,referral_bps:1500,ppc:400,units:100,fixed_launch_cost:10000}:{market:'AMAZON_JP',currency:'JPY',capital:300000,min_price:3000,max_price:6000,max_weight_g:700,minimum_margin_bps:2500,max_inventory_per_sku:150000,positive_trend:true,exclude_regulated:true},null,2);p.append(input);
  p.append(button(economics?'Simulate economics':'Create plan',async()=>{try{const result=await api('/api/tools/'+(economics?'ecdev.economics.simulate':'ecdev.opportunity.search'),JSON.parse(input.value));panel(String(result.mode)).append(pretty(result));}catch(e){showError(e);}}));
 }else{panel('System introspection').append(pretty(await api('/api/status')));panel('Implemented tools').append(pretty(await api('/api/tools')));}
 document.querySelector('#connection')!.textContent='Connected · localhost';
 }catch(e){showError(e);document.querySelector('#connection')!.textContent='Connection error';}
}
function showError(e:unknown){const n=el('p',String(e));n.className='error';content.append(n);}
titles.forEach(title=>document.querySelector('#nav')!.append(button(title,()=>{selected=title;void render();})));
const events=new EventSource('/events');events.addEventListener('run',()=>{if(selected==='Runs')void render();});
void render();
