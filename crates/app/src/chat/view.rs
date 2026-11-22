//! The chat page: what is set for the session, the conversation, and the box to write in.

use std::collections::{BTreeMap, HashMap, HashSet};

use detached_core::api;
use detached_core::backend::{Limit, Model, Past, Piece, Question, Setting, Tool, TurnConfig};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, InputEvent, MoveDown, MoveUp, Textarea, TextareaState,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardItem, Context, Div, Entity, EntityId, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity, Window, div,
    prelude::FluentBuilder, px, relative,
};

use detached_core::docs::Need;
use detached_core::git::snapshot::Skip;
use detached_core::host::Act;
use detached_core::types::Side;

use super::session::Chat;
use crate::files::{Links, Markdown};
use crate::ui::{self, Icon, Menu, radius, text, theme};
use crate::util::{age, now, span};
use detached_core::adapters::CLAUDE;
use kernel::blocking;
use kernel::doc::{DiffDoc, Doc, Location};
use kernel::inbox::{self, Messages};
use kernel::resource::Interest;
use kernel::session::{AUTO, Answer, Back, Item, Session};
use kernel::settings::Settings;
use kernel::workspace::Workspace;

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
    /// The past conversations of the provider it names.
    past: (&'static str, Interest<Vec<Past>>),
    /// The provider the box's placeholder names.
    named: &'static str,
    me: WeakEntity<ChatView>,
    /// A `/` typed before any session listed its commands.
    unlisted: bool,
    /// Entries before this one were there on open and do not animate in.
    seen: usize,
    /// The `/clear` whose earlier entries show.
    earlier: Option<usize>,
    _files: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatView {
    pub fn new(session: Entity<Chat>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let p = session.read(cx).provider;
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(3)
                .placeholder(placeholder(p.name))
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
            cx.observe(&Messages::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&Settings::global(cx), |this: &mut ChatView, _, cx| {
                if this.session.read(cx).current().is_none() {
                    this.session.update(cx, |s, cx| {
                        if s.untouched(cx) {
                            s.refresh_defaults(cx);
                        }
                        s.ensure(cx);
                        if s.current().is_some() {
                            s.check_trust(cx);
                        }
                    });
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
        let past = Interest::view(kernel::session::past_of(p, &repo, cx), cx);
        scroll.scroll_to_bottom();
        let seen = session.read(cx).feed(cx).len();
        ChatView {
            session,
            input,
            scroll,
            suggest: None,
            toggled: HashSet::new(),
            outputs: HashSet::new(),
            texts: HashMap::new(),
            picked: HashMap::new(),
            past: (p.id, past),
            named: p.id,
            me: cx.weak_entity(),
            unlisted: false,
            seen,
            earlier: None,
            _files: None,
            _subscriptions: subscriptions,
        }
    }

    /// Follows a change of the chat's provider: the box's placeholder and the past conversations.
    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat = self.session.read(cx);
        let (p, repo) = (chat.provider, chat.repo.clone());
        if self.named != p.id {
            self.named = p.id;
            let text = placeholder(p.name);
            self.input
                .update(cx, |s, cx| s.set_placeholder(text, window, cx));
        }
        if self.past.0 != p.id {
            let past = kernel::session::past_of(p, &repo, cx);
            self.past = (p.id, Interest::view(past, cx));
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
        if self.session.read(cx).current().is_none() && !super::can_start(cx) {
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
            .child(
                div()
                    .flex_none()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(name),
            )
            .child(div().flex_1())
            .child(
                ui::titled(ui::icon_button("chat.new", Icon::Plus, cx), "New chat")
                    .test_support()
                    .on_click(move |_, _, cx| {
                        let repo = session.read(cx).repo.clone();
                        super::new_chat(repo, cx)
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
                ui::icon_button(id, div().child(label.clone()), cx)
                    .h(px(24.))
                    .px(px(8.))
                    .children(chevron),
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
        let p = chat.provider;
        // The session's own mode once it says one: a refused switch leaves it as it was.
        let now = s
            .and_then(|s| s.option("mode"))
            .unwrap_or_else(|| chat.mode());
        let mode = (p.modes.iter())
            .find(|(m, ..)| *m == now)
            .or(p.modes.iter().find(|(m, ..)| *m == chat.mode()))
            .unwrap_or(&p.modes[0]);
        let modes = (p.modes.iter())
            .map(|(m, label, _)| (label.to_string(), Some(Setting::Mode(m.to_string()))))
            .collect();
        let modes = menu("chat.mode", mode.2, mode.1.to_string(), modes);
        let models: &[Model] = s.map_or(&[], |s| &s.models);
        let has_auto = p.modes.iter().any(|(m, ..)| *m == AUTO);
        let entries = models
            .iter()
            .map(|m| {
                let label = match m.auto || !has_auto {
                    true => m.label.clone(),
                    false => format!("{} (no auto)", m.label),
                };
                (label, Some(Setting::Model(m.value.clone())))
            })
            .collect();
        let current = match s {
            Some(s) if s.version.is_some() => s.option("model"),
            _ => chat.model.as_deref(),
        };
        let model = current.map_or("Default model".into(), |m| {
            model_name(m, models).map_or_else(|| friendly_model_id(m), String::from)
        });
        let picker = menu("chat.model", "Model", model, entries);
        let efforts = current.and_then(|id| Model::find(models, id));
        let efforts = efforts.map_or(&[][..], |m| &m.efforts);
        // An agent that lists efforts as an option of the session, not per model.
        let listed = s.and_then(|s| s.config.iter().find(|c| c.id == "effort"));
        let efforts = match (efforts.is_empty(), listed) {
            (true, Some(c)) => &c.values[..],
            _ => efforts,
        };
        let efforts: Vec<_> = (efforts.iter())
            .map(|e| (e.clone(), Some(Setting::Effort(e.clone()))))
            .collect();
        let effort = (!efforts.is_empty()).then(|| {
            let now = match s {
                Some(s) if s.version.is_some() => s.option("effort").filter(|e| !e.is_empty()),
                _ => chat.effort.as_deref(),
            };
            let label = now.unwrap_or("Effort");
            menu("chat.effort", "Effort", label.to_string(), efforts)
        });
        let usage = s.and_then(|s| s.usage.filter(|(_, max)| *max > 0).map(|u| (u, &s.parts)));
        let usage = usage.map(|((used, max), parts)| {
            let parts: Vec<String> = (parts.iter())
                .map(|p| format!("{} {}", p.name, tokens(p.tokens)))
                .collect();
            let pct = used * 100 / max;
            let label = format!("Context {pct}%");
            let entries = parts.iter().map(|p| (p.clone(), None)).collect();
            menu(
                "chat.usage",
                &format!("{} of {}", tokens(used), tokens(max)),
                label.clone(),
                entries,
            )
            .aria_label(SharedString::from(label))
            .aria_description(SharedString::from(parts.join(", ")))
            .child(meter(pct as f32, pct as f32, cx))
        });
        let account = s.and_then(|s| s.account.as_ref());
        let plan = account.map(|a| SharedString::from(a.plan.clone()));
        let tip = account.map(|a| match &a.email {
            Some(e) => SharedString::from(format!("{} · {e}", a.plan)),
            None => SharedString::from(a.plan.clone()),
        });
        // Another provider is picked while nothing is said.
        let others: Vec<_> = (super::providers(cx).into_iter())
            .filter(|o| o.id != p.id && chat.is_new(cx))
            .collect();
        let pick = (!others.is_empty()).then(|| {
            let chat = self.session.clone();
            move |e: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
                let menu = others.iter().fold(Menu::default(), |menu, o| {
                    let (chat, o) = (chat.clone(), *o);
                    menu.item(o.name, move |_, cx| {
                        chat.update(cx, |c, cx| c.set_provider(o, cx))
                    })
                });
                menu.show(e.position(), window, cx);
            }
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
                    .aria_label(p.name)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(p.name),
            )
            .when_some(pick, |d, pick| {
                let button = ui::icon_button("chat.provider.pick", Icon::Down.size(px(10.)), cx);
                d.child(
                    ui::titled(button, "Who answers")
                        .test_support()
                        .on_click(pick),
                )
            })
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
        let session = self.session.clone();
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
            .child(self.messages(cx))
            .child(
                ui::icon_button("chat.save-default", "Save as default", cx)
                    .aria_label("Save as default")
                    .test_support()
                    .on_click(move |_, _, cx| {
                        session.update(cx, |s, cx| s.save_defaults(cx));
                    }),
            )
            .child(div().flex_1())
            .children(usage)
            .children(limits)
            .children(unknown)
    }

    /// The group whose sessions this chat's agent messages, and whether new messages wake it.
    fn messages(&self, cx: &App) -> AnyElement {
        let chat = self.session.read(cx);
        let id = chat.current().and_then(|s| s.read(cx).id.clone());
        let m = Messages::get(cx);
        let mine = id
            .as_deref()
            .and_then(|id| m.group(id))
            .map(|g| g.name.clone());
        let wake = id.as_deref().is_some_and(|id| m.wake.contains(id));
        let label = match (&mine, wake) {
            (Some(g), true) => format!("{g} · wakes"),
            (Some(g), false) => g.clone(),
            (None, true) => "Wakes".into(),
            (None, false) => "No group".into(),
        };
        let groups: Vec<(String, usize)> = (m.groups.iter())
            .filter(|g| Some(&g.name) != mine.as_ref())
            .map(|g| (g.name.clone(), g.members.len()))
            .collect();
        let fresh = m.fresh_name();
        ui::titled(
            ui::icon_button("chat.group", div().child(label.clone()), cx)
                .h(px(24.))
                .px(px(8.))
                .child(Icon::Down.size(px(10.))),
            "Messages between agents",
        )
        .aria_description(SharedString::from(label))
        .test_support()
        .on_click(move |e, window, cx| {
            let on = id.is_some();
            let id = id.clone().unwrap_or_default();
            let join = |name: Option<String>| {
                let id = id.clone();
                move |_: &mut Window, cx: &mut App| inbox::join(cx, &id, name.clone())
            };
            let mut menu = Menu::default();
            for (name, n) in &groups {
                let label = format!("Join {name} ({n})");
                menu = menu.item_if(on, label, join(Some(name.clone())));
            }
            let wakes = match wake {
                true => "Stop waking on messages",
                false => "Wake on messages",
            };
            let id = id.clone();
            menu.item_if(on, format!("New group: {fresh}"), join(Some(fresh.clone())))
                .item_if(mine.is_some(), "Leave the group", join(None))
                .separator()
                .item_if(on, wakes, move |_, cx| inbox::set_wake(cx, &id, !wake))
                .show(e.position(), window, cx);
        })
        .into_any_element()
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
                _ => cx.new(|cx| Markdown::new(text.clone(), Links::Agent, true, cx)),
            };
            self.texts.insert(key, (text, md));
        }
    }

    /// The items, with each run of tool calls folded into one group.
    fn feed(&self, cx: &App) -> Vec<AnyElement> {
        let chat = self.session.read(cx);
        let feed = chat.feed(cx);
        let tool = |it: &Item| matches!(it, Item::Tool(t) if t.status.as_deref() != Some("denied"));
        // Widest before a turn (None past the end is the queued message), tight between actions
        // and above a turn's end.
        let gap = |i: usize| match (
            i.checked_sub(1).map(|p| feed[p].1),
            feed.get(i).map(|f| f.1),
        ) {
            (None, _) => 0.,
            (_, Some(Item::User(_)) | None) => 24.,
            (Some(Item::Tool(_)), Some(Item::Tool(_)))
            | (_, Some(Item::End { .. } | Item::PastTurn(_))) => 8.,
            _ => 16.,
        };
        let cut = chat.cleared(&feed, cx);
        let open = cut.is_some() && self.earlier == cut;
        let mut divider = cut.map(|_| self.divider(open, cut, cx));
        let mut out = vec![];
        let mut push = |i: usize, el: AnyElement| {
            if Some(i) == cut {
                out.extend(divider.take());
            }
            let el = div().flex().flex_col().pt(px(gap(i))).child(el);
            out.push(match i >= self.seen {
                true => ui::appear(("chat.in", i), el, 4., ui::motion::BASE).into_any_element(),
                false => el.into_any_element(),
            });
        };
        let mut i = cut.filter(|_| !open).unwrap_or(0);
        while i < feed.len() {
            match feed[i..].iter().take_while(|(_, it)| tool(it)).count() {
                0 | 1 => {
                    push(i, self.item(i, feed[i], cx));
                    i += 1;
                }
                n => {
                    push(i, self.group(i, &feed[i..i + n], cx));
                    i += n;
                }
            }
        }
        if let Some(text) = chat.current().and_then(|s| s.read(cx).queued.clone()) {
            push(feed.len(), self.user(feed.len(), text, cx));
        }
        out
    }

    /// "Context cleared" over the last `/clear`, folding what came before it.
    fn divider(&self, open: bool, cut: Option<usize>, cx: &App) -> AnyElement {
        let t = theme(cx);
        let me = self.me.clone();
        let action = if open { "Hide earlier" } else { "Show earlier" };
        let rule = || div().flex_1().h(px(1.)).bg(t.line);
        div()
            .id("chat.earlier")
            .test_support()
            .role(Role::Button)
            .aria_label(action)
            .flex()
            .items_center()
            .gap(px(8.))
            .when(open, |d| d.pt(px(24.)))
            .text_size(text::LABEL)
            .text_color(t.muted)
            .cursor_pointer()
            .hover(|s| s.text_color(t.fg))
            .on_click(move |_, _, cx| {
                _ = me.update(cx, |v, cx| {
                    v.earlier = if open { None } else { cut };
                    cx.notify();
                })
            })
            .child(rule())
            .child(format!("Context cleared · {action}"))
            .child(rule())
            .into_any_element()
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
                .gap(px(6.))
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
                    .bg(t.selected_inactive)
                    .line_height(relative(theme::doc::LINE_HEIGHT))
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
                        .font_weight(FontWeight::MEDIUM)
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
                    None => div()
                        .text_size(theme::doc::BODY)
                        .child(text.clone())
                        .into_any_element(),
                };
                self.row(i, text.clone())
                    .line_height(relative(theme::doc::LINE_HEIGHT))
                    .child(body)
                    .into_any_element()
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
            Item::End { stop, ms, config } => {
                self.end(i, at, super::done(*stop), Some(*ms), config, cx)
            }
            Item::PastTurn(config) => self.end(i, at, true, None, config, cx),
            Item::Ask {
                tool,
                answer,
                questions,
            } => self.ask(i, at, tool, *answer, questions, cx),
            Item::Changes { .. } => self.changes(i, at, item, cx),
            Item::Docs { repo, needs } => self.docs(i, repo, needs, cx),
            Item::Note(text) => self
                .row(i, text.clone())
                .text_size(text::SMALL)
                .text_color(t.warning)
                .child(text.clone())
                .into_any_element(),
            Item::Woken => self
                .row(i, "Woken by new messages".into())
                .text_size(text::SMALL)
                .text_color(t.muted)
                .child("Woken by new messages")
                .into_any_element(),
            Item::Act(act) => {
                let text = match act {
                    Act::Proc(p) => format!("{:?} the process {} in {}", p.op, p.name, p.repo),
                    Act::Send(m) => format!("To {}: {}", super::name_of(&m.to, cx), m.text),
                    Act::Watch(w) => match &w.process {
                        Some(p) => format!("Watching the process {p} in {}", w.repo),
                        None => format!("Watching the pipeline in {}", w.repo),
                    },
                    act => format!("{act:?}"),
                };
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
    fn end(
        &self,
        i: usize,
        at: At,
        ok: bool,
        ms: Option<u64>,
        config: &TurnConfig,
        cx: &App,
    ) -> AnyElement {
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
        let mut details = vec![];
        if let Some(model) = &config.model {
            details.push(
                model_name(model, &s.models).map_or_else(|| friendly_model_id(model), String::from),
            );
        }
        if let Some(effort) = &config.effort {
            let mut chars = effort.chars();
            details.push(chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().to_string() + chars.as_str()
            }));
        }
        if let Some(ms) = ms {
            details.push(match ok {
                true => format!("Done in {}", seconds(ms)),
                false => format!("Stopped after {}", seconds(ms)),
            });
        }
        if tools > 0 {
            details.push(format!("{tools} actions"));
        }
        let label = details.join(" · ");
        let copy = (!answer.is_empty()).then(|| {
            ui::titled(
                ui::icon_button(("chat.copy", i), Icon::Copy.size(px(12.)), cx),
                "Copy the answer",
            )
            .test_support()
            // Transparent, not invisible: a hidden button takes no clicks.
            .opacity(0.)
            .group_hover(ui::HOVER, |s| s.opacity(1.))
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(answer.clone()))
            })
        });
        div()
            .id(("chat.item", i))
            .test_support()
            .aria_label(SharedString::from(label.clone()))
            .group(ui::HOVER)
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(text::LABEL)
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
        let name = self.session.read(cx).provider.name;
        let title = match asking {
            true => format!("{name} asks to use {}", kind(tool)),
            false => format!("{name} asks"),
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

    /// The docs whose covered files the turn changed, each opening on a click, and a button that
    /// asks the session to update them.
    fn docs(&self, i: usize, repo: &str, needs: &[Need], cx: &App) -> AnyElement {
        let t = theme(cx);
        let label = needs.iter().map(|n| format!("`{}` {}", n.doc, n.section));
        let label = format!("Docs may need: {}", label.collect::<Vec<_>>().join(", "));
        let docs = needs.iter().enumerate().map(|(j, n)| {
            let loc = Location {
                repo: repo.into(),
                rev: None,
                path: n.doc.clone(),
                line: None,
                doc: None,
            };
            div()
                .id(SharedString::from(format!("chat.docs.{i}.{j}")))
                .cursor_pointer()
                .font_family(theme::mono(cx))
                .hover(|s| s.text_color(t.fg))
                .on_click(move |_, _, cx| kernel::nav::open(cx, loc.clone()))
                .child(format!("{} {}", n.doc, n.section))
        });
        let prompt = kernel::docs::prompt(repo, needs);
        let chat = self.session.clone();
        let ask = (!chat.read(cx).busy(cx)).then(|| {
            ui::button(("chat.docs.ask", i), "Ask to update them", cx)
                .test_support()
                .on_click(move |_, _, cx| chat.update(cx, |c, cx| c.send(prompt.clone(), cx)))
        });
        self.row(i, label)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .text_size(text::SMALL)
            .text_color(t.muted)
            .child("Docs may need:")
            .children(docs)
            .children(ask)
            .into_any_element()
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
        let chat = self.session.read(cx);
        let p = chat.review.as_ref()?;
        let until = match chat.provider.id == CLAUDE.id {
            true => "Until you trust them, sessions here load only your own settings, so the repo's CLAUDE.md and .claude are skipped too.".to_string(),
            false => format!("{} loads them itself, so it runs here only once you trust them.", chat.provider.name),
        };
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
            .child(div().font_weight(FontWeight::SEMIBOLD).child("This repo's agent settings would run"))
            .children(p.items.iter().map(|i| {
                div().text_size(text::SMALL).font_family(theme::mono(cx)).child(i.clone())
            }))
            .child(small(&until))
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
        let enabled = super::provider_config(cx).shows(s.provider);
        if !enabled && s.current().is_none() {
            return div()
                .id("chat.empty")
                .test_support()
                .child("Enable a provider in Settings › Agents to start a chat.")
                .into_any_element();
        }
        let past = self.past.1.read(cx).value.iter().flatten();
        let mut past: Vec<&Past> = past.filter(|p| enabled && !p.archived()).collect();
        past.sort_by_key(|p| std::cmp::Reverse(p.modified));
        let rows: Vec<_> = past
            .into_iter()
            .take(PAST)
            .enumerate()
            .map(|(i, p)| {
                let (cwd, id, provider) = (s.repo.clone(), p.id.clone(), s.provider);
                ui::row(("chat.past", i), false, cx)
                    .test_support()
                    .aria_label(SharedString::from(p.title.clone()))
                    .h(px(28.))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // Another chat that shows it comes forward; else it opens here.
                        let open = super::open(cx);
                        match open.iter().any(|(.., s)| s.read(cx).shows(&id, cx)) {
                            true => super::resume(cwd.clone(), provider, id.clone(), cx),
                            false if super::provider_config(cx).shows(provider) => {
                                let id = id.clone();
                                this.session
                                    .update(cx, |s, cx| s.open_past(provider, id, cx));
                                this.focus(window, cx);
                            }
                            false => (),
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
                        s.provider.name,
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

fn model_name<'a>(id: &str, models: &'a [Model]) -> Option<&'a str> {
    Model::find(models, id).map(|m| m.label.as_str())
}

fn friendly_model_id(id: &str) -> String {
    if id.contains('-') && !id.starts_with("claude-") {
        return id.to_string();
    }
    let (id, wide) = match id.strip_suffix("[1m]") {
        Some(id) => (id, " (1M)"),
        None => (id, ""),
    };
    let mut parts = id.trim_start_matches("claude-").split('-');
    let mut name = parts.next().unwrap_or_default().chars();
    let Some(first) = name.next() else {
        return id.to_string();
    };
    let version: Vec<_> = parts.take_while(|p| p.len() <= 2).collect();
    let mut name = first.to_uppercase().to_string() + name.as_str();
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        self.follow(window, cx);
        self.lay_out(cx);
        let items = self.feed(cx);
        let empty = items.is_empty().then(|| self.empty(cx));
        let review = self.review(cx);
        let footer = self.footer(cx);
        let suggestions = self.suggestions(cx);
        let bar = self.bar(cx);
        // A column to read in, as wide as a line of text should be.
        let column = || div().flex().flex_col().w_full().max_w(px(760.)).mx_auto();
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
                    .pt(px(16.))
                    .pb(px(24.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        column()
                            .when(empty.is_some(), |c| c.flex_1().justify_center())
                            .children(items)
                            .children(empty),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(20.))
                    .pt(px(12.))
                    .pb(px(10.))
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
                            .child(
                                Textarea::new(&self.input)
                                    .rounded(radius::FIELD)
                                    .text_size(theme::doc::BODY),
                            )
                            .child(bar),
                    ),
            )
    }
}

fn placeholder(name: &str) -> String {
    format!("Message {name}…")
}

fn seconds(ms: u64) -> String {
    match ms {
        0..60_000 => format!("{:.1} s", ms as f64 / 1000.),
        _ => format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000),
    }
}

/// A plan limit as a short bar of what is left and when it resets.
fn limit(l: &Limit, cx: &App) -> impl IntoElement + use<> {
    let t = theme(cx);
    let used = l.used.clamp(0., 100.);
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
    let remaining = 100. - used;
    let left = format!("{remaining:.0}% left");
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
        .child(meter(remaining as f32, used as f32, cx))
        .child(left)
}

fn meter(fill: f32, used: f32, cx: &App) -> Div {
    let t = theme(cx);
    let color = match used {
        u if u >= 90. => t.error,
        u if u >= 70. => t.warning,
        _ => t.accent,
    };
    div()
        .flex_none()
        .w(px(36.))
        .h(px(4.))
        .rounded_full()
        .bg(t.line)
        .child(
            div()
                .id("chat.meter.fill")
                .test_support()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(relative(fill.clamp(0., 100.) / 100.)),
        )
}

#[cfg(test)]
mod tests {
    use super::friendly_model_id;

    #[test]
    fn a_model_id_reads_as_its_family_and_version() {
        assert_eq!(friendly_model_id("claude-opus-4-5-20251101"), "Opus 4.5");
        assert_eq!(
            friendly_model_id("claude-sonnet-4-5-20250929[1m]"),
            "Sonnet 4.5 (1M)"
        );
        assert_eq!(friendly_model_id("claude-fable-5-1"), "Fable 5.1");
        assert_eq!(friendly_model_id("good"), "Good");
    }
}
