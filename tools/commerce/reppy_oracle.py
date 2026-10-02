"""Execute the immutable donor parser, never a handwritten substitute oracle."""
import hashlib,json,pathlib,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[2]
DONOR=ROOT/'research/commerce/donors/checkouts/seomoz--reppy'
def main():
    commit=subprocess.check_output(['git','-C',str(DONOR),'rev-parse','HEAD'],text=True).strip()
    lock=json.loads((ROOT/'research/commerce/donors/census/seomoz--reppy/identity.json').read_text())['commit_sha'];assert commit==lock
    policies=[
        '# empty policy\n', 'User-agent: *\nDisallow:\n',
        'User-agent: *\nDisallow: /private\nAllow: /private/public\n',
        'User-agent: *\nDisallow: /*?*\n',
        'User-agent: *\nDisallow: /*.pdf$\n',
        'User-agent: *\nDisallow: /products/*?variant=*\n',
        'User-agent: *\nDisallow: /a**b***c*\n',
        'User-agent: *\nDisallow: /x$\n',
        'User-agent: *\nDisallow: /%7Euser\n',
        'User-agent: *\nDisallow: /a%3Cd.html\n',
        'User-agent: *\nDisallow: /caf%C3%A9\n',
        'User-agent: *\nDisallow: /\nUser-agent: ECDEV\nAllow: /products\nDisallow: /products/private\n',
        'User-agent: ecdev\nUser-agent: OtherBot\nDisallow: /private\n',
        'User-agent: ecdev\nDisallow: /private\nUser-agent: ecdev\nDisallow: /products\n',
        'User-agent: *\nDisallow: /private\nCrawl-delay: 1.5\n',
        'USER-AGENT: ECDEV\nDISALLOW: /private # comment\nALLOW: /private/public\nCRAWL-DELAY: 0\n',
        'User-agent: *\nDisallow: /assets/*\nAllow: /assets/safe/*\n',
        'User-agent: *\nDisallow: /products?sort_by=*\n',
        '\ufeffUser-agent: *\r\nDisallow: /private\r\n',
        'User-agent: *\nDisallow: https://foreign.example/products\n',
    ]
    paths=['/','/robots.txt','/private','/private/secret','/private/public','/products','/products/private','/products/item','/products/item?variant=2','/products?sort_by=price','/products?sku=7','/x','/xy','/file.pdf','/file.pdf?download=1','/axbxc','/abc','/~user','/%7euser','/a%3Cd.html','/a<d.html','/caf%C3%A9','/café','/assets/private/a','/assets/safe/a','/collections/all','/collections/all?page=2']
    cases=[dict(robots=p,url='https://shop.example'+path,agent=agent) for p in policies for path in paths for agent in ['ECDEV','OtherBot','UnknownBot']]
    stdin=''.join(' '.join(c[k].encode().hex() for k in ['robots','url','agent'])+'\n' for c in cases)
    exe=ROOT/'target/reppy-oracle.exe'
    outputs=subprocess.check_output([str(exe)],input=stdin,text=True).splitlines();assert len(outputs)==len(cases)
    for c,line in zip(cases,outputs):
        allowed,delay=line.split();c.update(allowed=allowed=='1',crawl_delay_seconds=None if float(delay)<0 else float(delay))
    sources=[]
    for path in ['reppy/rep-cpp/src/robots.cpp','reppy/rep-cpp/src/agent.cpp','reppy/rep-cpp/src/directive.cpp','reppy/rep-cpp/deps/url-cpp/src/url.cpp','reppy/rep-cpp/deps/url-cpp/src/utf8.cpp','reppy/rep-cpp/deps/url-cpp/src/punycode.cpp','reppy/rep-cpp/deps/url-cpp/src/psl.cpp','tests/test_agent.py','tests/test_robots.py','LICENSE']:
        checkout=DONOR;relative=path;source_commit=commit
        if path.startswith('reppy/rep-cpp/'):
            relative=path.removeprefix('reppy/rep-cpp/');checkout=DONOR/'reppy/rep-cpp'
            source_commit=subprocess.check_output(['git','-C',str(DONOR),'rev-parse',commit+':reppy/rep-cpp'],text=True).strip()
        if relative.startswith('deps/url-cpp/'):
            parent=checkout;relative=relative.removeprefix('deps/url-cpp/');checkout=parent/'deps/url-cpp'
            source_commit=subprocess.check_output(['git','-C',str(parent),'rev-parse',source_commit+':deps/url-cpp'],text=True).strip()
        assert subprocess.check_output(['git','-C',str(checkout),'rev-parse','HEAD'],text=True).strip()==source_commit
        blob=subprocess.check_output(['git','-C',str(checkout),'show',source_commit+':'+relative]);assert blob.decode().replace('\r\n','\n')==(DONOR/path).read_text(encoding='utf-8')
        sources.append(dict(path=path,commit_sha=source_commit,parent_commit=commit,blob_hash=subprocess.check_output(['git','-C',str(checkout),'rev-parse',source_commit+':'+relative],text=True).strip(),sha256=hashlib.sha256(blob).hexdigest()))
    fixture=dict(donor='seomoz/reppy',commit_sha=commit,source_evidence=sources,license='MIT',oracle='COMPILED_UNMODIFIED_DONOR_CPP',compiler='MSVC_CXX14',cases=cases,contract='Public same-origin robots allowed decisions and crawl-delay for exact case-insensitive agent selection; ambiguity remains conservatively denied')
    path=ROOT/'adapter/web/tests/fixtures/reppy-robots.json';path.parent.mkdir(parents=True,exist_ok=True);path.write_text(json.dumps(fixture,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print('Executed donor oracle cases:',len(cases),'at',commit)
if __name__=='__main__':main()
