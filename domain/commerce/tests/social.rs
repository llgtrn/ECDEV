use ecdev_core::{Engine, social::*};
use serde_json::{Value, json};
use uuid::Uuid;
fn post(id: &str, platform: &str, at: u64) -> SocialPost {
    SocialPost {
        platform: platform.into(),
        provider: "test-native-social".into(),
        source_url: format!("https://example.org/{id}"),
        native_id: id.into(),
        thread_id: None,
        author_id: Some("author".into()),
        publisher: None,
        published_at: Some(at),
        captured_at: 10000,
        text: "matcha glass good".into(),
        language: Some("en".into()),
        media: vec![],
        hashtags: vec!["matcha".into()],
        mentions: vec![],
        entities: vec![
            json!({"kind":"URL","value":"https://example.org/product","identity_state":"SOURCE_ASSERTED"}),
        ],
        propagation: "ORIGINAL".into(),
        parent_id: None,
        engagement: SocialEngagement {
            views: None,
            likes: Some(0),
            comments: None,
            reposts: None,
            favorites: None,
            followers: None,
        },
        raw_hash: "a".repeat(64),
        raw_locator: "/items/0".into(),
        extraction_method: "FIXTURE".into(),
        state: EvidenceState::Observed,
        capture_mode: "FIXTURE".into(),
        evidence_id: format!("evidence-{platform}-{id}"),
        freshness_seconds: 10000_u64.checked_sub(at),
        origin_evidence_id: None,
    }
}
fn root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("ecdev-social-{}", Uuid::new_v4()))
}
#[test]
fn social_live_capture_requires_http_witness_and_preserves_unknown_counts() {
    use ecdev_core::provider::{AcquireError, AcquireRequest, AcquireResult, Provider};
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    struct Witness(Value);
    impl Provider for Witness {
        fn id(&self) -> &str {
            "native-social"
        }
        fn metadata(&self) -> Value {
            json!({"class":"PUBLIC","cost_minor":0})
        }
        fn acquire(&self, _: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
            let raw = b"mock social capture";
            let mut p = post("unwitnessed", "HACKER_NEWS", 1);
            p.capture_mode = "LIVE".into();
            p.raw_hash = format!("{:x}", Sha256::digest(raw));
            Ok(AcquireResult {
                observations: vec![],
                result: json!({"posts":[p]}),
                raw_payload: raw.to_vec(),
                provider_cost: json!({"request_count":self.0,"cost_minor":0}),
            })
        }
    }
    for count in [json!(0), Value::Null, json!(2)] {
        let path = root();
        let engine = Engine::open(&path)
            .unwrap()
            .with_provider(Arc::new(Witness(count.clone())));
        let result = engine
            .trend_discover(json!({"query":"matcha","sources":[{"platform":"HACKER_NEWS"}]}))
            .unwrap();
        let run = engine.run(result["run_id"].as_str().unwrap()).unwrap();
        if count == 2 {
            assert_eq!(run["mode"], "LIVE");
            assert_eq!(run["observations"].as_array().unwrap().len(), 1);
            assert_eq!(result["source_complete"], true);
        } else {
            assert_eq!(
                run["mode"],
                if count.is_null() {
                    "INFERRED"
                } else {
                    "PLAN_ONLY"
                }
            );
            assert_eq!(run["observations"], json!([]));
            assert_eq!(result["source_complete"], false);
            assert_eq!(result["budget_usage"]["request_count"], count);
            assert_eq!(
                result["acquisition_provenance"]["live_acquisition_established"],
                false
            );
            assert_eq!(
                result["provider_failures"][0]["reason"],
                "SOCIAL_CAPTURE_HTTP_WITNESS_MISSING_OR_MODE_MISMATCH"
            );
            assert!(!path.join(".ecdev-data/runtime/social-captures").exists());
        }
        drop(engine);
        std::fs::remove_dir_all(path).unwrap();
    }
    let path = root();
    let engine = Engine::open(&path).unwrap();
    let result = engine
        .trend_discover(
            json!({"query":"matcha","sources":[{"platform":"HACKER_NEWS"}],"request_budget":0}),
        )
        .unwrap();
    assert_eq!(
        engine.run(result["run_id"].as_str().unwrap()).unwrap()["mode"],
        "PLAN_ONLY"
    );
    assert_eq!(result["source_complete"], false);
    assert_eq!(result["budget_usage"]["request_count"], 0);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn cached_captures_require_original_bytes_and_never_establish_live_acquisition() {
    use sha2::{Digest, Sha256};
    let root = root();
    let engine = Engine::open(&root).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let raw = b"original source payload";
    let mut p = post("cached", "HACKER_NEWS", now - 10);
    p.capture_mode = "LIVE".into();
    p.captured_at = now;
    p.raw_hash = format!("{:x}", Sha256::digest(raw));
    let dir = root.join(".ecdev-data/runtime/social-captures");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.raw", p.raw_hash));
    std::fs::write(&path, raw).unwrap();
    let source = json!({"platform":"HACKER_NEWS"});
    let key = format!(
        "{:x}",
        Sha256::digest(
            json!({"query":"matcha","source":source})
                .to_string()
                .as_bytes()
        )
    );
    let db = rusqlite::Connection::open(root.join(".ecdev-data/runtime/ecdev.sqlite")).unwrap();
    db.execute(
        "INSERT INTO social_cache VALUES(?1,?2,?3)",
        rusqlite::params![key, now, json!([p]).to_string()],
    )
    .unwrap();
    drop(db);
    let request = json!({"query":"matcha","sources":[source],"cache_only":true});
    let valid = engine.trend_discover(request.clone()).unwrap();
    assert_eq!(valid["mention_count"], 1);
    assert_eq!(valid["budget_usage"]["cache_hits"], 1);
    assert_eq!(valid["budget_usage"]["request_count"], 0);
    assert_eq!(
        valid["acquisition_provenance"]["live_acquisition_established"],
        false
    );
    assert_eq!(valid["acquisition_provenance"]["new_observation_count"], 0);
    std::fs::write(&path, b"changed bytes").unwrap();
    for missing in [false, true] {
        if missing {
            std::fs::remove_file(&path).unwrap();
        }
        let rejected = engine.trend_discover(request.clone()).unwrap();
        assert_eq!(rejected["mention_count"], 0);
        assert_eq!(rejected["budget_usage"]["cache_hits"], 0);
        assert_eq!(rejected["budget_usage"]["request_count"], 0);
        assert_eq!(rejected["source_complete"], false);
        assert_eq!(
            rejected["provider_failures"][0]["reason"],
            "CACHE_CAPTURE_HASH_UNAVAILABLE_OR_MISMATCH"
        );
        assert!(rejected["velocity"]["value"].is_null());
        assert!(
            engine
                .trend_inspect(json!({"snapshot_id":valid["snapshot_id"]}))
                .is_err()
        );
    }
    drop(engine);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn unknown_counter_is_not_zero() {
    let p = post("1", "HACKER_NEWS", 9000);
    let v = serde_json::to_value(p).unwrap();
    assert_eq!(v["engagement"]["likes"], 0);
    assert!(v["engagement"]["views"].is_null());
}
#[test]
fn simulated_and_untrusted_states_cannot_be_observations() {
    let p = post("1", "HACKER_NEWS", 9000);
    for state in [
        EvidenceState::Simulated,
        EvidenceState::Derived,
        EvidenceState::Estimated,
        EvidenceState::Unknown,
        EvidenceState::Conflict,
    ] {
        let mut fake = p.clone();
        fake.state = state;
        assert!(fake.validate().is_err());
        assert_eq!(
            snapshot(&[fake], &[], "matcha", 10000, 86400, "FIXTURE")["mention_count"],
            0
        );
    }
    let mut live = p;
    live.capture_mode = "LIVE".into();
    assert_eq!(
        snapshot(&[live], &[], "matcha", 10000, 86400, "FIXTURE")["mention_count"],
        0
    );
}
#[test]
fn duplicate_resolution_preserves_repost_conflicts() {
    let a = post("1", "BLUESKY", 9000);
    let mut b = a.clone();
    b.text = "matcha glass bad".into();
    b.captured_at = 11000;
    b.propagation = "REPOST".into();
    b.parent_id = Some("original".into());
    let (posts, conflicts) = deduplicate(&[a, b]);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].parent_id.as_deref(), Some("original"));
    assert_eq!(posts[0].propagation, "REPOST");
    assert_eq!(conflicts.len(), 1);
}
#[test]
fn entity_identity_is_conservative() {
    let a = json!({"kind":"TOPIC","value":"matcha cup"});
    let b = json!({"kind":"TOPIC","value":"glass matcha cup"});
    assert_eq!(entity_link(&a, &b), IdentityState::DerivedWeakMatch);
    let g = json!({"kind":"GTIN","value":"1234567890128"});
    assert_eq!(entity_link(&g, &g), IdentityState::ExactIdentifier);
    let bad = json!({"kind":"GTIN","value":"1234567890129"});
    assert_eq!(entity_link(&bad, &bad), IdentityState::Conflict);
    let a = json!({"kind":"ASIN","value":"B012345678","market":"JP"});
    let b = json!({"kind":"ASIN","value":"B012345678","market":"US"});
    assert_ne!(entity_link(&a, &b), IdentityState::ExactIdentifier);
}
#[test]
fn temporal_windows_velocity_acceleration_transparency() {
    let a = post("1", "HACKER_NEWS", 9000);
    let s1 = snapshot(
        std::slice::from_ref(&a),
        &[],
        "matcha",
        10000,
        86400,
        "FIXTURE",
    );
    assert!(s1["velocity"]["value"].is_null());
    let b = post("2", "BLUESKY", 12000);
    let s2 = snapshot(
        &[a.clone(), b.clone()],
        std::slice::from_ref(&s1),
        "matcha",
        13600,
        86400,
        "FIXTURE",
    );
    assert_eq!(s2["velocity"]["value"], 1.);
    let c = post("3", "BLUESKY", 14000);
    let d = post("4", "HACKER_NEWS", 15000);
    let s3 = snapshot(&[a, b, c, d], &[s1, s2], "matcha", 17200, 86400, "FIXTURE");
    assert_eq!(s3["velocity"]["value"], 2.);
    assert_eq!(s3["acceleration"]["value"], 1.);
    assert_eq!(s3["platform_count"], 2);
    assert_eq!(s3["publisher_count"], 0);
    assert_eq!(s3["independent_original_publishers"]["state"], "UNKNOWN");
    for c in s3["score"]["components"].as_array().unwrap() {
        for k in [
            "value",
            "window_seconds",
            "denominator",
            "source_count",
            "evidence_ids",
            "normalization",
            "confidence",
            "state",
        ] {
            assert!(c["metric"].get(k).is_some(), "missing {k}");
        }
    }
    assert_eq!(s3["score"]["learned"], false);
    assert_eq!(s3["simulation_contribution"], 0);
}
#[test]
fn clustering_keeps_evidence_and_unknown_timestamps() {
    let a = post("1", "HACKER_NEWS", 9000);
    let b = post("2", "BLUESKY", 9200);
    let mut c = post("3", "HACKER_NEWS", 9100);
    c.text = "unrelated rocket engineering".into();
    c.hashtags.clear();
    c.entities.clear();
    assert_eq!(clusters(&[a.clone(), b.clone(), c]).len(), 2);
    let mut unknown = b;
    unknown.published_at = None;
    let s = snapshot(&[a, unknown], &[], "matcha", 10000, 86400, "FIXTURE");
    assert_eq!(s["mention_count"], 1);
    assert_eq!(s["timestamp_unknown_excluded"], 1);
    assert!(
        !s["clusters"][0]["source_edges"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn sentiment_is_secondary_and_language_scoped() {
    let mut p = post("1", "BLUESKY", 9000);
    assert_eq!(sentiment(&p)["label"], "POSITIVE");
    p.text = "love this but awful".into();
    assert_eq!(sentiment(&p)["label"], "MIXED");
    p.language = Some("ja".into());
    assert_eq!(sentiment(&p)["label"], "UNKNOWN");
}
#[test]
fn engagement_growth_compares_same_post_platform_counter() {
    let p = post("1", "BLUESKY", 9000);
    let s1 = snapshot(
        std::slice::from_ref(&p),
        &[],
        "matcha",
        10000,
        86400,
        "FIXTURE",
    );
    let mut p2 = p;
    p2.engagement.likes = Some(4);
    let s2 = snapshot(&[p2], &[s1], "matcha", 13600, 86400, "FIXTURE");
    assert_eq!(s2["engagement_growth"]["value"], 4.);
    assert_eq!(
        s2["engagement_observations"][0]["metrics"]["views"],
        Value::Null
    );
}
#[test]
fn watch_policy_rejects_noise_failures_and_incomplete_disappearance() {
    use ecdev_core::social::runtime::watch_events;
    let a = post("1", "BLUESKY", 9000);
    let mut before = snapshot(
        std::slice::from_ref(&a),
        &[],
        "matcha",
        10000,
        86400,
        "FIXTURE",
    );
    before["source_complete"] = json!(true);
    let mut after = before.clone();
    after["mention_count"] = json!(4);
    after["platform_count"] = json!(2);
    after["captured_at"] = json!(13600);
    let policy = json!({"minimum_mentions":3,"threshold":1,"triggers":["TOPIC_MENTION_GROWTH","CROSS_PLATFORM_APPEARANCE"]});
    assert_eq!(
        watch_events(&before, &after, &policy)
            .as_array()
            .unwrap()
            .len(),
        2
    );
    after["source_complete"] = json!(false);
    assert_eq!(watch_events(&before, &after, &policy), json!([]));
    after["source_complete"] = json!(true);
    after["mention_count"] = json!(2);
    assert_eq!(watch_events(&before, &after, &policy), json!([]));
    before["mention_count"] = json!(10);
    after["mention_count"] = json!(0);
    after["captured_at"] = json!(100000);
    assert_eq!(
        watch_events(
            &before,
            &after,
            &json!({"triggers":["TREND_DISAPPEARANCE"]})
        ),
        json!([])
    );
}
#[test]
fn volume_spike_and_sentiment_drop_follow_harken_thresholds() {
    use ecdev_core::social::runtime::watch_events;
    let a = post("1", "BLUESKY", 9000);
    let mut before = snapshot(
        std::slice::from_ref(&a),
        &[],
        "matcha",
        10000,
        86400,
        "FIXTURE",
    );
    before["source_complete"] = json!(true);
    before["mention_count"] = json!(4);
    before["sentiment"] =
        json!([{"label":"POSITIVE"},{"label":"POSITIVE"},{"label":"NEUTRAL"},{"label":"POSITIVE"}]);
    let mut after = before.clone();
    after["captured_at"] = json!(13600);
    let policy = json!({"minimum_mentions":3,"volume_multiplier":2.0,"sentiment_drop":0.5,"triggers":["VOLUME_SPIKE","SENTIMENT_DROP"]});
    let kinds = |b: &serde_json::Value, a: &serde_json::Value| -> Vec<String> {
        watch_events(b, a, &policy)
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["trigger"].as_str().unwrap().to_owned())
            .collect()
    };
    // 4 -> 7 is below 2x the baseline; 4 -> 8 reaches it.
    after["mention_count"] = json!(7);
    assert!(kinds(&before, &after).is_empty());
    after["mention_count"] = json!(8);
    assert_eq!(kinds(&before, &after), vec!["VOLUME_SPIKE"]);
    // Net sentiment 0.75 -> -0.5 is a drop of at least 0.5 with enough samples on both sides.
    after["mention_count"] = json!(4);
    after["sentiment"] =
        json!([{"label":"NEGATIVE"},{"label":"NEGATIVE"},{"label":"POSITIVE"},{"label":"NEUTRAL"}]);
    assert_eq!(kinds(&before, &after), vec!["SENTIMENT_DROP"]);
    // Unknown (non-English) sentiment never drops; incomplete acquisitions never fire.
    after["sentiment"] = json!([{"label":"UNKNOWN"}]);
    assert!(kinds(&before, &after).is_empty());
    after["mention_count"] = json!(8);
    after["source_complete"] = json!(false);
    assert!(kinds(&before, &after).is_empty());
}
#[test]
fn watch_and_simulation_boundary_persist_after_restart() {
    let path = root();
    let engine = Engine::open(&path).unwrap();
    let watch=engine.trend_watch(json!({"query":"matcha","research":{"query":"matcha","sources":[{"platform":"HACKER_NEWS"}]},"triggers":["TOPIC_MENTION_GROWTH"],"interval_seconds":60,"enabled":false})).unwrap();
    drop(engine);
    let engine = Engine::open(&path).unwrap();
    assert_eq!(
        engine.trend_watch(json!({"action":"list"})).unwrap()[0]["watch_id"],
        watch["watch_id"]
    );
    assert_eq!(
        engine.trend_watch_tick(u64::MAX / 2).unwrap()["status"],
        "NO_DUE_TREND_WATCH"
    );
    let invalid = ForecastScenario {
        id: "sim".into(),
        seed_snapshot_id: "none".into(),
        question: "future".into(),
        population: vec![],
        posts: vec![SimulatedPost {
            actor_id: "1".into(),
            text: "matcha".into(),
            state: EvidenceState::Observed,
        }],
        reactions: vec![],
        state: EvidenceState::Simulated,
    };
    assert!(engine.store_social_scenario(invalid).is_err());
    assert!(
        engine.trend_inspect(json!({})).unwrap()["snapshots"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn allocator_redundancy_and_zero_paid_gate() {
    let a = json!({"candidate_id":"candidate","action":"PUBLIC_SOCIAL_QUERY","provider":"native-social","class":"PUBLIC","target_unknown":"social","evidence_gap":"topic-evidence","source_group":"HN","expected_cost_minor":0,"expected_requests":1,"expected_latency_ms":100,"uncertainty_reduction_points":10});
    let mut b = a.clone();
    b["provider"] = json!("alternative");
    let mut paid = a.clone();
    paid["class"] = json!("PAID");
    paid["expected_cost_minor"] = json!(1);
    let result = ecdev_core::planner::allocate_information_actions(&[a, b, paid], 0, 10);
    assert_eq!(result["selected_actions"].as_array().unwrap().len(), 1);
    assert!(
        result["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "PAID_BUDGET_ZERO")
    );
    assert!(
        result["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reason"] == "REDUNDANT_EVIDENCE_GAP_SOURCE")
    );
}

#[test]
fn expired_trend_watch_leases_recover_and_stale_workers_cannot_publish() {
    use ecdev_core::provider::{AcquireError, AcquireRequest, AcquireResult, Provider};
    use std::sync::Arc;
    struct FencedProvider(std::path::PathBuf);
    impl Provider for FencedProvider {
        fn id(&self) -> &str {
            "native-social"
        }
        fn metadata(&self) -> Value {
            json!({"class":"PUBLIC","cost_minor":0})
        }
        fn normalize_query(&self, q: &Value) -> Result<Value, String> {
            Ok(q.clone())
        }
        fn acquire(&self, _: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
            let db = rusqlite::Connection::open(&self.0).unwrap();
            db.execute(
                "UPDATE trend_watches SET lease_token='RECLAIMED_BY_ANOTHER_WORKER'",
                [],
            )
            .unwrap();
            Err("SOURCE_UNAVAILABLE_TEST_NO_NETWORK".into())
        }
    }
    let path = root();
    let engine = Engine::open(&path).unwrap();
    let watch=engine.trend_watch(json!({"query":"matcha","research":{"query":"matcha","sources":[{"platform":"HACKER_NEWS"}]},"triggers":["TOPIC_MENTION_GROWTH"],"interval_seconds":60,"enabled":true})).unwrap();
    let dbpath = path.join(".ecdev-data/runtime/ecdev.sqlite");
    let db = rusqlite::Connection::open(&dbpath).unwrap();
    db.execute(
        "UPDATE trend_watches SET next_due=0,lease_until=1000,lease_token='OLD'",
        [],
    )
    .unwrap();
    assert_eq!(
        engine.trend_watch_tick(100).unwrap()["status"],
        "NO_DUE_TREND_WATCH"
    );
    let recovered = engine.trend_watch_tick(1001).unwrap();
    assert_eq!(recovered["watch_id"], watch["watch_id"]);
    assert!(recovered["baseline"].is_null());
    assert_eq!(recovered["events"], json!([]));
    db.execute("UPDATE trend_watches SET next_due=0,lease_until=0", [])
        .unwrap();
    let fenced = Engine::open(&path)
        .unwrap()
        .with_provider(Arc::new(FencedProvider(dbpath)));
    assert_eq!(
        fenced.trend_watch_tick(2000).unwrap_err(),
        "STALE_TREND_WATCH_LEASE"
    );
    drop(fenced);
    drop(db);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn nine_watch_trigger_contracts_preserve_baseline_and_counter_scope() {
    use ecdev_core::social::runtime::watch_events;
    let before = json!({"query":"matcha","capture_mode":"FIXTURE","window_seconds":60,"source_complete":true,"population_complete":true,"captured_at":100,"mention_count":3,"platform_count":1,"unique_sources":1,"velocity":{"value":0},"acceleration":{"value":0},"entity_links":[],"sentiment":[{"label":"POSITIVE"},{"label":"POSITIVE"},{"label":"POSITIVE"}],"engagement_observations":[{"post_key":"p","platform":"BLUESKY","metrics":{"likes":0}}]});
    let mut after = before.clone();
    after["mention_count"] = json!(6);
    after["platform_count"] = json!(2);
    after["unique_sources"] = json!(3);
    after["captured_at"] = json!(200);
    after["velocity"]["value"] = json!(2);
    after["acceleration"]["value"] = json!(2);
    after["entity_links"] = json!([{"kind":"URL","value":"https://example.org"}]);
    after["sentiment"] = json!([{"label":"NEGATIVE"},{"label":"NEGATIVE"},{"label":"NEGATIVE"}]);
    after["engagement_observations"][0]["metrics"]["likes"] = json!(3);
    let triggers = [
        "TOPIC_MENTION_GROWTH",
        "ENTITY_GROWTH",
        "CROSS_PLATFORM_APPEARANCE",
        "VELOCITY_THRESHOLD",
        "ACCELERATION_THRESHOLD",
        "NEW_SOURCE_APPEARANCE",
        "SENTIMENT_REGIME_CHANGE",
        "ENGAGEMENT_SPIKE",
    ];
    let policy = json!({"minimum_mentions":3,"threshold":1,"triggers":triggers});
    assert_eq!(
        watch_events(&before, &after, &policy)
            .as_array()
            .unwrap()
            .len(),
        8
    );
    after["engagement_observations"][0]["platform"] = json!("HACKER_NEWS");
    assert_eq!(
        watch_events(&before, &after, &policy)
            .as_array()
            .unwrap()
            .len(),
        7
    );
    after["mention_count"] = json!(0);
    assert_eq!(
        watch_events(
            &before,
            &after,
            &json!({"minimum_mentions":3,"triggers":["TREND_DISAPPEARANCE"]})
        )
        .as_array()
        .unwrap()
        .len(),
        1
    );
}
#[test]
fn mention_growth_needs_a_captured_prior_window_and_enough_counts() {
    let w = 3600;
    // Prior window (3600, 7200] holds 1 post; current (7200, 10800] holds 8 (1 -> 6 is p = 0.0625).
    let mut posts = vec![post("p0", "BLUESKY", 5000)];
    posts.extend((1..=8).map(|i| post(&format!("c{i}"), "BLUESKY", 7300 + i * 100)));
    let uncaptured = snapshot(&posts, &[], "matcha", 10800, w, "FIXTURE");
    assert_eq!(
        uncaptured["mention_growth"]["state"],
        "PRIOR_WINDOW_NOT_FULLY_CAPTURED"
    );
    assert!(uncaptured["mention_growth"]["comparison"].is_null());
    let prior = snapshot(&posts[..1], &[], "matcha", 7200, w, "FIXTURE");
    let s = snapshot(
        &posts,
        std::slice::from_ref(&prior),
        "matcha",
        10800,
        w,
        "FIXTURE",
    );
    let g = &s["mention_growth"];
    assert_eq!(
        (
            g["comparison"]["before"].clone(),
            g["comparison"]["after"].clone()
        ),
        (json!(1), json!(8))
    );
    assert_eq!(g["state"], "RISING");
    assert_eq!(g["score_weight"], 0);
    // Two mentions after none is not growth.
    let few = snapshot(&posts[..3], &[prior], "matcha", 10800, w, "FIXTURE");
    assert_eq!(few["mention_growth"]["comparison"]["before"], 1);
    assert_eq!(few["mention_growth"]["state"], "NO_DETECTABLE_CHANGE");
}
#[test]
fn sentiment_shares_carry_counts_and_intervals() {
    let rows =
        |labels: &[&str]| -> Vec<Value> { labels.iter().map(|l| json!({"label":l})).collect() };
    let s = sentiment_summary(
        &rows(&["POSITIVE", "POSITIVE", "NEGATIVE", "UNKNOWN"]),
        None,
    );
    assert_eq!(
        (s["labelled_count"].clone(), s["unknown_count"].clone()),
        (json!(3), json!(1))
    );
    assert_eq!(s["shares"]["POSITIVE"]["count"], 2);
    let ci = &s["shares"]["POSITIVE"]["interval_95"];
    // Wilson 2/3: [0.2077, 0.9385]; three posts say almost nothing.
    assert!(
        (ci[0].as_f64().unwrap() - 0.2077).abs() < 1e-3
            && (ci[1].as_f64().unwrap() - 0.9385).abs() < 1e-3
    );
    assert!(s["shares"]["POSITIVE"]["versus_prior"].is_null());
    // 2/3 positive -> 0/3 positive overlaps: not separated. 30/30 -> 0/30 is.
    let few = sentiment_summary(&rows(&["NEGATIVE"; 3]), Some(&s));
    assert_eq!(few["shares"]["POSITIVE"]["versus_prior"], "NOT_SEPARATED");
    let many_before = sentiment_summary(&rows(&["POSITIVE"; 30]), None);
    let many_after = sentiment_summary(&rows(&["NEGATIVE"; 30]), Some(&many_before));
    assert_eq!(many_after["shares"]["POSITIVE"]["versus_prior"], "LOWER");
    assert_eq!(many_after["shares"]["NEGATIVE"]["versus_prior"], "HIGHER");
    let none = sentiment_summary(&rows(&["UNKNOWN"]), None);
    assert!(
        none["shares"]["POSITIVE"]["share"].is_null()
            && none["shares"]["POSITIVE"]["interval_95"].is_null()
    );
    assert_eq!(wilson_interval(0, 0), None);
}
#[test]
fn cluster_representatives_rank_centrality_then_same_platform_engagement() {
    let mut a = post("a", "BLUESKY", 9000);
    a.text = "matcha glass cup review".into();
    a.engagement.likes = Some(1);
    let mut b = post("b", "BLUESKY", 9100);
    b.text = "matcha glass cup review".into();
    b.engagement.likes = Some(50);
    let mut c = post("c", "HACKER_NEWS", 9200);
    c.text = "matcha glass cup review".into();
    c.engagement.likes = None;
    let mut d = post("d", "BLUESKY", 9300);
    d.text = "matcha glass cup offtopic tangent words".into();
    d.engagement.likes = Some(1000);
    let reps = representatives(&[&a, &b, &c, &d]);
    let keys: Vec<_> = reps
        .iter()
        .map(|r| r["post_key"].as_str().unwrap())
        .collect();
    // Equal centrality: higher same-platform rank first, unknown engagement last; the most
    // engaged but least central post is not chosen.
    assert_eq!(keys, ["BLUESKY:b", "BLUESKY:a", "HACKER_NEWS:c"]);
    assert!(reps[2]["engagement_rank_within_platform"].is_null());
    assert_eq!(reps[0]["engagement_rank_within_platform"], 0.5);
    let single = representatives(&[&d]);
    assert_eq!(single[0]["centrality"], 1.);
    let clustered = clusters(&[a, b, c, d]);
    assert!(
        clustered[0]["representatives"]
            .as_array()
            .is_some_and(|r| !r.is_empty())
    );
}
#[test]
fn a_source_refusing_its_own_next_cursor_limits_coverage_without_an_outage() {
    use ecdev_core::provider::{AcquireError, AcquireRequest, AcquireResult, Provider};
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    // Like Bluesky's unauthenticated search: page one is served, its cursor is refused with 403.
    struct Refuses;
    impl Provider for Refuses {
        fn id(&self) -> &str {
            "native-social"
        }
        fn metadata(&self) -> Value {
            json!({"class":"PUBLIC","cost_minor":0})
        }
        fn acquire(&self, r: &AcquireRequest) -> Result<AcquireResult, AcquireError> {
            if r.query.get("page_cursor").is_some() {
                let mut e = AcquireError::from("PUBLIC_SOCIAL_HTTP_FAILURE");
                e.http_status = Some(403);
                e.request_count = Some(2);
                return Err(e);
            }
            let raw = b"first page";
            let mut p = post("first", "BLUESKY", ecdev_core::service::timestamp() - 60);
            p.capture_mode = "LIVE".into();
            p.raw_hash = format!("{:x}", Sha256::digest(raw));
            Ok(AcquireResult {
                observations: vec![],
                result: json!({"posts":[p],"pagination":{"next_cursor":"c1"},"source_total":{"value":10000,"exactness":"CAPPED_OR_ESTIMATED"}}),
                raw_payload: raw.to_vec(),
                provider_cost: json!({"request_count":2,"cost_minor":0}),
            })
        }
    }
    let path = root();
    let engine = Engine::open(&path)
        .unwrap()
        .with_provider(Arc::new(Refuses));
    let result = engine
        .trend_discover(json!({"query":"matcha","sources":[{"platform":"BLUESKY","max_pages":3}]}))
        .unwrap();
    let p = &result["pagination"][0];
    assert_eq!(p["stop"], "SOURCE_REFUSED_FURTHER_PAGES");
    assert_eq!(
        p["population_coverage"]["state"],
        "PARTIAL_SOURCE_REFUSED_PAGES"
    );
    assert_eq!(
        p["population_coverage"]["total_state"],
        "SOURCE_TOTAL_UNKNOWN"
    );
    assert!(p["population_coverage"]["coverage_ratio"].is_null());
    // The first page is kept and nothing is reported as an outage.
    assert_eq!(result["mention_count"], 1);
    assert_eq!(result["provider_failures"], json!([]));
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}
