use gpui_kit::base::{Disableable, TestSupportExt};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Escape;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, ElementId, FontWeight, InteractiveElement, KeyDownEvent, ParentElement, Role,
    StatefulInteractiveElement, Styled, div, px,
};
use kernel::doc::Doc;

use crate::ui::{self, GAP, text, theme};

use super::*;

impl Render for Sheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(dialog) = &self.dialog else {
            return div().into_any_element();
        };
        let title = match dialog {
            Dialog::Form(f) => f.set.as_ref().map_or_else(
                || "New session".into(),
                |s| format!("Add a repository to {}", s.name),
            ),
            Dialog::Manage(m) => match m.mode {
                Manage::Archive => format!("Archive {}?", m.set.name),
                Manage::Resume(_) => format!("Resume {}?", m.set.name),
                Manage::Cleanup(Some(_)) => "Remove from session".into(),
                Manage::Cleanup(None) => "Remove worktrees".into(),
            },
            Dialog::Result(_, set, _) if self.busy => format!("Preparing {}", set.name),
            Dialog::Result(_, set, _) => set.name.clone(),
        };
        let body = div()
            .id("sessions.provision.fields")
            .flex()
            .flex_col()
            .gap(GAP)
            .map(|d| match dialog {
                Dialog::Form(form) => self.render_form(d, form, cx),
                Dialog::Manage(m) => self.render_management(d, m, cx),
                Dialog::Result(ws, set, open) => self.render_result(d, ws, set, *open, cx),
            })
            .when_some(self.error.clone(), |d, error| {
                d.child(warning("sessions.provision.error", error, cx))
            })
            .when(self.busy && !matches!(dialog, Dialog::Result(..)), |d| {
                d.when_some(self.current_op(cx), |d, op| {
                    d.child(mono(op.line.clone(), cx))
                })
            });
        let header = div().flex().flex_none().items_center().gap(GAP).child(
            div()
                .flex_1()
                .text_size(text::TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.clone()),
        );
        let frame = div()
            .id("sessions.provision")
            .flex()
            .flex_col()
            .min_h_0()
            .track_focus(&self.focus)
            .test_support()
            .role(Role::Dialog)
            .aria_label(title)
            .capture_action(cx.listener(|s, _: &Escape, _, cx| {
                s.close(cx);
                cx.stop_propagation();
            }))
            .capture_key_down(cx.listener(|s, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    s.close(cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }));
        if let Dialog::Form(form) = dialog
            && form.set.is_none()
        {
            let cancel = self.action(
                "sessions.provision.cancel",
                "Cancel",
                Kind::Quiet,
                true,
                |s, _, cx| s.close(cx),
                cx,
            );
            let t = theme(cx);
            return frame
                .size_full()
                .items_center()
                .child(
                    div()
                        .size_full()
                        .max_w(px(760.))
                        .flex()
                        .flex_col()
                        .min_h_0()
                        .child(
                            header
                                .px(GAP * 2.)
                                .h(px(46.))
                                .border_b_1()
                                .border_color(t.line)
                                .child(cancel),
                        )
                        .child(
                            body.test_support()
                                .p(GAP * 2.)
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll(),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .justify_end()
                                .px(GAP * 2.)
                                .py(GAP)
                                .border_t_1()
                                .border_color(t.line)
                                .child(self.form_button(form, cx)),
                        ),
                )
                .into_any_element();
        }
        let content = frame
            .gap(GAP)
            .p(GAP * 2.)
            .max_h(window.viewport_size().height * 0.78)
            .overflow_y_scroll()
            .child(header)
            .child(body)
            .map(|d| match dialog {
                Dialog::Form(form) => d.child(self.footer(self.form_button(form, cx), cx)),
                _ => d,
            });
        if self.page_mode() {
            return div()
                .size_full()
                .flex()
                .justify_center()
                .min_h_0()
                .child(content.max_w(px(760.)).w_full().max_h_full())
                .into_any_element();
        }
        let handle = cx.weak_entity();
        ui::modal(
            px(560.),
            0.09,
            ui::radius::GLASS,
            move |_, cx| {
                _ = handle.update(cx, |s, cx| s.close(cx));
            },
            content,
            cx,
        )
        .into_any_element()
    }
}

impl Sheet {
    fn current_op(&self, cx: &App) -> Option<kernel::activity::Op> {
        let id = self.op?;
        Activity::global(cx)
            .read(cx)
            .ops
            .iter()
            .find(|op| op.id == id)
            .cloned()
    }

    pub(super) fn page_mode(&self) -> bool {
        matches!(
            &self.dialog,
            Some(Dialog::Form(Form { set: None, .. }) | Dialog::Result(_, _, true))
        )
    }

    fn render_form(
        &self,
        mut d: gpui_kit::Stateful<gpui_kit::Div>,
        f: &Form,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let adding = f.set.is_some();
        let name = self.name.read(cx).value();
        let branch = self.branch.read(cx).value();
        let t = theme(cx);
        if !adding {
            d = d
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(GAP)
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(4.))
                                .rounded(ui::radius::SEGMENT)
                                .bg(t.active_bg)
                                .font_weight(FontWeight::MEDIUM)
                                .child("A new branch"),
                        )
                        .child(button(
                            "sessions.mode.adopt",
                            "Worktrees I already have…",
                            Kind::Quiet,
                            !self.busy,
                            move |window, cx| {
                                crate::sessions::sheet::adopt_from_new(name.to_string(), window, cx)
                            },
                            cx,
                        )),
                )
                .child(field("Session name", "sessions.new.name", &self.name, cx));
        }
        d = d
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(GAP)
                    .child(detail("Branch", cx))
                    .child(
                        div()
                            .id("sessions.branch.preview")
                            .test_support()
                            .aria_label(branch.clone())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(if branch.is_empty() {
                                detail("From the session name", cx)
                            } else {
                                mono(branch.clone(), cx)
                            }),
                    )
                    .child(button(
                        "sessions.branch.edit",
                        if f.edit_branch { "Done" } else { "Edit" },
                        Kind::Quiet,
                        !self.busy,
                        self.command(
                            |s, window, cx| {
                                if let Some(Dialog::Form(f)) = &mut s.dialog {
                                    f.edit_branch = !f.edit_branch;
                                    if f.edit_branch {
                                        s.branch.update(cx, |input, cx| input.focus(window, cx));
                                    }
                                    cx.notify();
                                }
                            },
                            cx,
                        ),
                        cx,
                    )),
            )
            .when(f.edit_branch, |d| {
                d.child(field(
                    "Shared branch",
                    "sessions.new.branch",
                    &self.branch,
                    cx,
                ))
            })
            .when_some(
                f.shared_preview
                    .as_ref()
                    .and_then(|result| result.as_ref().err()),
                |d, error| d.child(warning("sessions.branch.error", error.to_string(), cx)),
            );
        d.child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Repositories"),
        )
        .child(
            div().flex().flex_col().children(
                f.rows
                    .iter()
                    .map(|row| self.repository(f, row, &branch, cx)),
            ),
        )
        .when(f.rows.is_empty(), |d| {
            d.child(detail("No available repositories in this workspace.", cx))
        })
    }

    fn repository(
        &self,
        f: &Form,
        row: &Row,
        branch: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let i = row.i;
        let request = row.request(branch, cx);
        let candidate = existing(f, &row.repo.path, &request.branch);
        let ready_to_check = !request.branch.trim().is_empty();
        let t = theme(cx);
        let card = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .py(px(4.))
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(GAP)
                    .h(px(28.))
                    .child(
                        Checkbox::new(("sessions.repository", i))
                            .checked(row.checked)
                            .disabled(self.busy)
                            .label(row.repo.name.clone())
                            .on_click(cx.listener(move |s, checked: &bool, _, cx| {
                                if let Some(Dialog::Form(f)) = &mut s.dialog {
                                    if f.set.is_some() {
                                        for row in &mut f.rows {
                                            row.checked = false;
                                            row.adopt = false;
                                        }
                                    }
                                    f.rows[i].checked = *checked;
                                }
                                s.preview(cx);
                            })),
                    )
                    .child(div().flex_1())
                    .when(row.checked && !row.adopt, |d| {
                        d.when(ready_to_check && row.preview.is_none(), |d| {
                            d.child(detail("Checking…", cx))
                        })
                        .child(button(
                            ("sessions.customize", i),
                            if row.customize {
                                "Done"
                            } else {
                                "Customize…"
                            },
                            Kind::Quiet,
                            !self.busy,
                            {
                                let handle = cx.weak_entity();
                                move |_, cx| {
                                    _ = handle.update(cx, |s, cx| {
                                        if let Some(Dialog::Form(f)) = &mut s.dialog {
                                            f.rows[i].customize = !f.rows[i].customize;
                                            cx.notify();
                                        }
                                    });
                                }
                            },
                            cx,
                        ))
                    }),
            );
        card.when(row.checked, |card| {
            card.when_some(candidate, |d, c| {
                d.child(button(
                    ("sessions.useWorktree", i),
                    if row.adopt {
                        "A new worktree".into()
                    } else {
                        format!("Use the worktree at {}", c.path)
                    },
                    Kind::Quiet,
                    !self.busy,
                    {
                        let handle = cx.weak_entity();
                        move |_, cx| {
                            _ = handle.update(cx, |s, cx| {
                                if let Some(Dialog::Form(f)) = &mut s.dialog {
                                    f.rows[i].adopt = !f.rows[i].adopt;
                                }
                                s.preview(cx);
                            });
                        }
                    },
                    cx,
                ))
            })
            .when(!row.adopt, |d| {
                d.when(row.customize, |d| {
                    let base = field("Base", ("sessions.base", i), &row.base, cx);
                    d.child(field(
                        "Branch override",
                        ("sessions.override", i),
                        &row.branch,
                        cx,
                    ))
                    .map(|d| match &row.preview {
                        Some(Ok(p)) if p.new_branch => d
                            .child(self.base_picker(i, row, p, &request, cx))
                            .child(base),
                        Some(Ok(_)) => d,
                        _ => d.child(base),
                    })
                    .when_some(
                        row.preview.as_ref().and_then(|result| result.as_ref().ok()),
                        |d, p| {
                            d.child(detail(format!("{} · as of the last fetch", p.decision), cx))
                                .child(mono(p.path.display().to_string(), cx))
                                .child(detail(recipe_text(p), cx))
                        },
                    )
                })
                .map(|d| match &row.preview {
                    Some(Err(e)) => {
                        let error = e.to_string();
                        d.child(warning(
                            ("sessions.refusal", i),
                            format!("Refused: {error}"),
                            cx,
                        ))
                        .when(
                            error.contains("Accept the worktree recipe"),
                            |d| {
                                let repo = row.repo.path.clone();
                                d.child(button(
                                    ("sessions.trust", i),
                                    "Trust…",
                                    Kind::Quiet,
                                    !self.busy,
                                    move |_, cx| {
                                        Workspace::global(cx).update(cx, |w, cx| {
                                            w.set_current(Some(repo.clone()), cx)
                                        });
                                        crate::settings::open(cx, "sharing");
                                    },
                                    cx,
                                ))
                            },
                        )
                    }
                    _ => d,
                })
            })
            .when_some(f.failures.get(&row.repo.path), |d, error| {
                d.child(warning(("sessions.creationError", i), error.clone(), cx))
            })
        })
    }

    fn base_picker(
        &self,
        i: usize,
        row: &Row,
        p: &Preview,
        request: &Request,
        cx: &App,
    ) -> AnyElement {
        let base = row.base.clone();
        let options = p.bases.clone();
        let caption = request.base.as_ref().map_or_else(
            || {
                format!(
                    "{} · recipe",
                    p.decision.trim_start_matches("New branch from ")
                )
            },
            |b| format!("{} · chosen", b.trim_start_matches("refs/remotes/")),
        );
        button(
            ("sessions.basePicker", i),
            caption,
            Kind::Quiet,
            !self.busy,
            move |window, cx| {
                let mut menu = ui::Menu::default();
                for (label, value) in [
                    ("Recipe base".to_string(), String::new()),
                    ("The main checkout’s HEAD".into(), "HEAD".into()),
                ]
                .into_iter()
                .chain(
                    options
                        .iter()
                        .map(|b| (b.trim_start_matches("refs/remotes/").to_owned(), b.clone())),
                ) {
                    let base = base.clone();
                    menu = menu.item(label, move |window, cx| {
                        base.update(cx, |input, cx| input.set_value(&value, window, cx));
                    });
                }
                menu.show(window.mouse_position(), window, cx);
            },
            cx,
        )
    }

    fn form_button(&self, f: &Form, cx: &Context<Self>) -> AnyElement {
        let mut selected = f.rows.iter().filter(|r| r.checked);
        let label = if f.set.is_some() {
            selected.next().map_or_else(
                || "Add repository".into(),
                |r| format!("Add {}", r.repo.name),
            )
        } else {
            match selected.count() {
                0 => "Create worktrees".into(),
                1 => "Create 1 worktree".into(),
                n => format!("Create {n} worktrees"),
            }
        };
        button(
            "sessions.create.submit",
            label,
            Kind::Primary,
            !self.busy
                && valid(
                    f,
                    &self.name.read(cx).value(),
                    &self.branch.read(cx).value(),
                ),
            self.command(Self::submit, cx),
            cx,
        )
    }

    fn render_management(
        &self,
        mut d: gpui_kit::Stateful<gpui_kit::Div>,
        m: &Management,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let t = theme(cx);
        let run = self.command(Self::commit_manage, cx);
        match &m.mode {
            Manage::Archive => d
                .child(format!(
                    "Hides {}. Keeps its worktrees, changes, branches, chats and tabs.",
                    m.set.name
                ))
                .child(self.footer(
                    button(
                        "sessions.archive.confirm",
                        "Archive",
                        Kind::Primary,
                        !self.busy,
                        run,
                        cx,
                    ),
                    cx,
                )),
            Manage::Resume(_) => {
                for member in &m.set.members {
                    if !member.state.usable() && member.stage != Stage::Pending {
                        d = d.child(detail(
                            format!(
                                "Recreate {} from {}.",
                                member.path,
                                member.branch.as_deref().unwrap_or("its branch")
                            ),
                            cx,
                        ));
                    }
                }
                d.child(self.footer(
                    button(
                        "sessions.resume.confirm",
                        "Recreate and resume",
                        Kind::Primary,
                        !self.busy,
                        run,
                        cx,
                    ),
                    cx,
                ))
            }
            Manage::Cleanup(scope) => {
                if let Some(repo) = scope {
                    let repo = Workspace::get(cx).name_of(repo);
                    d = d.child(detail(
                        format!("Remove {repo} from {} and keep its branch.", m.set.name),
                        cx,
                    ));
                }
                if let Some(repo) = &m.confirm {
                    let row = m.rows.iter().find(|(member, _)| &member.repo == repo);
                    if let Some((member, preview)) = row {
                        return d
                            .child(warning(
                                "sessions.remove.discard",
                                format!(
                                    "Discards {} in {}. Uncommitted files cannot be recovered.",
                                    preview
                                        .as_ref()
                                        .map_or_else(|_| "worktree contents".into(), cleanup_text),
                                    member.path
                                ),
                                cx,
                            ))
                            .child(self.footer(
                                button(
                                    "sessions.remove.discard.confirm",
                                    "Discard and remove",
                                    Kind::Danger,
                                    !self.busy,
                                    run,
                                    cx,
                                ),
                                cx,
                            ));
                    }
                }
                if self.loading.is_some() {
                    d = d.child(ui::spinner(t.accent));
                }
                let mut clean = 0;
                for (i, (member, preview)) in m
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, (m, _))| scope.as_ref().is_none_or(|r| r == &m.repo))
                {
                    let repo = member.repo.clone();
                    let last = m.set.members.iter().find(|m| m.repo == repo);
                    let error = last.and_then(|m| m.error.as_ref());
                    let ready = preview.as_ref().is_ok_and(Cleanup::clean);
                    if ready {
                        clean += 1;
                    }
                    d = d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(GAP / 2.)
                            .border_b_1()
                            .border_color(t.line)
                            .py(GAP)
                            .child(mono(member.path.clone(), cx))
                            .map(|d| match preview {
                                Ok(p) => d
                                    .child(detail(cleanup_text(p), cx))
                                    .child(detail(
                                        match &p.teardown {
                                            Ok(Some(script)) => format!("Teardown: {script}"),
                                            Ok(None) => "No teardown".into(),
                                            Err(e) => e.to_string(),
                                        },
                                        cx,
                                    ))
                                    .child(detail(
                                        if ready {
                                            "Remove the worktree and keep its branch"
                                        } else {
                                            "Keep the worktree"
                                        },
                                        cx,
                                    ))
                                    .when(p.locked, |d| {
                                        d.child(warning(
                                            ("sessions.locked", i),
                                            "Locked. git worktree unlock frees it.",
                                            cx,
                                        ))
                                    })
                                    .when(p.detached, |d| {
                                        d.child(warning(
                                            ("sessions.detached", i),
                                            "Detached HEAD. Check out a branch before removing it.",
                                            cx,
                                        ))
                                    })
                                    .when(
                                        !p.locked && !p.detached && (!ready || error.is_some()),
                                        |d| {
                                            let handle = cx.weak_entity();
                                            let repo = repo.clone();
                                            d.child(button(
                                                ("sessions.force", i),
                                                "Remove anyway…",
                                                Kind::Danger,
                                                !self.busy,
                                                move |_, cx| {
                                                    _ = handle.update(cx, |s, cx| {
                                                        if let Some(Dialog::Manage(m)) =
                                                            &mut s.dialog
                                                        {
                                                            m.confirm = Some(repo.clone());
                                                        }
                                                        cx.notify();
                                                    });
                                                },
                                                cx,
                                            ))
                                        },
                                    ),
                                Err(e) => d.child(warning(
                                    ("sessions.cleanup.error", i),
                                    e.to_string(),
                                    cx,
                                )),
                            })
                            .when_some(error, |d, e| {
                                d.child(warning(("sessions.cleanup.result", i), e.clone(), cx))
                            }),
                    );
                }
                for (i, member) in m
                    .set
                    .members
                    .iter()
                    .filter(|m| m.stage == Stage::Removed)
                    .enumerate()
                {
                    d = d.child(
                        div()
                            .id(("sessions.cleanup.removed", i))
                            .test_support()
                            .aria_label(member.path.clone())
                            .child(detail(format!("Removed {}", member.path), cx)),
                    );
                }
                if let Some(repo) = scope
                    && m.set.archived.is_none()
                    && self.loading.is_none()
                {
                    let (ws, set, repo) = (m.ws.clone(), m.set.clone(), repo.clone());
                    let handle = cx.weak_entity();
                    d = d.child(button(
                        "sessions.drop",
                        "Keep the worktree, drop it from the session",
                        Kind::Quiet,
                        !self.busy,
                        move |window, cx| {
                            _ = handle.update(cx, |s, cx| {
                                if !s.refuse(&set, std::slice::from_ref(&repo), cx) {
                                    s.commit(
                                        ws.clone(),
                                        set.clone(),
                                        Commit::Drop(repo.clone()),
                                        window,
                                        cx,
                                    );
                                }
                            });
                        },
                        cx,
                    ));
                }
                let label = match clean {
                    0 => "Remove clean worktrees".into(),
                    1 => "Remove 1 clean worktree".into(),
                    n => format!("Remove {n} clean worktrees"),
                };
                d.child(detail(
                    "A kept branch does not bring back an uncommitted file.",
                    cx,
                ))
                .when(clean == 0 && self.loading.is_none(), |d| {
                    d.child(detail("None is clean.", cx))
                })
                .child(self.footer(
                    button(
                        "sessions.remove.clean",
                        label,
                        Kind::Danger,
                        clean > 0 && !self.busy,
                        run,
                        cx,
                    ),
                    cx,
                ))
            }
        }
    }

    fn render_result(
        &self,
        mut d: gpui_kit::Stateful<gpui_kit::Div>,
        ws: &str,
        set: &Workset,
        open: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let count = set.members.iter().filter(|m| m.state.usable()).count();
        if !self.busy {
            let message = if count == set.members.len() {
                format!("{} is ready.", set.name)
            } else {
                format!(
                    "{} of {} repositories have worktrees.",
                    count,
                    set.members.len()
                )
            };
            d = d.child(detail(message, cx));
        }
        for (i, m) in set.members.iter().enumerate() {
            let repo = m.repo.clone();
            let (workspace, workset) = (ws.to_owned(), set.clone());
            let path = m.path.clone();
            let handle = cx.weak_entity();
            d = d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(GAP / 2.)
                    .py(GAP)
                    .child(mono(path.clone(), cx))
                    .child(detail(m.state.label(), cx))
                    .children(m.steps.iter().map(|step| {
                        detail(step.clone(), cx).when(step.starts_with("Fetch failed"), |d| {
                            d.text_color(theme(cx).warning)
                        })
                    }))
                    .when_some(
                        self.current_op(cx).filter(|op| op.repo == m.repo),
                        |d, op| {
                            d.child(
                                div()
                                    .id("sessions.provision.progress")
                                    .test_support()
                                    .aria_label(op.line)
                                    .children(op.recent.into_iter().map(|line| mono(line, cx))),
                            )
                        },
                    )
                    .when_some(m.error.clone(), |d, e| {
                        let (message, tail) = e.split_once('\n').unwrap_or((&e, ""));
                        d.child(warning(
                            ("sessions.result.error", i),
                            message.to_owned(),
                            cx,
                        ))
                        .when(!tail.trim().is_empty(), |d| {
                            d.child(
                                mono(tail.to_owned(), cx)
                                    .rounded(ui::radius::WIDGET)
                                    .bg(theme(cx).code_block_bg)
                                    .p(GAP),
                            )
                        })
                    })
                    .when(
                        !self.busy && (!m.state.usable() || m.stage != Stage::Ready),
                        |d| {
                            let ws = workspace.clone();
                            let set = workset.clone();
                            let repo = repo.clone();
                            let handle = handle.clone();
                            let input = self
                                .retries
                                .get(&m.repo)
                                .filter(|_| m.stage == Stage::Pending)
                                .cloned();
                            let label = match m.stage {
                                Stage::Install => "Retry install",
                                Stage::Copy | Stage::Adding => "Retry copy",
                                Stage::Pending => "Create",
                                _ => "Create it again",
                            };
                            d.when_some(input.as_ref(), |d, input| {
                                d.child(field(
                                    "Another branch name",
                                    ("sessions.retry.branch", i),
                                    input,
                                    cx,
                                ))
                            })
                            .child(button(
                                ("sessions.retry", i),
                                label,
                                Kind::Quiet,
                                true,
                                move |window, cx| {
                                    _ = handle.update(cx, |s, cx| {
                                        s.stop = false;
                                        let branch = input
                                            .as_ref()
                                            .map(|input| input.read(cx).value().to_string());
                                        s.run(
                                            ws.clone(),
                                            set.clone(),
                                            vec![repo.clone()],
                                            Change::Provision(branch),
                                            window,
                                            cx,
                                        );
                                    });
                                },
                                cx,
                            ))
                        },
                    )
                    .when(
                        !self.busy && m.state.usable() && m.stage != Stage::Ready,
                        |d| {
                            d.child(button(
                                ("sessions.term", i),
                                "Terminal there",
                                Kind::Quiet,
                                true,
                                {
                                    let (path, ws, set) =
                                        (path.clone(), workspace.clone(), workset.clone());
                                    let handle = handle.clone();
                                    move |window, cx| {
                                        save(&ws, set.clone(), true, cx);
                                        _ = handle.update(cx, |s, cx| s.close(cx));
                                        dock::show(
                                            Doc::Terminal {
                                                id: 0,
                                                cwd: path.clone(),
                                            },
                                            window,
                                            cx,
                                        )
                                    }
                                },
                                cx,
                            ))
                            .child(button(
                                ("sessions.removeFailed", i),
                                "Remove this worktree",
                                Kind::Danger,
                                true,
                                move |_, cx| {
                                    manage(
                                        workset.clone(),
                                        Manage::Cleanup(Some(repo.clone())),
                                        cx,
                                    );
                                },
                                cx,
                            ))
                        },
                    ),
            );
        }
        let enabled = !self.busy && open && count > 0;
        let (set, ws) = (set.clone(), ws.to_owned());
        let title = format!("Open {}", set.name);
        d.child(self.footer(
            button(
                "sessions.openCreated",
                title,
                Kind::Primary,
                enabled,
                {
                    let handle = cx.weak_entity();
                    move |window, cx| {
                        save(&ws, set.clone(), true, cx);
                        _ = handle.update(cx, |s, cx| s.close(cx));
                        crate::files::choose_explorer(
                            crate::files::ExplorerMode::Files,
                            window,
                            cx,
                        );
                    }
                },
                cx,
            ),
            cx,
        ))
    }

    fn footer(&self, button: AnyElement, cx: &Context<Self>) -> gpui_kit::Div {
        div()
            .flex()
            .justify_end()
            .gap(GAP)
            .child(self.action(
                "sessions.provision.cancel",
                "Cancel",
                Kind::Quiet,
                true,
                |s, _, cx| s.close(cx),
                cx,
            ))
            .child(button)
    }

    fn command(
        &self,
        action: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &Context<Self>,
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        let handle = cx.weak_entity();
        move |window, cx| {
            _ = handle.update(cx, |s, cx| action(s, window, cx));
        }
    }

    fn action(
        &self,
        id: &'static str,
        label: &'static str,
        kind: Kind,
        enabled: bool,
        action: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &Context<Self>,
    ) -> AnyElement {
        button(id, label, kind, enabled, self.command(action, cx), cx)
    }
}

fn field(
    label: &str,
    id: impl Into<ElementId>,
    input: &Entity<InputState>,
    cx: &App,
) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap(GAP / 3.)
        .child(detail(label.to_owned(), cx))
        .child(ui::input(input, cx).id(id))
}

fn detail(text: impl Into<gpui_kit::SharedString>, cx: &App) -> gpui_kit::Div {
    div()
        .text_color(theme(cx).muted)
        .text_size(text::SMALL)
        .child(text.into())
}

fn mono(text: impl Into<gpui_kit::SharedString>, cx: &App) -> gpui_kit::Div {
    detail(text, cx).font_family(ui::theme::mono(cx))
}

fn warning(
    id: impl Into<ElementId>,
    message: impl Into<gpui_kit::SharedString>,
    cx: &App,
) -> AnyElement {
    let message = message.into();
    div()
        .id(id)
        .test_support()
        .aria_label(message.clone())
        .text_color(theme(cx).warning)
        .child(message)
        .into_any_element()
}

fn recipe_text(p: &Preview) -> String {
    let mut steps = vec![];
    if !p.recipe.copies.is_empty() {
        steps.push(format!("copies {}", p.recipe.copies.join(", ")));
    }
    if let Some(install) = &p.recipe.install {
        steps.push(format!("runs {install}"));
    }
    if steps.is_empty() {
        "No recipe".into()
    } else {
        format!("Recipe: {}", steps.join(" · "))
    }
}

pub(in crate::sessions) fn cleanup_text(p: &Cleanup) -> String {
    let mut parts = vec![];
    if p.changes > 0 {
        parts.push(format!("{} changed file(s)", p.changes));
    }
    if p.untracked > 0 {
        parts.push(format!("{} untracked file(s)", p.untracked));
    }
    if p.detached {
        parts.push("detached HEAD".into());
    }
    if p.submodules {
        parts.push("has submodules".into());
    }
    if parts.is_empty() {
        parts.push("clean".into());
    }
    if !p.ignored.is_empty() {
        parts.push(format!("ignored: {}", p.ignored.join(", ")));
    }
    parts.join(" · ")
}

enum Kind {
    Primary,
    Quiet,
    Danger,
}

fn button(
    id: impl Into<ElementId>,
    label: impl Into<gpui_kit::SharedString>,
    kind: Kind,
    enabled: bool,
    click: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let label = label.into();
    let button = match kind {
        Kind::Primary => ui::primary_button(id, label.clone(), cx),
        Kind::Quiet => ui::quiet_button(id, label.clone(), cx),
        Kind::Danger => ui::danger_button(id, label.clone(), cx),
    }
    .role(Role::Button)
    .aria_label(label)
    .when(!enabled, |d| d.opacity(0.5))
    .when(enabled, |d| {
        d.on_click(move |_, window, cx| click(window, cx))
    });
    ui::disabled(button, !enabled)
        .test_support()
        .into_any_element()
}
