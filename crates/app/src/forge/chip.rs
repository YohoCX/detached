use detached_core::api;
use detached_core::types::{CreateMr, MergeRequest, NewMr};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, BoxShadow, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, StatefulInteractiveElement, Styled,
    Subscription, Window, anchored, deferred, div, point, prelude::FluentBuilder, px,
};

use super::core;
use super::mr::{bare_title, number, show_mr};
use crate::actions::{self, Group, Registration};
use crate::ui::{self, Icon, radius, text, theme};
use kernel::activity::Activity;
use kernel::blocking;
use kernel::forge::{self, setup_problem};
use kernel::repo::Repo;
use kernel::resource::Interest;
use kernel::workspace::Workspace;

struct Form {
    new: NewMr,
    title: Entity<InputState>,
    target: Entity<InputState>,
    description: Entity<TextareaState>,
    draft: bool,
    busy: bool,
    error: Option<String>,
    _changes: [Subscription; 2],
}

/// The current branch's MR, or a button that creates one. It also runs `mr.create`, being always
/// mounted.
pub struct MrChip {
    repo: Option<String>,
    /// None without a branch with an upstream.
    key: Option<String>,
    mr: Option<Interest<Option<MergeRequest>>>,
    form: Option<Form>,
    _create: Registration,
    _subscriptions: Vec<Subscription>,
}

impl MrChip {
    pub fn new(_: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let create = actions::register(
            cx,
            "mr.create",
            "Create Merge Request…",
            Group::Vcs,
            move |window, cx| {
                this.update(cx, |this, cx| this.open_form(window, cx)).ok();
            },
        );
        let subscriptions = vec![Workspace::observe(cx, |this, cx| this.sync(cx))];
        let mut this = MrChip {
            repo: None,
            key: None,
            mr: None,
            form: None,
            _create: create,
            _subscriptions: subscriptions,
        };
        this.sync(cx);
        this
    }

    /// A new key reloads and drops the form.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        let key = repo.as_deref().and_then(|r| {
            let st = Repo::get(cx, r)?.status()?;
            let (branch, upstream) = (st.branch.as_ref()?, st.upstream.as_ref()?);
            Some(format!("{branch}\0{upstream}"))
        });
        if (&repo, &key) != (&self.repo, &self.key) {
            (self.repo, self.key, self.form) = (repo, key, None);
            self.mr = self
                .repo
                .as_ref()
                .zip(self.key.as_ref())
                .map(|(repo, key)| Interest::view(forge::current_mr(cx, repo, key), cx));
            cx.notify();
        }
    }

    /// The MR, Some(None) when there is none; None while unknown or failing with nothing loaded.
    fn mr(&self, cx: &App) -> Option<Option<MergeRequest>> {
        self.mr.as_ref()?.read(cx).value.clone()
    }

    fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(repo), Some(_)) = (self.repo.clone(), &self.key) else {
            let text = "Create Merge Request needs a branch with an upstream";
            return Activity::global(cx).update(cx, |a, cx| a.say(text, false, cx));
        };
        if let Some(Some(m)) = self.mr(cx) {
            return show_mr(cx, repo, m.number);
        }
        let (core, r) = (core(cx), repo.clone());
        // A repo off any forge gets a hint, not an error.
        let new = blocking::run(cx, move || {
            match setup_problem(&api::forge_hosts(&core, &r)?) {
                Some(s) => Ok(Err(s)),
                None => api::new_mr(&core, &r).map(Ok),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let new = new.await;
            this.update_in(cx, |this, window, cx| match new {
                // Not the key: new_mr reads the branch on disk, which the status may catch up with later.
                Ok(Ok(new)) if this.repo.as_ref() == Some(&repo) => {
                    this.form = Some(Form::new(new, window, cx));
                    cx.notify();
                }
                Ok(Ok(_)) => {}
                Ok(Err(hint)) => Activity::global(cx).update(cx, |a, cx| a.say(hint, false, cx)),
                Err(e) => Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx)),
            })
            .ok();
        })
        .detach();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.form = None;
        cx.notify();
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let (Some(repo), Some(f)) = (self.repo.clone(), self.form.as_mut()) else {
            return;
        };
        let value = |s: &Entity<InputState>| s.read(cx).value().trim().to_string();
        let input = CreateMr {
            title: value(&f.title),
            description: f.description.read(cx).value().to_string(),
            target_branch: value(&f.target),
            draft: f.draft,
        };
        (f.busy, f.error) = (true, None);
        cx.notify();
        let (core, r) = (core(cx), repo.clone());
        let done = blocking::run(cx, move || api::create_mr(&core, &r, &input));
        cx.spawn(async move |this, cx| {
            let done = done.await;
            this.update(cx, |this, cx| {
                match done {
                    Ok(m) => {
                        this.form = None;
                        if let Some(mr) = &this.mr {
                            mr.refresh(cx);
                        }
                        show_mr(cx, repo, m.number);
                    }
                    Err(e) => {
                        if let Some(f) = &mut this.form {
                            (f.busy, f.error) = (false, Some(e.to_string()));
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn chip(&self, m: &MergeRequest, cx: &App) -> AnyElement {
        let t = theme(cx);
        let (repo, n, tip) = (
            self.repo.clone().unwrap_or_default(),
            m.number,
            m.title.clone(),
        );
        let glow = BoxShadow::new(px(0.), px(0.), t.selected).blur_radius(px(12.));
        ui::pressable("mr.chip")
            .test_support()
            .role(Role::Button)
            .aria_label(format!("{} {}", number(m), bare_title(m)))
            .gap(px(6.))
            .h(px(28.))
            .max_w(px(260.))
            .px(px(10.))
            .border_1()
            .border_color(t.selected_ring)
            .rounded(radius::PILL)
            .bg(t.selected)
            .hover(|s| s.shadow(vec![glow]))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(move |_, _, cx| show_mr(cx, repo.clone(), n))
            .child(Icon::Mr.size(px(14.)))
            .child(div().font_weight(FontWeight::EXTRA_BOLD).child(number(m)))
            .child(div().min_w_0().truncate().child(bare_title(m).to_string()))
            .into_any_element()
    }

    fn popup(&self, f: &Form, window: &Window, cx: &mut Context<Self>) -> [AnyElement; 2] {
        let t = theme(cx);
        let blank = |s: &Entity<InputState>| s.read(cx).value().trim().is_empty();
        let can = !f.busy && !blank(&f.title) && !blank(&f.target);
        let n = &f.new;
        let from = format!("{}:{}", n.source_project.path, n.source_branch);
        let into = n.target_project.path.clone();
        let field = |label: &'static str, input: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .text_size(text::SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(label)
                .child(div().text_color(t.fg).child(input))
        };
        let form = div()
            .id("mr.form")
            .test_support()
            .role(Role::Dialog)
            .aria_label("Create Merge Request")
            .occlude()
            .flex()
            .flex_col()
            .gap(px(10.))
            .w(px(440.))
            .px(px(16.))
            .py(px(14.))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    this.close(cx);
                }
            }))
            .child(
                div()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("New merge request"),
            )
            .child(
                div()
                    .id("mr.from")
                    .test_support()
                    .aria_label(format!("{from} into {into}"))
                    .flex()
                    .gap(px(4.))
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(div().font_family(theme::mono(cx)).child(from))
                    .child("into")
                    .child(div().font_family(theme::mono(cx)).child(into)),
            )
            .child(field(
                "Title",
                Input::new(&f.title).id("mr.title").into_any_element(),
            ))
            .child(field(
                "Target branch",
                Input::new(&f.target).id("mr.target").into_any_element(),
            ))
            .child(field(
                "Description",
                div()
                    .id("mr.description")
                    .test_support()
                    .child(Textarea::new(&f.description))
                    .into_any_element(),
            ))
            .child(
                Checkbox::new("mr.draft")
                    .label("Draft")
                    .checked(f.draft)
                    .on_click(cx.listener(|this, draft: &bool, _, cx| {
                        if let Some(f) = &mut this.form {
                            f.draft = *draft;
                        }
                        cx.notify();
                    })),
            )
            .children(f.error.clone().map(|e| {
                div()
                    .id("mr.error")
                    .test_support()
                    .aria_label(e.clone())
                    .text_color(t.error)
                    .child(e)
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        ui::button("mr.cancel", "Cancel", cx)
                            .test_support()
                            .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                    )
                    .child(
                        ui::button("mr.submit", "Create", cx)
                            .test_support()
                            .bg(t.primary)
                            .text_color(t.primary_fg)
                            .when(!can, |b| b.opacity(0.5))
                            .when(can, |b| {
                                b.on_click(cx.listener(|this, _, _, cx| this.create(cx)))
                            }),
                    ),
            );
        let size = window.viewport_size();
        let backdrop = div()
            .id("mr.backdrop")
            .test_support()
            .occlude()
            .w(size.width)
            .h(size.height)
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)));
        [
            deferred(anchored().position(point(px(0.), px(0.))).child(backdrop)).into_any_element(),
            // Our overlay rather than the kit's Dialog, which restores focus to a dead element:
            // https://github.com/longbridge/gpui-kit/issues/3760
            deferred(
                anchored()
                    .offset(point(px(0.), px(8.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(ui::glass(form, cx)),
            )
            .with_priority(1)
            .into_any_element(),
        ]
    }
}

impl Form {
    fn new(new: NewMr, window: &mut Window, cx: &mut Context<MrChip>) -> Self {
        let mut input = |text: &str, cx: &mut Context<MrChip>| {
            let text = text.to_string();
            cx.new(|cx| InputState::new(window, cx).default_value(text))
        };
        let (title, target) = (input(&new.title, cx), input(&new.target_branch, cx));
        let description = cx.new(|cx| TextareaState::new(window, cx).rows(5));
        title.update(cx, |s, cx| s.focus(window, cx));
        let changed = |s: &Entity<InputState>, cx: &mut Context<MrChip>| {
            cx.subscribe(s, |_, _, _: &InputEvent, cx| cx.notify())
        };
        Form {
            _changes: [changed(&title, cx), changed(&target, cx)],
            new,
            title,
            target,
            description,
            draft: false,
            busy: false,
            error: None,
        }
    }
}

impl Render for MrChip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(mr) = self.mr(cx) else {
            return div().hidden().into_any_element();
        };
        let anchor = match &mr {
            Some(m) => self.chip(m, cx),
            None => ui::icon_button("mr.new", Icon::Mr.size(px(14.)), cx)
                .test_support()
                .role(Role::Button)
                .aria_label("Create Merge Request")
                .tooltip(|window, cx| Tooltip::new("Create Merge Request").build(window, cx))
                .on_click(cx.listener(|this, _, window, cx| this.open_form(window, cx)))
                .child(Icon::Plus.size(px(12.)))
                .into_any_element(),
        };
        let popup = self.form.as_ref().map(|f| self.popup(f, window, cx));
        div()
            .relative()
            .min_w_0()
            .child(anchor)
            .children(popup.into_iter().flatten())
            .into_any_element()
    }
}
