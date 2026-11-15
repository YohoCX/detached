//! A scripted Anthropic Messages API, so the real `claude` runs tool calls with its own permissions
//! and sandbox, and no model. A request with tools gets step N of the script, N being the assistant
//! messages it carries; past the end, or without tools, it gets the text "done".

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{Value, json};

pub struct Api {
    pub url: String,
    seen: Arc<Mutex<Vec<Value>>>,
}

impl Api {
    /// `script` holds one array of blocks per step: `{"text": ..}` or `{"tool": .., "input": ..}`.
    pub fn serve(script: Value) -> Api {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(vec![]));
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let (script, log) = (script.clone(), log.clone());
                thread::spawn(move || {
                    let Some((path, req)) = read(&stream) else {
                        return;
                    };
                    let reply = if !path.starts_with("/v1/messages") || path.contains("count") {
                        json!({"input_tokens": 100}).to_string()
                    } else {
                        let reply = answer(&req, &script);
                        log.lock().unwrap().push(req.clone());
                        reply
                    };
                    let ctype = match req["stream"] == true {
                        true => "text/event-stream",
                        false => "application/json",
                    };
                    let _ = write!(
                        &stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{reply}",
                        reply.len()
                    );
                });
            }
        });
        Api { url, seen }
    }

    /// The Messages requests so far, whole.
    pub fn requests(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }

    /// The text of the tool results in the longest conversation sent so far.
    pub fn results(&self) -> Vec<String> {
        let requests = self.requests();
        let messages = |r: &Value| r["messages"].as_array().cloned().unwrap_or_default();
        let longest = requests.iter().map(messages).max_by_key(Vec::len);
        let blocks = longest.into_iter().flatten();
        let blocks = blocks
            .filter_map(|m| m["content"].as_array().cloned())
            .flatten();
        let text = |b: Value| match &b["content"] {
            Value::String(s) => s.clone(),
            v => (v.as_array().into_iter().flatten())
                .filter_map(|c| c["text"].as_str())
                .collect(),
        };
        blocks
            .filter(|b| b["type"] == "tool_result")
            .map(text)
            .collect()
    }
}

/// The request's path and JSON body.
fn read(stream: &TcpStream) -> Option<(String, Value)> {
    let mut r = BufReader::new(stream.try_clone().ok()?);
    let (mut head, mut len) = (String::new(), 0);
    while r.read_line(&mut head).is_ok_and(|n| n > 2) {
        let line = head.lines().last()?.to_ascii_lowercase();
        if let Some(v) = line.strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body).ok()?;
    let path = head.split_whitespace().nth(1)?.to_string();
    Some((path, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

fn answer(req: &Value, script: &Value) -> String {
    let messages = req["messages"].as_array().map_or(&[][..], Vec::as_slice);
    let step = messages.iter().filter(|m| m["role"] == "assistant").count();
    let tools = req["tools"].as_array().is_some_and(|t| !t.is_empty());
    let blocks = match script.get(step) {
        Some(Value::Array(b)) if tools => b.clone(),
        _ => vec![json!({"text": "done"})],
    };
    let content: Vec<Value> = (blocks.iter().enumerate())
        .map(|(i, b)| match b.get("tool") {
            Some(name) => json!({"type": "tool_use", "id": format!("toolu_{step}_{i}"),
                "name": name, "input": b["input"]}),
            None => json!({"type": "text", "text": b["text"]}),
        })
        .collect();
    let stop = match content.iter().any(|c| c["type"] == "tool_use") {
        true => "tool_use",
        false => "end_turn",
    };
    let usage = json!({"input_tokens": 100, "output_tokens": 10, "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0});
    let msg = json!({"id": format!("msg_{step}"), "type": "message", "role": "assistant",
        "model": req["model"], "content": content, "stop_reason": stop, "stop_sequence": null,
        "usage": usage});
    if req["stream"] != true {
        return msg.to_string();
    }
    let mut out = String::new();
    let mut event = |e: Value| {
        out.push_str(&format!(
            "event: {}\ndata: {e}\n\n",
            e["type"].as_str().unwrap()
        ))
    };
    let mut start = msg.clone();
    (start["content"], start["stop_reason"]) = (json!([]), Value::Null);
    event(json!({"type": "message_start", "message": start}));
    for (i, c) in content.iter().enumerate() {
        let (empty, delta) = match c["type"] == "tool_use" {
            true => {
                let mut empty = c.clone();
                empty["input"] = json!({});
                let partial = c["input"].to_string();
                (
                    empty,
                    json!({"type": "input_json_delta", "partial_json": partial}),
                )
            }
            false => (
                json!({"type": "text", "text": ""}),
                json!({"type": "text_delta", "text": c["text"]}),
            ),
        };
        event(json!({"type": "content_block_start", "index": i, "content_block": empty}));
        event(json!({"type": "content_block_delta", "index": i, "delta": delta}));
        event(json!({"type": "content_block_stop", "index": i}));
    }
    event(
        json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": null},
        "usage": {"output_tokens": 10}}),
    );
    event(json!({"type": "message_stop"}));
    out
}
