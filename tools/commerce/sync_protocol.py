"""Materialize canonical subsystem bytes from locked Git blobs, preserving repository state."""
import json,pathlib,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
repo=ROOT/'research/commerce/donors/checkouts/llgtrn--Ynventa'
sha=subprocess.check_output(['git','rev-parse','HEAD'],cwd=repo,text=True).strip()
record=json.loads((ROOT/'research/commerce/protocol-origin.json').read_text())
assert sha==record['commit_sha']
exclude={'declared','evidence','history','knowledge','materialized','target','Cargo.lock'}
rows=subprocess.check_output(['git','ls-files','--stage','-z','.ynventa'],cwd=repo).decode().split('\0')
count=0
for row in rows:
 if not row:continue
 meta,name=row.split('\t',1);mode,blob,_=meta.split();parts=pathlib.PurePosixPath(name).parts
 if parts[1] in exclude:continue
 assert mode=='100644' or mode=='100755'
 dest=ROOT/name;dest.parent.mkdir(parents=True,exist_ok=True)
 dest.write_bytes(subprocess.check_output(['git','cat-file','blob',blob],cwd=repo));count+=1
print('Canonical Git blobs copied:',count,'locked commit:',sha)
