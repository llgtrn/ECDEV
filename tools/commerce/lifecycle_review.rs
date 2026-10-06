//! Donor lifecycle snapshot from the ECDEV governance assessment.
use ecdev_governance::formats::json::Json;
fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let assessment = ecdev_governance::assess(std::path::Path::new(&root)).expect("governance assessment");
    let rows: Vec<_> = assessment
        .analysis
        .donors
        .iter()
        .map(|donor| {
            let capabilities: Vec<_> = donor
                .capabilities
                .iter()
                .map(|capability| {
                    Json::obj()
                        .with("key", &capability.key)
                        .with("required", capability.required)
                        .with("replacement_exists", capability.replacement_exists)
                        .with("native", capability.native)
                        .with("native_detail", &capability.native_detail)
                        .with("relevance_resolved", capability.relevance_resolved)
                        .with("parity_pass", capability.parity_pass())
                        .with("regression_pass", capability.regression_pass())
                })
                .collect();
            Json::obj()
                .with("donor_id", &donor.key)
                .with("claimed", donor.claimed.wire())
                .with("effective", donor.effective.wire())
                .with("stopped_by", &donor.stopped_by)
                .with("capabilities", Json::Array(capabilities))
                .with("extinct", donor.extinct())
        })
        .collect();
    println!("{}", Json::Array(rows).render());
}
