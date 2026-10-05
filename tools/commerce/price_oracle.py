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
 from price_parser.parser import parse_number, Price, parse_price, extract_price_text, extract_currency_symbol, get_decimal_separator, SAFE_CURRENCY_SYMBOLS, OTHER_CURRENCY_SYMBOLS, DOLLAR_CODES
 # Execute the actual tests module and its original Example data, without replacing donor code.
 spec=importlib.util.spec_from_file_location('locked_price_tests',DONOR/'tests/test_price_parsing.py');module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
 if '--decimal-helper' in sys.argv:
  import unicodedata
  starts=[i for i in range(0x110000) if unicodedata.category(chr(i))=='Nd' and unicodedata.decimal(chr(i))==0]
  cases=[]
  for start in starts:
   for separator in ['.',',','€']:
    for count in [1,2,3,4]:
     for suffix in ['', '\n', '\r\n', ' ', '\n\n']:
      text='prefix 1,234'+separator+''.join(chr(start+i%10) for i in range(count))+suffix
      cases.append(dict(partition='UNICODE_DIGIT_COUNT_AND_REGEX_END',input=text,expected=get_decimal_separator(text)))
  for text in ['', '.', ',12', '1.²', '1.①', '1.Ⅳ', '1.12x', '1.12\u2028', '1.12\u0085', '1.12\x00', '1.12.34', '1.1234,123', '1.12\nmore', '1.12\n\n']:
   cases.append(dict(partition='NON_DECIMAL_AND_LAST_SEPARATOR',input=text,expected=get_decimal_separator(text)))
  record=dict(oracle='UNMODIFIED_LOCKED_PRICE_PARSER_DECIMAL_SEPARATOR_HELPER',commit_sha=COMMIT,license='BSD-3-Clause',source_evidence=sources,unicode_version=unicodedata.unidata_version,decimal_alphabets=len(starts),scope='Direct get_decimal_separator behavior; Unicode Nd, 1/2/3/4 digits, Python regex final-newline semantics, invalid suffix and last-separator selection',whole_donor_parity=False,cases=cases)
  (ROOT/'adapter/web/tests/fixtures/price-decimal-helper.json').write_bytes((json.dumps(record,indent=2,ensure_ascii=False)+'\n').encode())
  print('Executed locked decimal helper:',len(cases),'cases;',len(starts),'decimal alphabets');return
 if '--full-contract' in sys.argv:
  import unicodedata
  full=[]
  for name,value in vars(module).items():
   if name.startswith('PRICE_PARSING_') and isinstance(value,list):
    for index,example in enumerate(value):
     # Compare executable donor behavior, including upstream XFAIL examples.
     # Example does not retain digit_group_separator; read call arguments from source below.
     full.append((f'{name}/{index}',example.price_raw,example.currency_raw,example.decimal_separator,None))
  texts=[None,'','free','FREE shipping','foo','50%','50% OFF','50 %','$12.99','35€ 99','35€ 999','99 € 79 €','1,235€99','12.345','1\u00a0234,56','١٢٣٫٤٥','１２３.４５','1_234.50','-12.99','USD$12','USD1$12','AUD $12 NZD','1,234.56','12..34','2 items $99','NaN','Infinity']
  for text in texts:
   for hint in [None,'USD','$','EUR','NZD $']:
    full.append(('GENERATED_PUBLIC_API',text,hint,None,None))
  for text in ['1.234,56','1,234.56','1 234.56','35€99','12.345','1\u00a0234,56']:
   for separator in [None,'.',',','€','']:
    for group in [None,'.',',',' ','\u00a0',"'",'']:
     full.append(('EXPLICIT_SEPARATOR_AND_GROUP_OVERRIDE',text,'USD',separator,group))
  for token in SAFE_CURRENCY_SYMBOLS+OTHER_CURRENCY_SYMBOLS+DOLLAR_CODES:
   for text,hint in [(token+' 12.99',None),('12.99',token),('USD 12.99',token+' $')]:
    full.append(('CURRENCY_TOKEN_PRIORITY',text,hint,None,None))
  cases=[]
  for name,text,hint,separator,group in full:
   try:
    result=Price.fromstring(text,hint,separator,group);alias=parse_price(text,hint,separator,group)
    assert (result.amount,result.currency,result.amount_text)==(alias.amount,alias.currency,alias.amount_text)
    amount=None if result.amount is None else format(result.amount,'f')
    if amount is not None and '.' in amount:amount=amount.rstrip('0').rstrip('.')
    if amount in ('-0',''):amount='0'
    expected={'amount':amount,'currency':result.currency,'amount_text':result.amount_text,'amount_float':result.amount_float}
    error=None
   except (AssertionError,ValueError) as e:expected=None;error=type(e).__name__
   cases.append(dict(name=name,input=text,currency_hint=hint,decimal_separator=separator,digit_group_separator=group,expected=expected,error=error))
  record=dict(oracle='UNMODIFIED_LOCKED_PRICE_PARSER_FULL_PUBLIC_PRICE_API',commit_sha=COMMIT,license='BSD-3-Clause',source_evidence=sources,scope='Actual Price.fromstring and parse_price alias behavior, exact amount/currency/amount_text/amount_float; source fixtures and generated API partitions, not native parity or extinction',native_parity=False,whole_donor_parity=False,cases=cases,currency_tokens={'safe':SAFE_CURRENCY_SYMBOLS,'unsafe':OTHER_CURRENCY_SYMBOLS,'dollar_codes':DOLLAR_CODES},limitations=['Source Example digit-group override is not stored on Example; generated explicit group overrides cover the executable API independently.','Helper parse_number direct nonfinite/exponent/resource cases and packaging/attrs semantics require separate coverage before full donor replacement.','Currency tables contain data under the locked BSD license; no executable donor code is copied into production.'])
  folder=ROOT/'adapter/web/tests/fixtures';(folder/'price-contract.json').write_text(json.dumps(record,indent=2,ensure_ascii=False,allow_nan=False)+'\n',encoding='utf-8',newline='\n')
  data=ROOT/'adapter/web/data';data.mkdir(exist_ok=True)
  lexicon=dict(record['currency_tokens'],license='BSD-3-Clause',notice='adapter/web/tests/fixtures/price-parser-LICENSE.txt',source_commit=COMMIT,source_sha256=sources[1]['sha256'],unicode_version=unicodedata.unidata_version,decimal_digit_starts=[i for i in range(0x110000) if unicodedata.category(chr(i))=='Nd' and unicodedata.decimal(chr(i))==0])
  (data/'price-currency.json').write_text(json.dumps(lexicon,indent=2,ensure_ascii=False)+'\n',encoding='utf-8',newline='\n')
  print('Executed locked full public Price API oracle:',len(cases),'cases; native comparison pending');return
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
