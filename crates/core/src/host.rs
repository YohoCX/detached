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
    Done(Done),
}

impl Act {
    pub fn name(&self) -> &'static str {
        match self {
            Act::ReportFinding(_) => "report_finding",
            Act::ReportVerdict(_) => "report_verdict",
            Act::ReportGroup(_) => "report_group",
            Act::Done(_) => "done",
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Done {
    pub nothing_found: bool,
    pub note: Option<String>,
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
            "quote": s("The code as it is"), "body": s("Why")}),
        &["finding", "verdict", "quote"],
    );
    let group = object(
        json!({"ids": {"type": "array", "items": {"type": "string"},
            "description": "The findings' ids, the best-described first"}}),
        &["ids"],
    );
    let done = object(
        json!({"nothing_found": {"type": "boolean"}, "note": s("Anything to add")}),
        &["nothing_found"],
    );
    let read = object(
        json!({"query": s("A GraphQL query, such as { schema }")}),
        &["query"],
    );
    let act = object(
        json!({"action": one_of(&["report_finding", "report_verdict", "report_group", "done"]),
            "args": {"anyOf": [finding, verdict, group, done],
                "description": "The action's arguments"}}),
        &["action", "args"],
    );
    json!({"tools": [
        {"name": READ, "inputSchema": read, "description": "Reads Detached's data: its repos, \
            their status, merge requests, skills and docs, and the agent sessions. Ask a repo's \
            `docs(query)` how its code works before reading the source: it answers the doc \
            sections whose index routes there. `{ schema }` returns the schema. Queries only, without fragments, variables, directives or introspection; at \
            most 8 deep and 256 KB of result."},
        {"name": ACT, "inputSchema": act, "description": "Reports to Detached: a finding, a \
            verdict on a finding, a group of duplicate findings, or `done` at the end of the work, which every turn of a review \
            ends with."},
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
}
