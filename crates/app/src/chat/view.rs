//! The chat page: what is set for the session, the conversation, and the box to write in.

use std::collections::{BTreeMap, HashMap, HashSet};

use detached_core::api;
use detached_core::backend::{Limit, Model, Past, Piece, Question, Setting, Tool, TurnConfig};
use gpui_kit::base::{Button, TestSupportExt};
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, InputEvent, InputState, MoveDown, MoveUp, Textarea, TextareaState,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext, ClipboardItem, Context, Div, Entity, EntityId,
    FocusHandle, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Render, Role, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, WeakEntity, Window, canvas, div, prelude::FluentBuilder, px, relative,
};

use detached_core::docs::Need;
use detached_core::git::snapshot::Skip;
use detached_core::host::Act;
use detached_core::types::Side;

use super::session::Chat;
use crate::files::{Links, Markdown};
use crate::ui::theme::chat as size;
use crate::ui::{self, DrawnMenu, Icon, radius, text, theme};
use crate::util::{age, folder_name, now, span, status_of};
use detached_core::adapters::{CLAUDE, LoginStatus, Provider};
use detached_core::backend::Name;
use detached_core::chat_settings::Preset;
use kernel::agents::Agents;
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

type AskKey = (EntityId, usize);

#[derive(Default)]
struct Choice {
    options: Vec<usize>,
    other: Option<(Entity<InputState>, Subscription)>,
}

pub struct ChatView {
    session: Entity<Chat>,
    input: Entity<TextareaState>,
    composer_focus: FocusHandle,
    provider_focus: FocusHandle,
    /// The agent's row the keys are on while its menu is open.
    menu: Option<usize>,
    /// How many of `size::INLINE` and `size::WEEK` the composer is as wide as.
    room: usize,
    scroll: ScrollHandle,
    suggest: Option<Suggest>,
    /// Groups of tool calls opened or closed against their default, by their first call's id.
    toggled: HashSet<String>,
    /// Tool calls whose output shows.
    outputs: HashSet<String>,
    /// Each whole text laid out, with the text it was laid out from.
    texts: HashMap<(EntityId, usize), (String, Entity<Markdown>)>,
    picked: HashMap<AskKey, Vec<Choice>>,
    changed_files: HashSet<At>,
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
                .auto_grow(1, 8)
                .placeholder(placeholder(p.name))
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let composer_focus = cx.focus_handle();
        let agents = Agents::global(cx);
        agents.update(cx, |a, cx| a.ensure_loaded(cx));
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
            cx.observe(&agents, |_, _, cx| cx.notify()),
            cx.on_focus_in(&composer_focus, window, |_, _, cx| cx.notify()),
            cx.on_focus_out(&composer_focus, window, |_, _, _, cx| cx.notify()),
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
        let past = Interest::view(kernel::session::past_of(p, &repo, false, cx), cx);
        scroll.scroll_to_bottom();
        let seen = session.read(cx).feed(cx).len();
        ChatView {
            session,
            input,
            composer_focus,
            provider_focus: cx.focus_handle(),
            menu: None,
            room: 0,
            scroll,
            suggest: None,
            toggled: HashSet::new(),
            outputs: HashSet::new(),
            texts: HashMap::new(),
            picked: HashMap::new(),
            changed_files: HashSet::new(),
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
            let past = kernel::session::past_of(p, &repo, false, cx);
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
        let icon = match Workspace::get(cx).repos().iter().any(|r| r.path == s.repo) {
            true => Icon::Repo,
            false => Icon::Folder,
        };
        let topic = s
            .topic(cx)
            .and_then(|t| t.lines().next())
            .unwrap_or("New chat")
            .to_string();
        ui::band(div(), cx)
            .id("chat.header")
            .test_support()
            .aria_label(SharedString::from(format!("{topic} {name}")))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(44.))
            .px(px(12.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(topic),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(icon.size(px(12.)))
                    .child(name),
            )
    }

    fn textarea(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
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
            .child(
                Textarea::new(&self.input)
                    .appearance(false)
                    .text_size(theme::doc::BODY),
            )
    }

    /// One row under the text: Agent, titled Model, Effort and Permissions, More, then the week's limit
    /// and Send or Stop. Narrower, Week goes into More, then Model and Effort.
    fn bar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let chat = self.session.read(cx);
        let s = chat.current().map(|s| s.read(cx));
        let p = chat.provider;
        let pick = |setting: Setting| {
            let (session, me) = (self.session.clone(), self.me.clone());
            move |window: &mut Window, cx: &mut App| {
                session.update(cx, |s, cx| s.set(setting.clone(), cx));
                me.update(cx, |this, cx| this.focus(window, cx)).ok();
            }
        };
        // The session's own mode once it says one: a refused switch leaves it as it was.
        let mode_id = s
            .and_then(|s| s.option("mode"))
            .unwrap_or_else(|| chat.mode());
        let mode = (p.modes.iter())
            .find(|(m, ..)| *m == mode_id)
            .or(p.modes.iter().find(|(m, ..)| *m == chat.mode()))
            .unwrap_or(&p.modes[0]);
        let modes = (p.modes.iter()).fold(DrawnMenu::default(), |menu, &(id, label, about)| {
            (menu.item(label, pick(Setting::Mode(id.into()))))
                .checked(id == mode.0)
                .detail(about)
        });
        // Before the session lists its models, the ones it listed last time.
        let preset = Preset::read(Settings::get(cx));
        let models = match s.map(|s| &s.models[..]) {
            Some(m) if !m.is_empty() => m,
            _ => preset.models(p.id),
        };
        let has_auto = p.modes.iter().any(|(m, ..)| *m == AUTO);
        let current = match s {
            Some(s) if s.version.is_some() => s.option("model"),
            _ => chat.model.as_deref(),
        };
        let model = current
            .filter(|m| *m != "default")
            .map_or("Default".into(), |m| {
                model_name(m, models).map_or_else(|| friendly_model_id(m), String::from)
            });
        let chosen = Model::find(models, current.unwrap_or("default"));
        let add_models = |menu: DrawnMenu| {
            models.iter().fold(menu, |menu, m| {
                let label = match m.auto || !has_auto {
                    true => m.label.clone(),
                    false => format!("{} (no auto)", m.label),
                };
                let setting = Setting::Model(m.value.clone());
                menu.item(label, pick(setting)).checked(chosen == Some(m))
            })
        };
        // An agent that lists efforts as an option of the session, not per model.
        let listed = s.and_then(|s| s.config.iter().find(|c| c.id == "effort"));
        let efforts = match (chosen.map_or(&[][..], |m| &m.efforts), listed) {
            ([], Some(c)) => &c.values[..],
            (e, _) => e,
        };
        let effort = match s {
            Some(s) if s.version.is_some() => s.option("effort").filter(|e| !e.is_empty()),
            _ => chat.effort.as_deref(),
        };
        let add_efforts = |menu: DrawnMenu| {
            efforts.iter().fold(menu, |menu, e| {
                let setting = Setting::Effort(e.clone());
                menu.item(e.clone(), pick(setting))
                    .checked(effort == Some(e.as_str()))
            })
        };
        let mut more = DrawnMenu::default();
        let mut inline = vec![];
        if self.room == 0 {
            more = add_models(more.section("Model"));
            if !efforts.is_empty() {
                more = add_efforts(more.section("Effort"));
            }
        } else {
            let menu = (!models.is_empty()).then(|| add_models(DrawnMenu::default()));
            inline.push(("chat.model", "Model", model, None, menu));
            if !efforts.is_empty() {
                let label = effort.unwrap_or("Default").to_string();
                let menu = Some(add_efforts(DrawnMenu::default()));
                inline.push(("chat.effort", "Effort", label, None, menu));
            }
        }
        let modes = (
            "chat.mode",
            "Permissions",
            mode.1.into(),
            Some(mode.2),
            Some(modes),
        );
        let usage = s.and_then(|s| {
            let (used, max) = s.usage.filter(|(_, max)| *max > 0)?;
            let pct = (used * 100 / max) as f32;
            let label = format!("Context {pct:.0}%");
            let parts: Vec<_> = (s.parts.iter())
                .map(|p| format!("{} {}", p.name, tokens(p.tokens)))
                .collect();
            let row = div()
                .id("chat.usage")
                .test_support()
                .aria_label(label)
                .aria_description(SharedString::from(parts.join(", ")))
                .flex()
                .items_center()
                .gap(px(4.))
                .child("Context")
                .child(number(format!("{pct:.0}%"), pct >= 90., cx))
                .child(meter(pct, pct >= 90., cx).ml(px(2.)))
                .child(
                    div()
                        .ml_auto()
                        .child(format!("{} of {}", tokens(used), tokens(max))),
                );
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(row)
                    .children(parts),
            )
        });
        let limits = s.and_then(|s| s.limits.as_deref()).unwrap_or_default();
        let week = (limits.iter())
            .find(|l| l.name == "Week" && self.room == 2)
            .map(|l| limit(l, false, cx));
        let unknown = (limits.is_empty() && s.is_some_and(|s| s.limits_settled())).then(|| {
            let said = "No plan limits reported";
            div()
                .id("chat.limits")
                .test_support()
                .aria_label(said)
                .child(said)
        });
        // The week's number may be in the bar already.
        let limits = (limits.iter())
            .filter(|l| l.name != "Week" || week.is_none() || l.resets.is_some())
            .map(|l| {
                let resets = l.resets.map(|r| {
                    let left = span(r as i64 / 1000 - now());
                    div().ml_auto().child(format!("Resets in {left}"))
                });
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .map(|d| match l.name == "Week" && week.is_some() {
                        true => d.child("Week"),
                        false => d.child(limit(l, true, cx)),
                    })
                    .children(resets)
            });
        let meters = div()
            .flex()
            .flex_col()
            .gap(size::GAP)
            .text_size(text::SMALL)
            .text_color(t.muted)
            .children(usage)
            .children(limits)
            .children(unknown);
        let session = self.session.clone();
        let me = self.me.clone();
        let more = (self.messages(more, cx)).child(meters).separator().item(
            "Save as default",
            move |window, cx| {
                session.update(cx, |s, cx| s.save_defaults(cx));
                me.update(cx, |this, cx| this.focus(window, cx)).ok();
            },
        );
        let busy = chat.busy(cx);
        let empty = self.input.read(cx).value().trim().is_empty();
        // A pill puts its label first.
        let send = if busy {
            ui::button("chat.stop", "Stop", cx)
                .role(Role::Button)
                .aria_label("Stop")
                .test_support()
                .flex_none()
                .flex_row_reverse()
                .gap(px(6.))
                .child(Icon::Stop.size(px(12.)))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.session.update(cx, |s, cx| s.stop(cx));
                    this.focus(window, cx);
                }))
                .into_any_element()
        } else {
            let send = match empty {
                true => ui::pill("chat.send", "Send")
                    .bg(t.header_bg)
                    .border_color(t.panel_edge)
                    .text_color(t.muted),
                false => ui::primary_button("chat.send", "Send", cx),
            };
            let send = send
                .flex_none()
                .role(Role::Button)
                .aria_label("Send")
                .flex_row_reverse()
                .gap(px(4.))
                .child(Icon::Up.size(px(12.)))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.send(window, cx);
                    this.focus(window, cx);
                }));
            ui::disabled(send, empty).test_support().into_any_element()
        };
        let more_button = ui::titled(
            ui::icon_button("chat.more", Icon::More.size(px(14.)), cx),
            "More",
        )
        .test_support();
        let provider = self.provider(window, cx);
        let pickers: Vec<_> = (inline.into_iter().chain([modes]))
            .map(|(id, title, value, tip, menu)| picker(id, title, value, tip, menu, window, cx))
            .collect();
        let more = (more.button("chat.more.menu", more_button, window, cx))
            .anchor(Anchor::BottomLeft)
            .flex_none();
        let pickers = div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(size::GAP)
            .overflow_hidden()
            .child(provider)
            .children(pickers)
            .child(more);
        div()
            .flex()
            .items_center()
            .gap(size::GAP)
            .text_size(text::SMALL)
            .child(pickers)
            .children(week)
            .child(send)
    }

    fn provider(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let t = theme(cx);
        let chat = self.session.read(cx);
        let p = chat.provider;
        let account = chat.current().and_then(|s| s.read(cx).account.as_ref());
        let name = div()
            .id("chat.name")
            .test_support()
            .aria_label(p.name)
            .child(p.name);
        let label = div()
            .id("chat.provider")
            .test_support()
            .flex()
            .flex_none()
            .when_some(account, |d, a| {
                let tip = match &a.email {
                    Some(e) => format!("{} · {e}", a.plan),
                    None => a.plan.clone(),
                };
                d.aria_label(SharedString::from(a.plan.clone()))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .child(name);
        if !chat.is_new(cx) {
            return label.px(size::CONTROL_PAD).into_any_element();
        }
        let rows = provider_rows(cx);
        let manage = rows.len();
        let current = rows.iter().position(|(o, ..)| o.id == p.id);
        let initial = current.filter(|&i| rows[i].2).unwrap_or(manage);
        let selected = self.menu;
        let trigger = ui::icon_button("chat.provider.pick", label, cx)
            .role(Role::Button)
            .aria_label(SharedString::from(format!("Agent {}", p.name)))
            .aria_expanded(selected.is_some())
            .test_support()
            .text_color(t.fg)
            .track_focus(&self.provider_focus.clone().tab_stop(true))
            .child(Icon::Down.size(size::CHEVRON))
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(i) = selected {
                    this.choose_provider(i, window, cx);
                } else {
                    this.provider_focus.focus(window, cx);
                    this.menu = Some(initial);
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                let Some(i) = this.menu else {
                    return;
                };
                let down = match e.keystroke.key.as_str() {
                    "up" => false,
                    "down" => true,
                    _ => return,
                };
                let rows = provider_rows(cx);
                let n = rows.len() + 1;
                if let Some(next) = (1..=n)
                    .map(|step| {
                        if down {
                            (i + step) % n
                        } else {
                            (i + n - step) % n
                        }
                    })
                    .find(|&i| rows.get(i).is_none_or(|r| r.2))
                {
                    this.menu = Some(next);
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let row = |i, label: &'static str, status: &'static str, enabled: bool, checked: bool| {
            Button::new(("chat.agent", i))
                .role(Role::MenuItem)
                .accessibility_label(label)
                .aria_description(status)
                .aria_selected(checked)
                .focusable(false)
                .disabled(!enabled)
                .h(size::OPTION)
                .px(theme::GAP)
                .gap(size::GAP)
                .justify_start()
                .rounded(radius::ITEM)
                .when(selected == Some(i) && enabled, |d| d.bg(t.popover_selected))
                .when(enabled, |d| d.hover(|s| s.bg(t.hover)))
                .when(!enabled, |d| d.text_color(t.muted))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.choose_provider(i, window, cx)),
                )
                .child(
                    div()
                        .w(size::CHECK)
                        .children(checked.then(|| Icon::Check.size(size::CHECK))),
                )
                .child(label)
                .child(div().flex_1())
                .child(div().text_color(t.muted).child(status))
        };
        ui::popup(
            "chat.provider.menu",
            trigger,
            selected.map(|_| {
                div()
                    .id("chat.provider.options")
                    .test_support()
                    .role(Role::Menu)
                    .flex()
                    .flex_col()
                    .px(size::GAP)
                    .min_w(size::MENU)
                    .children(
                        rows.into_iter()
                            .enumerate()
                            .map(|(i, (o, status, enabled))| {
                                row(i, o.name, status, enabled, Some(i) == current)
                            }),
                    )
                    .child(div().h(size::LINE).my(size::GAP).bg(t.line))
                    .child(row(manage, "Manage agents…", "", true, false))
                    .into_any_element()
            }),
            self.close_menu(),
            window,
            cx,
        )
        .anchor(Anchor::BottomLeft)
        .flex_none()
        .into_any_element()
    }

    fn close_menu(&self) -> impl Fn(&mut Window, &mut App) + use<> {
        let me = self.me.clone();
        move |_, cx| {
            me.update(cx, |this, cx| {
                this.menu = None;
                cx.notify();
            })
            .ok();
        }
    }

    fn choose_provider(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let rows = provider_rows(cx);
        match rows.get(i) {
            Some((p, _, true)) => {
                self.session.update(cx, |s, cx| s.set_provider(p, cx));
                self.focus(window, cx);
            }
            None => crate::settings::open(cx, "agents"),
            _ => return,
        }
        self.menu = None;
        cx.notify();
    }

    /// The groups this chat's agent can message, and whether new messages wake it, in `menu`.
    fn messages(&self, menu: DrawnMenu, cx: &App) -> DrawnMenu {
        let chat = self.session.read(cx);
        let id = chat.current().and_then(|s| s.read(cx).id.clone());
        let m = Messages::get(cx);
        let mine = (id.as_deref())
            .and_then(|id| m.group(id))
            .map(|g| g.name.clone());
        let wake = id.as_deref().is_some_and(|id| m.wake.contains(id));
        let on = id.is_some();
        let id = id.unwrap_or_default();
        let join = |name: Option<String>| {
            let (id, me) = (id.clone(), self.me.clone());
            move |window: &mut Window, cx: &mut App| {
                inbox::join(cx, &id, name.clone());
                me.update(cx, |this, cx| this.focus(window, cx)).ok();
            }
        };
        let mut menu = menu.section("Messages between agents");
        for g in m.groups.iter().filter(|g| Some(&g.name) != mine.as_ref()) {
            let label = format!("Join {} ({})", g.name, g.members.len());
            menu = menu.item_if(on, label, join(Some(g.name.clone())));
        }
        let fresh = m.fresh_name();
        menu = menu.item_if(on, format!("New group: {fresh}"), join(Some(fresh.clone())));
        if let Some(g) = &mine {
            menu = menu.item(format!("Leave {g}"), join(None));
        }
        let me = self.me.clone();
        menu.item_if(on, "Wake on messages", move |window, cx| {
            inbox::set_wake(cx, &id, !wake);
            me.update(cx, |this, cx| this.focus(window, cx)).ok();
        })
        .checked(wake)
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
                _ => cx.new(|cx| Markdown::new(text.clone(), Links::Agent, true, cx).chat()),
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
        let muted = theme(cx).muted;
        let gap = |i: usize| match (
            i.checked_sub(1).map(|p| feed[p].1),
            feed.get(i).map(|f| f.1),
        ) {
            (None, _) => 0.,
            (_, Some(Item::User(_)) | None) => 24.,
            (Some(Item::Tool(_)), Some(Item::Tool(_)))
            | (_, Some(Item::End { .. } | Item::PastTurn(_) | Item::Changes { .. })) => 8.,
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
            if matches!(feed[i].1, Item::Changes { .. } | Item::Docs { .. }) {
                let from = i;
                let session = feed[i].0.0;
                let mut changes = Vec::new();
                while let Some(&(at, item)) = feed.get(i).filter(|(at, _)| at.0 == session) {
                    match item {
                        Item::Changes { .. } => changes.push(self.changes(i, at, item, cx)),
                        Item::Docs { repo, needs } => push(i, self.docs(i, repo, needs, cx)),
                        _ => break,
                    }
                    i += 1;
                }
                let mut row = div().flex().flex_wrap().items_center().gap(size::GAP);
                let mut separator = false;
                if let Some(&(at, item @ Item::End { .. })) =
                    feed.get(i).filter(|(at, _)| at.0 == session)
                {
                    row = row.child(self.item(i, (at, item), cx));
                    i += 1;
                    separator = true;
                }
                let mut details = Vec::new();
                for (summary, detail) in changes {
                    row = row
                        .when(separator, |row| {
                            row.child(div().text_color(muted).child("·"))
                        })
                        .child(summary);
                    separator = true;
                    details.extend(detail);
                }
                push(
                    from,
                    div()
                        .flex()
                        .flex_col()
                        .gap(size::GAP)
                        .child(row)
                        .children(details)
                        .into_any_element(),
                );
                continue;
            }
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
            .h(px(24.))
            .gap(px(6.))
            .px(px(8.))
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
            .child(div().min_w_0().truncate().child(summary))
            .child(match open {
                true => Icon::Down.size(px(12.)).into_any_element(),
                false => Icon::Chevron.size(px(12.)).into_any_element(),
            });
        let lines = open.then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
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
                    .bg(t.user_bg)
                    .text_color(t.fg)
                    .text_size(theme::doc::BODY)
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
                .h(px(24.))
                .gap(px(6.))
                .px(px(8.))
                .rounded(radius::WIDGET)
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
                    None => text.clone().into_any_element(),
                };
                self.row(i, text.clone())
                    .text_size(theme::doc::BODY)
                    .line_height(relative(theme::doc::LINE_HEIGHT))
                    .text_color(t.fg)
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
                let icon = div().child(Icon::Close.size(px(12.))).into_any_element();
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
                        .ml(px(26.))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(radius::ITEM)
                        .border_1()
                        .border_color(t.line)
                        .bg(t.code_block_bg)
                        .text_size(theme::doc::CODE)
                        .line_height(px(20.))
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
            Item::Changes { .. } | Item::Docs { .. } => unreachable!(),
            Item::Note(text) => self
                .row(i, text.clone())
                .text_size(text::SMALL)
                .text_color(t.muted)
                .child(text.clone())
                .into_any_element(),
            Item::Error(text) => self
                .row(i, text.clone())
                .text_size(text::SMALL)
                .text_color(t.error)
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
        let name = self.session.read(cx).provider.name;
        let title = match asking {
            true => format!("{name} asks to use {}", kind(tool)),
            false => format!("{name} asks"),
        };
        let detail = title_of(tool, asking);
        let open = answer == Answer::Waiting;
        let key = (self.session.read(cx).sessions[at.0].entity_id(), at.1);
        let choices = self.picked.get(&key);
        let button = |id: &'static str, label: &'static str, primary: bool, answer: Answer| {
            let session = self.session.clone();
            let b = match primary {
                true => ui::primary_button((id, i), label, cx),
                false => ui::button((id, i), label, cx),
            };
            b.test_support()
                .on_click(move |_, _, cx| session.update(cx, |s, cx| s.answer(at, answer, cx)))
        };
        let complete = open && self.answers(key, questions, cx).is_some();
        let rows = questions.iter().enumerate().map(|(q, question)| {
            let choice = choices.and_then(|choices| choices.get(q));
            let input = choice
                .and_then(|c| c.other.as_ref())
                .map(|(input, _)| input);
            let options = question.options.iter().enumerate().map(|(o, c)| {
                let on = choice.is_some_and(|c| c.options.contains(&o));
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
            let me = self.me.clone();
            let other = open.then(|| {
                let multi = question.multi;
                Button::new(SharedString::from(format!("chat.other.{i}.{q}")))
                    .accessibility_label("Other…")
                    .aria_expanded(input.is_some())
                    .justify_start()
                    .h(size::CONTROL)
                    .px(theme::GAP)
                    .rounded(radius::ITEM)
                    .when(input.is_some(), |d| d.bg(t.selected))
                    .hover(|s| s.bg(t.hover))
                    .child("Other…")
                    .on_click(move |_, window, cx| {
                        me.update(cx, |this, cx| this.edit_other(key, q, multi, window, cx))
                            .ok();
                    })
            });
            let field = input.filter(|_| open).map(|input| {
                let (me, input_state) = (self.me.clone(), input.clone());
                div()
                    .id(SharedString::from(format!("chat.other.input.{i}.{q}")))
                    .test_support()
                    .capture_action(move |_: &Enter, window, cx| {
                        cx.stop_propagation();
                        if !input_state.read(cx).value().trim().is_empty() {
                            me.update(cx, |this, cx| this.submit_questions(at, window, cx))
                                .ok();
                        }
                    })
                    .child(ui::input(input, cx).aria_label("Your answer"))
            });
            let answered = input
                .filter(|_| answer == Answer::Once)
                .map(|input| input.read(cx).value())
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    div()
                        .text_color(t.muted)
                        .child(format!("Other: {}", value.trim()))
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
                .children(other)
                .children(field)
                .children(answered)
        });
        let buttons = open.then(|| {
            let b = div().flex().gap(px(8.));
            match asking {
                true => b
                    .child(button("chat.allow", "Allow", true, Answer::Once))
                    .child(button("chat.always", "Always allow", false, Answer::Always))
                    .child(button("chat.deny", "Deny", false, Answer::Rejected)),
                false => {
                    let me = self.me.clone();
                    b.when(complete, |b| {
                        b.child(
                            ui::primary_button(("chat.allow", i), "Answer", cx)
                                .test_support()
                                .on_click(move |_, window, cx| {
                                    me.update(cx, |this, cx| this.submit_questions(at, window, cx))
                                        .ok();
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
            .border_color(if open { t.warning } else { t.line })
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

    fn choice(&mut self, key: AskKey, q: usize) -> &mut Choice {
        let choices = self.picked.entry(key).or_default();
        if choices.len() <= q {
            choices.resize_with(q + 1, Choice::default);
        }
        &mut choices[q]
    }

    fn pick(&mut self, key: AskKey, q: usize, o: usize, multi: bool, cx: &mut Context<Self>) {
        let choice = self.choice(key, q);
        match choice.options.iter().position(|x| *x == o) {
            Some(at) if multi => _ = choice.options.remove(at),
            _ if multi => choice.options.push(o),
            _ => {
                choice.options = vec![o];
                choice.other = None;
            }
        }
        cx.notify();
    }

    fn edit_other(
        &mut self,
        key: AskKey,
        q: usize,
        multi: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choice = self.choice(key, q);
        if !multi {
            choice.options.clear();
        }
        let (input, _) = choice.other.get_or_insert_with(|| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type your answer…"));
            let changed = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
            (input, changed)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn answers(
        &self,
        key: AskKey,
        questions: &[Question],
        cx: &App,
    ) -> Option<BTreeMap<String, String>> {
        let choices = self.picked.get(&key)?;
        questions
            .iter()
            .enumerate()
            .map(|(q, question)| {
                let choice = choices.get(q)?;
                let typed = choice
                    .other
                    .as_ref()
                    .map(|(input, _)| input.read(cx).value());
                let labels: Vec<_> = choice
                    .options
                    .iter()
                    .map(|&o| question.options[o].label.as_str())
                    .chain(
                        typed
                            .as_deref()
                            .map(str::trim)
                            .filter(|text| !text.is_empty()),
                    )
                    .collect();
                (!labels.is_empty()).then(|| (question.question.clone(), labels.join(", ")))
            })
            .collect()
    }

    fn submit_questions(&mut self, at: At, window: &mut Window, cx: &mut Context<Self>) {
        let s = &self.session.read(cx).sessions[at.0];
        let Some(Item::Ask {
            questions,
            answer: Answer::Waiting,
            ..
        }) = s.read(cx).items.get(at.1)
        else {
            return;
        };
        if let Some(answers) = self.answers((s.entity_id(), at.1), questions, cx) {
            self.session
                .update(cx, |s, cx| s.answer_questions(at, answers, cx));
            self.focus(window, cx);
        }
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

    fn changes(&self, i: usize, at: At, item: &Item, cx: &App) -> (AnyElement, Option<AnyElement>) {
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
            1 => "1 file changed".to_string(),
            _ => format!("{n} files changed"),
        };
        if self.session.read(cx).repo != *repo {
            title += &format!(" in {}", Workspace::get(cx).name_of(repo));
        }
        let expanded = self.changed_files.contains(&at);
        let mono = theme::mono(cx);
        let rows = || {
            files.iter().enumerate().map(|(j, f)| {
                let doc = Doc::Diff(DiffDoc {
                    repo: repo.to_string(),
                    path: f.path.clone(),
                    orig_path: f.orig_path.clone(),
                    left: Side::Rev { rev: start.into() },
                    right: Side::Rev { rev: end.into() },
                });
                let dir = f.path.rsplit_once('/').map(|(dir, _)| dir.to_string());
                ui::row(SharedString::from(format!("chat.file.{i}.{j}")), false, cx)
                    .test_support()
                    .aria_label(SharedString::from(f.path.clone()))
                    .h(px(24.))
                    .gap(px(6.))
                    .on_click(move |_, _, cx| {
                        let doc = doc.clone();
                        Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx))
                    })
                    .child(ui::status(status_of(f), cx))
                    .child(
                        div()
                            .flex_none()
                            .font_family(mono.clone())
                            .text_color(t.fg)
                            .child(folder_name(&f.path).to_string()),
                    )
                    .children(dir.map(|d| div().min_w_0().truncate().child(d)))
            })
        };
        let session = self.session.clone();
        let button =
            move |id: &'static str,
                  label: String,
                  run: fn(&mut Session, usize, &mut Context<Session>)| {
                let session = session.clone();
                let button = match id {
                    "chat.back.confirm" => ui::primary_button((id, i), label, cx).h(px(24.)),
                    "chat.back.cancel" => ui::button((id, i), label, cx).h(px(24.)),
                    _ => ui::icon_button((id, i), label, cx).h(px(24.)),
                };
                button
                    .role(Role::Button)
                    .test_support()
                    .on_click(move |_, _, cx| session.update(cx, |s, cx| s.back(at, run, cx)))
            };
        let restore = |label: &'static str| {
            (!busy)
                .then(|| button("chat.back", label.into(), Session::plan_back).into_any_element())
        };
        let (state, action, detail) = match back {
            Back::Idle => ("", restore("Restore…"), None),
            Back::Failed(e) => (
                "failed",
                restore("Restore…"),
                Some(format!("Cannot go back: {e}").into_any_element()),
            ),
            Back::Working => (
                "working",
                Some(ui::spinner(t.accent).into_any_element()),
                None,
            ),
            Back::Planned(plan) => {
                let todo: Vec<&str> = (plan.changes.iter())
                    .filter(|c| c.skip.is_none())
                    .map(|c| c.path.as_str())
                    .collect();
                let mut said = todo[..todo.len().min(3)].join(", ");
                if todo.len() > 3 {
                    said += &format!(" +{} more", todo.len() - 3);
                }
                let kept = plan.changes.iter().filter_map(|c| {
                    let why = match c.skip {
                        Some(Skip::Edited) => "kept: changed since the turn",
                        Some(Skip::Ignored) => "kept: ignored now, no copy to undo with",
                        Some(Skip::Submodule) => "kept: a submodule",
                        None if c.edited && !c.remove => {
                            "restored, over changes made since the turn"
                        }
                        None => return None,
                    };
                    Some(div().child(format!("{}: {why}", c.path)))
                });
                let confirm = (!todo.is_empty() && !busy).then(|| {
                    let label = match todo.len() {
                        1 => "Restore 1 file".to_string(),
                        n => format!("Restore {n} files"),
                    };
                    button("chat.back.confirm", label, Session::confirm_back)
                });
                let ask = div()
                    .flex()
                    .items_center()
                    .gap(size::GAP)
                    .child(div().pl(px(4.)).text_color(t.fg).child("Restore files?"))
                    .child(button(
                        "chat.back.cancel",
                        "Cancel".into(),
                        Session::cancel_back,
                    ))
                    .children(confirm);
                let plan = div()
                    .id(("chat.back.plan", i))
                    .test_support()
                    .role(Role::Group)
                    .aria_label("Confirm restore")
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(!said.is_empty(), |d| d.child(said))
                    .children(kept)
                    .child("Ignored files and submodules are not covered. You can undo this.");
                (
                    "planned",
                    Some(ask.into_any_element()),
                    Some(plan.into_any_element()),
                )
            }
            Back::Done { .. } => ("restored", restore("Undo restore…"), None),
        };
        let me = self.me.clone();
        let count = ui::titled(
            ui::icon_button(("chat.files", i), title.clone(), cx).h(size::CONTROL),
            "Edits anyone made in this folder during the turn show here too.",
        )
        .aria_label(SharedString::from(title.clone()))
        .aria_expanded(expanded)
        .test_support()
        .on_click(move |_, _, cx| {
            me.update(cx, |this, cx| {
                if !this.changed_files.remove(&at) {
                    this.changed_files.insert(at);
                }
                cx.notify();
            })
            .ok();
        });
        let summary = div()
            .id(("chat.item", i))
            .test_support()
            .aria_label(SharedString::from(
                format!("{title} {state}").trim_end().to_string(),
            ))
            .flex()
            .items_center()
            .text_size(text::SMALL)
            .text_color(t.muted)
            .child(count)
            .when(matches!(back, Back::Done { .. }), |d| d.child("· Restored"))
            .when(action.is_some(), |d| d.child("·"))
            .children(action);
        let below = (expanded || detail.is_some()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(size::GAP)
                .text_size(text::SMALL)
                .text_color(t.muted)
                .when(expanded, |b| b.children(rows()))
                .children(detail)
                .into_any_element()
        });
        (summary.into_any_element(), below)
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
        let mut past: Vec<&Past> = past
            .filter(|p| enabled && p.owned && !p.archived())
            .collect();
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
        let working = s.busy().then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(24.))
                .text_size(text::SMALL)
                .text_color(t.muted)
                .child(ui::spinner(t.accent))
                .child("Working")
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
    t.output_text()
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
        if !self.session.read(cx).is_new(cx) {
            self.menu = None;
        }
        let bar = self.bar(window, cx);
        let me = self.me.clone();
        let measure = canvas(
            move |bounds, _, cx| {
                let w = bounds.size.width;
                let room = [size::INLINE, size::WEEK]
                    .iter()
                    .filter(|&&t| w >= t)
                    .count();
                me.update(cx, |this, cx| {
                    if this.room != room {
                        this.room = room;
                        cx.notify();
                    }
                })
                .ok();
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        // A column to read in, as wide as a line of text should be.
        let column = || div().flex().flex_col().w_full().max_w(px(760.)).mx_auto();
        div()
            .id("chat")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.panel_bg)
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
                div().flex_none().px(px(20.)).pt(px(8.)).pb(px(10.)).child(
                    column()
                        .id("chat.input")
                        .test_support()
                        .gap(px(8.))
                        .children(review)
                        .children(footer)
                        .children(suggestions)
                        .child(
                            div()
                                .id("chat.composer")
                                .test_support()
                                .relative()
                                .track_focus(&self.composer_focus)
                                // A click beside the text puts the caret in it.
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, window, cx| {
                                        if !window.default_prevented() {
                                            window.prevent_default();
                                            this.focus(window, cx);
                                        }
                                    }),
                                )
                                .bg(t.header_bg)
                                .rounded(radius::FIELD)
                                .border_1()
                                .border_color(
                                    match self.composer_focus.contains_focused(window, cx) {
                                        true => t.selected_ring,
                                        false => t.line,
                                    },
                                )
                                .flex()
                                .flex_col()
                                .p(size::PAD)
                                .gap(size::GAP)
                                .child(measure)
                                .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                                    if e.keystroke.key == "escape" && this.menu.take().is_some() {
                                        cx.stop_propagation();
                                        cx.notify();
                                    }
                                }))
                                .child(self.textarea(cx))
                                .child(bar),
                        ),
                ),
            )
    }
}

/// A picker in the bar: its muted title and its value, opening `menu` upward.
fn picker(
    id: &'static str,
    title: &'static str,
    value: String,
    tip: Option<&'static str>,
    menu: Option<DrawnMenu>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme(cx);
    // Only a model's name is long enough to need cutting.
    let shrink = id == "chat.model";
    let trigger = ui::icon_button(id, title, cx)
        .role(Role::Button)
        .aria_label(title)
        .aria_description(SharedString::from(value.clone()))
        .when_some(tip, |b, tip| {
            b.tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        })
        .test_support()
        .h(size::CONTROL)
        .px(size::CONTROL_PAD)
        .whitespace_nowrap()
        .when(shrink, |b| b.min_w_0())
        .child(div().min_w_0().truncate().text_color(t.fg).child(value))
        .children(menu.is_some().then(|| Icon::Down.size(size::CHEVRON)));
    match menu {
        Some(menu) => (menu.button(
            SharedString::from(format!("{id}.menu")),
            trigger,
            window,
            cx,
        ))
        .anchor(Anchor::BottomLeft)
        .map(|m| match shrink {
            true => m.min_w_0(),
            false => m.flex_none(),
        })
        .into_any_element(),
        None => trigger.flex_none().into_any_element(),
    }
}

fn provider_rows(cx: &App) -> Vec<(&'static Provider, &'static str, bool)> {
    let agents = Agents::global(cx);
    let a = agents.read(cx);
    let config = super::provider_config(cx);
    a.rows
        .iter()
        .map(|r| {
            let p = r.provider;
            let login = r.cli.as_ref().map(|c| c.login);
            let (status, enabled) = match login {
                _ if !config.shows(p) => ("Off", false),
                _ if !a.loaded() => ("Checking…", false),
                None => ("CLI not found", false),
                _ if p.backend == Name::Acp && r.installed.current.is_none() => {
                    ("No adapter", false)
                }
                Some(LoginStatus::LoggedOut) => ("Not logged in", false),
                Some(LoginStatus::LoggedIn) => ("Logged in", true),
                Some(LoginStatus::Unknown) => ("Login status unknown", true),
            };
            (p, status, enabled)
        })
        .collect()
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

/// A plan limit, "Week 11% left", with a bar of what is left when `track`.
fn limit(l: &Limit, track: bool, cx: &App) -> impl IntoElement + use<> {
    let t = theme(cx);
    let used = l.used.clamp(0., 100.);
    let left = 100. - used;
    let low = left <= 10.;
    let name = match l.name.as_str() {
        "5h" => "5-hour",
        n => n,
    };
    let window = if l.name == "Week" { "Weekly" } else { name };
    let mut tip = format!("{window} limit: {used:.0}% used");
    if let Some(r) = l.resets {
        tip += &format!(", resets in {}", span(r as i64 / 1000 - now()));
    }
    div()
        .id(SharedString::from(format!("chat.limit.{}", l.name)))
        .test_support()
        .aria_label(SharedString::from(format!("{} {left:.0}% left", l.name)))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .text_size(text::SMALL)
        .text_color(t.muted)
        .whitespace_nowrap()
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(name.to_string())
        .child(number(format!("{left:.0}%"), low, cx))
        .child("left")
        .when(track, |d| d.child(meter(left as f32, low, cx).ml(px(2.))))
}

/// A meter's number: `fg`, or `warning` when it runs low.
fn number(n: String, low: bool, cx: &App) -> Div {
    let t = theme(cx);
    div()
        .text_color(if low { t.warning } else { t.fg })
        .child(n)
}

/// A 48px bar `fill` percent full, in `warning` when it runs low: low is a caution, not a failure.
fn meter(fill: f32, low: bool, cx: &App) -> Div {
    let t = theme(cx);
    div()
        .flex_none()
        .w(px(48.))
        .h(px(4.))
        .rounded_full()
        .bg(t.line)
        .child(
            div()
                .id("chat.meter.fill")
                .test_support()
                .h_full()
                .rounded_full()
                .bg(if low { t.warning } else { t.accent })
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
