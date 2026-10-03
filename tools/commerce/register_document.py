"""Register the bounded document decoder proof without claiming full donor absorption."""
import hashlib,json,pathlib
ROOT=pathlib.Path(__file__).resolve().parents[2]
def write(path,value):path.write_text(json.dumps(value,indent=2,ensure_ascii=False)+'\n',encoding='utf-8',newline='\n')
def main():
 fixture=ROOT/'adapter/web/tests/fixtures/w3lib-document.json';oracle=json.loads(fixture.read_text(encoding='utf-8'))
 path=ROOT/'research/commerce/capabilities.json';records=json.loads(path.read_text(encoding='utf-8'))
 records=[r for r in records if r['capability_id']!='extract.declared-document']
 records.append(dict(capability_id='extract.declared-document',donor_id='dependency--github-com-scrapy-w3lib',status='VERIFIED',evidence=oracle['source_evidence'],native_implementation='adapter/web/src/document.rs',native_status='EXPERIMENTAL_BOUNDED_REPLACEMENT',oracle_status='54_DECLARED_DOCUMENT_CASES_MATCHED',oracle_cases=54,commit_sha=oracle['commit_sha'],fixture=str(fixture.relative_to(ROOT)).replace('\\','/'),fixture_sha256=hashlib.sha256(fixture.read_bytes()).hexdigest(),license='BSD-3-Clause',runtime_donor_source_dependency=False,remaining=['Full Scrapy and w3lib census and parity','Malformed or undeclared legacy encodings','UTF-32','Statistical charset guessing intentionally unavailable'],scope=oracle['scope']))
 write(path,records)
 p=ROOT/'.ynventa/declared/repository.rs';text=p.read_text(encoding='utf-8');text=text.replace('provides: &["extract.microdata-graph"','provides: &["extract.declared-document", "extract.microdata-graph"');p.write_text(text,encoding='utf-8',newline='\n')
 p=ROOT/'.ynventa/declared/technologies.rs';text=p.read_text(encoding='utf-8').rstrip();assert text.endswith(']')
 if 'commerce.declared-document' not in text:
  text=text[:-1]+'Technology { key: "commerce.declared-document", name: "Declared HTML decoding with wire provenance", kind: TechnologyKind::Parser, claimed: TechnologyLifecycle::Experimental, purpose: "54 real locked w3lib outputs for valid declared encodings and BOM precedence; raw capture hash retained; full Scrapy absorption unproven", implements: &["extract.declared-document"], node: "commerce.web-research", sources: &["adapter/web/src/document.rs"], invariants: &[], proofs: &[Proof { kind: ProofKind::Parity, locator: "adapter/web/src/document.rs::locked_w3lib_declared_document_oracle" }, Proof { kind: ProofKind::Regression, locator: "adapter/web/src/document.rs::declared_japanese_preserves_wire_provenance" }, Proof { kind: ProofKind::Regression, locator: "adapter/web/src/document.rs::precedence_and_failure_are_explicit" }], lineage: &["dependency--github-com-scrapy-w3lib"], relations: &[], norl: NorlRelevance::Unresolved, claims: &[] },\n]'
 p.write_text(text+'\n',encoding='utf-8',newline='\n')
 # Dependency source checkout is research-only. Production encoding_rs is counted separately.
 p=ROOT/'research/commerce/dependencies.json';deps=json.loads(p.read_text(encoding='utf-8'))
 for package in deps['packages']:
  if package['package']=='w3lib':package.update(commit_sha=oracle['commit_sha'],clone_status='FULL_CLONE',notes='Pinned v2.5.0 source and selected decoding contract oracle reviewed; no full census or absorption claim.')
 write(p,deps)
 print('Registered bounded declared-document oracle: 54 cases')
if __name__=='__main__':main()
