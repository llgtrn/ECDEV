//! The Model Context Protocol over JSON-RPC 2.0: the methods ECDEV serves (initialize, ping, tools,
//! resources) as pure functions of a message, so stdio and Streamable HTTP share one implementation.

use ecdev_core::json;
use ecdev_core::json::Value;
use ecdev_core::{Engine, service::tool_definitions};

/// Protocol revisions this server speaks, newest first.
pub const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Native/public research is available without paid providers. Supplied HTML is FIXTURE; unknown commercial evidence remains unknown. Keepa is optional and denied by zero budget before IO; live auth unverified. Economics SIMULATED; opportunity search PLAN_ONLY. Donor extinction requires proofs.";

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

#[derive(Clone)]
pub struct Mcp {
    engine: Engine,
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

/// The outcome of one JSON-RPC message.
pub enum Reply {
    /// A request: the response to send.
    Response(Value),
    /// A notification or a client response: nothing to send.
    None,
}

impl Mcp {
    pub fn new(engine: Engine) -> Mcp {
        Mcp { engine }
    }

    /// Handles one message. Never panics on hostile input.
    pub fn handle(&self, message: &Value) -> Reply {
        let Some(object) = message.as_object() else {
            return Reply::Response(error(
                Value::Null,
                INVALID_REQUEST,
                "a JSON-RPC message is an object",
            ));
        };
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Reply::Response(error(
                object.get("id").cloned().unwrap_or(Value::Null),
                INVALID_REQUEST,
                "jsonrpc must be \"2.0\"",
            ));
        }
        let method = object.get("method").and_then(Value::as_str);
        let id = object.get("id");
        match (method, id) {
            // A response from the client to a server request: the server sends none.
            (None, Some(_)) if object.contains_key("result") || object.contains_key("error") => {
                Reply::None
            }
            (None, _) => Reply::Response(error(
                id.cloned().unwrap_or(Value::Null),
                INVALID_REQUEST,
                "a message needs a method",
            )),
            (Some(_), None) => Reply::None,
            (Some(method), Some(id)) => {
                if !(id.is_string() || id.is_number()) {
                    return Reply::Response(error(
                        Value::Null,
                        INVALID_REQUEST,
                        "an id is a string or a number",
                    ));
                }
                let params = object.get("params").cloned().unwrap_or(Value::Null);
                Reply::Response(match self.dispatch(method, &params) {
                    Ok(v) => result(id.clone(), v),
                    Err((code, message)) => error(id.clone(), code, &message),
                })
            }
        }
    }

    /// Handles a line of text (stdio): a parse failure is answered with -32700.
    pub fn handle_text(&self, text: &str) -> Option<String> {
        match json::from_str::<Value>(text) {
            Err(e) => {
                Some(error(Value::Null, PARSE_ERROR, &format!("parse error: {e}")).to_string())
            }
            Ok(Value::Array(batch)) if !batch.is_empty() => {
                let out: Vec<Value> = batch
                    .iter()
                    .filter_map(|m| match self.handle(m) {
                        Reply::Response(v) => Some(v),
                        Reply::None => None,
                    })
                    .collect();
                (!out.is_empty()).then(|| Value::Array(out).to_string())
            }
            Ok(Value::Array(_)) => {
                Some(error(Value::Null, INVALID_REQUEST, "an empty batch").to_string())
            }
            Ok(v) => match self.handle(&v) {
                Reply::Response(r) => Some(r.to_string()),
                Reply::None => None,
            },
        }
    }

    fn dispatch(&self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .filter(|v| PROTOCOLS.contains(v))
                    .unwrap_or(PROTOCOLS[0]);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}, "resources": {}},
                    "serverInfo": {"name": "ecdev", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "tools/call" => self.call_tool(params),
            "resources/list" => Ok(json!({"resources": []})),
            "resources/templates/list" => Ok(json!({"resourceTemplates":[
                {"uriTemplate":"ecdev://run/{id}","name":"Persisted run","mimeType":"application/json"},
                {"uriTemplate":"ecdev://governance/donor/{id}","name":"Donor census","mimeType":"application/json"}
            ]})),
            "resources/read" => self.read_resource(params),
            other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
        }
    }

    fn call_tool(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "tools/call needs a tool name".to_string()))?;
        let args = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(v) if v.is_object() => v.clone(),
            Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".into())),
        };
        Ok(match self.engine.call(name, args) {
            Ok(v) => {
                let structured = if v.is_object() {
                    v
                } else {
                    json!({"items": v})
                };
                json!({"content":[{"type":"text","text":structured.to_string()}],"structuredContent":structured,"isError":false})
            }
            Err(e) => json!({"content":[{"type":"text","text":e}],"isError":true}),
        })
    }

    fn read_resource(&self, params: &Value) -> Result<Value, (i64, String)> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "resources/read needs a uri".to_string()))?;
        let found = if let Some(id) = uri.strip_prefix("ecdev://run/") {
            self.engine.run(id)
        } else if let Some(id) = uri.strip_prefix("ecdev://governance/donor/") {
            self.engine.census(id)
        } else {
            Err("Unknown resource".into())
        };
        let value = found.map_err(|e| (INVALID_PARAMS, e))?;
        Ok(json!({"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp() -> Mcp {
        let root = std::env::temp_dir().join(format!(
            "ecdev-mcp-unit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Mcp::new(Engine::open(&root).unwrap())
    }

    fn ask(m: &Mcp, text: &str) -> Value {
        json::from_str(&m.handle_text(text).expect("a reply")).unwrap()
    }

    #[test]
    fn envelope_errors_are_json_rpc_errors_with_the_right_ids() {
        let m = mcp();
        assert_eq!(ask(&m, "{nope")["error"]["code"], PARSE_ERROR);
        assert_eq!(ask(&m, "[]")["error"]["code"], INVALID_REQUEST);
        assert_eq!(ask(&m, "7")["error"]["code"], INVALID_REQUEST);
        let wrong = ask(&m, r#"{"jsonrpc":"1.0","id":5,"method":"ping"}"#);
        assert_eq!(
            (wrong["error"]["code"].clone(), wrong["id"].clone()),
            (json!(INVALID_REQUEST), json!(5))
        );
        let bad_id = ask(&m, r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#);
        assert_eq!(bad_id["error"]["code"], INVALID_REQUEST);
        assert!(bad_id["id"].is_null());
        assert_eq!(
            ask(&m, r#"{"jsonrpc":"2.0","id":1}"#)["error"]["code"],
            INVALID_REQUEST
        );
        assert_eq!(
            ask(&m, r#"{"jsonrpc":"2.0","id":"x","method":"nope"}"#)["error"]["code"],
            METHOD_NOT_FOUND
        );
    }

    #[test]
    fn notifications_and_client_responses_get_no_reply_and_batches_answer_requests_only() {
        let m = mcp();
        assert!(
            m.handle_text(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
        assert!(
            m.handle_text(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#)
                .is_none()
        );
        let batch = ask(
            &m,
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/x"},{"jsonrpc":"2.0","id":2,"method":"ping"}]"#,
        );
        let ids: Vec<Value> = batch
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].clone())
            .collect();
        assert_eq!(ids, vec![json!(1), json!(2)]);
        assert!(
            m.handle_text(r#"[{"jsonrpc":"2.0","method":"notifications/x"}]"#)
                .is_none()
        );
    }

    #[test]
    fn initialize_negotiates_a_supported_revision_and_offers_the_newest_otherwise() {
        let m = mcp();
        for v in PROTOCOLS {
            let r = ask(&m, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":v}}).to_string());
            assert_eq!(r["result"]["protocolVersion"], v);
        }
        let unknown = ask(
            &m,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        );
        assert_eq!(unknown["result"]["protocolVersion"], PROTOCOLS[0]);
        assert_eq!(unknown["result"]["serverInfo"]["name"], "ecdev");
        assert!(unknown["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_and_resources_follow_the_specification_shapes() {
        let m = mcp();
        let tools = ask(&m, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        let list = tools["result"]["tools"].as_array().unwrap();
        assert_eq!(list.len(), tool_definitions().len());
        assert!(
            list.iter()
                .all(|t| t["name"].is_string() && t["inputSchema"].is_object())
        );
        let ok = ask(
            &m,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ecdev.system.health"}}"#,
        );
        assert_eq!(ok["result"]["isError"], false, "{ok}");
        assert!(ok["result"]["structuredContent"].is_object());
        assert_eq!(ok["result"]["content"][0]["type"], "text");
        let failed = ask(
            &m,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ecdev.no.such.tool","arguments":{}}}"#,
        );
        assert_eq!(failed["result"]["isError"], true);
        assert_eq!(
            ask(
                &m,
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{}}"#
            )["error"]["code"],
            INVALID_PARAMS
        );
        assert_eq!(
            ask(
                &m,
                r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"x","arguments":[1]}}"#
            )["error"]["code"],
            INVALID_PARAMS
        );
        let templates = ask(
            &m,
            r#"{"jsonrpc":"2.0","id":6,"method":"resources/templates/list"}"#,
        );
        assert_eq!(
            templates["result"]["resourceTemplates"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            ask(
                &m,
                r#"{"jsonrpc":"2.0","id":7,"method":"resources/read","params":{"uri":"ecdev://nothing"}}"#
            )["error"]["code"],
            INVALID_PARAMS
        );
        assert_eq!(
            ask(
                &m,
                r#"{"jsonrpc":"2.0","id":8,"method":"resources/read","params":{}}"#
            )["error"]["code"],
            INVALID_PARAMS
        );
    }
}
