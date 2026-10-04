"""Execute two unmodified pinned donor functions on bounded deterministic cases."""
import ast, dataclasses, hashlib, json, random, subprocess, types, sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
BASE=ROOT/'research/commerce'

def load(donor,path,names):
 identity=json.loads((BASE/'donors/census'/donor/'identity.json').read_text(encoding='utf-8'))
 raw=subprocess.check_output(['git','-C',str(BASE/'donors/checkouts'/donor),'show',identity['commit_sha']+':'+path])
 tree=ast.parse(raw.decode('utf-8')); nodes=[n for n in tree.body if getattr(n,'name',None) in names]
 assert len(nodes)==len(names)
 module=types.ModuleType('locked_'+donor.replace('-','_'));sys.modules[module.__name__]=module
 module.__dict__.update(Dict=dict,dataclass=dataclasses.dataclass)
 exec(compile(ast.Module(body=nodes,type_ignores=[]),path,'exec'),module.__dict__)
 return module,{'donor':donor,'commit':identity['commit_sha'],'source':path,'symbols':names,'sha256':hashlib.sha256(raw).hexdigest(),'execution':'Unmodified selected AST nodes from pinned Git blob; no module imports/network/runtime integration','scope':'Bounded numeric behavior only; not whole donor parity'}

def main():
 rng=random.Random(634901); cases=[]; families=[]
 module,meta=load('sansan0--TrendRadar','trendradar/core/analyzer.py',['calculate_news_weight'])
 for i in range(240):
  ranks=[rng.randint(1,25) for _ in range(i%13)]; count=rng.randint(0,30); threshold=rng.randint(1,12); weights=[.5,.3,.2] if i%2 else [1.,0.,0.]
  data={'ranks':ranks,'count':count}; w=dict(zip(['RANK_WEIGHT','FREQUENCY_WEIGHT','HOTNESS_WEIGHT'],weights))
  cases.append({'family':'rank_exposure','input':{'ranks':ranks,'count':count,'threshold':threshold,'weights':weights},'expected':module.calculate_news_weight(data,threshold,w)})
 meta.update(case_count=240,test_source='No donor test found for selected symbol in pinned test inventory; generated bounded inputs execute donor independently',known_differences='Unsigned native inputs; oracle domain uses positive ranks and nonnegative counts. Negative inputs and the whole ranking system are outside scope')
 families.append(meta)
 module,meta=load('VladUZH--harken','src/harken/thresholds.py',['ThresholdEvent','evaluate_thresholds'])
 for i in range(320):
  metrics={'current_count':rng.randint(0,30),'baseline_count':rng.randint(0,30),'baseline_average':float(rng.randint(0,15)),'current_net_sentiment':None if i%7==0 else rng.choice([-1.,-.5,0.,.5,1.]),'baseline_net_sentiment':None if i%11==0 else rng.choice([-1.,-.5,0.,.5,1.])}
  config={'window_hours':24,'minimum_mentions':rng.randint(1,10),'volume_multiplier':rng.choice([0.,1.,2.,3.]),'sentiment_drop':rng.choice([0.,.25,.5,1.])}
  result=module.evaluate_thresholds('fixture',metrics,**config)
  cases.append({'family':'thresholds','input':{'metrics':metrics,**config},'expected':{k:v is not None for k,v in result.items()}})
 test=BASE/'donors/checkouts/VladUZH--harken/tests/test_thresholds.py'
 raw=subprocess.check_output(['git','-C',str(test.parents[1]),'show',meta['commit']+':tests/test_thresholds.py'])
 meta.update(case_count=320,test_source='tests/test_thresholds.py',test_sha256=hashlib.sha256(raw).hexdigest(),known_differences='Compare event activation only; donor notification prose and rounded payload presentation are outside oracle scope')
 families.append(meta)
 out=BASE/'social-oracle-fixtures.json';out.write_text(json.dumps({'families':families,'cases':cases},ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
 print('Independent donor families:',len(families),'cases:',len(cases))
if __name__=='__main__':main()
