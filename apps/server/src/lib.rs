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
        serde_json::from_value(json!({"resourceTemplates":[{"uriTemplate":"ecdev://run/{id}","name":"Persisted run","mimeType":"application/json"},{"uriTemplate":"ecdev://governance/donor/{id}","name":"Donor census","mimeType":"application/json"}]})).map_err(|e|ErrorData::internal_error(e.to_string(),None))
    }
    async fn read_resource(
        &self,
        r: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let result = if let Some(id) = r.uri.strip_prefix("ecdev://run/") {
            self.0.run(id)
        } else if let Some(id) = r.uri.strip_prefix("ecdev://governance/donor/") {
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
    if cmd == "governance" {
        let status = std::process::Command::new("cargo")
            .args(["run", "--quiet", "--manifest-path"])
            .arg(root().join(".ecdev/Cargo.toml"))
            .arg("--")
            .args(&a[1..])
            .arg("--root")
            .arg(root())
            .status()?;
        if !status.success() {
            return Err("ECDEV governance command reported failing gates".into());
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
        "doctor" => doctor_command(&e, a.get(1).map(String::as_str))?,
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
                "ecdev server | stdio | doctor [sp-api] | status | providers | runs [inspect|replay ID] | call TOOL [input.json] | governance COMMAND | mcp-config"
            );
            return Ok(());
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
/// `ecdev doctor` and `ecdev doctor sp-api`. Neither probes the network nor reveals secrets.
pub fn doctor_command(e: &Engine, target: Option<&str>) -> Result<Value, String> {
    match target {
        None => {
            let official = e.call("ecdev.provider.doctor", json!({}))?;
            Ok(
                json!({"storage":e.call("ecdev.system.health",json!({}))?,"provider_config":e.providers(),"web_assets":"EMBEDDED","network":"NOT_PROBED","MCP_endpoint":"NOT_PROBED","market_support":"NATIVE_PUBLIC_SOURCES; OFFICIAL_API_UNAVAILABLE","official_sp_api":{"status":official["status"],"detail":"ecdev doctor sp-api"},"cache":"PERSISTENT_TTL_CONDITIONAL","budget":e.budget_status()?}),
            )
        }
        Some("sp-api") => e.call("ecdev.provider.doctor", json!({"provider":"amazon-sp-api"})),
        Some(_) => Err("Unknown doctor target; expected: sp-api".into()),
    }
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
#[cfg(all(test, unix))]
mod metrics_tests {
    use super::*;
    /// Extinction counts come from the recorded governance assessment, never a constant.
    #[test]
    fn extinct_metric_follows_the_recorded_lifecycle() {
        let root = std::env::temp_dir().join(format!("ecdev-metrics-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let link = root.join("research");
        if !link.exists() {
            std::os::unix::fs::symlink(super::root().join("research"), &link).unwrap();
        }
        let engine = Engine::open(&root).unwrap();
        let metrics = engine.call("ecdev.system.metrics", json!({})).unwrap();
        let extinct = engine.lifecycle().unwrap()["extinct"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(metrics["extinct"].as_u64().unwrap() as usize, extinct);
        assert!(extinct >= 1);
    }
}
#[cfg(test)]
mod doctor_tests {
    use super::*;
    use ecdev_marketplace::{doctor::MapEnv, transport::ConfiguredAmazon};
    fn engine(pairs: &[(&str, &str)]) -> Engine {
        let root = std::env::temp_dir().join(format!(
            "ecdev-doctor-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ConfiguredAmazon::from_source(Arc::new(
                MapEnv::new(pairs),
            ))))
    }
    #[test]
    fn doctor_sp_api_cli_and_mcp_report_blocked_credentials_without_secrets() {
        let e = engine(&[]);
        let cli = doctor_command(&e, Some("sp-api")).unwrap();
        assert_eq!(cli["status"], "LIVE_AUTH_BLOCKED_BY_CREDENTIALS");
        assert_eq!(cli["source_layer"], "OFFICIAL_SP_API");
        assert_eq!(cli["token_state"], "NEVER_REQUESTED");
        assert_eq!(cli["live_calls_made_by_doctor"], 0);
        assert!(cli["core_monetary_ceiling"]["state"].is_string());
        let mcp = e.call("ecdev.provider.doctor", json!({})).unwrap();
        assert_eq!(mcp["status"], cli["status"]);
        assert_eq!(mcp["operations"], cli["operations"]);
        assert!(
            tool_definitions()
                .iter()
                .any(|t| t["name"] == "ecdev.provider.doctor")
        );
        assert_eq!(
            doctor_command(&e, None).unwrap()["official_sp_api"]["status"],
            "LIVE_AUTH_BLOCKED_BY_CREDENTIALS"
        );
        assert!(doctor_command(&e, Some("keepa")).is_err());
        assert!(
            e.call("ecdev.provider.doctor", json!({"provider":"keepa"}))
                .is_err()
        );
        assert!(
            e.call("ecdev.provider.doctor", json!({"extra":true}))
                .is_err()
        );
        let fake = [
            ("SP_API_CLIENT_ID", "srv-FAKE-cid-Qw8xZr"),
            ("SP_API_CLIENT_SECRET", "srv-FAKE-csec-Vb3nYt"),
            ("SP_API_REFRESH_TOKEN", "Atzr|srv-FAKE-rtok-Kp4mJh"),
        ];
        let e = engine(&fake);
        let report = doctor_command(&e, Some("sp-api")).unwrap();
        assert_eq!(report["status"], "LIVE_READ_BLOCKED_BY_OPERATOR_GATE");
        assert_eq!(report["app_credentials"]["state"], "PRESENT_UNVALIDATED");
        let text = report.to_string() + &e.providers().to_string();
        for (_, secret) in fake {
            assert!(!text.contains(secret));
            assert!(!text.contains(&secret[..12]));
        }
        let bare = Engine::open(
            &std::env::temp_dir().join(format!("ecdev-doctor-bare-{}", std::process::id())),
        )
        .unwrap();
        assert_eq!(
            bare.call("ecdev.provider.doctor", json!({})).unwrap()["status"],
            "OFFICIAL_ADAPTER_UNAVAILABLE"
        );
    }
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
        let hyp = engine
            .call(
                "ecdev.trend.hypothesize",
                json!({"snapshot_id": before["snapshot_id"]}),
            )
            .unwrap();
        assert_eq!(hyp["network_calls"], 0);
        assert!(hyp["run_id"].is_string());
        for h in hyp["hypotheses"].as_array().unwrap() {
            assert_eq!(h["shortlist_eligible"], false);
            assert!(
                h["blocked_by"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("SOCIAL_SIGNAL_IS_NOT_DEMAND"))
            );
        }
        assert!(!hyp["hypotheses"].as_array().unwrap().is_empty());
        assert!(
            engine
                .call("ecdev.trend.hypothesize", json!({"snapshot_id": "missing"}))
                .is_err()
        );
        assert_eq!(engine.candidates().unwrap(), commercial_before);
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
                    ".ecdev-data/raw/{}.{extension}",
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
            .join(".ecdev-data/runtime/social-captures")
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
    fn social_sources_walk_their_own_cursors_with_a_stall_guard() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-social-pages-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::social::Social::default()));
        let hn = |page: u64, ids: &[&str]| {
            json!({"page":page,"nbPages":3,"hits":ids.iter().enumerate().map(|(i,id)|json!({"objectID":id,"title":"matcha glass","created_at_i":9000+i as u64})).collect::<Vec<_>>()}).to_string()
        };
        let run = engine.trend_discover(json!({"query":"matcha","fixture_now":10000,"sources":[{"platform":"HACKER_NEWS","max_pages":5,"fixture_raw":hn(0,&["1","2"]),"fixture_pages":[hn(1,&["3"]),hn(2,&["4","5"])]}]})).unwrap();
        assert_eq!(run["mention_count"], 5);
        assert_eq!(run["pagination"][0]["pages"], 3);
        assert_eq!(run["pagination"][0]["stop"], "END_OF_RESULTS");
        // Each page is its own verified capture.
        let hashes: std::collections::BTreeSet<String> = run["captured_posts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["raw_hash"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(hashes.len(), 3);
        // Default: one page, as before.
        let one = engine.trend_discover(json!({"query":"matcha","fixture_now":10000,"sources":[{"platform":"HACKER_NEWS","fixture_raw":hn(0,&["1","2"])}]})).unwrap();
        assert_eq!(
            (
                one["pagination"][0]["pages"].clone(),
                one["pagination"][0]["stop"].clone()
            ),
            (json!(1), json!("PAGE_LIMIT"))
        );
        // A source that hands back a cursor it already gave stops instead of looping.
        let sky = |rkey: &str, cursor: &str| {
            json!({"cursor":cursor,"posts":[{"uri":format!("at://did:plc:a/app.bsky.feed.post/{rkey}"),"author":{"did":"did:plc:a","handle":"a.bsky.social"},"record":{"text":"matcha glass","createdAt":"1970-01-01T02:30:00Z"}}]}).to_string()
        };
        let stalled = engine.trend_discover(json!({"query":"matcha","fixture_now":10000,"sources":[{"platform":"BLUESKY","max_pages":5,"fixture_raw":sky("a","c1"),"fixture_pages":[sky("b","c2"),sky("c","c1"),sky("d","c3")]}]})).unwrap();
        assert_eq!(stalled["pagination"][0]["stop"], "REPEATED_CURSOR");
        assert_eq!(stalled["pagination"][0]["pages"], 3);
        assert_eq!(stalled["mention_count"], 3);
    }
    #[test]
    fn sliced_social_windows_report_their_temporal_coverage() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-social-slices-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::social::Social::default()));
        let day = 86_400u64;
        let now = 10 * day;
        let hn = |page: u64, pages: u64, at: &[(&str, u64)]| {
            json!({"page":page,"nbPages":pages,"nbHits":at.len(),"exhaustive":{"nbHits":false},"hits":at.iter().map(|(id,t)|json!({"objectID":id,"title":"matcha glass","created_at_i":t})).collect::<Vec<_>>()}).to_string()
        };
        // Three daily slices, newest first: two pages, one page, then an empty slice.
        let slices = json!([
            [
                hn(0, 2, &[("101", now - 100), ("102", now - 200)]),
                hn(1, 2, &[("103", now - 300)])
            ],
            [hn(0, 1, &[("104", now - day - 50)])],
            [hn(0, 0, &[])]
        ]);
        let run = engine.trend_discover(json!({"query":"matcha","fixture_now":now,"window_seconds":3 * day,"sources":[{"platform":"HACKER_NEWS","max_pages":5,"slice_seconds":day,"fixture_slices":slices}]})).unwrap();
        let p = &run["pagination"][0];
        assert_eq!(p["slices"].as_array().unwrap().len(), 3);
        assert_eq!(
            (
                p["slices"][0]["since"].clone(),
                p["slices"][0]["until"].clone()
            ),
            (json!(now - day), json!(now))
        );
        assert_eq!(p["slices"][0]["stop"], "END_OF_RESULTS");
        assert_eq!(p["slices"][2]["stop"], "EMPTY_PAGE");
        let c = &p["population_coverage"];
        assert_eq!(c["state"], "COMPLETE_BY_SOURCE");
        assert_eq!(c["temporal_span_coverage"], 1.0);
        assert_eq!(
            (c["observed_earliest"].clone(), c["observed_latest"].clone()),
            (json!(now - day - 50), json!(now - 100))
        );
        assert_eq!(c["items_observed"], 4);
        assert!(c["coverage_ratio"].is_null());
        assert_eq!(c["total_state"], "SOURCE_TOTAL_UNKNOWN");
        assert_eq!(run["mention_count"], 4);
        assert_eq!(run["population_evidence"]["grade"], "COMPLETE_BY_SOURCE");
        assert_eq!(
            run["population_evidence"]["sources"][0]["basis"],
            "TIME_SLICED"
        );
        // Hypotheses carry what their snapshot rests on, beside (not inside) the signal.
        let h = engine
            .trend_hypothesize(json!({"snapshot_id":run["snapshot_id"]}))
            .unwrap();
        let first = &h["hypotheses"][0];
        assert_eq!(
            first["evidence_completeness"]["grade"],
            "COMPLETE_BY_SOURCE"
        );
        assert_eq!(first["evidence_completeness"]["items_observed"], 4);
        assert_eq!(first["shortlist_eligible"], false);
        // One first page over the same posts: same mention count, weaker evidence.
        let one = engine.trend_discover(json!({"query":"matcha","fixture_now":now,"window_seconds":3 * day,"sources":[{"platform":"HACKER_NEWS","fixture_raw":hn(0, 2, &[("101", now - 100), ("102", now - 200), ("103", now - 300), ("104", now - day - 50)])}]})).unwrap();
        assert_eq!(one["mention_count"], run["mention_count"]);
        assert_eq!(one["population_evidence"]["grade"], "PARTIAL");
        assert_eq!(
            one["population_evidence"]["sources"][0]["basis"],
            "FIRST_PAGE_ONLY"
        );
        let h1 = engine
            .trend_hypothesize(json!({"snapshot_id":one["snapshot_id"]}))
            .unwrap();
        assert_eq!(
            h1["hypotheses"][0]["evidence_completeness"]["grade"],
            "PARTIAL"
        );
        // A slice the page limit cut short is partial, and its span is not counted as covered.
        let cut = engine.trend_discover(json!({"query":"matcha","fixture_now":now,"window_seconds":2 * day,"sources":[{"platform":"HACKER_NEWS","max_pages":1,"slice_seconds":day,"fixture_slices":[[hn(0, 2, &[("101", now - 100)])],[hn(0, 1, &[("104", now - day - 50)])]]}]})).unwrap();
        let c = &cut["pagination"][0]["population_coverage"];
        assert_eq!(
            (c["state"].clone(), c["temporal_span_coverage"].clone()),
            (json!("PARTIAL_PAGE_LIMIT"), json!(0.5))
        );
        assert_eq!(c["gaps"][0]["reason"], "PAGE_LIMIT");
        // Sources that cannot bound time refuse slicing rather than pretend.
        let feed = engine.trend_discover(json!({"query":"matcha","fixture_now":now,"window_seconds":day,"sources":[{"platform":"JSON_FEED","url":"https://f.example/feed.json","slice_seconds":day,"fixture_slices":[["{}"]]}]})).unwrap();
        assert_eq!(
            feed["provider_failures"][0]["reason"],
            "TIME_BOUND_UNSUPPORTED_BY_SOURCE"
        );
    }
    #[test]
    fn mastodon_tag_usage_is_an_attention_counter_with_its_capture() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-social-tag-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::social::Social::default()));
        let day = 86_400u64;
        let today = 20_000 * day;
        // Seven days, newest first as the instance reports them; today is still open.
        let history: Vec<Value> = (0..7)
            .map(|d| {
                let (uses, accounts) = if (1..=3).contains(&d) { (60, 25) } else { (6, 3) };
                json!({"day":(today - d * day).to_string(),"uses":uses.to_string(),"accounts":accounts.to_string()})
            })
            .collect();
        let raw =
            json!({"name":"matcha","url":"https://mastodon.social/tags/matcha","history":history})
                .to_string();
        let run = engine.trend_discover(json!({"query":"matcha","fixture_now":today + 3_600,"sources":[{"platform":"MASTODON_TAG","fixture_raw":raw}]})).unwrap();
        let c = &run["source_counters"][0];
        assert_eq!(c["evidence_class"], "ATTENTION_SIGNAL_NOT_DEMAND");
        assert_eq!(c["scope"], "INSTANCE_FEDERATED_VIEW");
        assert_eq!(c["series"].as_array().unwrap().len(), 7);
        assert_eq!(c["growth"]["state"], "RISING");
        assert_eq!(
            c["growth"]["after"]["until"], today,
            "the open day is not compared"
        );
        assert_eq!(c["capture_mode"], "FIXTURE");
        // Tag usage is no post and no demand: nothing enters the post population.
        assert_eq!(run["pagination"][0]["posts"], 0);
        // Inspection re-verifies the counter's raw capture.
        let id = run["snapshot_id"].as_str().unwrap();
        let inspected = engine.trend_inspect(json!({"snapshot_id":id})).unwrap();
        assert_eq!(inspected["source_counters"][0]["raw_hash"], c["raw_hash"]);
        // A rising counter with no posts is a research reason, never a shortlist.
        let hyp = engine.trend_hypothesize(json!({"snapshot_id":id})).unwrap();
        let h = &hyp["hypotheses"][0];
        assert_eq!(h["basis"], "ATTENTION_COUNTER_ONLY");
        assert_eq!(h["shortlist_eligible"], false);
        assert_eq!(h["attention_counters"][0]["growth_state"], "RISING");
        std::fs::remove_file(
            root.join(".ecdev-data/runtime/social-captures")
                .join(format!("{}.raw", c["raw_hash"].as_str().unwrap())),
        )
        .unwrap();
        assert!(engine.trend_inspect(json!({"snapshot_id":id})).is_err());
        // Time slices and reply trees are refused for a tag record.
        let sliced = engine.trend_discover(json!({"query":"matcha","fixture_now":today,"window_seconds":2 * day,"sources":[{"platform":"MASTODON_TAG","slice_seconds":day,"fixture_slices":[[raw],[raw]]}]})).unwrap();
        assert_eq!(
            sliced["provider_failures"][0]["reason"],
            "TIME_BOUND_UNSUPPORTED_BY_SOURCE"
        );
        assert!(sliced["source_counters"].as_array().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn trending_lists_become_research_leads_with_next_actions() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-social-feeds-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::social::Social::default()));
        let day = 86_400u64;
        let today = 20_000 * day;
        let history: Vec<Value> = (0..7)
            .map(|d| {
                let n = if (1..=3).contains(&d) { 80 } else { 5 };
                json!({"day":(today - d * day).to_string(),"uses":n.to_string(),"accounts":n.to_string()})
            })
            .collect();
        let mastodon = json!([{"name":"MatchaLatte","url":"https://mastodon.social/tags/matchalatte","history":history}]).to_string();
        let bluesky = json!({"trends":[{"topic":"t","displayName":"Matcha Whisk","postCount":900,"status":"hot","category":"food","actors":[{"handle":"someone.bsky.social"}]}]}).to_string();
        let out = engine.call("ecdev.trend.feeds", json!({"fixture_now":today + 60,"sources":[{"platform":"MASTODON_TRENDS","fixture_raw":mastodon},{"platform":"BLUESKY_TRENDS","fixture_raw":bluesky}]})).unwrap();
        assert_eq!(out["capture_mode"], "FIXTURE");
        let m = &out["feeds"][0]["entries"][0];
        assert_eq!(m["state"], "DISCOVERY_LEAD_UNVERIFIED");
        assert_eq!(m["growth"]["state"], "RISING");
        assert_eq!(m["next_action"]["tool"], "ecdev.trend.discover");
        assert_eq!(m["next_action"]["input_template"]["query"], "matcha latte");
        assert_eq!(
            m["next_action"]["input_template"]["sources"][0]["platform"],
            "MASTODON_TAG"
        );
        let b = &out["feeds"][1]["entries"][0];
        assert_eq!(b["source_post_count"], 900);
        assert!(!out.to_string().contains("someone.bsky.social"));
        // The raw captures are kept by hash.
        for f in out["feeds"].as_array().unwrap() {
            assert!(
                root.join(".ecdev-data/runtime/social-captures")
                    .join(format!("{}.raw", f["raw_hash"].as_str().unwrap()))
                    .is_file()
            );
        }
        assert_eq!(m["list_persistence"]["state"], "NEW_ON_LIST");
        // A day later the tag is still listed: its identity and every rank carry over.
        let later = engine.call("ecdev.trend.feeds", json!({"fixture_now":today + day,"sources":[{"platform":"MASTODON_TRENDS","fixture_raw":json!([{"name":"MatchaLatte","history":[]}]).to_string()}]})).unwrap();
        let p = &later["feeds"][0]["entries"][0]["list_persistence"];
        assert_eq!(p["state"], "SUSTAINED");
        assert_eq!(
            (p["first_seen"].clone(), p["captures_on_list"].clone()),
            (json!(today + 60), json!(2))
        );
        assert_eq!(later["feeds"][0]["prior_captures_of_this_feed"], 1);
        // Mixed fixture and live sources, posts platforms and a live clock override are refused.
        assert!(engine.call("ecdev.trend.feeds", json!({"sources":[{"platform":"MASTODON_TRENDS","fixture_raw":"[]"},{"platform":"BLUESKY_TRENDS"}]})).is_err());
        assert!(
            engine
                .call(
                    "ecdev.trend.feeds",
                    json!({"fixture_now":1,"sources":[{"platform":"BLUESKY_TRENDS"}]})
                )
                .is_err()
        );
        let wrong = engine.call(
            "ecdev.trend.feeds",
            json!({"sources":[{"platform":"HACKER_NEWS","fixture_raw":"{}"}]}),
        );
        assert!(
            wrong.is_err()
                || wrong.unwrap()["provider_failures"][0]["reason"] == "NOT_A_TREND_FEED"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rss_and_atom_feeds_are_read_as_feed_posts() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-social-xml-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::social::Social::default()));
        let rss = r#"<?xml version="1.0" encoding="utf-8"?><rss version="2.0"><channel><title>Supplier news</title>
<item><guid>s-1</guid><title>Matcha whisk wholesale</title><link>https://supplier.example/n/1</link><pubDate>Mon, 05 Oct 2026 10:00:00 GMT</pubDate><description>Bamboo matcha whisk lots</description></item>
<item><guid>s-2</guid><title>Matcha bowl</title><link>https://supplier.example/n/2</link><pubDate>Tue, 06 Oct 2026 10:00:00 GMT</pubDate></item></channel></rss>"#;
        let now = 1_791_331_200 + 86_400;
        let run = engine.trend_discover(json!({"query":"matcha","fixture_now":now,"window_seconds":7 * 86_400,"sources":[{"platform":"XML_FEED","url":"https://supplier.example/feed.xml","fixture_raw":rss}]})).unwrap();
        assert!(
            run["provider_failures"].as_array().unwrap().is_empty(),
            "{}",
            run["provider_failures"]
        );
        assert_eq!(run["pagination"][0]["posts"], 2);
        let posts = run["captured_posts"].as_array().unwrap();
        assert!(
            posts
                .iter()
                .all(|p| p["platform"] == "XML_FEED"
                    && p["extraction_method"] == "NATIVE_XML_FEED_V1")
        );
        assert!(
            posts
                .iter()
                .any(|p| p["native_id"] == "https://supplier.example/feed.xml#s-1")
        );
        assert!(posts.iter().any(|p| p["published_at"] == 1_791_194_400u64));
        // Japanese has no spaces: a two-character query is valid and matches inside titles.
        let jp = r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>新商品</title>
<item><guid>j-1</guid><title>新作の抹茶ラテを限定発売</title><link>https://supplier.example/j/1</link><pubDate>Mon, 05 Oct 2026 10:00:00 +0900</pubDate></item>
<item><guid>j-2</guid><title>ほうじ茶ラテが人気</title><link>https://supplier.example/j/2</link><pubDate>Mon, 05 Oct 2026 11:00:00 +0900</pubDate></item></channel></rss>"#;
        let ja = engine.trend_discover(json!({"query":"抹茶","fixture_now":now,"window_seconds":7 * 86_400,"sources":[{"platform":"XML_FEED","url":"https://supplier.example/jp.xml","fixture_raw":jp}]})).unwrap();
        assert_eq!(ja["mention_count"], 1, "{}", ja["mention_count"]);
        // A DTD is refused, never expanded; time slices are refused like any feed.
        let dtd = r#"<!DOCTYPE r [<!ENTITY x "matcha">]><rss version="2.0"><channel><item><title>&x;</title></item></channel></rss>"#;
        let refused = engine.trend_discover(json!({"query":"matcha","fixture_now":now,"sources":[{"platform":"XML_FEED","url":"https://supplier.example/feed.xml","fixture_raw":dtd}]})).unwrap();
        assert_eq!(refused["provider_failures"][0]["reason"], "XML_DTD_REFUSED");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reply_trees_come_from_the_source_with_their_structure() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-social-trees-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let engine = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::social::Social::default()));
        let now = 10_000u64;
        let search = json!({"page":0,"nbPages":1,"hits":[
            {"objectID":"101","title":"matcha glass review","created_at_i":now - 500,"num_comments":3,"points":9,"_tags":["story"]},
            {"objectID":"102","title":"matcha whisk","created_at_i":now - 400,"num_comments":0,"_tags":["story"]}]}).to_string();
        let tree = json!({"id":101,"type":"story","author":"a","title":"matcha glass review","text":null,"created_at_i":now - 500,"parent_id":null,"points":9,"children":[
            {"id":201,"type":"comment","author":"b","text":"matcha glass cracked","created_at_i":now - 450,"parent_id":101,"children":[
                {"id":202,"type":"comment","author":"c","text":"mine too, matcha glass","created_at_i":now - 440,"parent_id":201,"children":[]}]},
            {"id":203,"type":"comment","author":null,"text":null,"created_at_i":now - 430,"parent_id":101,"children":[]}]}).to_string();
        let run = engine.trend_discover(json!({"query":"matcha glass","fixture_now":now,"sources":[{"platform":"HACKER_NEWS","fixture_raw":search,"reply_trees":true,"fixture_threads":{"101":tree}}]})).unwrap();
        let p = &run["pagination"][0];
        assert_eq!(p["threads"].as_array().unwrap().len(), 1);
        let t = &p["threads"][0];
        assert_eq!(
            (
                t["root"].clone(),
                t["reported_comments"].clone(),
                t["comments_observed"].clone(),
                t["deleted"].clone(),
                t["state"].clone()
            ),
            (
                json!("101"),
                json!(3),
                json!(2),
                json!(1),
                json!("COMPLETE_BY_SOURCE")
            )
        );
        let c = &p["population_coverage"];
        assert_eq!(
            (
                c["comments_observed"].clone(),
                c["comment_tree_state"].clone(),
                c["comment_tree_complete"].clone()
            ),
            (json!(2), json!("COMPLETE_BY_SOURCE"), json!(true))
        );
        let replies: Vec<&Value> = run["captured_posts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["propagation"] == "REPLY")
            .collect();
        let deep = replies.iter().find(|p| p["native_id"] == "202").unwrap();
        assert_eq!(
            (
                deep["parent_id"].clone(),
                deep["thread_id"].clone(),
                deep["depth"].clone(),
                deep["raw_locator"].clone()
            ),
            (
                json!("201"),
                json!("101"),
                json!(2),
                json!("/children/0/children/0")
            )
        );
        // Replies have their own raw capture, distinct from the search page.
        assert_ne!(
            deep["raw_hash"],
            run["captured_posts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["native_id"] == "101")
                .unwrap()["raw_hash"]
        );
        // Two replies mention the query and so count as mentions; the root's search hit as well.
        assert_eq!(run["mention_count"], 3);
        // A feed exposes no tree, and says so.
        let feed = json!({"version":"https://jsonfeed.org/version/1.1","items":[{"id":"1","url":"https://f.example/1","content_text":"matcha glass","date_published":"1970-01-01T02:40:00Z"}]}).to_string();
        let f = engine.trend_discover(json!({"query":"matcha glass","fixture_now":now,"sources":[{"platform":"JSON_FEED","url":"https://f.example/feed.json","fixture_raw":feed,"reply_trees":true}]})).unwrap();
        assert_eq!(
            f["pagination"][0]["population_coverage"]["comment_tree_state"],
            "SOURCE_DOES_NOT_EXPOSE_TREE"
        );
        assert_eq!(
            f["pagination"][0]["population_coverage"]["comment_tree_complete"],
            false
        );
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
        let path = root.join(".ecdev-data/raw").join(format!(
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
        let path = root.join(".ecdev-data/raw").join(format!(
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
    fn stored_captures_reextract_into_typed_series_without_network() {
        let root = std::env::temp_dir().join(format!(
            "ecdev-reextract-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let e = Engine::open(&root)
            .unwrap()
            .with_provider(Arc::new(ecdev_web::Web::default()));
        let page = |price: &str| {
            format!(
                r#"<meta property="og:type" content="product"><meta property="og:title" content="Cup"><meta property="product:price:amount" content="{price}"><meta property="product:price:currency" content="JPY">"#
            )
        };
        let args = |html: String| json!({"market":"PUBLIC_WEB","query":"Reextraction fixture","sources":[{"url":"https://example.org/cup","fixture_html":html}],"max_pages":1});
        let first = e.research(args(page("2,980"))).unwrap();
        let second = e.research(args(page("2,500"))).unwrap();
        let ids = vec![
            first["run_id"].as_str().unwrap().to_string(),
            second["run_id"].as_str().unwrap().to_string(),
        ];
        let access = &first["access_diagnostics"][0];
        assert_eq!(access["state"], "REACHABLE_WITH_PRODUCT_DATA");
        assert_eq!(access["source"], "https://example.org/cup");
        assert_eq!(access["response_bytes"], page("2,980").len());
        let out = e.research_reextract(&ids).unwrap();
        assert_eq!(out["network"], "NONE");
        assert!(
            out["captures"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c["state"] == "SAME_AS_RECORDED")
        );
        let price = out["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["field"] == "price_minor")
            .unwrap();
        let values: Vec<_> = price["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["value"].clone())
            .collect();
        assert_eq!(values, [json!(2980), json!(2500)]);
        // Altered bytes contribute nothing.
        let hash = out["captures"][0]["capture"]["raw_hash"].as_str().unwrap();
        std::fs::write(
            root.join(format!(".ecdev-data/raw/{hash}.html")),
            b"altered",
        )
        .unwrap();
        let altered = e.research_reextract(&ids[..1]).unwrap();
        assert_eq!(
            altered["captures"][0]["state"],
            "RAW_UNAVAILABLE_OR_ALTERED"
        );
        assert_eq!(altered["series"], json!([]));
        assert_eq!(
            e.research_reextract(&["missing".into()]).unwrap_err(),
            "RUN_NOT_FOUND"
        );
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
