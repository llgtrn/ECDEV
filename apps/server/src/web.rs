//! The HTTP surface: the embedded web console, the JSON API, the event stream and the MCP
//! Streamable HTTP endpoint (JSON responses; the server starts no stream of its own).

use std::{collections::VecDeque, sync::Mutex, time::Duration};

use ecdev_core::identifier::Uuid;
use ecdev_core::json;
use ecdev_core::json::Value;
use ecdev_core::{Engine, service::tool_definitions};
use ecdev_net::{
    http::Headers,
    server::{Handler, Reply, Request, Response},
};

use crate::mcp::{self, Mcp, PARSE_ERROR, PROTOCOLS};

const SESSION_LIMIT: usize = 1024;

pub struct App {
    engine: Engine,
    mcp: Mcp,
    sessions: Mutex<VecDeque<String>>,
}

fn json_response(status: u16, v: &Value) -> Response {
    Response::bytes(status, "application/json", v.to_string())
}

fn result_response(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => json_response(200, &v),
        Err(e) => json_response(400, &json!({"error": e})),
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(k)? == key)
            .then(|| percent_decode(&v.replace('+', " ")))
            .flatten()
    })
}

fn not_allowed(allow: &str) -> Response {
    Response::text(405, "Method Not Allowed").with_header("Allow", allow)
}

impl App {
    pub fn new(engine: Engine) -> App {
        App {
            mcp: Mcp::new(engine.clone()),
            engine,
            sessions: Mutex::new(VecDeque::new()),
        }
    }

    /// Requests must address the loopback host they were sent to, and a browser Origin must be
    /// that same origin: a page on another site cannot drive the local server.
    fn origin_guard(&self, req: &Request) -> Option<Response> {
        let host = req.headers.get("host").unwrap_or("");
        if !host.starts_with("127.0.0.1:") && !host.starts_with("localhost:") {
            return Some(Response::text(403, "Invalid local host"));
        }
        if let Some(origin) = req.headers.get_bytes("origin")
            && origin != format!("http://{host}").as_bytes()
        {
            return Some(Response::text(403, "Invalid origin"));
        }
        None
    }

    fn metrics_text(&self) -> Response {
        match self.engine.metrics() {
            Ok(v) => {
                let lines: String = v
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| v.as_u64().map(|n| format!("ecdev_{k} {n}\n")))
                            .collect()
                    })
                    .unwrap_or_default();
                Response::bytes(200, "text/plain; version=0.0.4", lines)
            }
            Err(e) => Response::text(500, e),
        }
    }

    fn events(&self, req: &Request) -> Reply {
        let after = req
            .headers
            .get("last-event-id")
            .map(str::to_string)
            .or_else(|| query_param(req.query.as_deref(), "after"))
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let engine = self.engine.clone();
        let mut headers = Headers::new();
        headers
            .append("Content-Type", "text/event-stream")
            .expect("valid");
        headers.append("Cache-Control", "no-cache").expect("valid");
        Reply::Stream {
            status: 200,
            headers,
            produce: Box::new(move |stream| {
                let mut after = after;
                let mut idle_ticks = 0u32;
                while !stream.should_stop() {
                    let next = engine
                        .events(after)
                        .ok()
                        .and_then(|v| v.as_array().and_then(|a| a.first().cloned()));
                    if let Some(event) = next {
                        after = event["id"].as_u64().unwrap_or(after + 1);
                        stream.send(
                            format!("id: {after}\nevent: run\ndata: {}\n\n", event["data"])
                                .as_bytes(),
                        )?;
                        idle_ticks = 0;
                        continue;
                    }
                    std::thread::sleep(Duration::from_millis(500));
                    idle_ticks += 1;
                    // A comment line keeps proxies open and notices a client that has gone.
                    if idle_ticks.is_multiple_of(30) {
                        stream.send(b": keep-alive\n\n")?;
                    }
                }
                Ok(())
            }),
        }
    }

    fn mcp_post(&self, req: &Request) -> Response {
        if let Some(accept) = req.headers.get("accept")
            && !["application/json", "*/*", "application/*"]
                .iter()
                .any(|t| accept.contains(t))
        {
            return Response::text(406, "The client must accept application/json");
        }
        if !req.headers.get("content-type").is_some_and(|c| {
            c.trim_start()
                .to_ascii_lowercase()
                .starts_with("application/json")
        }) {
            return Response::text(415, "Content-Type must be application/json");
        }
        if let Some(version) = req.headers.get("mcp-protocol-version")
            && !PROTOCOLS.contains(&version)
        {
            return Response::text(400, "Unsupported MCP-Protocol-Version");
        }
        let Ok(text) = std::str::from_utf8(&req.body) else {
            return json_response(
                400,
                &json!({"jsonrpc":"2.0","id":null,"error":{"code":PARSE_ERROR,"message":"the body is not UTF-8"}}),
            );
        };
        let parsed = json::from_str::<Value>(text);
        let starts_session = matches!(&parsed, Ok(v) if v.get("method").and_then(Value::as_str) == Some("initialize"));
        let mut session = None;
        if starts_session {
            let id = Uuid::new_v4().to_string();
            let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if sessions.len() >= SESSION_LIMIT {
                sessions.pop_front();
            }
            sessions.push_back(id.clone());
            session = Some(id);
        } else {
            match req.headers.get("mcp-session-id") {
                None => return Response::text(400, "Mcp-Session-Id header is required"),
                Some(id) => {
                    if !self
                        .sessions
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter()
                        .any(|s| s == id)
                    {
                        return Response::text(404, "Unknown session");
                    }
                }
            }
        }
        match self.mcp.handle_text(text) {
            None => Response::new(202),
            Some(body) => {
                let status = if parsed.is_err() { 400 } else { 200 };
                let r = Response::bytes(status, "application/json", body);
                match session {
                    Some(id) => r.with_header("Mcp-Session-Id", &id),
                    None => r,
                }
            }
        }
    }

    fn mcp_delete(&self, req: &Request) -> Response {
        let Some(id) = req.headers.get("mcp-session-id") else {
            return Response::text(400, "Mcp-Session-Id header is required");
        };
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        match sessions.iter().position(|s| s == id) {
            Some(i) => {
                sessions.remove(i);
                Response::new(200)
            }
            None => Response::text(404, "Unknown session"),
        }
    }

    fn route(&self, req: &Request) -> Reply {
        let get = req.method == "GET" || req.method == "HEAD";
        let path = req.path.as_str();
        let e = &self.engine;
        let asset = |content_type: &str, body: &'static str| -> Reply {
            if get {
                Response::bytes(200, content_type, body).into()
            } else {
                not_allowed("GET, HEAD").into()
            }
        };
        let api = |f: &dyn Fn() -> Response| -> Reply {
            if get {
                f().into()
            } else {
                not_allowed("GET, HEAD").into()
            }
        };
        match path {
            "/" => asset(
                "text/html; charset=utf-8",
                include_str!("../../web/index.html"),
            ),
            "/app.js" => asset("text/javascript", include_str!("../../web/app.js")),
            "/style.css" => asset("text/css", include_str!("../../web/style.css")),
            "/health" => api(&|| result_response(e.call("ecdev.system.health", json!({})))),
            "/metrics" => api(&|| self.metrics_text()),
            "/events" => {
                if get {
                    self.events(req)
                } else {
                    not_allowed("GET, HEAD").into()
                }
            }
            "/api/status" => api(&|| result_response(e.status())),
            "/api/providers" => api(&|| json_response(200, &e.providers())),
            "/api/donors" => api(&|| result_response(e.registry())),
            "/api/metrics" => api(&|| result_response(e.metrics())),
            "/api/tools" => api(&|| json_response(200, &json!(tool_definitions()))),
            "/api/research/example" => api(&|| {
                Response::bytes(
                    200,
                    "application/json",
                    include_str!("../../../domain/commerce/tests/fixtures/research-household.json"),
                )
            }),
            "/api/runs" => api(&|| result_response(e.runs())),
            "/mcp" => match req.method.as_str() {
                "POST" => self.mcp_post(req).into(),
                "DELETE" => self.mcp_delete(req).into(),
                _ => not_allowed("POST, DELETE").into(),
            },
            _ => {
                if let Some(id) = path.strip_prefix("/api/donors/") {
                    return match percent_decode(id) {
                        Some(id) if get => result_response(e.census(&id)).into(),
                        Some(_) => not_allowed("GET, HEAD").into(),
                        None => Response::text(400, "Bad path").into(),
                    };
                }
                if let Some(id) = path.strip_prefix("/api/runs/") {
                    return match percent_decode(id) {
                        Some(id) if get => result_response(e.run(&id)).into(),
                        Some(_) => not_allowed("GET, HEAD").into(),
                        None => Response::text(400, "Bad path").into(),
                    };
                }
                if let Some(name) = path.strip_prefix("/api/tools/") {
                    let Some(name) = percent_decode(name) else {
                        return Response::text(400, "Bad path").into();
                    };
                    if req.method != "POST" {
                        return not_allowed("POST").into();
                    }
                    let Ok(args) = std::str::from_utf8(&req.body)
                        .map_err(|e| e.to_string())
                        .and_then(|t| json::from_str::<Value>(t).map_err(|e| e.to_string()))
                    else {
                        return Response::text(400, "The body must be a JSON document").into();
                    };
                    return result_response(e.call(&name, args)).into();
                }
                Response::text(404, "Not Found").into()
            }
        }
    }
}

impl Handler for App {
    fn handle(&self, req: &Request) -> Reply {
        if let Some(denied) = self.origin_guard(req) {
            return denied.into();
        }
        self.route(req)
    }
}

pub use mcp::Mcp as McpHandler;

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let root = std::env::temp_dir().join(format!(
            "ecdev-web-unit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../research"),
            root.join("research"),
        )
        .unwrap();
        App::new(Engine::open(&root).unwrap())
    }

    fn request(method: &str, target: &str, headers: &[(&str, &str)], body: &str) -> Request {
        let mut h = Headers::new();
        h.append("Host", "127.0.0.1:8765").unwrap();
        for (k, v) in headers {
            h.set(k, v).unwrap();
        }
        let (path, query) = match target.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None => (target.to_string(), None),
        };
        Request {
            method: method.into(),
            target: target.into(),
            path,
            query,
            minor: 1,
            headers: h,
            body: body.as_bytes().to_vec(),
            peer: "127.0.0.1:1".parse().unwrap(),
        }
    }

    fn status(a: &App, r: &Request) -> u16 {
        match a.handle(r) {
            Reply::Response(r) => r.status,
            Reply::Stream { status, .. } => status,
        }
    }

    #[test]
    fn percent_decoding_and_query_lookup_are_strict() {
        assert_eq!(percent_decode("a%20b%2Fc").as_deref(), Some("a b/c"));
        assert_eq!(percent_decode("%e3%81%82").as_deref(), Some("あ"));
        assert_eq!(percent_decode("%"), None);
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%ff"), None);
        assert_eq!(
            query_param(Some("a=1&after=42&b"), "after").as_deref(),
            Some("42")
        );
        assert_eq!(query_param(Some("q=a+b%21"), "q").as_deref(), Some("a b!"));
        assert_eq!(query_param(Some("b"), "b").as_deref(), Some(""));
        assert_eq!(query_param(None, "a"), None);
        assert_eq!(query_param(Some("a=%zz"), "a"), None);
    }

    #[test]
    fn only_the_loopback_host_and_its_own_origin_are_served() {
        let a = app();
        assert_eq!(status(&a, &request("GET", "/health", &[], "")), 200);
        assert_eq!(
            status(
                &a,
                &request("GET", "/health", &[("Host", "evil.example")], "")
            ),
            403
        );
        assert_eq!(
            status(
                &a,
                &request(
                    "GET",
                    "/health",
                    &[("Host", "127.0.0.1.evil.example:80")],
                    ""
                )
            ),
            403
        );
        assert_eq!(
            status(
                &a,
                &request("GET", "/health", &[("Origin", "https://evil.example")], "")
            ),
            403
        );
        assert_eq!(
            status(
                &a,
                &request("GET", "/health", &[("Origin", "http://127.0.0.1:8765")], "")
            ),
            200
        );
        assert_eq!(
            status(
                &a,
                &request("GET", "/health", &[("Host", "localhost:8765")], "")
            ),
            200
        );
    }

    #[test]
    fn routes_methods_and_paths_answer_with_the_right_status() {
        let a = app();
        for path in [
            "/",
            "/app.js",
            "/style.css",
            "/api/providers",
            "/api/tools",
            "/api/research/example",
        ] {
            assert_eq!(status(&a, &request("GET", path, &[], "")), 200, "{path}");
            assert_eq!(status(&a, &request("HEAD", path, &[], "")), 200, "{path}");
            assert_eq!(status(&a, &request("POST", path, &[], "")), 405, "{path}");
        }
        #[cfg(unix)]
        for path in [
            "/metrics",
            "/api/status",
            "/api/donors",
            "/api/metrics",
            "/api/runs",
        ] {
            assert_eq!(status(&a, &request("GET", path, &[], "")), 200, "{path}");
        }
        assert_eq!(status(&a, &request("GET", "/nope", &[], "")), 404);
        assert_eq!(status(&a, &request("GET", "/api/donors/%zz", &[], "")), 400);
        assert_eq!(
            status(
                &a,
                &request("GET", "/api/tools/ecdev.system.health", &[], "")
            ),
            405
        );
        assert_eq!(
            status(
                &a,
                &request("POST", "/api/tools/ecdev.system.health", &[], "{")
            ),
            400
        );
        assert_eq!(
            status(
                &a,
                &request("POST", "/api/tools/ecdev.system.health", &[], "{}")
            ),
            200
        );
        assert_eq!(
            status(&a, &request("POST", "/api/tools/ecdev.no.such", &[], "{}")),
            400
        );
        assert_eq!(status(&a, &request("PUT", "/mcp", &[], "")), 405);
        let Reply::Response(r) = a.handle(&request("GET", "/mcp", &[], "")) else {
            panic!()
        };
        assert_eq!(r.headers.get("allow"), Some("POST, DELETE"));
    }

    #[test]
    fn streamable_http_enforces_headers_sessions_and_the_notification_status() {
        let a = app();
        let ok = [
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
        ];
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;
        assert_eq!(
            status(
                &a,
                &request(
                    "POST",
                    "/mcp",
                    &[
                        ("Content-Type", "application/json"),
                        ("Accept", "text/html")
                    ],
                    init
                )
            ),
            406
        );
        assert_eq!(
            status(
                &a,
                &request("POST", "/mcp", &[("Content-Type", "text/plain")], init)
            ),
            415
        );
        assert_eq!(
            status(
                &a,
                &request(
                    "POST",
                    "/mcp",
                    &[
                        ("Content-Type", "application/json"),
                        ("MCP-Protocol-Version", "x")
                    ],
                    init
                )
            ),
            400
        );
        let Reply::Response(r) = a.handle(&request("POST", "/mcp", &ok, init)) else {
            panic!()
        };
        assert_eq!(r.status, 200);
        let session = r.headers.get("mcp-session-id").unwrap().to_string();
        assert_eq!(session.len(), 36);
        let with = |id: &str| {
            [("Content-Type", "application/json"), ("Mcp-Session-Id", id)]
                .map(|(k, v)| (k.to_string(), v.to_string()))
        };
        let go = |id: &str, body: &str| {
            let h = with(id);
            let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            status(&a, &request("POST", "/mcp", &h, body))
        };
        assert_eq!(
            go(
                &session,
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
            ),
            202
        );
        assert_eq!(
            go(&session, r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#),
            200
        );
        assert_eq!(go(&session, "{broken"), 400);
        assert_eq!(
            go(
                "00000000-0000-0000-0000-000000000000",
                r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#
            ),
            404
        );
        assert_eq!(
            status(
                &a,
                &request(
                    "POST",
                    "/mcp",
                    &[("Content-Type", "application/json")],
                    r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#
                )
            ),
            400
        );
        assert_eq!(
            status(
                &a,
                &request("DELETE", "/mcp", &[("Mcp-Session-Id", &session)], "")
            ),
            200
        );
        assert_eq!(
            status(
                &a,
                &request("DELETE", "/mcp", &[("Mcp-Session-Id", &session)], "")
            ),
            404
        );
        assert_eq!(status(&a, &request("DELETE", "/mcp", &[], "")), 400);
    }

    #[test]
    fn the_session_table_is_bounded() {
        let a = app();
        let ok = [("Content-Type", "application/json")];
        let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let Reply::Response(first) = a.handle(&request("POST", "/mcp", &ok, init)) else {
            panic!()
        };
        let first = first.headers.get("mcp-session-id").unwrap().to_string();
        for _ in 0..SESSION_LIMIT {
            a.handle(&request("POST", "/mcp", &ok, init));
        }
        assert_eq!(a.sessions.lock().unwrap().len(), SESSION_LIMIT);
        assert_eq!(
            status(
                &a,
                &request("DELETE", "/mcp", &[("Mcp-Session-Id", &first)], "")
            ),
            404
        );
    }
}
