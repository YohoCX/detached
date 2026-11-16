//! The chat page: what is set for the session, the conversation, and the box to write in.

use std::collections::{BTreeMap, HashMap, HashSet};

use detached_core::adapters::CLAUDE;
use detached_core::api;
use detached_core::backend::{Limit, Model, Past, Piece, Question, Setting, Tool};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, InputEvent, MoveDown, MoveUp, Textarea, TextareaState,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardItem, Context, Entity, EntityId, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity, Window, div,
    prelude::FluentBuilder, px, relative,
};

use detached_core::git::snapshot::Skip;
use detached_core::types::Side;

use super::session::Chat;
use crate::files::{Links, Markdown};
use crate::ui::{self, Icon, Menu, radius, text, theme};
use crate::util::{age, now, span};
use kernel::blocking;
use kernel::doc::{DiffDoc, Doc, Location};
use kernel::resource::Interest;
use kernel::session::{Answer, Back, Item, Session};
use kernel::workspace::Workspace;

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
/// Past conversations an empty chat offers.
const PAST: usize = 5;

/// What the word at the cursor could become.
struct Suggest {
    /// Where the word starts.
    at: usize,
    word: String,
    /// Shown, and what replaces the word.
    items: Vec<(String, String)>,
    hi: usize,
}

/// An item by its session's place in the chat and its own in the session.
type At = (usize, usize);

pub struct ChatView {
    session: Entity<Chat>,
    input: Entity<TextareaState>,
    scroll: ScrollHandle,
    suggest: Option<Suggest>,
    /// Groups of tool calls opened or closed against their default, by their first call's id.
    toggled: HashSet<String>,
    /// Tool calls whose output shows.
    outputs: HashSet<String>,
    /// Each whole text laid out, with the text it was laid out from.
    texts: HashMap<(EntityId, usize), (String, Entity<Markdown>)>,
    /// Per question of a waiting ask, the options picked.
    picked: HashMap<(EntityId, usize), Vec<Vec<usize>>>,
    past: Interest<Vec<Past>>,
    me: WeakEntity<ChatView>,
    /// A `/` typed before any session listed its commands.
    unlisted: bool,
    _files: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatView {
    pub fn new(session: Entity<Chat>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx).rows(3).placeholder(format!(
                "Message {}. / for commands, @ for files, Enter sends.",
                CLAUDE.name
            ))
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
            s.ensure(cx);
            s.check_trust(cx);
        });
        let repo = session.read(cx).repo.clone();
        let past = Interest::view(kernel::session::past_of(&CLAUDE, &repo, cx), cx);
        scroll.scroll_to_bottom();
        ChatView {
            session,
            input,
            scroll,
            suggest: None,
            toggled: HashSet::new(),
            outputs: HashSet::new(),
            texts: HashMap::new(),
            picked: HashMap::new(),
            past,
            me: cx.weak_entity(),
            unlisted: false,
            _files: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.focus(window, cx));
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() || self.session.read(cx).busy(cx) {
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
            let commands = self.session.read(cx).commands(cx);
            self.unlisted = commands.is_empty();
            let mut items: Vec<_> = commands
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
            let core = kernel::core(cx).clone();
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
        let name = super::label(&s.repo, cx);
        let topic = s
            .topic(cx)
            .and_then(|t| t.lines().next())
            .unwrap_or("New chat")
            .to_string();
        let session = self.session.clone();
        div()
            .id("chat.header")
            .test_support()
            .aria_label(SharedString::from(format!("{topic} {name}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(6.))
            .border_b_1()
            .border_color(t.line)
            .child(div().text_color(t.accent).child(Icon::Chat))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(topic),
            )
            .child(div().flex_none().text_color(t.muted).child(name))
            .child(div().flex_1())
            .child(
                ui::titled(ui::icon_button("chat.new", Icon::Plus, cx), "New chat")
                    .test_support()
                    .on_click(move |_, _, cx| {
                        let repo = session.read(cx).repo.clone();
                        super::new_chat(repo, Some(&session), cx)
                    }),
            )
    }

    /// Under the box: who answers, the session's mode, model and effort; right, the context and
    /// what is left of the plan.
    fn bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let chat = self.session.read(cx);
        let s = chat.current().map(|s| s.read(cx));
        let menu = |id: &'static str,
                    title: &str,
                    label: String,
                    entries: Vec<(String, Option<Setting>)>| {
            let session = self.session.clone();
            let chevron =
                (!entries.is_empty() && id != "chat.usage").then(|| Icon::Down.size(px(10.)));
            ui::titled(
                ui::icon_button(id, div().child(label.clone()), cx).children(chevron),
                title.to_string(),
            )
            .aria_description(SharedString::from(label))
            .test_support()
            .when(!entries.is_empty(), |b| {
                b.on_click(move |e, window, cx| {
                    let menu = entries.iter().fold(Menu::default(), |menu, (label, pick)| {
                        let (session, pick) = (session.clone(), pick.clone());
                        menu.item_if(pick.is_some(), label.clone(), move |_, cx| {
                            if let Some(pick) = pick.clone() {
                                session.update(cx, |s, cx| s.set(pick, cx));
                            }
                        })
                    });
                    menu.show(e.position(), window, cx);
                })
            })
        };
        let asked = s.map_or(&chat.mode, |s| &s.spec.mode);
        let mode = MODES.iter().find(|(m, ..)| m == asked).unwrap_or(&MODES[0]);
        let modes = MODES
            .iter()
            .map(|(m, label, _)| (label.to_string(), Some(Setting::Mode(m.to_string()))))
            .collect();
        let modes = menu(
            "chat.mode",
            mode.2.unwrap_or(mode.1),
            mode.1.to_string(),
            modes,
        );
        let models: &[Model] = s.map_or(&[], |s| &s.models);
        let entries = models
            .iter()
            .map(|m| {
                let label = match m.auto {
                    true => m.label.clone(),
                    false => format!("{} (no auto)", m.label),
                };
                (label, Some(Setting::Model(m.value.clone())))
            })
            .collect();
        let current = s.and_then(|s| s.option("model")).or(chat.model.as_deref());
        let model = current.map_or("Default model".into(), |m| model_name(m, models));
        let picker = menu("chat.model", "Model", model, entries);
        let efforts = models.iter().find(|m| Some(m.value.as_str()) == current);
        let efforts: Vec<_> = (efforts.map_or(&[][..], |m| &m.efforts).iter())
            .map(|e| (e.clone(), Some(Setting::Effort(e.clone()))))
            .collect();
        let effort = (!efforts.is_empty()).then(|| {
            let now = s.and_then(|s| s.option("effort")).filter(|e| !e.is_empty());
            let label = now.or(chat.effort.as_deref()).unwrap_or("Effort");
            menu("chat.effort", "Effort", label.to_string(), efforts)
        });
        let usage = s.and_then(|s| s.usage.filter(|(_, max)| *max > 0).map(|u| (u, &s.parts)));
        let usage = usage.map(|((used, max), parts)| {
            let parts: Vec<String> = (parts.iter())
                .map(|p| format!("{} {}", p.name, tokens(p.tokens)))
                .collect();
            let label = format!("Context {}%", used * 100 / max);
            let entries = parts.iter().map(|p| (p.clone(), None)).collect();
            menu(
                "chat.usage",
                &format!("{} of {}", tokens(used), tokens(max)),
                label.clone(),
                entries,
            )
            .aria_label(SharedString::from(label))
            .aria_description(SharedString::from(parts.join(", ")))
            .text_color(t.muted)
        });
        let account = s.and_then(|s| s.account.as_ref());
        let plan = account.map(|a| SharedString::from(a.plan.clone()));
        let tip = account.map(|a| match &a.email {
            Some(e) => SharedString::from(format!("{} · {e}", a.plan)),
            None => SharedString::from(a.plan.clone()),
        });
        let provider = div()
            .id("chat.provider")
            .test_support()
            .flex()
            .items_center()
            .gap(px(6.))
            .pr(px(4.))
            .child(div().text_color(t.accent).child(Icon::Chat.size(px(12.))))
            .child(
                div()
                    .id("chat.name")
                    .test_support()
                    .aria_label(CLAUDE.name)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(CLAUDE.name),
            )
            .when_some(plan, |d, plan| d.aria_label(plan))
            .when_some(tip, |d, tip| {
                d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            });
        let limits = s.and_then(|s| s.limits.as_ref()).into_iter().flatten();
        let limits: Vec<_> = limits
            .filter(|l| l.name == "5h" || l.name == "Week")
            .map(|l| limit(l, cx).into_any_element())
            .collect();
        let unknown = s
            .filter(|s| limits.is_empty() && s.limits_settled())
            .map(|_| {
                let text = "No plan limits reported";
                div()
                    .id("chat.limits")
                    .test_support()
                    .aria_label(text)
                    .text_color(t.muted)
                    .child(text)
            });
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .text_size(text::SMALL)
            .child(provider)
            .child(modes)
            .child(picker)
            .children(effort)
            .child(div().flex_1())
            .children(usage)
            .children(limits)
            .children(unknown)
    }

    /// Lays out each whole text once, and drops what is gone.
    fn lay_out(&mut self, cx: &mut Context<Self>) {
        let chat = self.session.read(cx);
        let mut whole = vec![];
        for s in &chat.sessions {
            let (id, s) = (s.entity_id(), s.read(cx));
            let last = s.items.len().saturating_sub(1);
            for (i, it) in s.items.iter().enumerate() {
                if let Item::Text(t) = it
                    && (i < last || !s.busy())
                {
                    whole.push(((id, i), t.clone()));
                }
            }
        }
        let mut texts = std::mem::take(&mut self.texts);
        for (key, text) in whole {
            let md = match texts.remove(&key) {
                Some((was, md)) if was == text => md,
                _ => {
                    let links = Links::Forge {
                        project_url: String::new(),
                        base_url: String::new(),
                        gitlab: false,
                    };
                    cx.new(|cx| Markdown::new(text.clone(), links, true, cx))
                }
            };
            self.texts.insert(key, (text, md));
        }
    }

    /// The items, with each run of tool calls folded into one group.
    fn feed(&self, cx: &App) -> Vec<AnyElement> {
        let chat = self.session.read(cx);
        let feed = chat.feed(cx);
        let tool = |it: &Item| matches!(it, Item::Tool(t) if t.status.as_deref() != Some("denied"));
        let mut out = vec![];
        let mut i = 0;
        while i < feed.len() {
            let run = feed[i..].iter().take_while(|(_, it)| tool(it)).count();
            match run {
                0 | 1 => {
                    out.push(self.item(i, feed[i], cx));
                    i += 1;
                }
                n => {
                    out.push(self.group(i, &feed[i..i + n], cx));
                    i += n;
                }
            }
        }
        if let Some(text) = chat.current().and_then(|s| s.read(cx).queued.clone()) {
            out.push(self.user(feed.len(), text, cx));
        }
        out
    }

    /// Tool calls in a row: one line saying what ran, open while one runs or when one failed.
    fn group(&self, first: usize, items: &[(At, &Item)], cx: &App) -> AnyElement {
        let t = theme(cx);
        let tools: Vec<&Tool> = (items.iter())
            .filter_map(|(_, it)| match it {
                Item::Tool(t) => Some(t),
                _ => None,
            })
            .collect();
        let running = tools.iter().any(|t| ok(t).is_none());
        let failed = tools.iter().any(|t| ok(t) == Some(false));
        let Some(key) = tools.first().map(|t| t.id.clone()) else {
            return div().into_any_element();
        };
        let open = self.toggled.contains(&key) != (running || failed);
        let mut counts: Vec<(String, usize)> = vec![];
        for t in &tools {
            let name = t.kind.clone().unwrap_or_default();
            match counts.iter_mut().find(|(n, _)| *n == name) {
                Some((_, c)) => *c += 1,
                None => counts.push((name, 1)),
            }
        }
        let names = counts
            .iter()
            .map(|(n, c)| match c {
                1 => n.to_string(),
                c => format!("{n} ×{c}"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let summary = format!("{} actions · {names}", items.len());
        let icon = match (running, failed) {
            (true, _) => ui::spinner(t.accent).into_any_element(),
            (_, true) => div()
                .text_color(t.error)
                .child(Icon::Close.size(px(12.)))
                .into_any_element(),
            _ => div()
                .text_color(t.success)
                .child(Icon::Check.size(px(12.)))
                .into_any_element(),
        };
        let header = div()
            .id(("chat.group", first))
            .test_support()
            .aria_label(SharedString::from(summary.clone()))
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(4.))
            .rounded(radius::WIDGET)
            .text_size(text::SMALL)
            .text_color(t.muted)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover).text_color(t.fg))
            .on_click({
                let me = self.me.clone();
                move |_, _, cx| {
                    me.update(cx, |this, cx| {
                        if !this.toggled.remove(&key) {
                            this.toggled.insert(key.clone());
                        }
                        cx.notify();
                    })
                    .ok();
                }
            })
            .child(icon)
            .child(div().flex_1().min_w_0().truncate().child(summary))
            .child(match open {
                true => Icon::Down.size(px(12.)).into_any_element(),
                false => Icon::Chevron.size(px(12.)).into_any_element(),
            });
        let lines = open.then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .ml(px(14.))
                .pl(px(10.))
                .border_l_1()
                .border_color(t.line)
                .children(
                    items
                        .iter()
                        .enumerate()
                        .map(|(k, it)| self.item(first + k, *it, cx)),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(header)
            .children(lines)
            .into_any_element()
    }

    fn row(
        &self,
        i: usize,
        label: String,
    ) -> impl StatefulInteractiveElement + Styled + ParentElement + IntoElement + use<> {
        div()
            .id(("chat.item", i))
            .test_support()
            .aria_label(SharedString::from(label))
            .w_full()
            .min_w_0()
    }

    fn user(&self, i: usize, text: String, cx: &App) -> AnyElement {
        let t = theme(cx);
        div()
            .flex()
            .justify_end()
            .child(
                self.row(i, format!("You: {text}"))
                    .w_auto()
                    .max_w(relative(0.85))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(radius::FIELD)
                    .bg(t.hover)
                    .child(text),
            )
            .into_any_element()
    }

    fn item(&self, i: usize, (at, item): (At, &Item), cx: &App) -> AnyElement {
        let t = theme(cx);
        let line = |label: String, icon: AnyElement, name: String, detail: String| {
            self.row(i, label)
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
        let session = self.session.read(cx).sessions[at.0].entity_id();
        match item {
            Item::User(text) => self.user(i, text.clone(), cx),
            Item::Text(text) => {
                let body = match self.texts.get(&(session, at.1)) {
                    Some((_, md)) => md.clone().into_any_element(),
                    None => div().child(text.clone()).into_any_element(),
                };
                self.row(i, text.clone()).child(body).into_any_element()
            }
            Item::Thought(text) => self
                .row(i, format!("Thinking: {text}"))
                .text_size(text::SMALL)
                .text_color(t.muted)
                .italic()
                .child(text.clone())
                .into_any_element(),
            Item::Tool(tool) if tool.status.as_deref() == Some("denied") => {
                let icon = div()
                    .text_color(t.warning)
                    .child(Icon::Close.size(px(12.)))
                    .into_any_element();
                let (name, reason) = (kind(tool), output(tool));
                let label = format!("Blocked {name}: {reason}");
                line(label, icon, format!("Blocked {name}"), reason).into_any_element()
            }
            Item::Tool(tool) => {
                let (name, detail, output) = (kind(tool), title(tool), output(tool));
                let state = ok(tool);
                let icon = match state {
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
                let said = state.map_or("running", |ok| if ok { "ok" } else { "failed" });
                let label = format!("{name} {detail} {said}");
                let shown = self.outputs.contains(&tool.id);
                let key = tool.id.clone();
                let stats = tool.lines.map(|(add, del)| {
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(4.))
                        .font_family(theme::mono(cx))
                        .child(div().text_color(t.success).child(format!("+{add}")))
                        .child(div().text_color(t.error).child(format!("−{del}")))
                });
                let file = self.file_of(&name, &detail, cx);
                let head = line(label, icon, name, detail)
                    .children(stats)
                    .when(!output.is_empty(), |d| {
                        d.cursor_pointer()
                            .hover(|s| s.text_color(t.fg))
                            .child(match shown {
                                true => Icon::Down.size(px(12.)).into_any_element(),
                                false => Icon::Chevron.size(px(12.)).into_any_element(),
                            })
                            .on_click({
                                let me = self.me.clone();
                                move |_, _, cx| {
                                    me.update(cx, |this, cx| {
                                        if !this.outputs.remove(&key) {
                                            this.outputs.insert(key.clone());
                                        }
                                        cx.notify();
                                    })
                                    .ok();
                                }
                            })
                    })
                    .when_some(file, |d, loc| {
                        d.cursor_pointer()
                            .hover(|s| s.text_color(t.fg))
                            .on_click(move |_, _, cx| kernel::nav::open(cx, loc.clone()))
                    });
                let out = (shown && !output.is_empty()).then(|| {
                    div()
                        .id(("chat.output", i))
                        .test_support()
                        .aria_label(SharedString::from(output.clone()))
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .ml(px(18.))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(radius::WIDGET)
                        .bg(t.ground)
                        .text_size(text::SMALL)
                        .font_family(theme::mono(cx))
                        .whitespace_normal()
                        .child(output)
                });
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(head)
                    .children(out)
                    .into_any_element()
            }
            Item::End { stop, ms } => self.end(i, at, super::done(*stop), *ms, cx),
            Item::Ask {
                tool,
                answer,
                questions,
            } => self.ask(i, at, tool, *answer, questions, cx),
            Item::Changes { .. } => self.changes(i, at, item, cx),
            Item::Note(text) => self
                .row(i, text.clone())
                .text_size(text::SMALL)
                .text_color(t.warning)
                .child(text.clone())
                .into_any_element(),
            Item::Act(act) => {
                let text = format!("{act:?}");
                self.row(i, text.clone())
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(text)
                    .into_any_element()
            }
        }
    }

    /// The file a file tool worked on, to open on a click; None outside the repo.
    fn file_of(&self, name: &str, detail: &str, cx: &App) -> Option<Location> {
        if !matches!(
            name,
            "Read" | "Edit" | "MultiEdit" | "Write" | "NotebookEdit"
        ) {
            return None;
        }
        let repo = &self.session.read(cx).repo;
        let path = detail
            .strip_prefix(repo.as_str())?
            .trim_start_matches(['/', '\\']);
        Some(Location {
            repo: repo.clone(),
            rev: None,
            path: path.to_string(),
            line: None,
            doc: None,
        })
    }

    /// How the turn went and how long it took, and a button that copies its answer.
    fn end(&self, i: usize, at: At, ok: bool, ms: u64, cx: &App) -> AnyElement {
        let t = theme(cx);
        let s = self.session.read(cx).sessions[at.0].read(cx);
        let items = &s.items[..at.1];
        let from = items
            .iter()
            .rposition(|it| matches!(it, Item::User(_)))
            .map_or(0, |u| u + 1);
        let answer = items[from..]
            .iter()
            .filter_map(|it| match it {
                Item::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let tools = items[from..]
            .iter()
            .filter(|it| matches!(it, Item::Tool(_)))
            .count();
        let mut label = match ok {
            true => format!("Done in {}", seconds(ms)),
            false => format!("Stopped after {}", seconds(ms)),
        };
        if tools > 0 {
            label += &format!(" · {tools} actions");
        }
        let copy = (!answer.is_empty()).then(|| {
            ui::titled(
                ui::icon_button(("chat.copy", i), Icon::Copy.size(px(12.)), cx),
                "Copy the answer",
            )
            .test_support()
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(answer.clone()))
            })
        });
        div()
            .id(("chat.item", i))
            .test_support()
            .aria_label(SharedString::from(label.clone()))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(text::SMALL)
            .text_color(t.muted)
            .child(label)
            .children(copy)
            .into_any_element()
    }

    /// A permission prompt or the agent's questions, with the buttons that answer them.
    fn ask(
        &self,
        i: usize,
        at: At,
        tool: &Tool,
        answer: Answer,
        questions: &[Question],
        cx: &App,
    ) -> AnyElement {
        let t = theme(cx);
        let state = match answer {
            Answer::Waiting => "waiting",
            Answer::Once => "allowed",
            Answer::Always => "always allowed",
            Answer::Rejected => "denied",
            Answer::Gone => "no answer",
        };
        let asking = questions.is_empty();
        let title = match asking {
            true => format!("{} asks to use {}", CLAUDE.name, kind(tool)),
            false => format!("{} asks", CLAUDE.name),
        };
        let detail = title_of(tool, asking);
        let open = answer == Answer::Waiting;
        let key = (self.session.read(cx).sessions[at.0].entity_id(), at.1);
        let picked = self.picked.get(&key).cloned().unwrap_or_default();
        let picked = |q: usize| picked.get(q).cloned().unwrap_or_default();
        let button = |id: &'static str, label: &'static str, primary: bool, answer: Answer| {
            let session = self.session.clone();
            let b = match primary {
                true => ui::primary_button((id, i), label, cx),
                false => ui::button((id, i), label, cx),
            };
            b.test_support()
                .on_click(move |_, _, cx| session.update(cx, |s, cx| s.answer(at, answer, cx)))
        };
        let complete = (0..questions.len()).all(|q| !picked(q).is_empty());
        let answers: BTreeMap<String, String> = (questions.iter().enumerate())
            .map(|(q, question)| {
                let labels: Vec<&str> = (picked(q).iter())
                    .map(|o| question.options[*o].label.as_str())
                    .collect();
                (question.question.clone(), labels.join(", "))
            })
            .collect();
        let rows = questions.iter().enumerate().map(|(q, question)| {
            let options = question.options.iter().enumerate().map(|(o, c)| {
                let on = picked(q).contains(&o);
                let (me, multi) = (self.me.clone(), question.multi);
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
                    b.on_click(move |_, _, cx| {
                        me.update(cx, |v, cx| v.pick(key, q, o, multi, cx)).ok();
                    })
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
            let b = div().flex().gap(px(8.));
            match asking {
                true => b
                    .child(button("chat.allow", "Allow", true, Answer::Once))
                    .child(button("chat.always", "Always allow", false, Answer::Always))
                    .child(button("chat.deny", "Deny", false, Answer::Rejected)),
                false => {
                    let session = self.session.clone();
                    b.when(complete, |b| {
                        b.child(
                            ui::primary_button(("chat.allow", i), "Answer", cx)
                                .test_support()
                                .on_click(move |_, _, cx| {
                                    let answers = answers.clone();
                                    session.update(cx, |s, cx| s.answer_questions(at, answers, cx))
                                }),
                        )
                    })
                    .child(button(
                        "chat.deny",
                        "Skip",
                        false,
                        Answer::Rejected,
                    ))
                }
            }
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
                        .whitespace_normal()
                        .child(detail.clone()),
                )
            })
            .children(rows)
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

    fn pick(
        &mut self,
        key: (EntityId, usize),
        q: usize,
        o: usize,
        multi: bool,
        cx: &mut Context<Self>,
    ) {
        let all = self.picked.entry(key).or_default();
        if all.len() <= q {
            all.resize(q + 1, vec![]);
        }
        let p = &mut all[q];
        match p.iter().position(|x| *x == o) {
            Some(at) if multi => _ = p.remove(at),
            _ if multi => p.push(o),
            _ => *p = vec![o],
        }
        cx.notify();
    }

    /// What a turn changed, each file opening its diff, and going back to before it.
    fn changes(&self, i: usize, at: At, item: &Item, cx: &App) -> AnyElement {
        let Item::Changes {
            repo,
            start,
            end,
            files,
            back,
        } = item
        else {
            unreachable!()
        };
        let t = theme(cx);
        let busy = self.session.read(cx).busy(cx);
        let n = files.len();
        let mut title = match n {
            1 => "1 file changed in this turn".to_string(),
            _ => format!("{n} files changed in this turn"),
        };
        // A folder's chat says which of its repos.
        if self.session.read(cx).repo != *repo {
            title += &format!(" in {}", Workspace::get(cx).name_of(repo));
        }
        let rows = files.iter().enumerate().map(|(j, f)| {
            let doc = Doc::Diff(DiffDoc {
                repo: repo.to_string(),
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
                    Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx))
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
                    .on_click(move |_, _, cx| session.update(cx, |s, cx| s.back(at, run, cx)))
            };
        let small = |text: String| div().text_size(text::SMALL).text_color(t.muted).child(text);
        let restore = || {
            button(
                "chat.back",
                "Restore the files to before this turn".into(),
                Session::plan_back,
            )
            .into_any_element()
        };
        let (state, below) = match back {
            Back::Idle => ("", (!busy).then(restore)),
            Back::Failed(e) => (
                "failed",
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(small(format!("Cannot go back: {e}")))
                        .children((!busy).then(restore))
                        .into_any_element(),
                ),
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
                let confirm = (todo > 0 && !busy).then(|| {
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

    /// An empty chat: what to ask, the keys of the box, and the latest past conversations.
    fn empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let s = self.session.read(cx);
        let past = self.past.read(cx).value.iter().flatten();
        let mut past: Vec<&Past> = past.filter(|p| !p.archived()).collect();
        past.sort_by_key(|p| std::cmp::Reverse(p.modified));
        let rows: Vec<_> = past
            .into_iter()
            .take(PAST)
            .enumerate()
            .map(|(i, p)| {
                let (cwd, id) = (s.repo.clone(), p.id.clone());
                ui::row(("chat.past", i), false, cx)
                    .test_support()
                    .aria_label(SharedString::from(p.title.clone()))
                    .h(px(28.))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // Another chat that shows it comes forward; else it opens here.
                        let open = super::open(cx);
                        match open.iter().any(|(.., s)| s.read(cx).shows(&id, cx)) {
                            true => super::resume(cwd.clone(), id.clone(), cx),
                            false => {
                                this.session.update(cx, |s, cx| s.open_past(id.clone(), cx));
                                this.focus(window, cx);
                            }
                        }
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(p.title.clone()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(text::SMALL)
                            .text_color(t.muted)
                            .child(age(now() - p.modified as i64 / 1000)),
                    )
            })
            .collect();
        let hint = |key: &'static str, what: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(ui::kbd(key, cx))
                .child(what)
        };
        let recent = (!rows.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .w_full()
                .pt(px(16.))
                .child(
                    div()
                        .px(px(10.))
                        .pb(px(4.))
                        .text_size(text::LABEL)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.muted)
                        .child("Recent conversations"),
                )
                .children(rows)
        });
        div()
            .id("chat.empty")
            .test_support()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
            .w_full()
            .max_w(px(440.))
            .mx_auto()
            .child(div().text_color(t.accent).child(Icon::Chat.size(px(20.))))
            .child(
                div()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!(
                        "Ask {} about {}",
                        CLAUDE.name,
                        super::label(&s.repo, cx)
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(12.))
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(hint("/", "commands"))
                    .child(hint("@", "files"))
                    .child(hint("Enter", "sends"))
                    .child(hint("Shift Enter", "new line")),
            )
            .children(recent)
            .into_any_element()
    }

    fn footer(&self, cx: &App) -> Option<AnyElement> {
        let t = theme(cx);
        let chat = self.session.read(cx);
        let s = chat.current()?.read(cx);
        let stops = !s.lacks.contains(&Piece::StopTask);
        let tasks = s.tasks.iter().enumerate().map(|(i, task)| {
            let (session, id) = (chat.current().cloned(), task.id.clone());
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
                .when(stops, |d| {
                    d.child(
                        ui::button(("chat.task.stop", i), "Stop", cx)
                            .test_support()
                            .on_click(move |_, _, cx| {
                                if let Some(s) = &session {
                                    s.read(cx).stop_task(&id);
                                }
                            }),
                    )
                })
        });
        let stop = self.session.clone();
        let working = s.busy().then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(ui::spinner(t.accent))
                .child(div().flex_1().text_color(t.muted).child("Working"))
                .child(
                    ui::button("chat.stop", "Stop", cx)
                        .test_support()
                        .on_click(move |_, _, cx| stop.update(cx, |s, cx| s.stop(cx))),
                )
        });
        (s.busy() || !s.tasks.is_empty()).then(|| {
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
            let text = "Commands show once the session has started.";
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

/// A model by name: the label the session listed for it, else "Opus 4.5" from
/// "claude-opus-4-5-20251101".
fn model_name(id: &str, models: &[Model]) -> String {
    if let Some(m) = models.iter().find(|m| m.value == id) {
        return m.label.clone();
    }
    let (id, wide) = match id.strip_suffix("[1m]") {
        Some(id) => (id, " (1M)"),
        None => (id, ""),
    };
    let mut parts = id.trim_start_matches("claude-").split('-');
    let Some(name) = parts.next().filter(|n| !n.is_empty()) else {
        return id.to_string();
    };
    let version: Vec<_> = parts.take_while(|p| p.len() <= 2).collect();
    let mut name: String = name[..1].to_uppercase() + &name[1..];
    if !version.is_empty() {
        name = format!("{name} {}", version.join("."));
    }
    name + wide
}

fn kind(t: &Tool) -> String {
    t.kind.clone().unwrap_or_default()
}

fn title(t: &Tool) -> String {
    t.title.clone().unwrap_or_default()
}

/// What an ask is about: its title, else its whole input, which the host shows and never reads.
fn title_of(t: &Tool, asking: bool) -> String {
    match (title(t), &t.input) {
        (title, Some(input)) if title.is_empty() && asking => input.to_string(),
        (title, _) => title,
    }
}

/// The end of what a call printed, or why it was denied.
fn output(t: &Tool) -> String {
    match &t.output {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

/// Whether a call went well; None while it runs.
fn ok(t: &Tool) -> Option<bool> {
    match t.status.as_deref() {
        Some("completed") => Some(true),
        Some("failed") => Some(false),
        _ => None,
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
        self.lay_out(cx);
        let items = self.feed(cx);
        let empty = items.is_empty().then(|| self.empty(cx));
        let review = self.review(cx);
        let footer = self.footer(cx);
        let suggestions = self.suggestions(cx);
        let bar = self.bar(cx);
        // A column to read in, as wide as a line of text should be.
        let column = || div().flex().flex_col().w_full().max_w(px(860.)).mx_auto();
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
                    .px(px(20.))
                    .py(px(16.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        column()
                            .gap(px(12.))
                            .when(empty.is_some(), |c| c.flex_1().justify_center())
                            .children(items)
                            .children(empty),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .pt(px(10.))
                    .pb(px(8.))
                    .border_t_1()
                    .border_color(t.line)
                    .child(
                        column()
                            .id("chat.input")
                            .test_support()
                            .gap(px(8.))
                            .children(review)
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
                            .capture_action(
                                cx.listener(|this, _: &MoveUp, _, cx| this.step(false, cx)),
                            )
                            .capture_action(
                                cx.listener(|this, _: &MoveDown, _, cx| this.step(true, cx)),
                            )
                            .child(Textarea::new(&self.input))
                            .child(bar),
                    ),
            )
    }
}

fn seconds(ms: u64) -> String {
    match ms {
        0..60_000 => format!("{:.1} s", ms as f64 / 1000.),
        _ => format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000),
    }
}

/// A plan limit as a short bar of what is used, with what is left and when it resets.
fn limit(l: &Limit, cx: &App) -> impl IntoElement + use<> {
    let t = theme(cx);
    let used = l.used.clamp(0., 100.);
    let color = match used {
        u if u >= 90. => t.error,
        u if u >= 70. => t.warning,
        _ => t.accent,
    };
    let name = match l.name.as_str() {
        "5h" => "5-hour",
        "Week" => "Weekly",
        n => n,
    };
    let now = now();
    let mut tip = format!("{name} limit: {used:.0}% used");
    if let Some(r) = l.resets {
        tip += &format!(", resets in {}", span(r as i64 / 1000 - now));
    }
    let left = format!("{:.0}% left", 100. - used);
    div()
        .id(SharedString::from(format!("chat.limit.{}", l.name)))
        .test_support()
        .aria_label(SharedString::from(format!("{} {left}", l.name)))
        .flex()
        .items_center()
        .gap(px(5.))
        .px(px(6.))
        .py(px(2.))
        .rounded(radius::WIDGET)
        .text_color(t.muted)
        .hover(|s| s.bg(t.hover).text_color(t.fg))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(l.name.clone())
        .child(
            div().w(px(36.)).h(px(4.)).rounded_full().bg(t.line).child(
                div()
                    .h_full()
                    .rounded_full()
                    .bg(color)
                    .w(relative(used as f32 / 100.)),
            ),
        )
        .child(left)
}

#[cfg(test)]
mod tests {
    use super::model_name;

    #[test]
    fn a_model_id_reads_as_its_family_and_version() {
        assert_eq!(model_name("claude-opus-4-5-20251101", &[]), "Opus 4.5");
        assert_eq!(
            model_name("claude-sonnet-4-5-20250929[1m]", &[]),
            "Sonnet 4.5 (1M)"
        );
        assert_eq!(model_name("claude-fable-5-1", &[]), "Fable 5.1");
        assert_eq!(model_name("good", &[]), "Good");
    }
}
