//! Standalone source-side conformance for two finite original price components.
//! From the ECDEV repository root:
//! rustc --edition=2021 --test domain/commerce/tests/price_component_contract.rs -o /tmp/ecdev-price-conformance
//! /tmp/ecdev-price-conformance
//! The local source imports are test-only; there is no cross-repository build dependency.

#[path = "../src/price_ascii.rs"]
mod ascii;
#[path = "../src/price_explicit.rs"]
mod exact;
mod price_contract_cases;
