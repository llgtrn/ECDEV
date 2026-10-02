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
            Ok(v) => CallToolResult::structured(v),
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
    axum::serve(listener, router(e, port))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cmd = a.first().map(String::as_str).unwrap_or("help");
    if cmd == "server" {
        return serve().await;
    }
    if cmd == "stdio" {
        let service = Mcp(configured_engine()?)
            .serve(rmcp::transport::stdio())
            .await?;
        service.waiting().await?;
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
        .with_provider(std::sync::Arc::new(ecdev_keepa::client::Keepa::from_env()))
        .with_provider(std::sync::Arc::new(ecdev_web::Web::default())))
}
#[cfg(test)]
mod research_tests {
    use super::*;
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
    }
}
