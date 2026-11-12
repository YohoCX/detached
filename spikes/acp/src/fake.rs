//! A scripted Anthropic Messages API, so tool calls run on the real CLI without a model.
//! A request with tools gets step N of the script, N being its assistant messages so far; past the
//! end, or without tools, it gets the text "done".
use crate::mcp::read_request;
use serde_json::{Value, json};
use std::io::Write;

pub fn serve(script: Value, log: String) -> String {
    let (listener, addr) = crate::listen();
    let url = format!("http://{addr}");
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let (script, log) = (script.clone(), log.clone());
            std::thread::spawn(move || {
                let Some((head, body)) = read_request(&stream) else {
                    return;
                };
                let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let reply = answer(&path, &req, &script, &log);
                let ctype = if req["stream"] == true {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                let _ = write!(
                    &stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
            });
        }
    });
    url
}

fn answer(path: &str, req: &Value, script: &Value, log: &str) -> String {
    if !path.starts_with("/v1/messages") || path.contains("count_tokens") {
        note(log, json!({"path": path}));
        return json!({"input_tokens": 100}).to_string();
    }
    let msgs = req["messages"].as_array().cloned().unwrap_or_default();
    let tools = req["tools"].as_array().map_or(0, Vec::len);
    let step = msgs.iter().filter(|m| m["role"] == "assistant").count();
    let blocks = match script.get(step) {
        Some(Value::Array(b)) if tools > 0 => b.clone(),
        _ => vec![json!({"text": "done"})],
    };
    let results: Vec<String> = msgs
        .iter()
        .rev()
        .take_while(|m| m["role"] != "assistant")
        .filter_map(|m| m["content"].as_array())
        .flatten()
        .filter(|c| c["type"] == "tool_result")
        .map(|c| crate::cut(&c["content"], 300))
        .collect();
    note(
        log,
        json!({"path": path, "model": req["model"], "tools": tools, "step": step,
        "tool_results": results, "reply": blocks}),
    );
    let model = req["model"].clone();
    let tool_use = blocks.iter().any(|b| b.get("tool").is_some());
    let content: Vec<Value> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| match b.get("tool") {
            Some(name) => {
                json!({"type": "tool_use", "id": format!("toolu_{step}_{i}"), "name": name,
                "input": b["input"]})
            }
            None => json!({"type": "text", "text": b["text"]}),
        })
        .collect();
    let stop = if tool_use { "tool_use" } else { "end_turn" };
    let usage = json!({"input_tokens": 100, "output_tokens": 10, "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0});
    let msg = json!({"id": format!("msg_{step}"), "type": "message", "role": "assistant",
        "model": model, "content": content, "stop_reason": stop, "stop_sequence": null, "usage": usage});
    if req["stream"] != true {
        return msg.to_string();
    }
    let mut out = String::new();
    let mut ev = |e: Value| {
        out.push_str(&format!(
            "event: {}\ndata: {e}\n\n",
            e["type"].as_str().unwrap()
        ))
    };
    let mut start = msg.clone();
    start["content"] = json!([]);
    start["stop_reason"] = Value::Null;
    ev(json!({"type": "message_start", "message": start}));
    for (i, c) in content.iter().enumerate() {
        let (empty, delta) = if c["type"] == "tool_use" {
            let mut e = c.clone();
            e["input"] = json!({});
            (
                e,
                json!({"type": "input_json_delta", "partial_json": c["input"].to_string()}),
            )
        } else {
            (
                json!({"type": "text", "text": ""}),
                json!({"type": "text_delta", "text": c["text"]}),
            )
        };
        ev(json!({"type": "content_block_start", "index": i, "content_block": empty}));
        ev(json!({"type": "content_block_delta", "index": i, "delta": delta}));
        ev(json!({"type": "content_block_stop", "index": i}));
    }
    ev(
        json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": null},
        "usage": {"output_tokens": 10}}),
    );
    ev(json!({"type": "message_stop"}));
    out
}

fn note(log: &str, v: Value) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
    {
        let _ = writeln!(f, "{} {v}", crate::ms());
    }
}
