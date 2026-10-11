//! End to end over the real binary: the web console and JSON API, MCP over Streamable HTTP and
//! over standard input, and the event stream. This replaces the Node conformance smoke.
#![cfg(unix)]

use ecdev_core::json;
use ecdev_core::json::Value;
use ecdev_core::sha256::Sha256;
use ecdev_net::client::{Client, ClientRequest, ClientResponse, Scheme, Target};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Server {
    child: Child,
    port: u16,
    root: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn fresh_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "ecdev-protocol-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(repo().join("research"), root.join("research")).unwrap();
    root
}

fn command(root: &PathBuf, arg: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ecdev"));
    c.arg(arg)
        .env("ECDEV_ROOT", root)
        .env("ECDEV_PORT", "0")
        .env("ECDEV_MAX_PROVIDER_REQUESTS_PER_DAY", "0");
    c
}

fn start() -> Server {
    let root = fresh_root();
    let mut child = command(&root, "server")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let started = Instant::now();
    let port = loop {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the server did not announce itself"
        );
        let line = lines.next().expect("server output ended").unwrap();
        if let Some(rest) = line.strip_prefix("MCP     http://127.0.0.1:") {
            break rest.split('/').next().unwrap().parse::<u16>().unwrap();
        }
    };
    // Keep draining so the child never blocks on a full pipe.
    std::thread::spawn(move || for _ in lines {});
    Server { child, port, root }
}

fn http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> ClientResponse {
    let mut r = ClientRequest::new(
        method,
        Target {
            scheme: Scheme::Http,
            host: "127.0.0.1".into(),
            port,
            path_and_query: path.into(),
        },
    );
    for (k, v) in headers {
        r.headers.append(k, v).unwrap();
    }
    r.body = body.as_bytes().to_vec();
    Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .send(&r)
        .unwrap()
}

fn parse(r: &ClientResponse) -> Value {
    json::from_slice(&r.body)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&r.body)))
}

struct Mcp {
    port: u16,
    session: String,
    next: u64,
}

impl Mcp {
    fn connect(port: u16) -> Mcp {
        let mut c = Mcp {
            port,
            session: String::new(),
            next: 0,
        };
        let init = c.post(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ecdev-conformance-client","version":"1.0.0"}}}));
        assert_eq!(init.status, 200);
        c.session = init
            .headers
            .get("mcp-session-id")
            .expect("a session id")
            .to_string();
        assert_eq!(parse(&init)["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(
            c.post(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
                .status,
            202
        );
        c
    }

    fn post(&self, body: &Value) -> ClientResponse {
        let mut h = vec![
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", "2025-06-18"),
        ];
        if !self.session.is_empty() {
            h.push(("Mcp-Session-Id", &self.session));
        }
        http(self.port, "POST", "/mcp", &h, &body.to_string())
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let r = self.post(&json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params}));
        assert_eq!(r.status, 200, "{method}");
        let v = parse(&r);
        assert!(v.get("error").is_none(), "{method}: {}", v["error"]);
        v["result"].clone()
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.rpc("tools/call", json!({"name":name,"arguments":arguments}))
    }
}

fn sha256_hex(text: &str) -> String {
    Sha256::new()
        .chain_update(text.as_bytes())
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn the_console_the_api_and_mcp_over_http_conform() {
    let server = start();
    let port = server.port;
    let mut client = Mcp::connect(port);
    let tools = client.rpc("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 48);

    let official_fixtures: Value = json::from_slice(
        &std::fs::read(repo().join("adapter/marketplace/tests/fixtures/official-responses.json"))
            .unwrap(),
    )
    .unwrap();
    let official_case = official_fixtures["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["operation"] == "offers")
        .unwrap()
        .clone();
    let official = client.tool("ecdev.product.analyze", json!({"market":"AMAZON_US","asin":official_case["asin"],"evidence_layer":"OFFICIAL_SP_API","include":["OFFERS"],"fixture_responses":{"offers":official_case["response"]}}));
    assert_eq!(official["isError"], false);
    let s = &official["structuredContent"];
    assert_eq!(s["mode"], "FIXTURE");
    assert_eq!(s["network_calls"], 0);
    assert_eq!(s["result"]["official_live_validation"], "UNAVAILABLE");
    assert!(s["result"]["profit_expected"].is_null());
    assert_eq!(
        s["observations"][0]["raw_hash"].as_str().unwrap(),
        sha256_hex(official_case["response"]["raw_body"].as_str().unwrap())
    );
    let stored = client.tool("ecdev.runs.inspect", json!({"run_id": s["run_id"]}));
    assert_eq!(stored["isError"], false);
    assert_eq!(
        stored["structuredContent"]["raw_capture_verification"],
        "VERIFIED_LOCAL_SHA256_NO_NETWORK"
    );
    let denied = client.tool("ecdev.product.analyze", json!({"market":"AMAZON_JP","asin":official_case["asin"],"evidence_layer":"OFFICIAL_SP_API"}));
    assert_eq!(denied["isError"], false);
    assert_eq!(denied["structuredContent"]["mode"], "PLAN_ONLY");
    assert_eq!(denied["structuredContent"]["network_calls"], 0);
    assert_eq!(denied["structuredContent"]["fallback_providers"], json!([]));
    for name in [
        "ecdev.product.discover",
        "ecdev.product.inspect",
        "ecdev.product.compare",
        "ecdev.provider.status",
        "ecdev.provider.doctor",
        "ecdev.monitor.create",
        "ecdev.monitor.status",
        "ecdev.monitor.fact_as_of",
        "ecdev.monitor.export",
        "ecdev.monitor.import",
        "ecdev.trend.discover",
        "ecdev.trend.inspect",
        "ecdev.trend.compare",
        "ecdev.trend.watch",
        "ecdev.trend.explain",
    ] {
        assert!(names.contains(&name), "{name}");
    }

    // Hacker News ids are strictly numeric: malformed input must fail rather than create an observation.
    let social = client.tool("ecdev.trend.discover", json!({"query":"matcha","fixture_now":10000,"sources":[{"platform":"HACKER_NEWS","fixture_raw":json!({"hits":[{"objectID":"sdk-1","title":"matcha glass","created_at_i":9000}]}).to_string()}]}));
    assert_eq!(social["isError"], false);
    assert_eq!(social["structuredContent"]["source_complete"], false);
    assert!(
        social["structuredContent"]["provider_failures"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("INVALID_HN_ID")
    );
    let trend = client.tool("ecdev.trend.discover", json!({"query":"matcha","fixture_now":10000,"sources":[{"platform":"HACKER_NEWS","fixture_raw":json!({"hits":[{"objectID":"101","title":"matcha glass","created_at_i":9000,"points":0},{"objectID":"102","title":"matcha glass","created_at_i":9100}]}).to_string()}]}));
    let t = &trend["structuredContent"];
    assert_eq!(trend["isError"], false);
    assert_eq!(t["capture_mode"], "FIXTURE");
    assert_eq!(t["mention_count"], 2);
    assert_eq!(t["budget_usage"]["request_count"], 0);
    assert_eq!(t["simulation_contribution"], 0);
    let feed = json!({"version":"https://jsonfeed.org/version/1.1","items":[{"id":"zone-1","url":"https://example.org/zone-1","content_text":"matcha time","date_published":"1970-01-01T11:30:00+09:00"},{"id":"zone-unknown","url":"https://example.org/zone-unknown","content_text":"matcha time","date_published":"1970-01-01T02:30:00"}]});
    let zoned = client.tool("ecdev.trend.discover", json!({"query":"matcha","fixture_now":10000,"sources":[{"platform":"JSON_FEED","url":"https://example.org/feed.json","fixture_raw":feed.to_string()}]}));
    let z = &zoned["structuredContent"];
    assert_eq!(zoned["isError"], false);
    assert_eq!(z["capture_mode"], "FIXTURE");
    assert_eq!(z["mention_count"], 1);
    assert_eq!(z["timestamp_unknown_excluded"], 1);
    assert_eq!(z["captured_posts"][0]["published_at"], 9000);
    assert_eq!(z["captured_posts"][0]["captured_at"], 10000);
    assert_eq!(z["budget_usage"]["request_count"], 0);
    let explained = client.tool(
        "ecdev.trend.explain",
        json!({"snapshot_id": t["snapshot_id"]}),
    );
    assert_eq!(explained["isError"], false);
    assert_eq!(
        explained["structuredContent"]["observed_source_evidence"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(explained["structuredContent"]["score"]["learned"], false);
    let watch = client.tool("ecdev.trend.watch", json!({"query":"SDK social disabled watch","research":{"query":"SDK social disabled watch","sources":[{"platform":"HACKER_NEWS"}]},"triggers":["TOPIC_MENTION_GROWTH"],"interval_seconds":60,"enabled":false}));
    assert_eq!(watch["isError"], false);
    assert_eq!(watch["structuredContent"]["enabled"], false);

    let status = client.tool("ecdev.system.status", json!({}));
    assert_eq!(status["isError"], false);
    assert_eq!(status["structuredContent"]["name"], "ECDEV");
    let providers = parse(&http(port, "GET", "/api/providers", &[], ""));
    if providers
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == "keepa" && p["auth_state"] == "MISSING")
    {
        let missing = client.tool(
            "ecdev.product.analyze",
            json!({"market":"AMAZON_JP","asin":"B08N5WRWNW"}),
        );
        assert_eq!(missing["structuredContent"]["status"], "UNAVAILABLE");
        assert_eq!(missing["structuredContent"]["network_calls"], 0);
    }
    let intent = json!({"market":"AMAZON_JP","currency":"JPY","capital":300000,"min_price":3000,"max_price":6000,"max_weight_g":700,"minimum_margin_bps":2500,"max_inventory_per_sku":150000});
    let run = client.tool("ecdev.opportunity.search", intent);
    let value = run["structuredContent"].clone();
    assert_eq!(value["mode"], "PLAN_ONLY");
    assert_eq!(value["status"], "UNAVAILABLE");
    assert_eq!(value["plan"]["steps"].as_array().unwrap().len(), 11);
    assert_eq!(value["observations"], json!([]));
    let run_id = value["run_id"].as_str().unwrap().to_string();
    let resource = client.rpc(
        "resources/read",
        json!({"uri": format!("ecdev://run/{run_id}")}),
    );
    assert_eq!(
        json::from_str::<Value>(resource["contents"][0]["text"].as_str().unwrap()).unwrap()["run_id"],
        run_id
    );
    let replay = client.tool("ecdev.runs.replay", json!({"run_id": run_id}));
    assert_eq!(replay["structuredContent"]["mode"], "REPLAY");
    assert_eq!(replay["structuredContent"]["network_calls"], 0);
    let scenario = json!({"currency":"JPY","fulfillment":"FBA","selling_price":4000,"product_cost":800,"freight":200,"fulfillment_fee":500,"referral_bps":1500,"ppc":400,"units":100,"fixed_launch_cost":10000});
    let sim = json!({"name":"baseline","horizon_steps":5,"step_seconds":86400,"population":{"buyers":50,"budget_minor":{"low":2000,"high":9000},"price_sensitivity":{"low":0.2,"high":0.8},"brand_loyalty":{"low":0.3,"high":0.7},"category_interest":{"low":0.05,"high":0.2}},"market":{"currency":"JPY","our_price_minor":3000,"competitor_price_minor":3000,"reference_price_minor":3000},"review_probability":0.1,"return_base_probability":0.05});
    let mut variant = sim.clone();
    variant["name"] = json!("promotion");
    variant["shocks"] = json!([{"kind":"OurPriceChange","at_step":0,"bps":-1000}]);
    let simulated = client.tool(
        "ecdev.simulation.compare",
        json!({"baseline":sim,"variant":variant,"seed":7,"replicates":3}),
    );
    assert_eq!(simulated["isError"], false);
    assert_eq!(simulated["structuredContent"]["state"], "SIMULATED");
    assert_eq!(
        simulated["structuredContent"]["pairing"],
        "COMMON_RANDOM_NUMBERS_PER_REPLICATE"
    );
    assert_eq!(
        simulated["structuredContent"]["difference_variant_minus_baseline"]["units_ours"]["n"],
        3
    );
    let economics = client.tool("ecdev.economics.simulate", scenario.clone());
    assert_eq!(economics["structuredContent"]["mode"], "SIMULATED");
    assert_eq!(
        economics["structuredContent"]["result"]["expected_profit"],
        140000
    );
    let mut invalid_scenario = scenario;
    invalid_scenario["referral_bps"] = json!(10001);
    assert_eq!(
        client.tool("ecdev.economics.simulate", invalid_scenario)["isError"],
        true
    );

    let fixture = parse(&http(port, "GET", "/api/research/example", &[], ""));
    let research = client.tool("ecdev.research.run", fixture.clone());
    assert_eq!(research["isError"], false);
    let rs = &research["structuredContent"];
    let reextracted = client.tool(
        "ecdev.research.reextract",
        json!({"run_ids":[rs["run_id"]]}),
    );
    assert_eq!(reextracted["isError"], false);
    assert_eq!(reextracted["structuredContent"]["network"], "NONE");
    let captures = reextracted["structuredContent"]["captures"]
        .as_array()
        .unwrap();
    assert!(!captures.is_empty() && captures.iter().all(|c| c["state"] == "SAME_AS_RECORDED"));
    let formatted = client.tool("ecdev.research.run", json!({"market":"PUBLIC_WEB","query":"SDK formatted price interpretation","max_pages":1,"sources":[{"url":"https://example.org/formatted-price","fixture_html":"<script type=\"application/ld+json\">{\"@type\":\"Product\",\"name\":\"Formatted price\",\"offers\":{\"price\":\"$12.99\",\"priceCurrency\":\"USD\"}}</script>"}]}));
    assert_eq!(formatted["isError"], false);
    assert_eq!(formatted["structuredContent"]["mode"], "FIXTURE");
    assert_eq!(formatted["structuredContent"]["network_calls"], 0);
    let offer = &formatted["structuredContent"]["candidates"][0]["product"]["observed_offers"][0];
    assert_eq!(offer["raw_price"], "$12.99");
    assert!(offer["price_minor"].is_null());
    assert_eq!(offer["price_text_interpretation"]["state"], "DERIVED");
    assert_eq!(
        offer["price_text_interpretation"]["result"]["amount"],
        "12.99"
    );
    assert_eq!(
        offer["price_text_interpretation"]["result"]["currency"],
        "$"
    );
    assert_eq!(
        offer["price_text_interpretation"]["used_for_observed_price"],
        false
    );
    assert_eq!(rs["mode"], "FIXTURE");
    assert_eq!(rs["cost_minor"], 0);
    assert_eq!(rs["funnel"]["discovered"], 3);
    assert_eq!(rs["funnel"]["rejected"], 2);
    assert_eq!(rs["completeness"]["evidence_mode"], "FIXTURE");
    assert!(rs["completeness"]["live_zero_paid_research_coverage_bps"].is_null());
    let verified = client.tool("ecdev.runs.inspect", json!({"run_id": rs["run_id"]}));
    assert_eq!(verified["isError"], false);
    assert_eq!(
        verified["structuredContent"]["raw_capture_verification"],
        "VERIFIED_LOCAL_SHA256_NO_NETWORK"
    );
    let graph = client.tool("ecdev.evidence.graph", json!({}));
    assert_eq!(graph["isError"], false);
    assert_eq!(
        graph["structuredContent"]["raw_capture_verification"],
        "VERIFIED_LOCAL_SHA256_NO_NETWORK"
    );
    let forbidden = client.tool("ecdev.research.run", json!({"market":fixture["market"],"query":"Fixture isolation","follow_up_run_id":rs["run_id"]}));
    assert_eq!(forbidden["isError"], true);
    let discovery = client.tool("ecdev.product.discover", fixture);
    assert_eq!(discovery["isError"], false);
    assert_eq!(discovery["structuredContent"]["mode"], "FIXTURE");
    let candidates = discovery["structuredContent"]["candidates"]
        .as_array()
        .unwrap()
        .clone();
    let inspect = client.tool(
        "ecdev.product.inspect",
        json!({"candidate_id": candidates[0]["id"]}),
    );
    assert_eq!(inspect["isError"], false);
    assert_eq!(inspect["structuredContent"]["id"], candidates[0]["id"]);
    let comparison = client.tool(
        "ecdev.product.compare",
        json!({"candidate_ids":[candidates[0]["id"], candidates[1]["id"]]}),
    );
    assert_eq!(comparison["isError"], false);
    assert_eq!(comparison["structuredContent"]["network_calls"], 0);
    let provider_status = client.tool("ecdev.provider.status", json!({}));
    assert_eq!(provider_status["isError"], false);
    let items = provider_status["structuredContent"]["items"]
        .as_array()
        .unwrap();
    let official_profile = items.iter().find(|p| p["id"] == "amazon-sp-api").unwrap();
    assert_eq!(official_profile["live_supported"], true);
    assert_eq!(official_profile["live_authorized"], false);
    let watches = client.tool("ecdev.monitor.status", json!({}));
    assert!(watches["structuredContent"]["items"].is_array());
    let watch = client.tool("ecdev.monitor.create", json!({"enabled":false,"market":"PUBLIC_WEB","query":"SDK disabled watch","targets":["https://example.org/product"],"interval_seconds":60}));
    assert_eq!(watch["isError"], false);
    assert_eq!(watch["structuredContent"]["status"], "DISABLED");
    let watch_id = watch["structuredContent"]["watch_id"].clone();
    let watch_status = client.tool("ecdev.monitor.status", json!({"watch_id": watch_id}));
    assert_eq!(watch_status["isError"], false);
    assert_eq!(watch_status["structuredContent"]["status"], "DISABLED");
    let as_of = client.tool("ecdev.monitor.fact_as_of", json!({"watch_id":watch_id,"url":"https://shop.example/product","product":"none","field":"price_minor","at":1}));
    assert_eq!(as_of["isError"], false);
    assert_eq!(as_of["structuredContent"]["state"], "UNKNOWN");
    assert_eq!(as_of["structuredContent"]["reason"], "NEVER_OBSERVED");
    assert!(as_of["structuredContent"]["value"].is_null());
    let exported = client.tool("ecdev.monitor.export", json!({"watch_id": watch_id}));
    assert_eq!(exported["isError"], false);
    assert_eq!(
        exported["structuredContent"]["format"],
        "ECDEV_WATCH_EXPORT_V1"
    );
    let root_hash = exported["structuredContent"]["manifest"]["root_sha256"]
        .as_str()
        .unwrap();
    assert!(
        root_hash.len() == 64
            && root_hash
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    let reimport = client.tool(
        "ecdev.monitor.import",
        json!({"bundle": exported["structuredContent"]}),
    );
    assert_eq!(reimport["isError"], true);
    let budget = client.tool("ecdev.provider.budget", json!({}));
    assert_eq!(budget["structuredContent"]["policy"]["per_month_minor"], 0);
    let api = parse(&http(port, "GET", &format!("/api/runs/{run_id}"), &[], ""));
    assert_eq!(api["run_id"], run_id);
    for path in [
        "/health",
        "/api/donors",
        "/api/providers",
        "/metrics",
        "/",
        "/app.js",
        "/style.css",
    ] {
        assert_eq!(http(port, "GET", path, &[], "").status, 200, "{path}");
    }
    assert_eq!(
        http(
            port,
            "GET",
            "/api/status",
            &[("Origin", "https://example.org")],
            ""
        )
        .status,
        403
    );
    let bad_version = [
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
        ("MCP-Protocol-Version", "invalid"),
    ];
    assert_eq!(
        http(
            port,
            "POST",
            "/mcp",
            &bad_version,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string()
        )
        .status,
        400
    );

    // The event stream starts with a persisted run.
    let mut sse = TcpStream::connect(("127.0.0.1", port)).unwrap();
    sse.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    write!(
        sse,
        "GET /events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    )
    .unwrap();
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    while !String::from_utf8_lossy(&seen).contains("event: run") {
        let n = sse.read(&mut buf).unwrap();
        assert!(
            n > 0,
            "the stream ended early: {}",
            String::from_utf8_lossy(&seen)
        );
        seen.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&seen).into_owned();
    assert!(
        text.starts_with("HTTP/1.1 200 OK") && text.contains("Content-Type: text/event-stream"),
        "{text}"
    );
    drop(sse);

    // A closed session is gone.
    assert_eq!(
        http(
            port,
            "DELETE",
            "/mcp",
            &[("Mcp-Session-Id", &client.session)],
            ""
        )
        .status,
        200
    );
    assert_eq!(
        client
            .post(&json!({"jsonrpc":"2.0","id":99,"method":"ping"}))
            .status,
        404
    );
}

#[test]
fn mcp_over_standard_input_answers_one_line_per_request() {
    let root = fresh_root();
    let mut child = command(&root, "stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut ask = |stdin: &mut std::process::ChildStdin, line: &str| -> Value {
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        out.read_line(&mut reply).unwrap();
        json::from_str(&reply).unwrap_or_else(|e| panic!("reply {reply:?}: {e}"))
    };
    let init = ask(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
    );
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init["result"]["serverInfo"]["name"], "ecdev");
    // A notification gets no reply, so the next line read is the answer to the request after it.
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    let tools = ask(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 48);
    let status = ask(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ecdev.system.status","arguments":{}}}"#,
    );
    assert_eq!(status["result"]["structuredContent"]["name"], "ECDEV");
    let unknown = ask(&mut stdin, r#"{"jsonrpc":"2.0","id":4,"method":"nope"}"#);
    assert_eq!(unknown["error"]["code"], -32601);
    let broken = ask(&mut stdin, "{not json");
    assert_eq!(broken["error"]["code"], -32700);
    assert!(broken["id"].is_null());
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success());
    let _ = std::fs::remove_dir_all(root);
}
