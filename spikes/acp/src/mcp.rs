//! The host's MCP server: `report_finding` and `done`, each call appended to a JSON-lines file.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};

fn tools() -> Value {
    json!([
        {"name": "report_finding", "description": "Report one finding to the host.",
         "inputSchema": {"type": "object", "required": ["message"], "properties": {
            "file": {"type": "string"}, "line": {"type": "integer"},
            "severity": {"type": "string", "enum": ["low", "medium", "high"]},
            "message": {"type": "string"}}}},
        {"name": "done", "description": "Call once when the review is over.",
         "inputSchema": {"type": "object", "required": ["nothing_found"],
            "properties": {"nothing_found": {"type": "boolean"}}}}
    ])
}

/// The response to one JSON-RPC message, or None for a notification.
fn handle(msg: &Value, log: &str) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str().unwrap_or("") {
        "initialize" => json!({"protocolVersion": msg["params"]["protocolVersion"],
            "capabilities": {"tools": {}}, "serverInfo": {"name": "host", "version": "0"}}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log)
                .unwrap();
            writeln!(f, "{}", msg["params"]).unwrap();
            json!({"content": [{"type": "text", "text": "recorded"}]})
        }
        "ping" => json!({}),
        m => {
            return Some(
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": m}}),
            );
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

pub fn stdio(log: &str) {
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if let Some(r) = serde_json::from_str(&line)
            .ok()
            .and_then(|m| handle(&m, log))
        {
            println!("{r}");
        }
    }
}

/// Serves MCP's streamable HTTP on a free port with plain JSON answers; returns its URL.
pub fn http(log: &str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let log = log.to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let mut r = BufReader::new(stream.try_clone().unwrap());
            let (mut head, mut len) = (String::new(), 0);
            while r.read_line(&mut head).is_ok_and(|n| n > 2) {
                if let Some(v) = head.lines().last().and_then(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().to_string())
                }) {
                    len = v.parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            let _ = r.read_exact(&mut body);
            let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let reply = match (head.starts_with("POST"), handle(&msg, &log)) {
                (true, Some(v)) => format!(
                    "200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{v}",
                    v.to_string().len()
                ),
                (true, None) => "202 Accepted\r\nContent-Length: 0\r\n\r\n".into(),
                _ => "405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n".into(),
            };
            let _ = write!(
                &stream,
                "HTTP/1.1 {}",
                reply.replacen("\r\n", "\r\nConnection: close\r\n", 1)
            );
        }
    });
    url
}
