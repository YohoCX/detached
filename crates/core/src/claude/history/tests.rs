use std::fs;
use std::path::Path;

use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::backend::Tool;

const SUBAGENT: &str = "36022712-0b82-4d34-ae53-41a19e20f5e9";
const COMPACTION: &str = "479bb84f-197d-4dc6-a6f8-6777d8a2bb05";
const TITLED: &str = "f5dfa2ed-e3fa-45bd-a26e-8ca5fb27f700";
const WORKTREE: &str = "38d60b10-15d8-4def-a954-2ffb285421d1";
const LONG: &str = "8d01a7e2-5bd9-4eb4-af6d-a0c4614f6a93";

/// The CLI's config dir with the captured transcripts of `case` in `cwd`'s project folder.
fn laid(case: &str, cwd: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude")
        .join(case);
    copy(&from, &dir.path().join("projects").join(folder(cwd)));
    dir
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        if e.path().is_dir() {
            copy(&e.path(), &to.join(e.file_name()));
        } else {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

fn texts(h: &[Update]) -> Vec<String> {
    let text = |u: &Update| match u {
        Update::User(t) => Some(format!("user: {t}")),
        Update::Text(t) => Some(format!("text: {t}")),
        _ => None,
    };
    h.iter().filter_map(text).collect()
}

#[test]
fn a_long_cwd_s_folder_is_cut_at_200_with_its_hash_and_lists_its_sessions() {
    assert_eq!(folder("/tmp/a b/.c"), "-tmp-a-b--c");
    // The cwd and folder captured on macOS with claude 2.1.325.
    let cwd = format!(
        "/private/var/folders/vn/c7s5y01s7nb7b6bd_lf__cbh0000gn/T/claude-direct-h4c41056/\
         transcripts-long-{}/repo",
        "d".repeat(150)
    );
    let name = folder(&cwd);
    assert_eq!(name.len(), 207);
    assert!(name.starts_with("-private-var-folders-vn-c7s5y01s7nb7b6bd-lf--cbh0000gn-T-"));
    assert!(name.ends_with("-9ahn45"), "{name}");

    let dir = laid("long-cwd", &cwd);
    let past = past(dir.path(), &cwd);
    assert_eq!(past.len(), 1);
    assert_eq!(
        (past[0].id.as_str(), past[0].title.as_str()),
        (LONG, "first")
    );
    let h = read(dir.path(), LONG).unwrap();
    assert_eq!(texts(&h), ["user: first", "text: one"]);
}

#[test]
fn a_subagent_shows_as_its_tool_call_and_its_sidechain_is_skipped() {
    let dir = laid("subagent", "/tmp/h4c/transcripts-subagent/repo");
    let file = file(dir.path(), SUBAGENT).unwrap();
    // Older CLIs kept a subagent's lines in the main transcript.
    let sub = file
        .with_extension("")
        .join("subagents/agent-a592cb0c45ab9bdb1.jsonl");
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str(&fs::read_to_string(sub).unwrap());
    fs::write(&file, text).unwrap();

    let h = read(dir.path(), SUBAGENT).unwrap();
    assert_eq!(texts(&h), ["user: go", "text: main done", "text: done"]);
    // The notification between the two is noise: a break alone keeps them apart.
    let main = h
        .iter()
        .position(|u| *u == Update::Text("main done".into()));
    assert_eq!(h[main.unwrap() + 1], Update::Break);
    let tools: Vec<&Tool> = h
        .iter()
        .filter_map(|u| match u {
            Update::Tool(t) => Some(&**t),
            _ => None,
        })
        .collect();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].id, "toolu_0_0");
    assert_eq!(tools[0].title.as_deref(), Some("probe"));
    assert_eq!(tools[0].kind.as_deref(), Some("Task"));
    assert_eq!(tools[0].input.as_ref().unwrap()["prompt"], "Say hi.");
    assert_eq!(tools[1].id, "toolu_0_0");
    assert_eq!(tools[1].status.as_deref(), Some("completed"));
    assert_eq!(tools[1].output, None);
}

#[test]
fn a_compacted_session_starts_at_its_boundary() {
    let dir = laid("compaction", "/tmp/h4c/transcripts-compaction/repo");
    let h = read(dir.path(), COMPACTION).unwrap();
    assert_eq!(
        h,
        [
            Update::User("after".into()),
            Update::Break,
            Update::Text("one".into()),
            Update::Break
        ]
    );
}

#[test]
fn each_of_parallel_calls_has_its_result_though_one_alone_is_on_the_chain() {
    let call = |id: &str| json!({"type": "tool_use", "id": id, "name": "Bash", "input": {}});
    let result = |id: &str| json!({"type": "tool_result", "tool_use_id": id, "content": id});
    let line = |kind: &str, uuid: &str, parent: Option<&str>, content: Value| {
        let message = json!({ "content": content });
        json!({"type": kind, "uuid": uuid, "parentUuid": parent, "message": message})
    };
    let lines = [
        line("user", "u", None, json!("go")),
        line("assistant", "a1", Some("u"), json!([call("t1")])),
        line("assistant", "a2", Some("a1"), json!([call("t2")])),
        line("user", "r1", Some("a1"), json!([result("t1")])),
        line("user", "r2", Some("a2"), json!([result("t2")])),
        line("assistant", "a3", Some("r2"), json!("both")),
    ];
    let bytes: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let h = history(bytes.as_bytes()).unwrap();
    let outputs: Vec<_> = h
        .iter()
        .filter_map(|u| match u {
            Update::Tool(t) => t.output.as_ref(),
            _ => None,
        })
        .collect();
    assert_eq!(outputs, ["t1", "t2"]);
    assert_eq!(texts(&h), ["user: go", "text: both"]);
}

#[test]
fn the_list_reads_a_side_file_s_title_the_archived_tag_and_the_model() {
    let cwd = "/tmp/h4c/transcripts-titled/repo";
    let dir = laid("titled", cwd);
    // A request blob between the head and the tail the list reads.
    let file = file(dir.path(), TITLED).unwrap();
    let text = fs::read_to_string(&file).unwrap();
    let (head, tail) = text.split_at(text.find('\n').unwrap() + 1);
    let blob = format!(
        "{{\"type\":\"api-request-blob\",\"message\":\"{}\"}}\n",
        "x".repeat(300_000)
    );
    // The title only in the side file.
    let tail: String = tail
        .lines()
        .filter(|l| !l.contains("\"custom-title\""))
        .map(|l| format!("{l}\n"))
        .collect();
    fs::write(&file, format!("{head}{blob}{tail}")).unwrap();

    let past = past(dir.path(), cwd);
    assert_eq!(past.len(), 1);
    let p = &past[0];
    assert_eq!((p.id.as_str(), p.title.as_str()), (TITLED, "Fixture title"));
    assert!(p.archived());
    assert_eq!(p.model.as_deref(), Some("claude-opus-5-5"));
    assert_eq!(p.cwd.as_deref(), Some(cwd));
    assert!(p.modified > 0);
}

#[test]
fn a_worktree_s_session_is_in_its_own_folder_and_found_from_any() {
    let cwd = "/tmp/h4c/transcripts-worktree/repo";
    let dir = laid("worktree", cwd);
    fs::create_dir(
        dir.path()
            .join("projects")
            .join(folder("/tmp/h4c/transcripts-worktree/main")),
    )
    .unwrap();
    assert_eq!(past(dir.path(), cwd).len(), 1);
    assert!(past(dir.path(), "/tmp/h4c/transcripts-worktree/main").is_empty());
    assert!(read(dir.path(), WORKTREE).is_some());
}

#[test]
fn a_folder_s_list_takes_its_path_as_given_and_its_real_one_once() {
    let short = r"C:\Users\RUNNER~1\AppData\Local\Temp\x\repo";
    let real = r"C:\Users\runneradmin\AppData\Local\Temp\x\repo";
    let dir = laid("titled", short);
    assert!(past(dir.path(), real).is_empty());
    let ids = |real: &str| -> Vec<String> {
        here(dir.path(), short, real)
            .into_iter()
            .map(|p| p.id)
            .collect()
    };
    assert_eq!(ids(real), [TITLED]);
    // A case-blind file system opens one folder under both spellings.
    assert_eq!(ids(&short.to_lowercase()), [TITLED]);
}

#[test]
fn a_file_of_unknown_lines_has_no_history_and_no_list_entry() {
    let cwd = "/tmp/h4c/unknown";
    let dir = laid("titled", cwd);
    let id = "0a0a0a0a-0000-4000-8000-000000000000";
    let lines = "{\"type\":\"frame-from-later\",\"uuid\":\"u1\"}\nnot json\n{\"type\":7}\n";
    fs::write(
        dir.path()
            .join("projects")
            .join(folder(cwd))
            .join(format!("{id}.jsonl")),
        lines,
    )
    .unwrap();
    assert_eq!(read(dir.path(), id), None);
    let past = past(dir.path(), cwd);
    assert_eq!(
        past.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        [TITLED]
    );
}

#[test]
fn delete_removes_the_transcript_and_its_folder_and_never_follows_a_path() {
    let cwd = "/tmp/h4c/transcripts-subagent/repo";
    let dir = laid("subagent", cwd);
    let file = file(dir.path(), SUBAGENT).unwrap();
    assert!(delete(dir.path(), "../projects").is_err());
    assert!(delete(dir.path(), &format!("../{}/{SUBAGENT}", folder(cwd))).is_err());
    assert!(file.exists());

    delete(dir.path(), SUBAGENT).unwrap();
    assert!(!file.exists() && !file.with_extension("").exists());
    assert!(past(dir.path(), cwd).is_empty());
    assert_eq!(read(dir.path(), SUBAGENT), None);
    assert!(delete(dir.path(), SUBAGENT).is_err());
}

#[test]
fn a_distro_s_listing_reads_as_the_folder_does_past_a_nul_in_a_title() {
    let cwd = "/tmp/h4c/transcripts-titled/repo";
    let dir = laid("titled", cwd);
    let file = file(dir.path(), TITLED).unwrap();
    let side = fs::read(file.with_extension("").join("custom-title.json")).unwrap();
    let b64 = |b: &[u8]| {
        let b = STANDARD.encode(b).into_bytes();
        b.chunks(76).collect::<Vec<_>>().join(&b'\n')
    };
    let mut bytes = b"other\x001700000001\0".to_vec();
    for part in [&b"{\"customTitle\": \"a\0b\"}"[..], b"not\0json"] {
        bytes.extend(b64(part));
        bytes.push(0);
    }
    bytes.extend(format!("{TITLED}\x001700000000\0").into_bytes());
    for part in [side, fs::read(&file).unwrap()] {
        bytes.extend(b64(&part));
        bytes.push(0);
    }
    let listed = listed(&bytes, cwd);
    let here = past(dir.path(), cwd);
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].modified, &listed[0].title, &listed[0].tag),
        (1_700_000_000_000, &here[0].title, &here[0].tag)
    );
}
