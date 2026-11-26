use detached_core::adapters::CouncilStart;
use detached_core::backend::{Kind as Chat, Spec};
use detached_core::host::{CouncilAct, CouncilOver};

use super::*;
use crate::council::launch;
use crate::session::{Session, State as Run};

fn act(repo: Option<&str>) -> CouncilAct {
    CouncilAct {
        mode: "dialectic".into(),
        over: CouncilOver::Question("Why?".into()),
        repo: repo.map(Into::into),
        budget: None,
    }
}

fn chat(f: &Fixture, cx: &mut TestAppContext) -> Entity<Session> {
    f.list(cx);
    let spec = Spec::new(&CLAUDE, &f.repo, Chat::Chat);
    let chat = cx.update(|cx| Session::open(spec, None, cx));
    wait_for(cx, LONG, |cx| chat.read(cx).state == Run::Idle);
    chat
}

fn launched(
    chat: &Entity<Session>,
    act: CouncilAct,
    cx: &mut TestAppContext,
) -> gpui::Task<Result<serde_json::Value, String>> {
    cx.update(|cx| launch::start(cx, chat.clone(), act))
}

#[gpui::test]
fn a_council_starts_in_a_repo_of_the_asking_session_alone(cx: &mut TestAppContext) {
    let f = boot(cx);
    let chat = chat(&f, cx);
    let task = launched(&chat, act(Some("/elsewhere")), cx);
    cx.executor().allow_parking();
    let said = cx.foreground_executor().block_test(task).unwrap_err();
    assert!(
        said.starts_with("/elsewhere is not a repo of this session."),
        "{said}"
    );
    assert!(said.contains(&f.repo), "{said}");
}

#[gpui::test]
fn a_council_does_not_start_after_its_turn_was_stopped(cx: &mut TestAppContext) {
    let f = boot(cx);
    let chat = chat(&f, cx);
    f.core
        .change_block(BLOCK, |c: &mut Config| c.council = CouncilStart::Free)
        .unwrap();
    cx.update(crate::settings::refresh);
    wait_for(cx, LONG, |cx| {
        let c: Config = crate::settings::Settings::get(cx)
            .block(BLOCK)
            .unwrap_or_default();
        c.council == CouncilStart::Free
    });
    chat.update(cx, |s, cx| s.prompt("wait go".into(), cx));
    wait_for(cx, LONG, |cx| chat.read(cx).state == Run::Running);
    let task = launched(&chat, act(None), cx);
    chat.update(cx, |s, _| s.cancel());
    cx.executor().allow_parking();
    let said = cx.foreground_executor().block_test(task).unwrap_err();
    assert!(said.starts_with("Cancelled"), "{said}");
    assert!(cx.update(|cx| live::all(cx).is_empty()));
}
