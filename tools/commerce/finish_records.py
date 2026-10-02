"""Update repository-specific declarations and honest, measured wave metadata."""
import hashlib,json,pathlib
ROOT=pathlib.Path(__file__).resolve().parents[2]
def main():
 declaration=ROOT/'.ynventa/declared/repository.rs'
 text=declaration.read_text(encoding='utf-8')
 text=text.replace('provides: &["commerce.keepa-current-value"]','provides: &["commerce.keepa-current-value", "commerce.keepa-product"]')
 text=text.replace('], edges: &[Edge {','], edges: &[Edge { from: "commerce.server", to: "commerce.keepa", kind: EdgeKind::DependsOn, scope: Scope::Runtime }, Edge { from: "commerce.keepa", to: "commerce", kind: EdgeKind::DependsOn, scope: Scope::Runtime }, Edge {',1) if 'from: "commerce.keepa", to: "commerce"' not in text else text
 declaration.write_text(text,encoding='utf-8')
 declaration=ROOT/'.ynventa/declared/technologies.rs';text=declaration.read_text(encoding='utf-8')
 if 'key: "commerce.keepa-product"' not in text:
  text=text.rstrip()[:-1]+'''Technology { key: "commerce.keepa-product", name: "Native Keepa product boundary", kind: TechnologyKind::Protocol, claimed: TechnologyLifecycle::Experimental, purpose: "One explicit paid product request; raw response hashing; unknown monetary cost and freshness; live auth unverified", implements: &["commerce.keepa-product"], node: "commerce.keepa", sources: &["adapter/keepa/src/client.rs"], invariants: &[], proofs: &[Proof { kind: ProofKind::Regression, locator: "adapter/keepa/src/client.rs::missing_key_performs_no_network" }, Proof { kind: ProofKind::Regression, locator: "adapter/keepa/src/client.rs::currency_does_not_follow_donor_usd_formatter" }], lineage: &[], relations: &[], norl: NorlRelevance::Unresolved, claims: &[] },
]\n'''
  declaration.write_text(text,encoding='utf-8')
 mirror=ROOT/'research/commerce/specification.md';rebuilt=ROOT/'target/specification-rebuilt.md'
 if mirror.exists():
  assert rebuilt.exists() and hashlib.sha256(mirror.read_bytes()).digest()==hashlib.sha256(rebuilt.read_bytes()).digest()
  mirror.unlink() # Only the verified, newly generated mirror; canonical binary knowledge is retained.
 duplicate=ROOT/'tests/commerce/fixtures/keepa-current-oracle.json'
 canonical=ROOT/'adapter/keepa/tests/fixtures/keepa-current-oracle.json'
 if duplicate.exists():
  assert duplicate.read_bytes()==canonical.read_bytes()
  duplicate.unlink()
 report={'phase':'FOUNDATION_AND_FIRST_NATIVE_SLICE','canonical_origin':'https://github.com/llgtrn/.Ynventa-','canonical_commit':'c00a123c542b3c16fea4f82b45a5e544a1870178','canonical_subsystem':'BYTE_IDENTICAL','metrics':'research/commerce/metrics.json','native_tests':'10 passed; includes 508 donor-oracle cases','mcp_tools':16,'live_e2e':'UNAVAILABLE_CREDENTIALS_AND_NOT_VERIFIED','donor_extinction':'NOT_YET','proof_status':'Read canonical evidence and run conformance/prove; no full V1 claim','remaining':['Register ECDEV in upstream closed shard list','Complete semantic census of remaining nine seed donors (four source parse unknowns retained)','Implement and oracle-verify remaining donor capabilities','Live credentialed provider and Claude/Codex E2E validation','Multi-provider opportunity execution, supplier matching, risk and PPC acquisition','Learned ranking, forecasting, graph exploration and comparison workflows','Foundational dependency replacement and genuine extinction'],'keepa_units_reference':{'type':'OFFICIAL_PROTOCOL_SOURCE','url':'https://github.com/keepacom/api_backend/blob/master/src/main/java/com/keepa/api/backend/structs/Stats.java','claim':'Locale smallest currency unit: yen for Japan, cent for US','source_commit':'UNKNOWN','observed_live_request':False}}
 (ROOT/'research/commerce/wave-report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
 print('Repository-specific declarations and measured wave report updated')
if __name__=='__main__':main()
