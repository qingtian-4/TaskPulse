//! Source-bound MCP stdio bridge. Reports are producer evidence, never host control.
use super::{bridge, schema};
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const VERSION: &str = "2025-11-25";
const TOOL: &str = "taskpulse_report";
const KINDS: &[&str] = &[
    "turn.started",
    "phase.changed",
    "tool.started",
    "tool.finished",
    "input.requested",
    "input.resolved",
    "turn.completed",
    "turn.failed",
    "turn.interrupted",
    "session.renamed",
    "heartbeat",
    "connection.test",
];
const INSTRUCTIONS: &str = "Report only your own task's actual lifecycle to TaskPulse. Keep session_id stable across turns; use a new turn_id per execution and event_id per fact (reuse it on retries). Only report input.requested for an actual question or approval, and resolve its matching request_id. Never infer completion from silence or a tool return. This is self-reported monitoring, not an approval mechanism. Do not include prompts, credentials, tool arguments or output. Disconnecting does not complete tasks.";

pub fn source_binding(source: &str) -> Result<String, String> {
    let source = bridge::configured_source(source)?;
    let binding = source.integration.ok_or("integration_not_configured")?;
    if !source.enabled {
        return Err("source_disabled".into());
    }
    if binding.adapter_id != "standard-events" || binding.transport != "local" {
        return Err("mcp_requires_local_standard_events_source".into());
    }
    Ok(binding.agent_id)
}

fn tool() -> Value {
    let identifier = json!({"type":"string","minLength":1,"maxLength":512});
    let display = json!({"type":"string","maxLength":2048});
    json!({
        "name":TOOL,"description":INSTRUCTIONS,
        "annotations":{"readOnlyHint":false,"destructiveHint":false,"idempotentHint":false,"openWorldHint":false},
        "inputSchema":{"type":"object","additionalProperties":false,
            "required":["session_id","event_id","kind"],
            "properties":{
                "session_id":identifier,"turn_id":identifier,"event_id":identifier,
                "request_id":identifier,"call_id":identifier,
                "kind":{"type":"string","enum":KINDS},
                "payload":{"type":"object","additionalProperties":false,"properties":{
                    "title":display,"project":display,"project_path":display,"model":display,"activity":display,
                    "phase":{"type":"string","enum":["thinking","tool","responding","awaiting_model","awaiting_tool"]},
                    "request_kind":{"type":"string","enum":["approval","question"]}
                }}
            }
        }
    })
}

fn event(arguments: &Value, agent: &str) -> Result<schema::Event, String> {
    let mut input = arguments
        .as_object()
        .cloned()
        .ok_or("arguments_object_required")?;
    // Binding is owned by the configured process, never supplied by the model.
    if input.contains_key("agent_id") || input.contains_key("schema_version") {
        return Err("source_binding_is_server_owned".into());
    }
    if input.keys().any(|key| {
        ![
            "session_id",
            "turn_id",
            "event_id",
            "kind",
            "request_id",
            "call_id",
            "payload",
        ]
        .contains(&key.as_str())
    }) {
        return Err("unsupported_report_field".into());
    }
    input.insert("schema_version".into(), json!(1));
    input.insert("agent_id".into(), json!(agent));
    let event: schema::Event =
        serde_json::from_value(Value::Object(input)).map_err(|_| "event_schema_error")?;
    let kind = serde_json::to_value(event.kind).map_err(|_| "event_schema_error")?;
    if !kind.as_str().is_some_and(|kind| KINDS.contains(&kind)) {
        return Err("unsupported_report_kind".into());
    }
    event.validate(agent)?;
    Ok(event)
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

#[derive(Default)]
struct Server {
    initialized: bool,
    ready: bool,
}
impl Server {
    fn handle(
        &mut self,
        request: Value,
        agent: &str,
        receive: &mut impl FnMut(schema::Event) -> Result<(), String>,
    ) -> Option<Value> {
        let id = request.get("id").cloned();
        if !request.is_object()
            || request["jsonrpc"] != "2.0"
            || !request["method"].is_string()
            || id
                .as_ref()
                .is_some_and(|id| !id.is_string() && !id.is_i64() && !id.is_u64())
        {
            return Some(error(Value::Null, -32600, "invalid_request"));
        }
        let method = request["method"].as_str().unwrap_or_default();
        let Some(id) = id else {
            // Notifications cannot execute tools, write events or bypass initialization.
            if method == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return None;
        };
        let result = match method {
            "initialize" if !self.initialized => {
                let params = &request["params"];
                if !params["protocolVersion"].is_string()
                    || !params["capabilities"].is_object()
                    || !params["clientInfo"]["name"].is_string()
                    || !params["clientInfo"]["version"].is_string()
                {
                    return Some(error(id, -32602, "invalid_initialize_params"));
                }
                let version = match params["protocolVersion"].as_str() {
                    Some("2025-06-18") => "2025-06-18",
                    _ => VERSION,
                };
                self.initialized = true;
                json!({"protocolVersion":version,"capabilities":{"tools":{}},
                    "serverInfo":{"name":"taskpulse","version":env!("CARGO_PKG_VERSION")},"instructions":INSTRUCTIONS})
            }
            "ping" => json!({}),
            // Newer clients may probe discovery before falling back to a handshake.
            "server/discover" => return Some(error(id, -32601, "method_not_found")),
            _ if !self.ready => return Some(error(id, -32000, "initialize_required")),
            "tools/list" => json!({"tools":[tool()]}),
            "tools/call" => {
                if request["params"]["name"] != TOOL {
                    return Some(error(id, -32602, "unknown_tool"));
                }
                let result = event(&request["params"]["arguments"], agent).and_then(receive);
                match result {
                    Ok(()) => {
                        json!({"content":[{"type":"text","text":"Event accepted by TaskPulse (self-reported). This does not authorize a command or confirm user review."}],"isError":false})
                    }
                    Err(code) => json!({"content":[{"type":"text","text":code}],"isError":true}),
                }
            }
            _ => return Some(error(id, -32601, "method_not_found")),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}

fn serve(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    agent: &str,
    mut receive: impl FnMut(schema::Event) -> Result<(), String>,
) -> std::io::Result<()> {
    let mut server = Server::default();
    loop {
        let mut bytes = Vec::new();
        let read = std::io::Read::take(&mut *reader, (schema::MAX_INPUT + 1) as u64)
            .read_until(b'\n', &mut bytes)?;
        if read == 0 {
            return Ok(());
        }
        let oversized = bytes.len() > schema::MAX_INPUT;
        let response = if oversized || bytes.last() != Some(&b'\n') {
            Some(error(Value::Null, -32600, "frame_too_large_or_incomplete"))
        } else {
            match serde_json::from_slice(&bytes) {
                Ok(request) => server.handle(request, agent, &mut receive),
                Err(_) => Some(error(Value::Null, -32700, "invalid_json")),
            }
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut *writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
        if oversized || bytes.last() != Some(&b'\n') {
            return Ok(());
        }
    }
}

pub fn cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|arg| arg == "--integration-mcp") {
        return None;
    }
    if args.len() != 4 || args[1] != "--integration-mcp" || args[2] != "--source" {
        eprintln!("Usage: agent-tray --integration-mcp --source <saved-source-id>");
        return Some(2);
    }
    let source = &args[3];
    let agent = match source_binding(source) {
        Ok(agent) => agent,
        Err(code) => {
            eprintln!("{code}");
            return Some(1);
        }
    };
    let result = serve(
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &agent,
        |event| {
            let bytes = serde_json::to_vec(&event).map_err(|_| "event_serialize_failed")?;
            // Re-read scope/enabled/binding on every call, even in a long-lived server.
            bridge::receive(source, "standard-events", false, &bytes)
        },
    );
    Some(if result.is_ok() { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_never_writes_from_notifications_or_before_handshake() {
        let input = [
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":TOOL,"arguments":{}}}),
            json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":TOOL,"arguments":{}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"future","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":TOOL,"arguments":{"session_id":"s","event_id":"e","turn_id":"t","kind":"turn.started"}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":TOOL,"arguments":{"session_id":"s","event_id":"request","turn_id":"t","kind":"input.requested"}}}),
        ].iter().map(|r| format!("{r}\n")).collect::<String>();
        let mut output = Vec::new();
        let mut accepted = Vec::new();
        serve(
            &mut std::io::Cursor::new(input),
            &mut output,
            "custom-agent",
            |event| {
                accepted.push(event);
                Ok(())
            },
        )
        .unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 4);
        assert_eq!(replies[0]["error"]["message"], "initialize_required");
        assert_eq!(replies[1]["result"]["protocolVersion"], VERSION);
        assert_eq!(replies[2]["result"]["isError"], false);
        assert_eq!(replies[3]["result"]["isError"], true);
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].kind, schema::Kind::Started);
        assert_eq!(accepted[0].agent_id, "custom-agent");
    }
    #[test]
    fn oversized_truncated_invalid_and_spoofed_input_never_becomes_an_event() {
        for bytes in [
            vec![b'x'; schema::MAX_INPUT + 200],
            b"{\"jsonrpc\":\"2.0\"}".to_vec(),
            b"\xff\n".to_vec(),
        ] {
            let mut output = Vec::new();
            serve(
                &mut std::io::Cursor::new(bytes),
                &mut output,
                "custom-agent",
                |_| panic!("must not receive"),
            )
            .unwrap();
            let reply: Value = serde_json::from_slice(&output).unwrap();
            assert!(reply["error"].is_object());
            assert!(output.len() < 200);
        }
        for extra in [
            json!({"agent_id":"another"}),
            json!({"source_id":"another"}),
            json!({"payload":{"prompt":"private"}}),
        ] {
            let mut input =
                json!({"session_id":"s","event_id":"e","turn_id":"t","kind":"turn.started"});
            input
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(event(&input, "custom-agent").is_err());
        }
    }
}
