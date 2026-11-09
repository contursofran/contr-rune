//! The MCP protocol on stdio: JSON-RPC 2.0, one message per line. It serves tools only:
//! `initialize`, `tools/list`, `tools/call` and `ping`. A message without an id is a
//! notification and gets no reply (Go: internal/mcp/server.go). Calls run one after another.

use std::io::{BufRead, Write};

use serde_json::{Map, Value, json};

/// The newest protocol version this server speaks. A client that asks for another known version
/// gets that one; any other client gets this one.
const PROTOCOL_VERSION: &str = "2025-06-18";
const KNOWN_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", PROTOCOL_VERSION];

/// One tool as `tools/list` shows it.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    /// The JSON Schema of the arguments.
    pub schema: Value,
}

/// Runs a tool by name with its arguments and returns the text of its result.
pub type Run<'a> = dyn FnMut(&str, Value) -> Result<String, String> + 'a;

/// Reads requests from `input` and writes replies to `out` until `input` ends. An `Err` from
/// `run` becomes a result with `isError` set, as `<tool>: <error>`, so the model sees it.
pub fn serve(input: impl BufRead, out: &mut impl Write, name: &str, version: &str, tools: &[ToolDef], run: &mut Run) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = respond(&line, name, version, tools, run) {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

fn respond(line: &str, name: &str, version: &str, tools: &[ToolDef], run: &mut Run) -> Option<Value> {
    let req = match serde_json::from_str::<Value>(line) {
        Ok(Value::Object(o)) => o,
        Ok(_) => return Some(failure(Value::Null, -32700, "parse error: a request is a JSON object")),
        Err(e) => return Some(failure(Value::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = match req.get("id") {
        None | Some(Value::Null) => return None,
        Some(id) => id.clone(),
    };
    let method = req.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
            let agreed = if KNOWN_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSION };
            success(id, json!({"protocolVersion": agreed, "capabilities": {"tools": {}}, "serverInfo": {"name": name, "version": version}}))
        }
        "ping" => success(id, json!({})),
        "tools/list" => {
            let list: Vec<Value> = tools.iter().map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.schema})).collect();
            success(id, json!({"tools": list}))
        }
        "tools/call" => call(id, &params, tools, run),
        other => failure(id, -32601, &format!("method not found: {other}")),
    })
}

fn call(id: Value, params: &Value, tools: &[ToolDef], run: &mut Run) -> Value {
    let Some(params) = params.as_object() else {
        return failure(id, -32602, "bad params: params must be an object");
    };
    let tool = match params.get("name") {
        None => "",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return failure(id, -32602, "bad params: name must be a string"),
    };
    if !tools.iter().any(|t| t.name == tool) {
        return failure(id, -32602, &format!("unknown tool: {tool}"));
    }
    let args = match params.get("arguments") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(a) => a.clone(),
    };
    match run(tool, args) {
        Ok(text) => success(id, tool_result(&text, false)),
        Err(e) => success(id, tool_result(&format!("{tool}: {e}"), true)),
    }
}

fn tool_result(text: &str, is_error: bool) -> Value {
    let mut r = json!({"content": [{"type": "text", "text": text}]});
    if is_error {
        r["isError"] = json!(true);
    }
    r
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> Vec<ToolDef> {
        vec![ToolDef { name: "echo", description: "Echo.", schema: json!({"type": "object"}) }]
    }

    /// Feeds `lines` to a server whose only tool `echo` returns its arguments (or fails on `{"fail":true}`).
    fn exchange(lines: &[&str]) -> Vec<Value> {
        let mut out = Vec::new();
        let mut run = |_: &str, args: Value| if args["fail"] == json!(true) { Err("boom".to_owned()) } else { Ok(args.to_string()) };
        serve(lines.join("\n").as_bytes(), &mut out, "contr", "9.9", &defs(), &mut run).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn requests_map_to_replies() {
        let info = json!({"name": "contr", "version": "9.9"});
        let cases = [
            (
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
                json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":info}}),
            ),
            (
                r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999"}}"#,
                json!({"jsonrpc":"2.0","id":"a","result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":info}}),
            ),
            (r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#, json!({"jsonrpc":"2.0","id":2,"result":{}})),
            (
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
                json!({"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"echo","description":"Echo.","inputSchema":{"type":"object"}}]}}),
            ),
            (
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"echo","arguments":{"a":1}}}"#,
                json!({"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"{\"a\":1}"}]}}),
            ),
            (
                r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"echo","arguments":{"fail":true}}}"#,
                json!({"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"text","text":"echo: boom"}],"isError":true}}),
            ),
            (
                r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nope"}}"#,
                json!({"jsonrpc":"2.0","id":6,"error":{"code":-32602,"message":"unknown tool: nope"}}),
            ),
            (r#"{"jsonrpc":"2.0","id":7,"method":"resources/list"}"#, json!({"jsonrpc":"2.0","id":7,"error":{"code":-32601,"message":"method not found: resources/list"}})),
        ];
        for (request, reply) in cases {
            assert_eq!(exchange(&[request]), [reply], "{request}");
        }
    }

    #[test]
    fn a_call_without_arguments_runs_with_an_empty_object() {
        let r = exchange(&[r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo"}}"#]);
        assert_eq!(r[0]["result"]["content"][0]["text"], "{}");
    }

    #[test]
    fn notifications_and_blank_lines_get_no_reply_and_garbage_gets_a_parse_error() {
        let r = exchange(&[r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "", r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#, "not json", r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#]);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0]["id"].clone(), r[0]["error"]["code"].clone()), (Value::Null, json!(-32700)));
        assert_eq!(r[1]["id"], 9);
    }
}
