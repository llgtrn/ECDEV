"""Execute the unmodified pinned w3lib decoder for the declared, valid-byte subset."""
import hashlib,json,pathlib,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[2]
DONOR=ROOT/'research/commerce/donors/checkouts/scrapy--w3lib'
COMMIT='537c5d46455ae8b2c67b53fc03b36ef1da8c4837'
def git(*args):return subprocess.check_output(['git','-C',str(DONOR),*args])
def main():
 assert git('rev-parse','HEAD').decode().strip()==COMMIT
 evidence=[]
 for path in ['w3lib/encoding.py','tests/test_encoding.py','LICENSE']:
  raw=git('show',COMMIT+':'+path)
  assert raw.decode().replace('\r\n','\n')==(DONOR/path).read_text(encoding='utf-8')
  evidence.append(dict(path=path,commit_sha=COMMIT,blob_hash=git('rev-parse',COMMIT+':'+path).decode().strip(),sha256=hashlib.sha256(raw).hexdigest()))
 sys.path.insert(0,str(DONOR))
 from w3lib.encoding import html_to_unicode
 cases=[]
 for label,codec,text in [('utf-8','utf-8','耐熱ガラス'),('Shift_JIS','cp932','耐熱ガラス'),('EUC-JP','euc_jp','耐熱ガラス'),('windows-1252','cp1252','café €'),('ISO-8859-1','cp1252','café €'),('windows-1251','cp1251','товар'),('Big5','big5','商品'),('GBK','gb18030','商品'),('EUC-KR','cp949','상품')]:
  for declaration in ['header','quoted-header','meta','http-equiv','header-over-meta']:
   prefix={'meta':f'<meta charset="{label}">','http-equiv':f'<meta http-equiv="Content-Type" content="text/html; charset={label}">','header-over-meta':'<meta charset="utf-8">'}.get(declaration,'')
   html=prefix+f'<title>{text}</title>'
   raw=html.encode(codec)
   header=f'text/html; charset={label}' if declaration in ['header','header-over-meta'] else f'text/html; charset="{label}"' if declaration=='quoted-header' else 'text/html'
   _,expected=html_to_unicode(header,raw)
   cases.append(dict(name=label+'/'+declaration,bytes=list(raw),content_type=header,expected=expected))
 for codec,bom in [('utf-8',b'\xef\xbb\xbf'),('utf-16-le',b'\xff\xfe'),('utf-16-be',b'\xfe\xff')]:
  for header in ['text/html','text/html; charset=shift_jis','text/html; charset=unknown']:
   raw=bom+'<title>耐熱ガラス</title>'.encode(codec)
   _,expected=html_to_unicode(header,raw)
   cases.append(dict(name=codec+'/bom/'+header,bytes=list(raw),content_type=header,expected=expected))
 out=ROOT/'adapter/web/tests/fixtures'
 output=dict(oracle='UNMODIFIED_LOCKED_W3LIB_HTML_TO_UNICODE',commit_sha=COMMIT,license='BSD-3-Clause',source_evidence=evidence,scope='Valid explicitly declared HTML encodings and BOM precedence only; no replacement, statistical guessing, UTF-32, malformed declaration or complete Scrapy parity claim.',cases=cases)
 (out/'w3lib-document.json').write_text(json.dumps(output,ensure_ascii=False,indent=2)+'\n',encoding='utf-8',newline='\n')
 (out/'w3lib-LICENSE.txt').write_bytes(git('show',COMMIT+':LICENSE'))
 print('Executed locked w3lib:',len(cases),'cases',COMMIT)
if __name__=='__main__':main()
