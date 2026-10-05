use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use ecdev_core::{Engine, service::tool_definitions};
use futures::Stream;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::*,
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::{Value, json};
use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone)]
struct Mcp(Engine);
impl ServerHandler for Mcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().enable_resources().build()).with_instructions("Native/public research is available without paid providers. Supplied HTML is FIXTURE; unknown commercial evidence remains unknown. Keepa is optional and denied by zero budget before IO; live auth unverified. Economics SIMULATED; opportunity search PLAN_ONLY. Donor extinction requires proofs.")
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools: Vec<Tool> = tool_definitions()
            .into_iter()
            .map(|v| serde_json::from_value(v).unwrap())
            .collect();
        serde_json::from_value(json!({"tools":tools}))
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        tool_definitions()
            .into_iter()
            .find(|v| v["name"] == name)
            .and_then(|v| serde_json::from_value(v).ok())
    }
    async fn call_tool(
        &self,
        r: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let engine = self.0.clone();
        let name = r.name.to_string();
        let args = Value::Object(r.arguments.unwrap_or_default());
        let result = tokio::task::spawn_blocking(move || engine.call(&name, args))
            .await
            .map_err(|_| ErrorData::internal_error("Engine task failed", None))?;
        Ok(match result {
            Ok(v) => CallToolResult::structured(if v.is_object() {
                v
            } else {
                json!({"items": v})
            }),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
        }
        .into())
    }
    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        serde_json::from_value(json!({"resourceTemplates":[{"uriTemplate":"ecdev://run/{id}","name":"Persisted run","mimeType":"application/json"},{"uriTemplate":"ecdev://ynventa/donor/{id}","name":"Donor census","mimeType":"application/json"}]})).map_err(|e|ErrorData::internal_error(e.to_string(),None))
    }
    async fn read_resource(
        &self,
        r: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let result = if let Some(id) = r.uri.strip_prefix("ecdev://run/") {
            self.0.run(id)
        } else if let Some(id) = r.uri.strip_prefix("ecdev://ynventa/donor/") {
            self.0.census(id)
        } else {
            Err("Unknown resource".into())
        };
        let value = result.map_err(|e| ErrorData::invalid_params(e, None))?;
        serde_json::from_value::<ReadResourceResult>(json!({"contents":[{"uri":r.uri,"mimeType":"application/json","text":value.to_string()}]})).map(Into::into).map_err(|e|ErrorData::internal_error(e.to_string(),None))
    }
}
fn root() -> PathBuf {
    std::env::var_os("ECDEV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .canonicalize()
        .expect("ECDEV_ROOT must exist")
}
fn response(result: Result<Value, String>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error":e}))).into_response(),
    }
}
async fn status(State(e): State<Engine>) -> Response {
    response(e.status())
}
async fn providers(State(e): State<Engine>) -> Json<Value> {
    Json(e.providers())
}
async fn donors(State(e): State<Engine>) -> Response {
    response(e.registry())
}
async fn census(State(e): State<Engine>, AxumPath(id): AxumPath<String>) -> Response {
    response(e.census(&id))
}
async fn metrics_json(State(e): State<Engine>) -> Response {
    response(e.metrics())
}
async fn metrics(State(e): State<Engine>) -> Response {
    match e.metrics() {
        Ok(v) => {
            let lines = v
                .as_object()
                .unwrap()
                .iter()
                .filter_map(|(k, v)| v.as_u64().map(|n| format!("ecdev_{k} {n}\n")))
                .collect::<String>();
            ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], lines).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}
async fn health(State(e): State<Engine>) -> Response {
    response(e.call("ecdev.system.health", json!({})))
}
async fn tools() -> Json<Value> {
    Json(json!(tool_definitions()))
}
async fn runs(State(e): State<Engine>) -> Response {
    response(e.runs())
}
async fn run_detail(State(e): State<Engine>, AxumPath(id): AxumPath<String>) -> Response {
    response(e.run(&id))
}
async fn execute(
    State(e): State<Engine>,
    AxumPath(name): AxumPath<String>,
    Json(args): Json<Value>,
) -> Response {
    match tokio::task::spawn_blocking(move || e.call(&name, args)).await {
        Ok(v) => response(v),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Engine failed").into_response(),
    }
}
async fn events(
    State(e): State<Engine>,
    Query(q): Query<HashMap<String, String>>,
    request: Request,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let after = request
        .headers()
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .or_else(|| q.get("after").map(String::as_str))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let stream = futures::stream::unfold((e, after), |(e, mut after)| async move {
        loop {
            if let Ok(v) = e.events(after) {
                let a = v.as_array().unwrap();
                if let Some(event) = a.first() {
                    after = event["id"].as_u64().unwrap();
                    let item = Event::default()
                        .id(after.to_string())
                        .event("run")
                        .data(event["data"].to_string());
                    return Some((Ok(item), (e, after)));
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
async fn origin_guard(request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !host.starts_with("127.0.0.1:") && !host.starts_with("localhost:") {
        return (StatusCode::FORBIDDEN, "Invalid local host").into_response();
    }
    if let Some(origin) = request.headers().get("origin") {
        let expected = format!("http://{host}");
        if origin.to_str().ok() != Some(expected.as_str()) {
            return (StatusCode::FORBIDDEN, "Invalid origin").into_response();
        }
    }
    next.run(request).await
}
pub fn router(e: Engine, port: u16) -> Router {
    let m = e.clone();
    let mut config = StreamableHttpServerConfig::default();
    config.legacy_session_mode = false;
    config.json_response = true;
    config.allowed_origins = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    let mcp = StreamableHttpService::new(
        move || Ok(Mcp(m.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../web/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript")],
                    include_str!("../../web/dist/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css")],
                    include_str!("../../web/style.css"),
                )
            }),
        )
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/events", get(events))
        .route("/api/status", get(status))
        .route("/api/providers", get(providers))
        .route("/api/donors", get(donors))
        .route("/api/donors/:id", get(census))
        .route("/api/metrics", get(metrics_json))
        .route("/api/tools", get(tools))
        .route(
            "/api/research/example",
            get(|| async {
                Json(
                    serde_json::from_str::<Value>(include_str!(
                        "../../../domain/commerce/tests/fixtures/research-household.json"
                    ))
                    .unwrap(),
                )
            }),
        )
        .route("/api/runs", get(runs))
        .route("/api/runs/:id", get(run_detail))
        .route("/api/tools/:name", post(execute))
        .nest_service("/mcp", mcp)
        .with_state(e)
        .layer(middleware::from_fn(origin_guard))
}
pub async fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::var("ECDEV_PORT")
        .unwrap_or("8765".into())
        .parse::<u16>()?;
    let e = configured_engine()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!(
        "ECDEV SERVER READY\n\nMCP     http://127.0.0.1:{port}/mcp\nWEB     http://127.0.0.1:{port}/\nAPI     http://127.0.0.1:{port}/api/status\nHEALTH  http://127.0.0.1:{port}/health"
    );
    let scheduler = start_watch_scheduler(e.clone());
    axum::serve(listener, router(e, port))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    scheduler.abort();
    Ok(())
}
fn start_watch_scheduler(engine: Engine) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let e = engine.clone();
            let result = tokio::task::spawn_blocking(move || {
                e.monitor_tick(ecdev_core::service::timestamp())?;
                e.trend_watch_tick(ecdev_core::service::timestamp())
            })
            .await;
            if let Ok(Err(reason)) = result {
                eprintln!("Watch scheduler: {reason}");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cmd = a.first().map(String::as_str).unwrap_or("help");
    if cmd == "server" {
        return serve().await;
    }
    if cmd == "stdio" {
        let engine = configured_engine()?;
        let scheduler = start_watch_scheduler(engine.clone());
        let service = Mcp(engine).serve(rmcp::transport::stdio()).await?;
        service.waiting().await?;
        scheduler.abort();
        return Ok(());
    }
    if cmd == "ynventa" {
        let status = std::process::Command::new("cargo")
            .args(["run", "--quiet", "--manifest-path"])
            .arg(root().join(".ynventa/Cargo.toml"))
            .arg("--")
            .args(&a[1..])
            .arg("--root")
            .arg(root())
            .status()?;
        if !status.success() {
            return Err("Canonical Ynventa command reported failing gates".into());
        }
        return Ok(());
    }
    if cmd == "mcp-config" {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"mcpServers":{"ecdev":{"type":"http","url":"http://127.0.0.1:8765/mcp"}}})
            )?
        );
        return Ok(());
    }
    let e = configured_engine()?;
    let value = match cmd {
        "doctor" => {
            json!({"storage":e.call("ecdev.system.health",json!({}))?,"provider_config":e.providers(),"web_assets":"EMBEDDED","network":"NOT_PROBED","MCP_endpoint":"NOT_PROBED","market_support":"NATIVE_PUBLIC_SOURCES; OFFICIAL_API_UNAVAILABLE","cache":"PERSISTENT_TTL_CONDITIONAL","budget":e.budget_status()?})
        }
        "providers" => e.providers(),
        "status" => e.status()?,
        "runs" => {
            if a.get(1).is_some_and(|v| v == "inspect") {
                e.run(a.get(2).ok_or("run id required")?)?
            } else if a.get(1).is_some_and(|v| v == "replay") {
                e.replay(a.get(2).ok_or("run id required")?)?
            } else {
                e.runs()?
            }
        }
        "call" => {
            let name = a.get(1).ok_or("tool name required")?.clone();
            let args = if let Some(path) = a.get(2) {
                serde_json::from_slice(&std::fs::read(path)?)?
            } else {
                json!({})
            };
            tokio::task::spawn_blocking(move || e.call(&name, args)).await??
        }
        _ => {
            println!(
                "ecdev server | stdio | doctor | status | providers | runs [inspect|replay ID] | call TOOL [input.json] | ynventa COMMAND | mcp-config"
            );
            return Ok(());
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
fn configured_engine() -> Result<Engine, String> {
    Ok(Engine::open(&root())?
        .with_provider(std::sync::Arc::new(ecdev_marketplace::Amazon::from_env()))
        .with_provider(std::sync::Arc::new(ecdev_keepa::client::Keepa::from_env()))
        .with_provider(std::sync::Arc::new(ecdev_web::Web::default()))
        .with_provider(std::sync::Arc::new(ecdev_web::social::Social::default()))
        .with_provider(std::sync::Arc::new(
            ecdev_web::amazon::PublicAmazon::default(),
        )))
}
#[cfg(test)]
mod research_tests {
    use super::*;
    #[test]
    fn social_snapshot_mcp_projection_and_scenario_isolation() {
        use ecdev_core::social::{EvidenceState, ForecastScenario, SimulatedActor, SimulatedPost};
        let root = std::env::temp_dir().join(format!(
            "ecdev-social-e2e-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()))
            .with_provider(Arc::new(ecdev_web::social::Social::default()));
        let commerce_fixture: Value = serde_json::from_str(include_str!(
            "../../../domain/commerce/tests/fixtures/research-household.json"
        ))
        .unwrap();
        let commerce_run = engine.research(commerce_fixture).unwrap();
        let commercial_before = engine.candidates().unwrap();
        let raw=json!({"hits":[{"objectID":"1","title":"matcha glass good","created_at_i":9000,"points":0},{"objectID":"2","title":"matcha glass bad","created_at_i":9200}]}).to_string();
        let before=engine.call("ecdev.trend.discover",json!({"query":"matcha","sources":[{"platform":"HACKER_NEWS","fixture_raw":raw}],"fixture_now":10000})).unwrap();
        assert_eq!(before["mention_count"], 2);
        let empty_storage=engine.trend_discover(json!({"query":"storage","sources":[{"platform":"HACKER_NEWS","fixture_raw":"{\"hits\":[]}"}],"fixture_now":10000})).unwrap();
        assert_eq!(empty_storage["evidence_ids"], json!([]));
        assert_eq!(empty_storage["commerce_links"], json!([]));
        let linked=engine.trend_discover(json!({"query":"storage","sources":[{"platform":"HACKER_NEWS","fixture_raw":json!({"hits":[{"objectID":"33","title":"storage box","created_at_i":9000}]}).to_string()}],"fixture_now":10000})).unwrap();
        assert!(!linked["commerce_links"].as_array().unwrap().is_empty());
        assert!(
            linked["commerce_links"]
                .as_array()
                .unwrap()
                .iter()
                .all(|l| l["shortlist_permitted_by_social"] == false)
        );
        assert_eq!(engine.candidates().unwrap(), commercial_before);
        let captures: Vec<_> = commerce_run["observations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                let extension = if o["source_type"] == "PUBLIC_HTML" {
                    "html"
                } else {
                    "json"
                };
                let path = root.join(format!(
                    ".ynventa/materialized/raw/{}.{extension}",
                    o["raw_hash"].as_str().unwrap()
                ));
                let bytes = std::fs::read(&path).unwrap();
                (path, bytes)
            })
            .collect();
        for (path, _) in &captures {
            std::fs::write(path, b"corrupted commerce capture").unwrap();
        }
        let unavailable_links=engine.trend_discover(json!({"query":"storage","sources":[{"platform":"HACKER_NEWS","fixture_raw":json!({"hits":[{"objectID":"34","title":"storage box","created_at_i":9000}]}).to_string()}],"fixture_now":10000})).unwrap();
        assert!(
            !unavailable_links["evidence_ids"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(unavailable_links["commerce_links"], json!([]));
        for (path, bytes) in captures {
            std::fs::write(path, bytes).unwrap();
        }
        assert_eq!(engine.candidates().unwrap(), commercial_before);
        assert_eq!(before["budget_usage"]["request_count"], 0);
        assert_eq!(before["capture_mode"], "FIXTURE");
        let projection = engine
            .call(
                "ecdev.trend.explain",
                json!({"snapshot_id":before["snapshot_id"]}),
            )
            .unwrap();
        assert_eq!(
            projection["observed_source_evidence"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            projection["observed_source_evidence"][0]["engagement"]["likes"],
            0
        );
        assert!(projection["observed_source_evidence"][1]["engagement"]["likes"].is_null());
        let scenario = ForecastScenario {
            id: "simulation-1".into(),
            seed_snapshot_id: before["snapshot_id"].as_str().unwrap().into(),
            question: "illustrative scenario".into(),
            population: vec![SimulatedActor {
                id: "synthetic-1".into(),
                persona: "hypothetical".into(),
                state: EvidenceState::Simulated,
            }],
            posts: vec![SimulatedPost {
                actor_id: "synthetic-1".into(),
                text: "matcha glass".into(),
                state: EvidenceState::Simulated,
            }],
            reactions: vec![],
            state: EvidenceState::Simulated,
        };
        engine.store_social_scenario(scenario).unwrap();
        let after=engine.trend_discover(json!({"query":"matcha","sources":[{"platform":"HACKER_NEWS","fixture_raw":raw}],"fixture_now":13600})).unwrap();
        assert_eq!(after["mention_count"], 2);
        assert_eq!(after["velocity"]["value"], 0.);
        assert_eq!(after["simulation_contribution"], 0);
        let comparison=engine.call("ecdev.trend.compare",json!({"before_snapshot_id":before["snapshot_id"],"after_snapshot_id":after["snapshot_id"]})).unwrap();
        assert_eq!(comparison["mention_delta"], 0);
        let cached = engine
            .trend_discover(
                json!({"query":"matcha","sources":[{"platform":"HACKER_NEWS"}],"cache_only":true}),
            )
            .unwrap();
        assert_eq!(cached["capture_mode"], "CACHED");
        assert_eq!(cached["mention_count"], 0);
        assert_eq!(cached["budget_usage"]["request_count"], 0);
        assert!(!cached["provider_failures"].as_array().unwrap().is_empty());
        assert!(engine.call("ecdev.trend.compare",json!({"before_snapshot_id":before["snapshot_id"],"after_snapshot_id":cached["snapshot_id"]})).is_err());
        let denied = engine
            .trend_discover(
                json!({"query":"matcha","sources":[{"platform":"HACKER_NEWS"}],"request_budget":0}),
            )
            .unwrap();
        assert_eq!(denied["budget_usage"]["request_count"], 0);
        assert_eq!(denied["source_complete"], false);
        assert!(denied["velocity"]["value"].is_null());
        let changed_scope = engine.trend_discover(json!({"query":"matcha","sources":[{"platform":"JSON_FEED","url":"https://example.org/feed.json","fixture_raw":"{\"version\":\"https://jsonfeed.org/version/1.1\",\"items\":[]}"}],"fixture_now":17200})).unwrap();
        assert!(changed_scope["velocity"]["value"].is_null());
        assert_eq!(changed_scope["mention_count"], 0);
        let feed = |name: &str| json!({"platform":"JSON_FEED","url":format!("https://example.org/{name}.json"),"fixture_raw":json!({"version":"https://jsonfeed.org/version/1.1","items":[{"id":name,"url":format!("https://example.org/{name}/post"),"content_text":"matcha source scope","date_published":"1970-01-01T02:30:00Z"}]}).to_string()});
        let two_feeds = engine.trend_discover(json!({"query":"matcha","sources":[feed("alpha"),feed("beta")],"fixture_now":18000})).unwrap();
        assert_eq!(two_feeds["mention_count"], 2);
        assert_eq!(two_feeds["source_complete"], true);
        assert_eq!(two_feeds["budget_usage"]["request_count"], 0);
        assert_eq!(two_feeds["platform_count"], 1);
        assert_eq!(
            two_feeds["independent_original_publishers"]["state"],
            "UNKNOWN"
        );
        let only_alpha = engine
            .trend_discover(json!({"query":"matcha","sources":[feed("alpha")],"fixture_now":18001}))
            .unwrap();
        assert_eq!(only_alpha["mention_count"], 1);
        assert!(only_alpha["velocity"]["value"].is_null());

        let raw_path = root
            .join(".ynventa/materialized/runtime/social-captures")
            .join(format!(
                "{}.raw",
                before["captured_posts"][0]["raw_hash"].as_str().unwrap()
            ));
        let original_raw = std::fs::read(&raw_path).unwrap();
        std::fs::write(&raw_path, b"corrupted capture").unwrap();
        assert!(
            engine
                .trend_inspect(json!({"snapshot_id":before["snapshot_id"]}))
                .unwrap_err()
                .contains("RAW_CAPTURE_HASH")
        );
        let rejected = engine.trend_discover(json!({"query":"matcha","sources":[{"platform":"HACKER_NEWS","fixture_raw":"{\"hits\":[]}"}],"fixture_now":18002})).unwrap();
        assert_eq!(rejected["mention_count"], 0);
        assert_eq!(rejected["source_complete"], false);
        assert!(rejected["velocity"]["value"].is_null());
        assert_eq!(rejected["budget_usage"]["request_count"], 0);
        assert_eq!(
            rejected["acquisition_provenance"]["live_acquisition_established"],
            false
        );
        assert_eq!(
            rejected["provider_failures"][0]["reason"],
            "HISTORICAL_CAPTURE_HASH_UNAVAILABLE_OR_MISMATCH"
        );
        assert!(
            !engine.trend_inspect(json!({})).unwrap()["unavailable_snapshots"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        std::fs::write(&raw_path, original_raw).unwrap();

        assert!(engine.call("ecdev.trend.compare",json!({"before_snapshot_id":before["snapshot_id"],"after_snapshot_id":changed_scope["snapshot_id"]})).is_err());
        assert!(
            engine
                .trend_discover(
                    json!({"query":"matcha","sources":[{"platform":"HACKER_NEWS"}],"fixture_now":1})
                )
                .is_err()
        );
        let frozen = engine
            .trend_inspect(json!({"snapshot_id":before["snapshot_id"]}))
            .unwrap();
        assert_eq!(
            frozen["observed_source_evidence"],
            projection["observed_source_evidence"]
        );
        drop(engine);
        let reopened = Engine::open(&root).unwrap();
        assert_eq!(
            reopened
                .trend_inspect(json!({"snapshot_id":before["snapshot_id"]}))
                .unwrap()["mention_count"],
            2
        );
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn public_amazon_blocked_route_preserves_provider_identity_and_public_fallback() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-public-amazon-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let e = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()))
            .with_provider(Arc::new(ecdev_web::amazon::PublicAmazon::default()));
        let run = e.research(json!({"market":"AMAZON_JP","query":"Fixture source availability separation","max_pages":2,"sources":[{"url":"https://www.amazon.co.jp/dp/B012345678","fixture_html":"<form action='/errors/validateCaptcha'><input name='guess'></form>"},{"url":"https://retailer.example/cup","fixture_html":"<script type='application/ld+json'>{\"@type\":\"Product\",\"name\":\"Fallback cup\",\"offers\":{\"price\":2980,\"priceCurrency\":\"JPY\"}}</script>"}]})).unwrap();
        assert_eq!(run["mode"], "FIXTURE");
        assert_eq!(run["network_calls"], 0);
        assert_eq!(run["errors"][0]["status"], "SOURCE_BLOCKED");
        assert_eq!(run["errors"][0]["provider"], "public-amazon");
        assert_eq!(run["provider_calls"][0]["provider"], "public-amazon");
        assert_eq!(run["provider_calls"][1]["provider"], "native-web");
        assert_eq!(run["observations"][0]["provider"], "public-amazon");
        assert_eq!(run["candidates"].as_array().unwrap().len(), 1);
        assert_eq!(
            run["candidates"][0]["source"],
            "https://retailer.example/cup"
        );
        let profiles = e.providers();
        let official = profiles
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "amazon-sp-api")
            .unwrap();
        assert_eq!(official["source_layer"], "OFFICIAL_SP_API");
        assert_eq!(official["status"], "UNAVAILABLE");
    }
    #[test]
    fn multi_origin_identifier_evidence_reaches_research_shortlist() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-shortlist-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let e = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let manufacturer = r#"<script type='application/ld+json'>{"@type":"Product","name":"SEPIA mug","sku":"21741","offers":[{"price":1980,"priceCurrency":"JPY","availability":"https://schema.org/OutOfStock","gtin13":4963264503563}]}</script>"#;
        let retailer = r#"<script type='application/ld+json'>{"@type":"Product","name":"Retailer SEPIA mug","sku":"retail-21741","gtin":"4963264503563","offers":{"price":1980,"priceCurrency":"JPY","availability":"https://schema.org/InStock"}}</script>"#;
        let args = json!({"market":"PUBLIC_WEB","query":"Fixture shortlist policy","sources":[{"url":"https://manufacturer.example/product","fixture_html":manufacturer},{"url":"https://retailer.example/product","fixture_html":retailer}],"max_pages":2,"decision_policy":{"currency":"JPY"}});
        let run = e.research(args.clone()).unwrap();
        assert_eq!(run["mode"], "FIXTURE");
        assert_eq!(run["network_calls"], 0);
        assert_eq!(run["candidates"].as_array().unwrap().len(), 1);
        assert_eq!(run["funnel"]["shortlisted"], 1);
        let candidate = &run["candidates"][0];
        assert_eq!(candidate["decision"]["purpose"], "FURTHER_RESEARCH");
        assert_eq!(
            candidate["decision"]["listing_origins"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(candidate["economics_uncertainty"]["profit_expected"].is_null());
        assert_eq!(
            candidate["resolution"]["observations"][0]["product"]["fields"]["ean"]["evidence"][0]["json_pointer"],
            "/offers/0/gtin13"
        );
        let replay = e.replay(run["run_id"].as_str().unwrap()).unwrap();
        assert_eq!(replay["funnel"]["shortlisted"], 1);
        let mut constrained = args.clone();
        constrained["min_price_minor"] = json!(3000);
        assert_eq!(e.research(constrained).unwrap()["funnel"]["rejected"], 1);
        let mut single = args;
        single["sources"].as_array_mut().unwrap().truncate(1);
        single["max_pages"] = json!(1);
        let run = e.research(single).unwrap();
        assert_eq!(run["funnel"]["shortlisted"], 0);
        assert_eq!(run["funnel"]["insufficient_evidence"], 1);
    }
    #[test]
    fn corrupted_commerce_captures_cannot_be_replayed_compared_or_cached() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-integrity-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let input: Value = serde_json::from_str(include_str!(
            "../../../domain/commerce/tests/fixtures/research-household.json"
        ))
        .unwrap();
        let first = engine.research(input.clone()).unwrap();
        let id = first["run_id"].as_str().unwrap();
        let candidate = first["candidates"][0]["id"].as_str().unwrap();
        assert!(engine.run(id).is_ok());
        assert!(engine.inspect_candidate(candidate).is_ok());
        let replay = engine.replay(id).unwrap();
        let path = root.join(".ynventa/materialized/raw").join(format!(
            "{}.html",
            first["observations"][0]["raw_hash"].as_str().unwrap()
        ));
        std::fs::write(&path, b"tampered capture").unwrap();
        drop(engine);
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        for stored in [id, replay["run_id"].as_str().unwrap()] {
            assert_eq!(
                engine.run(stored).unwrap_err(),
                "RAW_CAPTURE_HASH_UNAVAILABLE_OR_MISMATCH"
            );
            assert!(engine.replay(stored).is_err());
        }
        assert!(engine.inspect_candidate(candidate).is_err());
        assert!(engine.evidence(id).is_err());
        assert!(
            engine
                .compare_snapshots(json!({"before_run_id":id,"after_run_id":replay["run_id"]}))
                .is_err()
        );
        assert!(engine.candidates().unwrap().as_array().unwrap().is_empty());
        let graph = engine.evidence_graph().unwrap();
        assert!(graph["entities"].as_array().unwrap().is_empty());
        assert!(graph["edges"].as_array().unwrap().is_empty());
        let unavailable = engine.runs().unwrap();
        assert!(
            unavailable
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["status"] == "UNAVAILABLE"
                    && r["candidates"].is_null()
                    && r["observations"].is_null())
        );
        let mut resume = input.clone();
        resume["crawl_run_id"] = first["crawl_run_id"].clone();
        let resumed = engine.research(resume).unwrap();
        assert!(resumed["observations"].as_array().unwrap().is_empty());
        assert_eq!(resumed["errors"][0]["origin"], "FRONTIER_CHECKPOINT");
        assert_eq!(resumed["network_calls"], 0);
        let repaired = engine.research(input.clone()).unwrap();
        assert_eq!(repaired["provider_calls"][0]["cache_hit"], false);
        assert_eq!(repaired["mode"], "FIXTURE");
        assert_eq!(repaired["network_calls"], 0);
        assert!(engine.run(id).is_ok()); // Restoring the exact bytes repairs historical verification.
        std::fs::remove_file(path).unwrap();
        assert!(engine.run(id).is_err());
        let reacquired = engine.research(input).unwrap();
        assert_eq!(reacquired["provider_calls"][0]["cache_hit"], false);
        assert_eq!(reacquired["network_calls"], 0);
        std::fs::remove_dir_all(root).ok();
    }
    #[test]
    fn official_product_fixture_isolated_from_public_keepa_and_historical_corruption() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-official-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_marketplace::Amazon))
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../adapter/marketplace/tests/fixtures/official-responses.json"
        ))
        .unwrap();
        let case = &fixtures["cases"][1];
        let run = engine.call("ecdev.product.analyze",json!({"market":"AMAZON_US","asin":case["asin"],"evidence_layer":"OFFICIAL_SP_API","include":["OFFERS"],"fixture_responses":{"offers":case["response"]}})).unwrap();
        assert_eq!(run["mode"], "FIXTURE");
        assert_eq!(run["network_calls"], 0);
        assert_eq!(run["result"]["official_live_validation"], "UNAVAILABLE");
        assert!(run["result"]["profit_expected"].is_null());
        assert_eq!(
            run["result"]["records"][0]["normalized"]["offers"][0]["ListingPrice"]["CurrencyCode"],
            "USD"
        );
        assert!(run["result"]["records"][0]["raw_body"].is_null());
        let id = run["run_id"].as_str().unwrap();
        assert_eq!(engine.evidence(id).unwrap().as_array().unwrap().len(), 1);
        assert_eq!(engine.replay(id).unwrap()["network_calls"], 0);
        let denied = engine.product(json!({"market":"AMAZON_JP","asin":"B00V5DG6IQ","evidence_layer":"OFFICIAL_SP_API"})).unwrap();
        assert_eq!(denied["mode"], "PLAN_ONLY");
        assert_eq!(denied["network_calls"], 0);
        assert_eq!(denied["fallback_providers"], json!([]));
        assert!(denied["observations"].as_array().unwrap().is_empty());
        assert_eq!(
            engine
                .product(json!({"market":"AMAZON_US","asin":"B00V5DG6IQ"}))
                .unwrap()["mode"],
            "PLAN_ONLY"
        );
        let path = root.join(".ynventa/materialized/raw").join(format!(
            "{}.json",
            run["observations"][0]["raw_hash"].as_str().unwrap()
        ));
        std::fs::write(path, b"changed official fixture").unwrap();
        drop(engine);
        let engine = Engine::open(&root).unwrap();
        assert!(engine.run(id).is_err());
        assert!(engine.replay(id).is_err());
        assert!(engine.evidence(id).is_err());
        std::fs::remove_dir_all(root).ok();
    }
    #[test]
    fn supplier_offer_terms_persist_without_becoming_product_cost() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-supplier-terms-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let e = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let html = r#"<script type='application/ld+json'>{"@type":"Organization","name":"Source supplier","makesOffer":{"@type":"Offer","eligibleQuantity":{"minValue":24,"unitCode":"C62"},"priceSpecification":{"@type":"UnitPriceSpecification","price":"2.50","priceCurrency":"USD"},"itemOffered":{"@type":"Product","name":"Cup","sku":"CUP","offers":{"price":4000,"priceCurrency":"JPY"}}}}</script>"#;
        let run = e.research(json!({"market":"PUBLIC_WEB","query":"Published supplier terms fixture","sources":[{"url":"https://supplier.example/terms","fixture_html":html}],"max_pages":1})).unwrap();
        assert_eq!(run["mode"], "FIXTURE");
        assert_eq!(run["network_calls"], 0);
        assert_eq!(
            run["supplier_leads"][0]["fields"]["unit_price"]["value"]["minor"],
            250
        );
        assert_eq!(
            run["supplier_leads"][0]["fields"]["moq"]["status"],
            "DERIVED"
        );
        assert_eq!(
            run["candidates"][0]["product"]["fields"]["supplier_moq"]["status"],
            "UNKNOWN"
        );
        assert!(run["candidates"][0]["economics_uncertainty"]["profit_expected"].is_null());
        let id = run["run_id"].as_str().unwrap().to_owned();
        drop(e);
        let reopened = Engine::open(&root).unwrap();
        let saved = reopened.run(&id).unwrap();
        assert_eq!(saved["supplier_leads"], run["supplier_leads"]);
        assert_eq!(
            saved["supplier_leads"][0]["fields"]["unit_price"]["evidence"][0]["raw_capture_sha256"],
            saved["observations"][0]["raw_hash"]
        );
    }
    #[test]
    fn supplier_research_plans_public_follow_up_without_invented_terms() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-suppliers-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let e = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let html = r#"<title>Example Wholesale</title><p>Wholesale enquiries and OEM customization welcome.</p><a href='/contact'>Contact</a><script type='application/ld+json'>{"@type":"Product","name":"Cup","brand":"Example","sku":"CUP","offers":{"price":4000,"priceCurrency":"JPY"}}</script>"#;
        let input = json!({"market":"PUBLIC_WEB","query":"Supplier research fixture","sources":[{"url":"https://example.org/partners","fixture_html":html}],"max_pages":1});
        let run = e.research(input).unwrap();
        assert_eq!(run["supplier_leads"].as_array().unwrap().len(), 1);
        assert_eq!(
            run["supplier_leads"][0]["fields"]["unit_price"]["status"],
            "UNKNOWN"
        );
        assert_eq!(
            run["next_actions"]["selected_actions"][0]["url"],
            "https://example.org/contact"
        );
        assert_eq!(
            run["candidates"][0]["economics_uncertainty"]["profit_expected"],
            Value::Null
        );
        assert_eq!(e.research(json!({"market":"PUBLIC_WEB","query":"No fixture-to-live transition","follow_up_run_id":run["run_id"]})).unwrap_err(),"FIXTURE_PLAN_CANNOT_LAUNCH_LIVE_FOLLOW_UP");
    }
    #[test]
    fn open_graph_products_reach_candidates_with_inspectable_unknowns() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-og-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let e = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let html = r#"<meta property="og:type" content="product"><meta property="og:title" content="Cup"><meta property="product:price:amount" content="2,980"><meta property="product:price:currency" content="JPY"><link rel="canonical" href="/cup">"#;
        let args = |html: &str| json!({"market":"PUBLIC_WEB","query":"OpenGraph evidence fixture","sources":[{"url":"https://example.org/cup","fixture_html":html}],"max_pages":1});
        let first = e.research(args(html)).unwrap();
        assert_eq!(first["mode"], "FIXTURE");
        assert_eq!(first["network_calls"], 0);
        assert_eq!(first["funnel"]["discovered"], 1);
        let candidate = &first["candidates"][0];
        assert_eq!(candidate["product"]["price_minor"], 2980);
        assert_eq!(
            candidate["product"]["fields"]["price_minor"]["status"],
            "DERIVED"
        );
        assert_eq!(
            candidate["product"]["fields"]["canonical_url"]["value"],
            "https://example.org/cup"
        );
        assert!(candidate["product"]["fields"]["supplier_moq"]["value"].is_null());
        let mixed = format!(
            "{html}<script type='application/ld+json'>{{\"@type\":\"Product\",\"name\":\"Cup\",\"offers\":{{\"price\":\"3200\",\"priceCurrency\":\"JPY\"}}}}</script>"
        );
        let run = e.research(args(&mixed)).unwrap();
        assert_eq!(run["candidates"].as_array().unwrap().len(), 1);
        assert!(run["candidates"][0]["product"]["price_minor"].is_null());
        assert_eq!(
            run["candidates"][0]["product"]["fields"]["price_minor"]["status"],
            "CONFLICT"
        );
        assert_eq!(run["funnel"]["shortlisted"], 0);
    }
    #[test]
    fn microdata_products_and_cross_format_conflicts_retain_evidence() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-microdata-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let e = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let html = r#"<div itemscope itemtype="https://schema.org/Product"><span itemprop="name">Cup</span><meta itemprop="sku" content="CUP"><span itemprop="brand">Example</span><div itemprop="offers" itemscope itemtype="https://schema.org/Offer"><meta itemprop="priceCurrency" content="JPY"><meta itemprop="price" content="2980"><link itemprop="availability" href="https://schema.org/InStock"></div></div>"#;
        let input = |html: &str| json!({"market":"PUBLIC_WEB","query":"Microdata fixture","sources":[{"url":"https://example.org/cup","fixture_html":html}],"max_pages":1});
        let first = e.research(input(html)).unwrap();
        assert_eq!(first["funnel"]["discovered"], 1);
        assert_eq!(first["candidates"][0]["product"]["price_minor"], 2980);
        let evidence = &first["candidates"][0]["product"]["fields"]["price_minor"]["evidence"][0];
        assert_eq!(evidence["source"], "MICRODATA");
        assert_eq!(
            evidence["json_pointer"],
            "/0/properties/offers/properties/price"
        );
        assert_eq!(
            first["snapshots"][0]["microdata"]
                .pointer(evidence["json_pointer"].as_str().unwrap())
                .unwrap(),
            "2980"
        );
        assert_eq!(
            first["snapshots"][0]["page_metadata"]["classification"]["role"],
            "PRODUCT"
        );
        let mixed = format!(
            "{html}<script type='application/ld+json'>{{\"@type\":\"Product\",\"name\":\"Cup\",\"sku\":\"CUP\",\"brand\":\"Example\",\"offers\":{{\"price\":3200,\"priceCurrency\":\"JPY\"}}}}</script>"
        );
        let conflict = e.research(input(&mixed)).unwrap();
        assert_eq!(conflict["funnel"]["discovered"], 1);
        assert_eq!(
            conflict["candidates"][0]["product"]["fields"]["price_minor"]["status"],
            "CONFLICT"
        );
        assert!(conflict["candidates"][0]["product"]["price_minor"].is_null());
        assert_eq!(conflict["candidates"][0]["state"], "INSUFFICIENT_EVIDENCE");
        let rows = conflict["candidates"][0]["product"]["fields"]["price_minor"]["evidence"]
            .as_array()
            .unwrap();
        assert!(rows.iter().any(|e| e["source"] == "MICRODATA"));
        assert!(rows.iter().any(|e| e["source"] == "JSON_LD"));
    }
    #[test]
    fn zero_paid_research_e2e() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-research-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let input: Value = serde_json::from_str(include_str!(
            "../../../domain/commerce/tests/fixtures/research-household.json"
        ))
        .unwrap();
        let first = engine.research(input.clone()).unwrap();
        assert_eq!(first["mode"], "FIXTURE");
        assert_eq!(first["network_calls"], 0);
        assert_eq!(first["cost_minor"], 0);
        assert_eq!(first["funnel"]["discovered"], 3);
        assert_eq!(first["funnel"]["rejected"], 2);
        assert_eq!(first["funnel"]["screened"], 1);
        assert!(first["candidates"][0]["product"]["sales"].is_null());
        assert_eq!(
            first["candidates"][0]["economics"]["result"]["expected_profit"],
            140000
        );
        assert_eq!(
            engine
                .evidence(first["run_id"].as_str().unwrap())
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let second = engine.research(input).unwrap();
        assert_eq!(second["provider_calls"][0]["cache_hit"], true);
        assert_eq!(engine.candidates().unwrap().as_array().unwrap().len(), 6);
        let replay = engine.replay(first["run_id"].as_str().unwrap()).unwrap();
        assert_eq!(replay["network_calls"], 0);
        assert_eq!(replay["observations"][0]["mode"], "REPLAY");
        let diff = engine
            .compare_snapshots(
                json!({"before_run_id":first["run_id"],"after_run_id":second["run_id"]}),
            )
            .unwrap();
        assert!(diff["changes"].as_array().unwrap().is_empty());
        assert_eq!(
            engine.evidence_graph().unwrap()["edges"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
        let mut changed: Value = serde_json::from_str(include_str!(
            "../../../domain/commerce/tests/fixtures/research-household.json"
        ))
        .unwrap();
        changed["sources"][0]["fixture_html"] = json!(
            changed["sources"][0]["fixture_html"]
                .as_str()
                .unwrap()
                .replace("4000", "4100")
        );
        let updated = engine.research(changed).unwrap();
        let diff = engine
            .compare_snapshots(
                json!({"before_run_id":first["run_id"],"after_run_id":updated["run_id"]}),
            )
            .unwrap();
        assert_eq!(diff["changes"][0]["field"], "price_minor");
        assert_eq!(diff["changes"][0]["after"], 4100);
        let mut resume: Value = serde_json::from_str(include_str!(
            "../../../domain/commerce/tests/fixtures/research-household.json"
        ))
        .unwrap();
        resume["crawl_run_id"] = first["crawl_run_id"].clone();
        drop(engine);
        let reopened = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let restored = reopened.research(resume).unwrap();
        assert_eq!(restored["frontier"]["states"]["HANDLED"], 1);
        assert_eq!(restored["frontier"]["acquisition_attempts"], 1);
        assert_eq!(restored["network_calls"], 0);
        assert_eq!(restored["funnel"]["discovered"], 3);
        assert_eq!(restored["funnel"]["rejected"], 2);
        assert_eq!(restored["provider_calls"][0]["request_count"], 0);
        let mut bounded: Value = serde_json::from_str(include_str!(
            "../../../domain/commerce/tests/fixtures/research-household.json"
        ))
        .unwrap();
        let mut second_source = bounded["sources"][0].clone();
        second_source["url"] = json!("https://fixture.example/second");
        bounded["sources"]
            .as_array_mut()
            .unwrap()
            .push(second_source);
        bounded["max_pages"] = json!(1);
        let partial = reopened.research(bounded.clone()).unwrap();
        assert_eq!(partial["frontier"]["states"]["HANDLED"], 1);
        assert_eq!(partial["frontier"]["states"]["PENDING"], 1);
        assert_eq!(partial["status"], "PARTIAL");
        bounded["crawl_run_id"] = partial["crawl_run_id"].clone();
        let bounded_resume = reopened.research(bounded).unwrap();
        assert_eq!(bounded_resume["status"], "PARTIAL");
        assert_eq!(bounded_resume["network_calls"], 0);
        assert_eq!(bounded_resume["frontier"]["acquisition_attempts"], 1);
    }
}
