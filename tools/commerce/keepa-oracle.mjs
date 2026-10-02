// Research-only oracle: transpile two exact donor modules; evaluate no network code.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { execFileSync } from 'node:child_process';
import ts from '../../apps/web/node_modules/typescript/lib/typescript.js';
const root=path.resolve(import.meta.dirname,'../..');
const donor=path.join(root,'research/commerce/donors/checkouts/purahmanian--keepa-mcp');
const sha=execFileSync('git',['rev-parse','HEAD'],{cwd:donor,encoding:'utf8'}).trim();
const registry=JSON.parse(fs.readFileSync(path.join(root,'research/commerce/donors/registry.json')));
if(registry.donors.find(d=>d.donor_id==='purahmanian--keepa-mcp').commit_sha!==sha)throw Error('Donor SHA mismatch');
const output=path.join(root,'target/keepa-oracle');fs.mkdirSync(path.join(output,'services'),{recursive:true});
fs.writeFileSync(path.join(output,'package.json'),'{"type":"module"}');
for(const file of ['constants.ts','services/keepa-values.ts']){
 const source=fs.readFileSync(path.join(donor,'src',file),'utf8');
 fs.writeFileSync(path.join(output,file.replace('.ts','.js')),ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText);
}
const {resolveCurrent}=await import(pathToFileURL(path.join(output,'services/keepa-values.js')));
const cases=[];
function add(id,source,index,legacy){const c={case_id:id,source,index,expected:resolveCurrent(source,index,legacy)};if(legacy!==undefined)c.legacy=legacy;cases.push(c);}
add('missing',{},0);add('zero-valid',{stats:{current:[0]},current:[90]},0);
add('stats-precedence',{stats:{current:[21]},csv:[[100,50]],current:[60]},0);
add('negative-stats-fallback',{stats:{current:[-1]},csv:[[100,22,101,-1]],current:[60]},0);
add('incomplete-pair-tail',{csv:[[100,12,101]]},0);
add('current-fallback',{csv:[[100,-1]],current:[13]},0);
add('legacy-fallback',{},0,14);
const triplets=Array(19).fill(null);triplets[18]=[100,17,99,101,-1,100,102];add('buybox-triples-incomplete-tail',{csv:triplets},18);
let seed=0x5eedc0de;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed;};
for(let i=0;i<500;i++){
 const index=[0,1,3,16,18][random()%5],stride=index===18?3:2;
 const stats=Array(index+1).fill(null),current=Array(index+1).fill(null),csv=Array(index+1).fill(null);
 stats[index]=random()%3===0?random()%10000:-1;current[index]=random()%3===0?random()%10000:-1;
 const series=[];for(let n=0,count=random()%10;n<count;n++){series.push(n*1440,random()%3===0?-1:random()%10000);if(stride===3)series.push(random()%10000);}
 if(random()%2)series.push(123456);csv[index]=series;
 const source={};if(random()%2)source.stats={current:stats};if(random()%2)source.csv=csv;if(random()%2)source.current=current;
 add('seeded-'+i,source,index,random()%2?random()%10000:undefined);
}
const destination=path.join(root,'adapter/keepa/tests/fixtures/keepa-current-oracle.json');fs.mkdirSync(path.dirname(destination),{recursive:true});
fs.writeFileSync(destination,JSON.stringify({mode:'FIXTURE',donor_id:'purahmanian--keepa-mcp',repository_url:'https://github.com/purahmanian/keepa-mcp',commit_sha:sha,symbol:'resolveCurrent',source_path:'src/services/keepa-values.ts',contract:'Finite integer JSON input; no live provider data',cases},null,2)+'\n');
console.log(`Generated ${cases.length} donor-executed oracle cases at ${sha}`);
