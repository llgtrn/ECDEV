"""Refine identifiable formats without resetting semantic reviews or hiding unknowns."""
import collections,json,pathlib,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[2];sys.path.insert(0,str(ROOT/'.venv/research'))
import census
def main():
 reg=json.loads((ROOT/'research/commerce/donors/registry.json').read_text(encoding='utf-8'))
 for donor in reg['donors']:
  path=ROOT/'research/commerce/donors/census'/donor['donor_id'];rows=[json.loads(l) for l in (path/'files.jsonl').read_text(encoding='utf-8').splitlines()];changed=False
  for row in rows:
   if row['classification']!='UNKNOWN':continue
   content=subprocess.check_output(['git','cat-file','blob',row['blob_hash']],cwd=ROOT/'research/commerce/donors/checkouts'/donor['donor_id']);kind=census.classify(row['path'],content)
   if kind!='UNKNOWN':
    row['classification']=kind;changed=True
    if kind=='FIRST_PARTY_SOURCE':row['parse_status']='PARSE_UNKNOWN';row['parse_reason']='Recognized source container/language; semantic parser not implemented'
  if changed:
   census.jsonl(path/'files.jsonl',rows);summary=json.loads((path/'summary.json').read_text(encoding='utf-8'));summary['classified_files']=sum(r['classification']!='UNKNOWN' for r in rows);summary['unknown_files']=sum(r['classification']=='UNKNOWN' for r in rows);summary['first_party_source_files']=sum(r['classification']=='FIRST_PARTY_SOURCE' for r in rows);summary['source_parsed']=sum(r['classification']=='FIRST_PARTY_SOURCE' and r.get('parse_status')=='PARSED' for r in rows);summary['parse_unknown']=sum(r['classification']=='FIRST_PARTY_SOURCE' and r.get('parse_status')!='PARSED' for r in rows);census.dump(path/'summary.json',summary)
   donor['tree_status']='PARTIAL' if summary['unknown_files'] else 'CLASSIFIED';census.dump(path/'identity.json',donor)
 census.dump(ROOT/'research/commerce/donors/registry.json',reg)
 print('Recognized formats classified; remaining unknowns retained')
if __name__=='__main__':main()
