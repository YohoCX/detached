//! The host's MCP server: `report_finding` and `done`, each call appended to a JSON-lines file.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

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
            append(log, &msg["params"]);
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

fn append(log: &str, v: &Value) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    writeln!(f, "{v}").unwrap();
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

/// One HTTP request: the head, lowercased, and the body.
pub fn read_request(stream: &TcpStream) -> Option<(String, Vec<u8>)> {
    let mut r = BufReader::new(stream.try_clone().ok()?);
    let (mut head, mut len) = (String::new(), 0);
    while r.read_line(&mut head).is_ok_and(|n| n > 2) {
        if let Some(v) = head
            .lines()
            .last()?
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
        {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body).ok()?;
    Some((head.to_ascii_lowercase(), body))
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':').map(str::trim))
}

/// Serves MCP's streamable HTTP at `/mcp/<secret>` with plain JSON answers; returns its URL.
/// The first `initialize` binds the path to the `Mcp-Session-Id` it returns; any later request
/// without that id, a second `initialize` included, is refused and logged.
pub fn http(log: &str, secret: &str) -> String {
    let (listener, addr) = crate::listen("HOST_IP");
    let url = format!("http://{addr}/mcp/{secret}");
    let (log, path) = (log.to_string(), format!("/mcp/{secret}"));
    let bound: Arc<Mutex<Option<String>>> = Default::default();
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let Some((head, body)) = read_request(&stream) else {
                continue;
            };
            let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let method = msg["method"].as_str().unwrap_or("").to_string();
            let sid = header(&head, "mcp-session-id").map(String::from);
            let mut b = bound.lock().unwrap();
            let verdict = if !head.starts_with("post ") || !head.contains(&format!(" {path} ")) {
                "405"
            } else if method == "initialize" {
                if b.is_some() {
                    "refused: second initialize"
                } else {
                    "bound"
                }
            } else if sid.is_some() && sid == *b {
                "ok"
            } else {
                "refused: no or foreign session id"
            };
            append(
                &log,
                &json!({"t": crate::ms(), "http": method, "sid": sid,
                "host": header(&head, "host"), "origin": header(&head, "origin"),
                "agent": header(&head, "user-agent"), "verdict": verdict}),
            );
            let mut extra = String::new();
            if verdict == "bound" {
                let id = format!("s{}", crate::ms());
                extra = format!("Mcp-Session-Id: {id}\r\n");
                *b = Some(id);
            }
            drop(b);
            let reply = if verdict == "405" {
                "405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n".into()
            } else if verdict.starts_with("refused") {
                "404 Not Found\r\nContent-Length: 0\r\n\r\n".into()
            } else if let Some(v) = handle(&msg, &log) {
                format!(
                    "200 OK\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{v}",
                    v.to_string().len()
                )
            } else {
                "202 Accepted\r\nContent-Length: 0\r\n\r\n".to_string()
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
