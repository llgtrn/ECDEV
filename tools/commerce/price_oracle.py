"""Run the unmodified locked donor numeric parser over all donor price-text fixtures."""
import ast,hashlib,importlib.util,json,pathlib,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[2];DONOR=ROOT/'research/commerce/donors/checkouts/scrapinghub--price-parser';COMMIT='64e213a46a40473ba4f8aa3b249917fdc64d8a16'
def git(*args):return subprocess.check_output(['git','-C',str(DONOR),*args])
def main():
 assert git('rev-parse','HEAD').decode().strip()==COMMIT
 sources=[]
 for path in ['price_parser/parser.py','price_parser/_currencies.py','tests/test_price_parsing.py','LICENSE']:
  raw=git('show',COMMIT+':'+path);assert raw.decode().replace('\r\n','\n')==(DONOR/path).read_text(encoding='utf-8')
  sources.append(dict(path=path,commit_sha=COMMIT,blob_hash=git('rev-parse',COMMIT+':'+path).decode().strip(),sha256=hashlib.sha256(raw).hexdigest()))
 sys.path[:0]=[str(DONOR),str(ROOT/'.venv/research')]
 from price_parser.parser import parse_number
 # Execute the actual tests module and its original Example data, without replacing donor code.
 spec=importlib.util.spec_from_file_location('locked_price_tests',DONOR/'tests/test_price_parsing.py');module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
 inputs=[]
 for name,value in vars(module).items():
  if name.startswith('PRICE_PARSING_') and isinstance(value,list):
   for index,example in enumerate(value):
    if example.amount_text is not None:inputs.append((f'{name}/{index}',example.amount_text,example.decimal_separator))
 for raw in ['','foo','140.000','140,000€33','1 235€99','1.235€99','.75','12.345','3,0000','+12.99','-12.99','1e3','1.2e-3','1_234.50','12..34','1,234.56','0.0000000001','000.00','999999999999999999999999.99']:
  for separator in [None,'.',',','€']:inputs.append(('derived/'+raw+'/'+str(separator),raw,separator))
 cases=[]
 for name,raw,separator in inputs:
  number=parse_number(raw,separator)
  expected=None
  if number is not None:
   expected=format(number,'f');expected=expected.rstrip('0').rstrip('.') if '.' in expected else expected
   if expected in ['-0','']:expected='0'
  cases.append(dict(name=name,input=raw,decimal_separator=separator,expected=expected))
 record=dict(oracle='UNMODIFIED_LOCKED_PRICE_PARSER_PARSE_NUMBER',commit_sha=COMMIT,license='BSD-3-Clause',source_evidence=sources,scope='Numeric format normalization including separator guessing and explicit separator overrides, exact decimal strings. Currency detection and first-price text selection are separate donor APIs, not claimed.',cases=cases)
 folder=ROOT/'adapter/web/tests/fixtures';(folder/'price-number.json').write_text(json.dumps(record,indent=2,ensure_ascii=False)+'\n',encoding='utf-8',newline='\n');(folder/'price-parser-LICENSE.txt').write_bytes(git('show',COMMIT+':LICENSE'))
 print('Executed actual price-parser numeric oracle:',len(cases),'cases')
if __name__=='__main__':main()
