"""Execute a locked Graphiti UTC normalization helper, without Graphiti runtime imports."""
import ast, calendar, datetime, hashlib, json, pathlib, random, subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2]
DONOR=ROOT/"research/commerce/donors/checkouts/getzep--graphiti"
COMMIT="5d47d4d0182aa6350edeb435b734837a88ed9738"
SOURCE="graphiti_core/utils/datetime_utils.py"
def main():
    raw=subprocess.check_output(["git","-C",str(DONOR),"show",COMMIT+":"+SOURCE])
    tree=ast.parse(raw)
    nodes=[node for node in tree.body if isinstance(node,ast.ImportFrom) and node.module=="datetime" or isinstance(node,ast.FunctionDef) and node.name=="ensure_utc"]
    namespace={}
    exec(compile(ast.Module(body=nodes,type_ignores=[]),SOURCE,"exec"),namespace)
    normalize=namespace["ensure_utc"]
    rng=random.Random(73291);cases=[]
    offsets=[-1439,-840,-570,-330,-60,0,60,330,345,570,840,1439]
    dates=[datetime.datetime(1970,1,2),datetime.datetime(2000,2,29),datetime.datetime(2024,2,29,23,59,59),datetime.datetime(2024,3,1,0,0,1),datetime.datetime(2100,3,1)]
    dates += [datetime.datetime(rng.randint(1971,2099),rng.randint(1,12),rng.randint(1,28),rng.randrange(24),rng.randrange(60),rng.randrange(60)) for _ in range(70)]
    for index,dt in enumerate(dates):
        for offset in offsets:
            aware=dt.replace(microsecond=123456 if index%3==0 else 0,tzinfo=datetime.timezone(datetime.timedelta(minutes=offset)))
            normalized=normalize(aware)
            text=aware.isoformat()
            cases.append({"partition":"UTC_OFFSET_DAY_MONTH_YEAR_CROSSING_AND_FRACTIONS","input":text,"expected":calendar.timegm(normalized.utctimetuple()),"normalized_utc":normalized.isoformat()})
    cases.append({"partition":"UTC_Z_SUFFIX","input":"2024-02-29T00:00:00Z","expected":1709164800})
    fixture={"oracle":"UNMODIFIED_LOCKED_GRAPHITI_ENSURE_UTC","donor":"getzep--graphiti","commit_sha":COMMIT,"source_path":SOURCE,"source_sha256":hashlib.sha256(raw).hexdigest(),"source_blob_hash":subprocess.check_output(["git","-C",str(DONOR),"rev-parse",COMMIT+":"+SOURCE]).decode().strip(),"source_symbol":"ensure_utc","scope":"Explicit RFC3339 timezone inputs, integer epoch seconds; no Graphiti model/graph/runtime/LLM execution","intentional_divergences":["Naive datetimes and RFC3339 unknown local offset -00:00 remain UNKNOWN; donor assumes UTC for naive values.","Native production publication timestamps are nonnegative integer epoch seconds; fractional precision is not retained."],"cases":cases}
    p=ROOT/"adapter/web/tests/fixtures/graphiti-publication-time.json";p.write_text(json.dumps(fixture,indent=2)+"\n",encoding="utf-8")
    print("Graphiti locked UTC cases",len(cases))
if __name__=="__main__":main()
