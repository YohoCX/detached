//! Dialectic and brainstorm on `fake_claude`: what each side sees in find and in rebut, the
//! synthesis, and ideas clustered and ranked, a rank without a reason refused.

use detached_core::host::{Rank, Synthesis};
use detached_core::panel::run::Over;
use serde_json::Value;

use super::*;

const QUESTION: &str = "Should x be two?";

fn ask(f: &Fixture, mode: &str, cx: &mut TestAppContext) -> Entity<Panel> {
    let s = Start {
        repo: f.repo.clone(),
        branch: "feat".into(),
        base: "feat".into(),
        mode: mode.into(),
        over: Over::Question(QUESTION.into()),
        ..Start::default()
    };
    cx.update(|cx| live::start(s, cx))
}

fn noted(note: &str) -> String {
    let done = json!({"action": "done", "args": {"nothing_found": false, "note": note}});
    format!("act {done}")
}

/// The JSON data block of a participant's brief.
fn data(f: &Fixture, p: &Entity<Panel>, name: &str, cx: &mut TestAppContext) -> Value {
    let (task, run) = p.read_with(cx, |p, _| (p.task.clone(), p.run.clone()));
    let file = record_dir(f, &task, &run).join(format!("brief/{name}.md"));
    let brief = std::fs::read_to_string(file).unwrap();
    let start = brief.find("```json\n").unwrap() + 8;
    let end = brief[start..].find("\n```").unwrap() + start;
    serde_json::from_str(&brief[start..end]).unwrap()
}

fn synthesis() -> Synthesis {
    Synthesis {
        recommendation: "Keep x one".into(),
        why: "The case against holds".into(),
    }
}

fn sides(f: &Fixture) {
    role(
        f,
        "thesis",
        &format!("fake: {};{}", finding(1, LINE), noted("THESIS-CASE")),
    );
    role(f, "antithesis", &format!("fake: {}", noted("ANTI-CASE")));
    role(f, "rebuttal", &format!("fake: {}", noted("REBUTTED")));
}

#[gpui::test]
fn in_rebut_each_side_gets_exactly_the_other_s_output_and_in_find_neither_sees_it(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    sides(&f);
    let s = json!({"action": "report_synthesis", "args": synthesis()});
    role(&f, "judge", &format!("fake: act {s};{}", done()));
    let p = ask(&f, "dialectic", cx);
    let s = ended(&p, cx);
    assert_eq!(s.synthesis, Some(synthesis()));
    let cases = [
        ("thesis".to_string(), "THESIS-CASE".to_string()),
        ("antithesis".into(), "ANTI-CASE".into()),
    ];
    assert_eq!(s.cases(), cases);
    let rebuttals: Vec<String> = s.rebuttals().into_iter().map(|r| r.0).collect();
    assert_eq!(rebuttals, ["thesis", "antithesis"]);
    let judge = s
        .participants
        .iter()
        .find(|p| p.name == "synthesize")
        .unwrap();
    assert_eq!(
        (judge.role.as_str(), judge.model.as_str()),
        ("judge", "sonnet")
    );
    assert!(p.read_with(cx, |p, _| p.task.starts_with("question_")));

    for side in ["thesis", "antithesis"] {
        let found = data(&f, &p, side, cx);
        assert_eq!(found["question"], QUESTION);
        let text = found.to_string();
        assert!(
            !text.contains("CASE") && !text.contains("x is one"),
            "{text}"
        );
    }
    let others = |side: &str, cx: &mut TestAppContext| data(&f, &p, &format!("rebut-{side}"), cx);
    let finding = json!({"file": "a.txt", "line": 1, "severity": "major", "title": "x is one",
        "body": "b", "by": "thesis"});
    assert_eq!(
        others("antithesis", cx)["others"],
        json!([{"by": "thesis", "case": "THESIS-CASE", "findings": [finding]}])
    );
    assert_eq!(
        others("thesis", cx)["others"],
        json!([{"by": "antithesis", "case": "ANTI-CASE", "findings": []}])
    );
    let judged = data(&f, &p, "synthesize", cx);
    assert_eq!(
        judged["rebuttals"][1],
        json!({"by": "antithesis", "rebuttal": "REBUTTED"})
    );
    assert_eq!(judged["cases"].as_array().unwrap().len(), 2);
}

#[gpui::test]
fn a_judge_done_without_a_synthesis_and_a_side_done_without_a_rebuttal_fail(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    sides(&f);
    role(&f, "rebuttal", &format!("fake: {}", done()));
    role(&f, "judge", &format!("fake: {}", done()));
    let s = ended(&ask(&f, "dialectic", cx), cx);
    assert_failed(&s, "rebut-thesis", "It called `done` without a rebuttal.");
    assert_failed(&s, "synthesize", "It called `done` without a synthesis.");
    assert_eq!(s.synthesis, None);
}

/// The participant ended degraded, its first attempt failed for `why`.
fn assert_failed(s: &State, name: &str, why: &str) {
    let p = s.participants.iter().find(|p| p.name == name).unwrap();
    assert_eq!(p.status, Status::Degraded);
    let first = (s.timeline.iter()).find(|e| e.kind == "attempt" && e.agent == name);
    assert_eq!(first.map(|e| e.text.as_str()), Some(why));
}

fn idea(title: &str) -> String {
    let idea = json!({"action": "report_idea", "args": {"title": title, "body": "how"}});
    format!("act {idea}")
}

fn rank(idea: &str, value: u8, cost: u8, reason: &str) -> String {
    let r = json!({"action": "report_rank", "args": {"idea": idea, "value": value, "cost": cost,
        "reason": reason}});
    format!("act {r}")
}

fn generators(f: &Fixture) {
    role(f, "smallest", &format!("fake: {};{}", idea("A"), done()));
    role(f, "user", &format!("fake: {};{}", idea("B"), done()));
    role(f, "bold", &format!("fake: {};{}", idea("C"), done()));
    role(f, "prior-art", &format!("fake: {}", done()));
    let group = json!({"action": "report_group", "args": {"ids": ["user-1", "smallest-1"]}});
    role(f, "cluster", &format!("fake: act {group};{}", done()));
}

#[gpui::test]
fn a_brainstorm_clusters_and_ranks_its_ideas_and_a_rank_without_a_reason_is_refused(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    generators(&f);
    let script = [
        rank("smallest-1", 5, 1, " "),
        rank("bold-1", 5, 5, "big"),
        rank("smallest-1", 4, 1, "cheap"),
        done(),
    ];
    role(&f, "ranker", &format!("fake: {}", script.join(";")));
    let p = ask(&f, "brainstorm", cx);
    let s = ended(&p, cx);
    let order: Vec<(&str, usize, Option<&Rank>)> = (s.clusters().into_iter())
        .map(|(lead, rest)| (lead.idea.title.as_str(), rest.len(), lead.rank.as_ref()))
        .collect();
    assert_eq!(order.len(), 2);
    assert_eq!(
        (order[0].0, order[0].1),
        ("B", 1),
        "a variant's rank is its lead's"
    );
    assert_eq!(order[0].2.map(|r| r.reason.as_str()), Some("cheap"));
    assert_eq!(order[1].0, "C");
    let ranks = s.timeline.iter().filter(|e| e.kind == "rank").count();
    assert_eq!(ranks, 2, "the blank reason was refused");
    // The generators report in parallel, so the brief's order is theirs.
    let ranked = data(&f, &p, "rank", cx)["ideas"]
        .as_array()
        .unwrap()
        .clone();
    let lead = ranked.iter().find(|i| i["id"] == "user-1").unwrap();
    assert_eq!(
        lead["variants"],
        json!([{"id": "smallest-1", "title": "A", "body": "how"}])
    );
    assert_eq!(ranked.len(), 2);
    assert_eq!(
        data(&f, &p, "cluster", cx)["ideas"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    role(
        &f,
        "ranker",
        &format!("fake: {};{}", rank("user-1", 4, 1, ""), done()),
    );
    let s = ended(&ask(&f, "brainstorm", cx), cx);
    assert_failed(&s, "rank", "It called `done` without ranking every idea.");
    assert!(s.clusters().iter().all(|(c, _)| c.rank.is_none()));
}
