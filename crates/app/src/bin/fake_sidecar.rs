//! Stands in for the Claude sidecar in the UI tests: a script per message, no model.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

fn emit(v: Value) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{v}").unwrap();
    out.flush().unwrap();
}

fn note(text: String) {
    emit(json!({"type": "error", "message": text}));
}

fn turn_end(ok: bool) {
    emit(json!({"type": "turn_end", "ok": ok, "ms": 12400}));
    let parts = [{ json!({"name": "Messages", "tokens": 200}) }];
    emit(json!({"type": "usage", "used": 250, "max": 1000, "parts": parts}));
}

fn main() {
    let mut cwd = String::new();
    let mut asking = None;
    let mut project = false;
    let models = json!([
        {"value": "good", "label": "Good", "auto": true, "efforts": ["low", "high"]},
        {"value": "old", "label": "Old", "auto": false},
    ]);
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let c: Value = serde_json::from_str(&line).unwrap();
        if let Some(dir) = c["cwd"].as_str() {
            cwd = dir.to_string();
        }
        match c["type"].as_str().unwrap() {
            "start" => {
                let id = c["resume"].as_str().unwrap_or("fresh");
                if !c["resume"].is_null() {
                    note(format!("resumed {id}"));
                }
                project = c["project"].as_bool().unwrap();
                if let Some(e) = c["effort"].as_str() {
                    note(format!("started at {e}"));
                }
                let mode = c["mode"].as_str().unwrap();
                emit(json!({"type": "ready", "session_id": id, "model": "good", "mode": mode}));
                emit(json!({"type": "models", "models": models}));
                let commands = [json!({"name": "review", "description": "Review", "hint": "<pr>"})];
                emit(json!({"type": "commands", "commands": commands}));
            }
            "send" => match c["text"].as_str().unwrap() {
                "tool" => {
                    let start = |id, cmd| {
                        emit(json!({"type": "tool", "id": id, "name": "Bash", "detail": cmd}))
                    };
                    start("a", "echo a");
                    emit(json!({"type": "tool_done", "id": "a", "ok": true, "output": "a"}));
                    start("b", "false");
                    emit(json!({"type": "tool_done", "id": "b", "ok": false}));
                    let edit = json!({"type": "tool", "id": "c", "name": "Edit", "detail": "README.md", "lines": [3, 1]});
                    emit(edit);
                    emit(json!({"type": "tool_done", "id": "c", "ok": true}));
                    turn_end(true);
                }
                "deny" => {
                    emit(json!({"type": "denied", "tool": "Bash", "reason": "needs approval"}));
                    turn_end(true);
                }
                "ask" => {
                    asking = Some("p1");
                    emit(
                        json!({"type": "ask", "id": "p1", "tool": "Bash", "detail": "rm -rf build", "always": true}),
                    );
                }
                "question" => {
                    asking = Some("q1");
                    let options = [
                        json!({"label": "Red", "description": "warm"}),
                        json!({"label": "Blue", "description": "cold"}),
                    ];
                    let q = json!({"question": "Which?", "header": "Color", "multi": false, "options": options});
                    emit(
                        json!({"type": "ask", "id": "q1", "tool": "AskUserQuestion", "detail": "", "questions": [q]}),
                    );
                }
                "background" => {
                    let tasks = [json!({"id": "t1", "description": "npm run dev"})];
                    emit(json!({"type": "tasks", "tasks": tasks}));
                    turn_end(true);
                }
                "edit" => {
                    let dir = std::path::Path::new(&cwd);
                    std::fs::write(dir.join("README.md"), "agent\n").unwrap();
                    std::fs::write(dir.join("made-by-agent.txt"), "x\n").unwrap();
                    turn_end(true);
                }
                "edit both" => {
                    let dir = std::path::Path::new(&cwd);
                    std::fs::write(dir.join("api/README.md"), "agent\n").unwrap();
                    std::fs::write(dir.join("web/by-agent.txt"), "x\n").unwrap();
                    turn_end(true);
                }
                "settings" => {
                    let on = if project { "on" } else { "off" };
                    emit(json!({"type": "text", "text": format!("project settings {on}")}));
                    turn_end(true);
                }
                "hang" => emit(json!({"type": "delta", "text": "Working"})),
                "crash" => {
                    eprintln!("boom");
                    std::process::exit(3);
                }
                text => {
                    let (a, b) = text.split_at(text.len() / 2);
                    emit(json!({"type": "delta", "text": a}));
                    emit(json!({"type": "delta", "text": b}));
                    emit(json!({"type": "text", "text": format!("said {text}")}));
                    turn_end(true);
                }
            },
            "answer" => {
                asking = None;
                let text = match (&c["answers"], c["allow"].as_bool().unwrap()) {
                    (Value::Object(a), _) => format!("answered {}", Value::Object(a.clone())),
                    (_, true) => format!("allowed always={}", c["always"]),
                    _ => "denied".into(),
                };
                emit(json!({"type": "text", "text": text}));
                turn_end(true);
            }
            "stop" => {
                if let Some(id) = asking.take() {
                    emit(json!({"type": "ask_done", "id": id}));
                }
                turn_end(false);
            }
            "stop_task" => emit(json!({"type": "tasks", "tasks": []})),
            "set_mode" => emit(json!({"type": "mode", "mode": c["mode"]})),
            "set_effort" => {
                emit(json!({"type": "effort", "effort": c["effort"]}));
                note(format!("effort {}", c["effort"].as_str().unwrap()));
            }
            "set_model" => {
                let model = c["model"].as_str().unwrap();
                let mode = if model == "old" { "default" } else { "auto" };
                emit(json!({"type": "mode", "mode": mode}));
            }
            "delete" | "archive" => {
                let file = std::env::current_exe()
                    .unwrap()
                    .with_file_name("sessions.json");
                let mut past: Vec<Value> =
                    serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
                past.retain(|p| p["id"] != c["id"] || c["type"] == "archive");
                for p in past.iter_mut().filter(|p| p["id"] == c["id"]) {
                    p["tag"] = if c["archived"] == true {
                        json!("archived")
                    } else {
                        Value::Null
                    };
                }
                // Renamed into place, so a list run beside this one never reads it half written.
                let tmp = file.with_extension(format!("{}", std::process::id()));
                std::fs::write(&tmp, Value::from(past).to_string()).unwrap();
                std::fs::rename(tmp, file).unwrap();
            }
            "sessions" if c["cwd"] == "" => {
                // A test lists every folder's past conversations in a file beside the sidecar.
                let file = std::env::current_exe()
                    .unwrap()
                    .with_file_name("sessions.json");
                let past: Value = std::fs::read_to_string(file)
                    .map_or(json!([]), |s| serde_json::from_str(&s).unwrap());
                emit(json!({"type": "sessions", "sessions": past}));
            }
            "sessions" => {
                let past = [json!({"id": "old1", "title": "Fix the build", "modified": 1})];
                emit(json!({"type": "sessions", "sessions": past}));
            }
            "info" => {
                let hours = |h: u64| {
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
                    now.unwrap().as_millis() as u64 + h * 3_600_000 + 60_000
                };
                let limits = [
                    json!({"name": "5h", "used": 34, "resets": hours(2)}),
                    json!({"name": "Week", "used": 17, "resets": hours(72)}),
                ];
                emit(json!({"type": "info", "provider": "Claude Max", "limits": limits}));
            }
            "load" => {
                let events = [
                    json!({"type": "user", "text": "fix it"}),
                    json!({"type": "text", "text": "fixed"}),
                ];
                emit(json!({"type": "history", "id": c["id"], "events": events}));
            }
            other => panic!("unknown command {other}"),
        }
    }
    if !cwd.is_empty() {
        std::fs::write(std::path::Path::new(&cwd).join("sidecar-eof"), "").unwrap();
    }
}
