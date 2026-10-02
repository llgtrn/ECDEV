"""Execute locked extruct modules as research-only oracles, never runtime imports."""
import hashlib, importlib.metadata, json, pathlib, subprocess, sys, types
ROOT=pathlib.Path(__file__).resolve().parents[2]
DONOR=ROOT/'research/commerce/donors/checkouts/scrapinghub--extruct'
sys.path.insert(0,str(ROOT/'.venv/research'))
def git(*args):return subprocess.check_output(['git','-C',str(DONOR),*args]).decode().strip()
def checked(path,commit):
    raw=subprocess.check_output(['git','-C',str(DONOR),'show',commit+':'+path])
    assert raw.decode('utf-8').replace('\r\n','\n')==(DONOR/path).read_text(encoding='utf-8')
    return dict(path=path,commit_sha=commit,blob_hash=git('rev-parse',commit+':'+path),sha256=hashlib.sha256(raw).hexdigest()),raw

def main():
    commit=git('rev-parse','HEAD')
    lock=json.loads((ROOT/'research/commerce/donors/census/scrapinghub--extruct/identity.json').read_text(encoding='utf-8'))['commit_sha'];assert commit==lock
    sources=[]
    for path in ['extruct/w3cmicrodata.py','extruct/utils.py','extruct/xmldom.py','tests/test_microdata.py','LICENSE']:
        evidence,_=checked(path,commit);sources.append(evidence)
    # Namespace package bypasses unrelated __init__ imports, not parser implementation.
    package=types.ModuleType('extruct');package.__path__=[str(DONOR/'extruct')];sys.modules['extruct']=package
    from extruct.w3cmicrodata import LxmlMicrodataExtractor
    fixtures=['schema.org/product.html','schema.org/product-ref.html','schema.org/CreativeWork.001.html','schema.org/LocalBusiness.002.html','schema.org/LocalBusiness.003.html','schema.org/MusicRecording.001.html','schema.org/SearchAction.001.html','w3c/microdata.4.2.strings.html','w3c/microdata.4.2.strings.unclean.html','w3c/microdata.4.2.data.html','w3c/microdata.4.2.meter.html','w3c/microdata.object.html','w3c/microdata.5.2.html','w3c/microdata.5.3.html','w3c/microdata.5.5.html','w3c/microdata.7.1.html','websites/microdata-with-description.html']
    inputs=[]
    for fixture in fixtures:
        evidence,raw=checked('tests/samples/'+fixture,commit);sources.append(evidence)
        inputs.append((fixture,raw.decode('utf-8'),evidence))
    for tag,attribute,value in [('meta','content','Cup'),('img','src','../cup.png'),('a','href','/cup?q=1'),('object','data','catalog.pdf'),('data','value','2980'),('meter','value','5'),('time','datetime','2026-10-02'),('video','src','/cup.mp4'),('iframe','src','/spec'),('embed','src','/info')]:
        inputs.append((f'derived-values-{tag}',f'<html><body><div itemscope itemtype="https://schema.org/Product"><{tag} itemprop="name secondary" {attribute}="{value}"></{tag}></div></body></html>',None))
    inputs += [
        ('derived-nested-offers','<div itemscope itemtype="https://schema.org/Product"><span itemprop="name">Cup</span><div itemprop="offers" itemscope itemtype="https://schema.org/Offer"><meta itemprop="price" content="2980"><meta itemprop="priceCurrency" content="JPY"><link itemprop="availability" href="https://schema.org/InStock"></div><div itemprop="offers" itemscope itemtype="https://schema.org/Offer"><meta itemprop="price" content="3200"></div></div>',None),
        ('derived-references-cycle','<div id="a" itemscope itemtype="https://schema.org/Product" itemref="b"><span itemprop="name">Cup</span></div><div id="b" itemscope itemtype="https://schema.org/Product" itemprop="related" itemref="a"><span itemprop="name">Other</span></div>',None),
        ('derived-multiple-types-id','<div itemscope itemtype="https://schema.org/Product https://schema.org/Thing" itemid=" /cup "><meta itemprop="sku" content="SKU"></div>',None),
        ('derived-clean-text','<div itemscope><div itemprop="description"><p>A <b>cup</b>, new.</p><script>bad</script><style>bad</style><p>Another<br>line.</p></div></div>',None),
        ('derived-repeat-property','<div itemscope><meta itemprop="sku" content="one"><meta itemprop="sku" content="two"><span itemprop="name brand">Example</span></div>',None),
        ('derived-action','<div itemscope itemtype="https://schema.org/SearchAction"><input itemprop="query-input" required name="q"><input itemprop="result-output" name="value"></div>',None),
    ]
    cases=[]
    for name,html,evidence in inputs:
        for base in [None,'https://shop.example/catalog/list']:
            for strict in [False,True]:
                expected=LxmlMicrodataExtractor(strict=strict).extract(html,base_url=base)
                cases.append(dict(name=f'{name}/base={base}/strict={strict}',html=html,base_url=base,strict=strict,expected=expected,fixture_source=evidence))
    output=dict(donor='scrapinghub/extruct',commit_sha=commit,license='BSD-3-Clause',oracle='UNMODIFIED_LOCKED_LXML_MICRODATA_EXTRACTOR',source_evidence=sources,environment={name:importlib.metadata.version(name) for name in ['lxml','html-text','w3lib','lxml_html_clean']},contract='Nested microdata scope and itemref graph, repeated/multi-name properties, item types and IDs, HTML value tags, URL resolution and cleaned text; strict and default scalar/list outputs. Flat iid, optional htmlNode/textContent and malformed-parser equivalence are not claimed.',cases=cases)
    destination=ROOT/'adapter/web/tests/fixtures';destination.mkdir(parents=True,exist_ok=True)
    (destination/'extruct-microdata.json').write_text(json.dumps(output,indent=2,ensure_ascii=False)+'\n',encoding='utf-8',newline='\n')
    _,license_bytes=checked('LICENSE',commit);(destination/'extruct-LICENSE.txt').write_bytes(license_bytes)
    print('Executed locked extruct oracle:',len(cases),'cases',commit)
if __name__=='__main__':main()
