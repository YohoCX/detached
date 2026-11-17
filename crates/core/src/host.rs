//! The host's tools as an MCP server: `detached_read` and `detached_act`, a JSON-RPC message in and
//! its answer out, whatever carries them.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::backend::Tools;

pub const READ: &str = "detached_read";
pub const ACT: &str = "detached_act";

/// A tool call: a read's query, or an action.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Read(String),
    Act(Act),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", content = "args", rename_all = "snake_case")]
pub enum Act {
    ReportFinding(Finding),
    ReportVerdict(Verdict),
    ReportGroup(Group),
    ReportOutcome(Outcome),
    Done(Done),
    Proc(ProcAct),
    Send(SendAct),
    Watch(WatchAct),
}

impl Act {
    pub fn name(&self) -> &'static str {
        match self {
            Act::ReportFinding(_) => "report_finding",
            Act::ReportVerdict(_) => "report_verdict",
            Act::ReportGroup(_) => "report_group",
            Act::ReportOutcome(_) => "report_outcome",
            Act::Done(_) => "done",
            Act::Proc(_) => "proc",
            Act::Send(_) => "send",
            Act::Watch(_) => "watch",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub class: Class,
    pub title: String,
    pub body: String,
    pub fix: Option<String>,
    pub file: String,
    pub line: u32,
    pub end_line: Option<u32>,
    pub quote: String,
    pub lenses: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Major,
    Minor,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Correctness,
    Security,
    Performance,
    Design,
    Docs,
    Tests,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub finding: String,
    pub verdict: Judged,
    pub quote: String,
    pub body: Option<String>,
    /// The `ref` of the earlier finding its brief shows that this one is the same issue as.
    pub same_as: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Judged {
    Confirmed,
    Refined,
    Rejected,
    Immaterial,
    PreExisting,
}

/// Findings that are one problem, the best-described first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub ids: Vec<String>,
}

/// A triage's outcome: an argument for each of the six answers, and the one it recommends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub recommend: Answer,
    pub accept: String,
    pub decline: String,
    pub defer: String,
    pub ask_author: String,
    pub split: String,
    pub duplicate: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Accept,
    Decline,
    Defer,
    AskAuthor,
    Split,
    Duplicate,
}

impl Answer {
    pub fn text(self) -> &'static str {
        match self {
            Answer::Accept => "Accept",
            Answer::Decline => "Decline",
            Answer::Defer => "Defer",
            Answer::AskAuthor => "Ask the author",
            Answer::Split => "Split",
            Answer::Duplicate => "Duplicate",
        }
    }
}

impl Outcome {
    /// Each answer with its argument, in the order the maintainer reads them.
    pub fn answers(&self) -> [(Answer, &str); 6] {
        [
            (Answer::Accept, &self.accept),
            (Answer::Decline, &self.decline),
            (Answer::Defer, &self.defer),
            (Answer::AskAuthor, &self.ask_author),
            (Answer::Split, &self.split),
            (Answer::Duplicate, &self.duplicate),
        ]
    }

    pub fn check(&self) -> Result<(), String> {
        match self.answers().iter().find(|(_, a)| a.trim().is_empty()) {
            Some((a, _)) => Err(format!("{} has no argument: argue each answer.", a.text())),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Done {
    pub nothing_found: bool,
    pub note: Option<String>,
}

/// Starts, stops or restarts a defined process, where its definition lets agents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcAct {
    pub op: Op,
    pub repo: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Start,
    Stop,
    Restart,
}

/// A message to a session of the sender's group, or a reply to one it got.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendAct {
    pub to: String,
    pub text: String,
    pub reply_to: Option<String>,
}

/// One message to the watcher when a running process is ready, or when a pipeline of the repo's
/// branch ends: `pipeline`'s, else the newest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WatchAct {
    pub repo: String,
    pub process: Option<String>,
    pub pipeline: Option<String>,
}

/// The server: `answer` takes each tool call, and its error goes back to the agent as the call's.
pub fn server(answer: impl Fn(Call) -> Result<Value, String> + Send + Sync + 'static) -> Tools {
    Arc::new(move |m: Value| {
        let p = &m["params"];
        let result = match m["method"].as_str().unwrap_or_default() {
            "initialize" => json!({"protocolVersion": p["protocolVersion"],
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "detached-host", "version": env!("CARGO_PKG_VERSION")}}),
            "tools/list" => tools(),
            "tools/call" => {
                let (name, args) = (p["name"].as_str().unwrap_or_default(), &p["arguments"]);
                let call = match name {
                    READ => args["query"]
                        .as_str()
                        .map(|q| Call::Read(q.into()))
                        .ok_or_else(|| "detached_read takes a `query` string".to_string()),
                    ACT => serde_json::from_value(args.clone())
                        .map(Call::Act)
                        .map_err(|e| format!("detached_act: {e}")),
                    _ => Err(format!("No tool {name}")),
                };
                let (text, error) = match call.and_then(&answer) {
                    Ok(Value::String(s)) => (s, false),
                    Ok(v) => (v.to_string(), false),
                    Err(e) => (e, true),
                };
                json!({"content": [{"type": "text", "text": text}], "isError": error})
            }
            _ if m.get("id").is_none() => json!({}),
            "ping" => json!({}),
            method => {
                return json!({"jsonrpc": "2.0", "id": m["id"],
                    "error": {"code": -32601, "message": format!("No method {method}")}});
            }
        };
        let id = m.get("id").cloned().unwrap_or(json!(0));
        json!({"jsonrpc": "2.0", "id": id, "result": result})
    })
}

fn tools() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    let one_of = |v: &[&str]| json!({"type": "string", "enum": v});
    let object = |props: Value, required: &[&str]| json!({"type": "object", "properties": props, "required": required});
    let finding = object(
        json!({"severity": one_of(&["critical", "major", "minor"]),
            "class": one_of(&["correctness", "security", "performance", "design", "docs", "tests"]),
            "title": s("One line"), "body": s("Markdown"), "fix": s("How to fix it"),
            "file": s("Relative to the tree"), "line": {"type": "integer"},
            "end_line": {"type": "integer"}, "quote": s("The code as it is at file:line"),
            "lenses": {"type": "array", "items": {"type": "string"}}}),
        &[
            "severity", "class", "title", "body", "file", "line", "quote", "lenses",
        ],
    );
    let verdict = object(
        json!({"finding": s("The finding's id"),
            "verdict": one_of(&["confirmed", "refined", "rejected", "immaterial", "pre_existing"]),
            "quote": s("The code as it is"), "body": s("Why"),
            "same_as": s("When the brief shows earlier findings: the `ref` of the one this is the \
                same issue as; left out when it is none of them")}),
        &["finding", "verdict", "quote"],
    );
    let group = object(
        json!({"ids": {"type": "array", "items": {"type": "string"},
            "description": "The findings' ids, the best-described first"}}),
        &["ids"],
    );
    let answer = |d: &str| s(&format!("The case for {d}"));
    let outcome = object(
        json!({"recommend": one_of(&["accept", "decline", "defer", "ask_author", "split", "duplicate"]),
            "accept": answer("accepting it"), "decline": answer("declining it"),
            "defer": answer("deferring it"), "ask_author": answer("asking its author first"),
            "split": answer("splitting it"), "duplicate": answer("closing it as a duplicate, and of what")}),
        &[
            "recommend",
            "accept",
            "decline",
            "defer",
            "ask_author",
            "split",
            "duplicate",
        ],
    );
    let done = object(
        json!({"nothing_found": {"type": "boolean"}, "note": s("Your case, or anything to add")}),
        &["nothing_found"],
    );
    let proc = object(
        json!({"op": one_of(&["start", "stop", "restart"]), "repo": s("The repo's path"),
            "name": s("The process's name")}),
        &["op", "repo", "name"],
    );
    let send = object(
        json!({"to": s("A session's id from `group`"), "text": s("The message"),
            "reply_to": s("The id of the message this answers")}),
        &["to", "text"],
    );
    let watch = object(
        json!({"repo": s("The repo's path"), "process": s("A running process's name"),
            "pipeline": s("A pipeline's id; the branch's newest when neither is given")}),
        &["repo"],
    );
    let read = object(
        json!({"query": s("A GraphQL query, such as { schema }")}),
        &["query"],
    );
    let act = object(
        json!({"action": one_of(&["report_finding", "report_verdict", "report_group",
                "report_outcome", "done", "proc", "send", "watch"]),
            "args": {"anyOf": [finding, verdict, group, outcome, done, proc, send, watch],
                "description": "The action's arguments"}}),
        &["action", "args"],
    );
    json!({"tools": [
        {"name": READ, "inputSchema": read, "description": "Reads Detached's data: its repos, \
            their status, merge requests, skills, docs and defined processes with their logs, the \
            agent sessions, and this session's `inbox` and `group`. Ask a repo's `docs(query)` how its code works before reading the \
            source: it answers the doc sections whose index routes there. `{ schema }` returns \
            the schema. Queries only, without fragments, variables, directives or introspection; at \
            most 8 deep and 256 KB of result."},
        {"name": ACT, "inputSchema": act, "description": "Reports to Detached: a finding, a \
            verdict on a finding, a group of duplicate findings, a triage's outcome, or `done` at the end of the work, which every turn of a review \
            ends with; `proc` starts, stops or restarts a defined process where its definition \
            lets agents; `send` messages a session of this one's group; `watch` puts one message \
            into this session's inbox when a process is ready or a pipeline ends."},
    ]})
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    fn call(tools: &Tools, name: &str, arguments: Value) -> Value {
        let m = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}});
        tools(m)["result"].clone()
    }

    #[test]
    fn answers_the_handshake_lists_two_tools_and_dispatches_their_calls() {
        let got = Arc::new(Mutex::new(vec![]));
        let seen = got.clone();
        let tools = server(move |c| {
            seen.lock().unwrap().push(c.clone());
            match c {
                Call::Read(q) if q == "{ bad }" => Err("refused: bad".into()),
                Call::Read(_) => Ok(json!({"data": {"schema": "type Query"}})),
                Call::Act(_) => Ok(json!("recorded")),
            }
        });
        let init = tools(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}}));
        assert_eq!(init["id"], 0);
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        let note = tools(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        assert_eq!(note["result"], json!({}));
        let list = tools(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
        let names: Vec<&str> = (list["result"]["tools"].as_array().unwrap().iter())
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, [READ, ACT]);
        let read = call(&tools, READ, json!({"query": "{ schema }"}));
        assert_eq!(read["isError"], false);
        assert_eq!(
            read["content"][0]["text"],
            r#"{"data":{"schema":"type Query"}}"#
        );
        let refused = call(&tools, READ, json!({"query": "{ bad }"}));
        assert_eq!(refused["isError"], true);
        assert_eq!(refused["content"][0]["text"], "refused: bad");
        let done = json!({"action": "done", "args": {"nothing_found": true}});
        assert_eq!(call(&tools, ACT, done)["content"][0]["text"], "recorded");
        let bad = call(&tools, ACT, json!({"action": "done", "args": {}}));
        assert_eq!(bad["isError"], true);
        let said = bad["content"][0]["text"].as_str().unwrap();
        assert!(said.contains("nothing_found"), "{said}");
        let unknown = tools(json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"}));
        assert_eq!(unknown["error"]["code"], -32601);
        let finding = json!({"action": "report_finding", "args": {"severity": "major",
            "class": "correctness", "title": "t", "body": "b", "file": "a.rs", "line": 3,
            "quote": "q", "lenses": []}});
        call(&tools, ACT, finding);
        let got = got.lock().unwrap();
        assert_eq!(got[0], Call::Read("{ schema }".into()));
        assert_eq!(
            got[2],
            Call::Act(Act::Done(Done {
                nothing_found: true,
                note: None
            }))
        );
        let Call::Act(f) = &got[3] else {
            panic!("{got:?}")
        };
        assert_eq!(f.name(), "report_finding");
    }

    #[test]
    fn an_outcome_argues_all_six_answers() {
        let mut args = json!({"recommend": "split", "accept": "a", "decline": "d", "defer": "f",
            "ask_author": "q", "split": "s", "duplicate": "none"});
        let act = |args: &Value| {
            serde_json::from_value::<Act>(json!({"action": "report_outcome", "args": args}))
        };
        let Ok(Act::ReportOutcome(o)) = act(&args) else {
            panic!("{:?}", act(&args))
        };
        assert_eq!(o.check(), Ok(()));
        assert_eq!(o.answers()[4], (Answer::Split, "s"));
        let blank = Outcome {
            ask_author: " ".into(),
            ..o
        };
        assert!(blank.check().unwrap_err().starts_with("Ask the author"));
        args.as_object_mut().unwrap().remove("duplicate");
        assert!(act(&args).unwrap_err().to_string().contains("duplicate"));
    }
}
