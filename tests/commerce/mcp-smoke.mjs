import assert from 'node:assert/strict';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
const base=process.env.ECDEV_TEST_URL||'http://127.0.0.1:8765';
const client=new Client({name:'ecdev-conformance-client',version:'1.0.0'});
await client.connect(new StreamableHTTPClientTransport(new URL(base+'/mcp')));
try {
 const tools=await client.listTools();assert.equal(tools.tools.length,31);
 for(const name of ['ecdev.product.discover','ecdev.product.inspect','ecdev.product.compare','ecdev.provider.status','ecdev.monitor.create','ecdev.monitor.status'])assert.ok(tools.tools.some(t=>t.name===name),name);
 const status=await client.callTool({name:'ecdev.system.status',arguments:{}});
 assert.equal(status.isError,false);assert.equal(status.structuredContent.name,'ECDEV');
 const providers=await (await fetch(base+'/api/providers')).json();
 if(providers.some(p=>p.id==='keepa'&&p.auth_state==='MISSING')) {
  const missing=await client.callTool({name:'ecdev.product.analyze',arguments:{market:'AMAZON_JP',asin:'B08N5WRWNW'}});
  assert.equal(missing.structuredContent.status,'UNAVAILABLE');assert.equal(missing.structuredContent.network_calls,0);
 }
 const intent={market:'AMAZON_JP',currency:'JPY',capital:300000,min_price:3000,max_price:6000,max_weight_g:700,minimum_margin_bps:2500,max_inventory_per_sku:150000};
 const run=await client.callTool({name:'ecdev.opportunity.search',arguments:intent});
 const value=run.structuredContent;assert.equal(value.mode,'PLAN_ONLY');assert.equal(value.status,'UNAVAILABLE');assert.equal(value.plan.steps.length,11);assert.deepEqual(value.observations,[]);
 const resource=await client.readResource({uri:'ecdev://run/'+value.run_id});assert.equal(JSON.parse(resource.contents[0].text).run_id,value.run_id);
 const replay=await client.callTool({name:'ecdev.runs.replay',arguments:{run_id:value.run_id}});assert.equal(replay.structuredContent.mode,'REPLAY');assert.equal(replay.structuredContent.network_calls,0);
 const scenario={currency:'JPY',fulfillment:'FBA',selling_price:4000,product_cost:800,freight:200,fulfillment_fee:500,referral_bps:1500,ppc:400,units:100,fixed_launch_cost:10000};
 const economics=await client.callTool({name:'ecdev.economics.simulate',arguments:scenario});assert.equal(economics.structuredContent.mode,'SIMULATED');assert.equal(economics.structuredContent.result.expected_profit,140000);
 const invalid=await client.callTool({name:'ecdev.economics.simulate',arguments:{...scenario,referral_bps:10001}});assert.equal(invalid.isError,true);
 const fixture=await (await fetch(base+'/api/research/example')).json();
 const research=await client.callTool({name:'ecdev.research.run',arguments:fixture});assert.equal(research.isError,false);
 assert.equal(research.structuredContent.mode,'FIXTURE');assert.equal(research.structuredContent.cost_minor,0);assert.equal(research.structuredContent.funnel.discovered,3);assert.equal(research.structuredContent.funnel.rejected,2);
 const discovery=await client.callTool({name:'ecdev.product.discover',arguments:fixture});assert.equal(discovery.isError,false);assert.equal(discovery.structuredContent.mode,'FIXTURE');
 const candidates=discovery.structuredContent.candidates;
 const inspect=await client.callTool({name:'ecdev.product.inspect',arguments:{candidate_id:candidates[0].id}});assert.equal(inspect.isError,false);assert.equal(inspect.structuredContent.id,candidates[0].id);
 const comparison=await client.callTool({name:'ecdev.product.compare',arguments:{candidate_ids:candidates.slice(0,2).map(c=>c.id)}});assert.equal(comparison.isError,false);assert.equal(comparison.structuredContent.network_calls,0);
 const providerStatus=await client.callTool({name:'ecdev.provider.status',arguments:{}});assert.equal(providerStatus.isError,false);assert.ok(Array.isArray(providerStatus.structuredContent.items));
 const watchList=await client.callTool({name:'ecdev.monitor.status',arguments:{}});assert.ok(Array.isArray(watchList.structuredContent.items));
 const watchArgs={enabled:false,market:'PUBLIC_WEB',query:'SDK disabled watch',targets:['https://example.org/product'],interval_seconds:60};
 const watch=await client.callTool({name:'ecdev.monitor.create',arguments:watchArgs});assert.equal(watch.isError,false);assert.equal(watch.structuredContent.status,'DISABLED');
 const watchStatus=await client.callTool({name:'ecdev.monitor.status',arguments:{watch_id:watch.structuredContent.watch_id}});assert.equal(watchStatus.isError,false);assert.equal(watchStatus.structuredContent.status,'DISABLED');
 const budget=await client.callTool({name:'ecdev.provider.budget',arguments:{}});assert.equal(budget.structuredContent.policy.per_month_minor,0);
 const api=await fetch(base+'/api/runs/'+value.run_id);assert.equal((await api.json()).run_id,value.run_id);
 for(const path of ['/health','/api/donors','/api/providers','/metrics','/','/app.js','/style.css'])assert.equal((await fetch(base+path)).status,200,path);
 assert.equal((await fetch(base+'/api/status',{headers:{Origin:'https://example.org'}})).status,403);
 assert.equal((await fetch(base+'/mcp',{method:'POST',headers:{'Content-Type':'application/json',Accept:'application/json, text/event-stream','MCP-Protocol-Version':'invalid'},body:JSON.stringify({jsonrpc:'2.0',id:1,method:'tools/list'})})).status,400);
 const abort=new AbortController();const sse=await fetch(base+'/events',{signal:abort.signal});const reader=sse.body.getReader();const chunk=await reader.read();assert.match(new TextDecoder().decode(chunk.value),/event: run/);abort.abort();
 console.log(JSON.stringify({status:'PASS',transport:'SDK_STREAMABLE_HTTP',client:'@modelcontextprotocol/sdk',tool_count:tools.tools.length,plan_run_id:value.run_id,replay_run_id:replay.structuredContent.run_id,economics_run_id:economics.structuredContent.run_id,fixture_research_run_id:research.structuredContent.run_id,zero_paid_research:'PASS_FIXTURE_MODE',live_e2e:'UNAVAILABLE'},null,2));
} finally {await client.close();}
