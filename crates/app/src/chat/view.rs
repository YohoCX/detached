//! The chat page: what is set for the session, the conversation, and the box to write in.

use detached_core::api;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, InputEvent, MoveDown, MoveUp, Textarea, TextareaState,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div, prelude::FluentBuilder, px,
};

use detached_core::git::snapshot::Skip;
use detached_core::types::{FileChange, Side};

use super::session::{Answer, Back, Item, Session};
use crate::blocking;
use crate::store::{DiffDoc, Doc, Store};
use crate::ui::{self, Icon, Menu, radius, text, theme};

const MODES: &[(&str, &str, Option<&str>)] = &[
    (
        "auto",
        "Auto",
        Some("A classifier approves or blocks each action, and asks you when it can't tell"),
    ),
    (
        "acceptEdits",
        "Edits",
        Some("File edits run; other actions that need approval ask you"),
    ),
    (
        "plan",
        "Plan",
        Some("Explores and plans without changing files"),
    ),
    (
        "default",
        "Manual",
        Some("Anything that needs approval asks you"),
    ),
];

/// How close to the end still counts as following it.
const FOLLOW: f32 = 24.;
const SUGGESTIONS: u32 = 8;

/// What the word at the cursor could become.
struct Suggest {
    /// Where the word starts.
    at: usize,
    word: String,
    /// Shown, and what replaces the word.
    items: Vec<(String, String)>,
    hi: usize,
}

pub struct ChatView {
    session: Entity<Session>,
    input: Entity<TextareaState>,
    scroll: ScrollHandle,
    suggest: Option<Suggest>,
    /// A `/` typed before any session listed its commands.
    unlisted: bool,
    _files: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatView {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(3)
                .placeholder("Message Claude. / for commands, @ for files, Enter sends.")
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let scroll = ScrollHandle::new();
        let follow = scroll.clone();
        let subscriptions = vec![
            cx.observe(&session, move |_, _, cx| {
                if f32::from(follow.max_offset().y + follow.offset().y) < FOLLOW {
                    follow.scroll_to_bottom();
                }
                cx.notify();
            }),
            cx.subscribe(&input, |this, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.suggest(cx);
                }
            }),
        ];
        session.update(cx, |s, cx| {
            if s.items.is_empty() && s.past.is_none() {
                s.load_past(cx);
            }
            s.check_trust(cx);
        });
        scroll.scroll_to_bottom();
        ChatView {
            session,
            input,
            scroll,
            suggest: None,
            unlisted: false,
            _files: None,
            _subscriptions: subscriptions,
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() || self.session.read(cx).busy {
            return;
        }
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.suggest = None;
        self.scroll.scroll_to_bottom();
        self.session.update(cx, |s, cx| s.send(text, cx));
    }

    /// Suggests commands for a `/` at the start and files for an `@`.
    fn suggest(&mut self, cx: &mut Context<Self>) {
        let input = self.input.read(cx);
        let (value, cursor) = (input.value(), input.cursor());
        let before = &value[..cursor.min(value.len())];
        let at = before
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let word = before[at..].to_string();
        self.suggest = None;
        self._files = None;
        self.unlisted = false;
        if at == 0 && word.starts_with('/') {
            let q = word[1..].to_lowercase();
            let s = self.session.read(cx);
            self.unlisted = s.commands.is_empty();
            let mut items: Vec<_> = s
                .commands
                .iter()
                .filter(|c| c.name.to_lowercase().contains(&q))
                .map(|c| {
                    let hint = [c.hint.as_str(), c.description.as_str()].join(" ");
                    (
                        format!("/{} {}", c.name, hint.trim()),
                        format!("/{} ", c.name),
                    )
                })
                .collect();
            items.sort_by_key(|(_, name)| !name[1..].to_lowercase().starts_with(&q));
            items.truncate(SUGGESTIONS as usize);
            self.show(at, word, items, cx);
        } else if let Some(q) = word.strip_prefix('@') {
            let core = Store::global(cx).read(cx).core().clone();
            let (repo, q) = (self.session.read(cx).repo.clone(), q.to_string());
            let found = blocking::run(cx, move || {
                api::quick_open(&core, &[repo], None, &q, SUGGESTIONS).unwrap_or_default()
            });
            self._files = Some(cx.spawn(async move |this, cx| {
                let found = found.await;
                let items = found
                    .into_iter()
                    .map(|m| (m.path.clone(), format!("@{} ", m.path)))
                    .collect();
                this.update(cx, |this, cx| this.show(at, word, items, cx))
                    .ok();
            }));
        }
        cx.notify();
    }

    fn show(
        &mut self,
        at: usize,
        word: String,
        items: Vec<(String, String)>,
        cx: &mut Context<Self>,
    ) {
        self.suggest = (!items.is_empty()).then_some(Suggest {
            at,
            word,
            items,
            hi: 0,
        });
        cx.notify();
    }

    fn accept(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.suggest.take() else {
            return;
        };
        let Some((_, with)) = s.items.get(i) else {
            return;
        };
        let range = s.at..s.at + s.word.len();
        self.input.update(cx, |input, cx| {
            input.set_selected_range(range, cx);
            input.replace(with.clone(), window, cx);
        });
        self.suggest = None;
        cx.notify();
    }

    /// Moves the highlighted suggestion, if any show.
    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.suggest {
            let n = s.items.len();
            s.hi = if down {
                (s.hi + 1) % n
            } else {
                (s.hi + n - 1) % n
            };
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let s = self.session.read(cx);
        let name = Store::global(cx).read(cx).name_of(&s.repo).to_string();
        let asked = MODES.iter().position(|(m, ..)| *m == s.asked);
        let options: Vec<_> = MODES
            .iter()
            .enumerate()
            .map(|(i, (_, label, tip))| (i, *label, *tip))
            .collect();
        let session = self.session.clone();
        let modes = ui::segmented_tips(
            "chat.mode",
            &options,
            asked.unwrap_or(0),
            move |i, _, cx| session.update(cx, |s, cx| s.set_mode(MODES[i].0, cx)),
            cx,
        );
        let menu = |id: &'static str,
                    title: &str,
                    label: String,
                    entries: Vec<(String, Option<String>)>| {
            let session = self.session.clone();
            ui::titled(
                ui::icon_button(id, div().child(label), cx),
                title.to_string(),
            )
            .test_support()
            .when(!entries.is_empty(), |b| {
                b.on_click(move |e, window, cx| {
                    let menu = entries.iter().fold(Menu::default(), |menu, (label, pick)| {
                        let (session, pick) = (session.clone(), pick.clone());
                        menu.item_if(pick.is_some(), label.clone(), move |_, cx| {
                            let Some(pick) = pick.clone() else { return };
                            session.update(cx, |s, cx| match id {
                                "chat.model" => s.set_model(pick, cx),
                                _ => s.set_effort(pick, cx),
                            })
                        })
                    });
                    menu.show(e.position(), window, cx);
                })
            })
        };
        let models = s
            .models
            .iter()
            .map(|m| {
                let label = match m.auto {
                    true => m.label.clone(),
                    false => format!("{} (no auto)", m.label),
                };
                (label, Some(m.value.clone()))
            })
            .collect();
        let model = s.model.clone().unwrap_or("Default model".into());
        let picker = menu("chat.model", "Model", model, models);
        let efforts: Vec<_> = s
            .efforts()
            .iter()
            .map(|e| (e.clone(), Some(e.clone())))
            .collect();
        let effort = (!efforts.is_empty()).then(|| {
            let label = s.effort.clone().unwrap_or("Effort".into());
            menu("chat.effort", "Effort", label, efforts)
        });
        let usage = s.usage.filter(|(_, max)| *max > 0).map(|(used, max)| {
            let parts = s
                .parts
                .iter()
                .map(|p| (format!("{} {}", p.name, tokens(p.tokens)), None))
                .collect();
            let label = format!("Context {}%", used * 100 / max);
            menu(
                "chat.usage",
                &format!("{} of {}", tokens(used), tokens(max)),
                label.clone(),
                parts,
            )
            .aria_label(SharedString::from(label))
            .text_color(t.muted)
        });
        let session = self.session.clone();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(8.))
            .border_b_1()
            .border_color(t.line)
            .child(div().font_weight(FontWeight::BOLD).child("Claude"))
            .child(div().min_w_0().truncate().text_color(t.muted).child(name))
            .child(div().flex_1())
            .children(usage)
            .child(picker)
            .children(effort)
            .child(modes)
            .child(
                ui::titled(
                    ui::icon_button("chat.new", Icon::Plus, cx),
                    "New conversation",
                )
                .test_support()
                .on_click(move |_, _, cx| session.update(cx, |s, cx| s.reset(cx))),
            )
    }

    fn item(&self, i: usize, item: &Item, cx: &App) -> AnyElement {
        let t = theme(cx);
        let row = |label: String| {
            div()
                .id(("chat.item", i))
                .test_support()
                .aria_label(SharedString::from(label))
                .w_full()
                .min_w_0()
        };
        let line = |label: String, icon: AnyElement, name: String, detail: String| {
            row(label)
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(text::SMALL)
                .text_color(t.muted)
                .child(icon)
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(theme::mono(cx))
                        .child(detail),
                )
        };
        match item {
            Item::User(text) => row(format!("You: {text}"))
                .px(px(12.))
                .py(px(8.))
                .rounded(radius::FIELD)
                .bg(t.hover)
                .child(text.clone())
                .into_any_element(),
            Item::Text(text, md) => {
                let body = match md {
                    Some(md) => md.clone().into_any_element(),
                    None => div().child(text.clone()).into_any_element(),
                };
                row(text.clone()).child(body).into_any_element()
            }
            Item::Tool {
                name, detail, ok, ..
            } => {
                let icon = match ok {
                    None => ui::spinner(t.accent).into_any_element(),
                    Some(true) => div()
                        .text_color(t.success)
                        .child(Icon::Check.size(px(12.)))
                        .into_any_element(),
                    Some(false) => div()
                        .text_color(t.error)
                        .child(Icon::Close.size(px(12.)))
                        .into_any_element(),
                };
                let state = ok.map_or("running", |ok| if ok { "ok" } else { "failed" });
                let label = format!("{name} {detail} {state}");
                line(label, icon, name.clone(), detail.clone()).into_any_element()
            }
            Item::Denied { tool, reason } => {
                let icon = div()
                    .text_color(t.warning)
                    .child(Icon::Close.size(px(12.)))
                    .into_any_element();
                let label = format!("Blocked {tool}: {reason}");
                line(label, icon, format!("Blocked {tool}"), reason.clone()).into_any_element()
            }
            Item::Ask { .. } => self.ask(i, item, cx),
            Item::Changes {
                start,
                end,
                files,
                back,
            } => self.changes(i, start, end, files, back, cx),
            Item::Note(text) => row(text.clone())
                .text_size(text::SMALL)
                .text_color(t.warning)
                .child(text.clone())
                .into_any_element(),
        }
    }

    /// A permission prompt or Claude's questions, with the buttons that answer them.
    fn ask(&self, i: usize, item: &Item, cx: &App) -> AnyElement {
        let Item::Ask {
            tool,
            detail,
            body,
            always,
            questions,
            picked,
            answer,
            ..
        } = item
        else {
            unreachable!()
        };
        let t = theme(cx);
        let state = match answer {
            Answer::Waiting => "waiting",
            Answer::Allowed => "allowed",
            Answer::Denied => "denied",
            Answer::Gone => "no answer",
        };
        let asking = questions.is_empty();
        let title = match asking {
            true => format!("Claude asks to use {tool}"),
            false => "Claude asks".to_string(),
        };
        let open = *answer == Answer::Waiting;
        let button =
            |id: &'static str, label: &'static str, primary: bool, allow: bool, always: bool| {
                let session = self.session.clone();
                let b = match primary {
                    true => ui::primary_button((id, i), label, cx),
                    false => ui::button((id, i), label, cx),
                };
                b.test_support().on_click(move |_, _, cx| {
                    session.update(cx, |s, cx| s.answer(i, allow, always, cx))
                })
            };
        let complete = picked.iter().all(|p| !p.is_empty());
        let questions = questions.iter().enumerate().map(|(q, question)| {
            let options = question.options.iter().enumerate().map(|(o, c)| {
                let on = picked[q].contains(&o);
                let session = self.session.clone();
                ui::row(
                    SharedString::from(format!("chat.option.{i}.{q}.{o}")),
                    on,
                    cx,
                )
                .test_support()
                .aria_label(SharedString::from(c.label.clone()))
                .flex_col()
                .items_start()
                .when(open, |b| {
                    b.on_click(move |_, _, cx| session.update(cx, |s, cx| s.pick(i, q, o, cx)))
                })
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(c.label.clone()),
                )
                .child(
                    div()
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .child(c.description.clone()),
                )
            });
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .child(question.header.clone()),
                )
                .child(div().child(question.question.clone()))
                .children(options)
        });
        let buttons = open.then(|| {
            let mut b = div().flex().gap(px(8.));
            b = match asking {
                true => b
                    .child(button("chat.allow", "Allow", true, true, false))
                    .when(*always, |b| {
                        b.child(button("chat.always", "Always allow", false, true, true))
                    })
                    .child(button("chat.deny", "Deny", false, false, false)),
                false => b
                    .when(complete, |b| {
                        b.child(button("chat.allow", "Answer", true, true, false))
                    })
                    .child(button("chat.deny", "Skip", false, false, false)),
            };
            b
        });
        div()
            .id(("chat.item", i))
            .test_support()
            .aria_label(SharedString::from(format!("{title}: {detail} {state}")))
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(radius::FIELD)
            .border_1()
            .border_color(if open { t.accent } else { t.line })
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .when(!detail.is_empty(), |b| {
                b.child(
                    div()
                        .text_size(text::SMALL)
                        .font_family(theme::mono(cx))
                        .child(detail.clone()),
                )
            })
            .children(body.clone())
            .children(questions)
            .children(buttons)
            .when(!open, |b| {
                b.child(
                    div()
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .child(state),
                )
            })
            .into_any_element()
    }

    /// What a turn changed, each file opening its diff, and going back to before it.
    fn changes(
        &self,
        i: usize,
        start: &str,
        end: &str,
        files: &[FileChange],
        back: &Back,
        cx: &App,
    ) -> AnyElement {
        let t = theme(cx);
        let busy = self.session.read(cx).busy;
        let n = files.len();
        let title = match n {
            1 => "1 file changed in this turn".to_string(),
            _ => format!("{n} files changed in this turn"),
        };
        let repo = self.session.read(cx).repo.clone();
        let rows = files.iter().enumerate().map(|(j, f)| {
            let doc = Doc::Diff(DiffDoc {
                repo: repo.clone(),
                path: f.path.clone(),
                orig_path: f.orig_path.clone(),
                left: Side::Rev { rev: start.into() },
                right: Side::Rev { rev: end.into() },
            });
            ui::row(SharedString::from(format!("chat.file.{i}.{j}")), false, cx)
                .test_support()
                .aria_label(SharedString::from(f.path.clone()))
                .gap(px(6.))
                .on_click(move |_, _, cx| {
                    let doc = doc.clone();
                    Store::global(cx).update(cx, |s, cx| s.show(doc, cx))
                })
                .child(
                    div()
                        .w(px(12.))
                        .text_color(t.muted)
                        .child(f.index.to_string()),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(theme::mono(cx))
                        .child(f.path.clone()),
                )
        });
        let session = self.session.clone();
        let button =
            move |id: &'static str,
                  label: String,
                  run: fn(&mut Session, usize, &mut Context<Session>)| {
                let session = session.clone();
                ui::button((id, i), label, cx)
                    .test_support()
                    .on_click(move |_, _, cx| session.update(cx, |s, cx| run(s, i, cx)))
            };
        let small = |text: String| div().text_size(text::SMALL).text_color(t.muted).child(text);
        let (state, below) = match back {
            Back::Idle => (
                "",
                (!busy).then(|| {
                    button(
                        "chat.back",
                        "Restore the files to before this turn".into(),
                        Session::plan_back,
                    )
                    .into_any_element()
                }),
            ),
            Back::Working => ("working", Some(ui::spinner(t.accent).into_any_element())),
            Back::Planned(plan) => {
                let todo = plan.changes.iter().filter(|c| c.skip.is_none()).count();
                let lines = plan.changes.iter().map(|c| {
                    let what = match (c.skip, c.remove) {
                        (Some(Skip::Edited), _) => "kept: changed since the turn",
                        (Some(Skip::Ignored), _) => "kept: ignored now, no copy to undo with",
                        (Some(Skip::Submodule), _) => "kept: a submodule",
                        (None, true) => "removed",
                        (None, false) if c.edited => "restored, over changes made since the turn",
                        (None, false) => "restored",
                    };
                    small(format!("{}: {what}", c.path))
                });
                let confirm = (todo > 0).then(|| {
                    let label = match todo {
                        1 => "Restore 1 file".to_string(),
                        _ => format!("Restore {todo} files"),
                    };
                    button("chat.back.confirm", label, Session::confirm_back)
                });
                let el = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .children(lines)
                    .child(small(
                        "Ignored files and submodules are not covered. You can undo this.".into(),
                    ))
                    .child(div().flex().gap(px(8.)).children(confirm).child(button(
                        "chat.back.cancel",
                        "Cancel".into(),
                        Session::cancel_back,
                    )));
                ("planned", Some(el.into_any_element()))
            }
            Back::Done { .. } => {
                let el = div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(small("Restored.".into()))
                    .when(!busy, |b| {
                        b.child(button(
                            "chat.back",
                            "Undo the restore".into(),
                            Session::plan_back,
                        ))
                    });
                ("restored", Some(el.into_any_element()))
            }
        };
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        div()
            .id(("chat.item", i))
            .test_support()
            .aria_label(SharedString::from(
                format!("{title}: {} {state}", paths.join(", "))
                    .trim()
                    .to_string(),
            ))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(12.))
            .rounded(radius::FIELD)
            .border_1()
            .border_color(t.line)
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .children(rows)
            .child(small(
                "Edits anyone made in this folder during the turn show here too.".into(),
            ))
            .children(below)
            .into_any_element()
    }

    /// The repo's own settings, until the user trusts them or puts it off.
    fn review(&self, cx: &App) -> Option<AnyElement> {
        let t = theme(cx);
        let p = self.session.read(cx).review.as_ref()?;
        let small = |text: &str| {
            div()
                .text_size(text::SMALL)
                .text_color(t.muted)
                .child(text.to_string())
        };
        let (trust, later) = (self.session.clone(), self.session.clone());
        let card = div()
            .id("chat.trust")
            .test_support()
            .aria_label(SharedString::from(format!("Review: {}", p.items.join("; "))))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(12.))
            .rounded(radius::FIELD)
            .border_1()
            .border_color(t.warning)
            .child(div().font_weight(FontWeight::SEMIBOLD).child("This repo's Claude settings would run"))
            .children(p.items.iter().map(|i| {
                div().text_size(text::SMALL).font_family(theme::mono(cx)).child(i.clone())
            }))
            .child(small(
                "Until you trust them, sessions here load only your own settings, so the repo's CLAUDE.md and .claude are skipped too.",
            ))
            .child(small(
                "Trusting covers these files as they are; a change asks again. It also trusts future versions of the scripts they run.",
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui::primary_button("chat.trust.accept", "Trust these settings", cx)
                            .test_support()
                            .on_click(move |_, _, cx| trust.update(cx, |s, cx| s.trust(cx))),
                    )
                    .child(
                        ui::button("chat.trust.later", "Not now", cx)
                            .test_support()
                            .on_click(move |_, _, cx| later.update(cx, |s, cx| s.not_now(cx))),
                    ),
            );
        Some(card.into_any_element())
    }

    /// Past conversations, for an empty one.
    fn past(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = theme(cx);
        let past = self
            .session
            .read(cx)
            .past
            .as_ref()
            .filter(|p| !p.is_empty())?;
        let rows = past.iter().enumerate().map(|(i, p)| {
            let id = p.id.clone();
            ui::row(("chat.past", i), false, cx)
                .test_support()
                .aria_label(SharedString::from(p.title.clone()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.session.update(cx, |s, cx| s.open_past(id.clone(), cx));
                    this.input.update(cx, |s, cx| s.focus(window, cx));
                }))
                .child(div().min_w_0().truncate().child(p.title.clone()))
        });
        let list = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .pb(px(4.))
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child("Recent conversations"),
            )
            .children(rows);
        Some(list.into_any_element())
    }

    fn footer(&self, cx: &App) -> Option<AnyElement> {
        let t = theme(cx);
        let s = self.session.read(cx);
        let tasks = s.tasks.iter().enumerate().map(|(i, task)| {
            let (session, id) = (self.session.clone(), task.id.clone());
            div()
                .id(("chat.task", i))
                .test_support()
                .aria_label(SharedString::from(format!(
                    "Background: {}",
                    task.description
                )))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(text::SMALL)
                .child(ui::spinner(t.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(t.muted)
                        .child(task.description.clone()),
                )
                .child(
                    ui::button(("chat.task.stop", i), "Stop", cx)
                        .test_support()
                        .on_click(move |_, _, cx| session.read(cx).stop_task(id.clone())),
                )
        });
        let stop = self.session.clone();
        let working = s.busy.then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(ui::spinner(t.accent))
                .child(div().flex_1().text_color(t.muted).child("Working"))
                .child(
                    ui::button("chat.stop", "Stop", cx)
                        .test_support()
                        .on_click(move |_, _, cx| stop.read(cx).stop()),
                )
        });
        (s.busy || !s.tasks.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(tasks)
                .children(working)
                .into_any_element()
        })
    }

    fn suggestions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.unlisted {
            let text = "Commands show once Claude has answered here.";
            let hint = div()
                .id("chat.unlisted")
                .test_support()
                .aria_label(text)
                .px(px(10.))
                .py(px(4.))
                .text_color(theme(cx).muted)
                .child(text);
            return Some(hint.into_any_element());
        }
        let s = self.suggest.as_ref()?;
        let rows = s.items.iter().enumerate().map(|(i, (label, _))| {
            ui::row(("chat.suggest", i), i == s.hi, cx)
                .test_support()
                .aria_label(SharedString::from(label.clone()))
                .on_click(cx.listener(move |this, _, window, cx| this.accept(i, window, cx)))
                .child(div().min_w_0().truncate().child(label.clone()))
        });
        Some(div().flex().flex_col().children(rows).into_any_element())
    }
}

fn tokens(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        _ => format!("{:.1}k", n as f64 / 1000.),
    }
}

impl Render for ChatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let s = self.session.read(cx);
        let items: Vec<_> = s
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| self.item(i, item, cx))
            .collect();
        let past = items.is_empty().then(|| self.past(cx)).flatten();
        let review = self.review(cx);
        let footer = self.footer(cx);
        let suggestions = self.suggestions(cx);
        div()
            .id("chat")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.code)
            .child(self.header(cx))
            .child(
                div()
                    .id("chat.items")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap(px(10.))
                    .p(px(16.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(review)
                    .children(items)
                    .children(past),
            )
            .child(
                div()
                    .id("chat.input")
                    .test_support()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(8.))
                    .p(px(12.))
                    .border_t_1()
                    .border_color(t.line)
                    .children(footer)
                    .children(suggestions)
                    // Taken before the textarea, which would act on them first.
                    .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                        if this.suggest.is_some() {
                            cx.stop_propagation();
                            let hi = this.suggest.as_ref().map_or(0, |s| s.hi);
                            this.accept(hi, window, cx);
                        } else if !window.modifiers().shift {
                            cx.stop_propagation();
                            this.send(window, cx);
                        }
                    }))
                    .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                        if let Some(hi) = this.suggest.as_ref().map(|s| s.hi) {
                            cx.stop_propagation();
                            this.accept(hi, window, cx);
                        }
                    }))
                    .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                        if this.suggest.take().is_some() {
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }))
                    .capture_action(cx.listener(|this, _: &MoveUp, _, cx| this.step(false, cx)))
                    .capture_action(cx.listener(|this, _: &MoveDown, _, cx| this.step(true, cx)))
                    .child(Textarea::new(&self.input)),
            )
    }
}
