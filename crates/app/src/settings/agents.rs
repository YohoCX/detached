//! The Agents page: each provider's own CLI, found, checked and logged in from a terminal tab, and an
//! ACP provider's adapter, installed from the catalog, updated and rolled back.

use detached_core::adapters::{self, Cli, PROVIDERS};
use detached_core::backend::Name;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use kernel::agents::{Agents, Row, State};

use crate::terminal;
use crate::ui::page::{button, card, item, labeled, layout, path};
use crate::ui::{self, text, theme};

pub fn page(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let paths = PROVIDERS.map(|p| {
            let hint = format!("Path to {}, empty for the one on PATH", p.cli);
            cx.new(|cx| InputState::new(window, cx).placeholder(hint))
        });
        Page {
            paths,
            loaded: false,
            _agents: cx.observe(&Agents::global(cx), |_, _, cx| cx.notify()),
        }
    })
    .into()
}

struct Page {
    /// By `PROVIDERS`' order.
    paths: [Entity<InputState>; 2],
    loaded: bool,
    _agents: Subscription,
}

fn act(
    f: impl Fn(&mut Agents, &mut Context<Agents>) + 'static,
) -> impl Fn(&mut Page, &mut Window, &mut Context<Page>) {
    move |_, _, cx| Agents::global(cx).update(cx, |a, cx| f(a, cx))
}

/// The adapter's version and the kept one, or what its install is doing.
fn adapter(r: &Row) -> String {
    let name = r.provider.adapter;
    match (&r.state, &r.installed.current) {
        (State::Failed(e), _) => e.clone(),
        (State::Downloading { received, total }, _) => match total {
            Some(t) if *t > 0 => format!("Downloading {name}: {}%", received * 100 / t),
            _ => format!("Downloading {name}: {:.1} MB", *received as f64 / 1e6),
        },
        (State::Verifying, _) => format!("Verifying {name}…"),
        (State::Idle, None) => format!("{name} is not installed."),
        (State::Idle, Some(v)) => match &r.installed.previous {
            Some(p) => format!("{name} {v} · {p} kept for rollback"),
            None => format!("{name} {v}"),
        },
    }
}

impl Page {
    fn provider(&self, i: usize, r: &Row, a: &Agents, cx: &Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let (p, id) = (r.provider, r.provider.id);
        let el = |what: &str| SharedString::from(format!("agents.{id}.{what}"));
        let offer = a
            .offer(id)
            .filter(|e| r.installed.current.as_ref() != Some(&e.version));
        let mut adapter_row = item(
            div()
                .child("Adapter")
                .child(
                    labeled(el("adapter"), adapter(r))
                        .text_size(text::SMALL)
                        .text_color(match r.state {
                            State::Failed(_) => t.error,
                            _ => t.muted,
                        }),
                )
                .children(offer.filter(|_| !r.busy()).map(|e| {
                    let text = format!("{} {} is available.", p.adapter, e.version);
                    labeled(el("offer"), text)
                        .text_size(text::SMALL)
                        .text_color(t.success)
                })),
            None,
            cx,
        );
        if offer.is_some() && !r.busy() {
            let label = if r.installed.current.is_some() {
                "Update"
            } else {
                "Install"
            };
            adapter_row = adapter_row.child(button(
                el("install"),
                label,
                true,
                cx,
                act(move |a, cx| a.install(id, cx)),
            ));
        }
        if r.busy() {
            adapter_row = adapter_row.child(button(
                el("cancel"),
                "Cancel",
                true,
                cx,
                act(move |a, _| a.cancel(id)),
            ));
        }
        if r.installed.previous.is_some() && !r.busy() {
            adapter_row = adapter_row.child(button(
                el("rollback"),
                "Roll back",
                true,
                cx,
                act(move |a, cx| a.rollback(id, cx)),
            ));
        }
        let verdict = r.verdict(a.catalog.as_ref());
        let c = p.cli;
        let (cli, said) = match &r.cli {
            None => (format!("{c} was not found."), "Not found".to_string()),
            Some((path, None)) => (
                format!("{} printed no version.", path.display()),
                "Printed no version".into(),
            ),
            Some((path, Some(v))) => match &verdict {
                Some(Cli::Old(need)) => {
                    let s = format!("{c} {v} is older than {need}: update {c}.");
                    (s.clone(), s)
                }
                Some(Cli::Blocked) => {
                    let of = if p.backend == Name::Acp {
                        " for this adapter"
                    } else {
                        ""
                    };
                    let s = format!(
                        "{c} {v} is blocked{of}: update {c}. The chat warns; isolated councils refuse it."
                    );
                    (s.clone(), s)
                }
                _ => (format!("{c} {v} at {}", path.display()), format!("{c} {v}")),
            },
        };
        let warn = !matches!(verdict, None | Some(Cli::Ok)) || r.cli.is_none();
        let found = r.cli.as_ref().map(|c| c.0.clone());
        let input = self.paths[i].clone();
        let login = found.clone().map(|cli| adapters::login(p, &cli));
        let cli = div()
            .id(el("cli"))
            .test_support()
            .aria_label(cli)
            .text_size(text::SMALL)
            .text_color(if warn { t.warning } else { t.muted })
            .child(said);
        let cli_row = item(div().child("CLI").child(cli), None, cx)
            .children(found.map(|f| path(el("found"), f.display().to_string(), cx)))
            .when_some(login, |row, line| {
                row.child(button(el("login"), "Log in", true, cx, move |_, w, cx| {
                    terminal::run(&line, w, cx);
                    crate::settings::close(cx);
                }))
            })
            .when_some(
                adapters::installer(p).filter(|_| r.cli.is_none()),
                |row, line| {
                    row.child(button(
                        el("installer"),
                        "Run installer",
                        true,
                        cx,
                        move |_, w, cx| {
                            terminal::run(line, w, cx);
                            crate::settings::close(cx);
                        },
                    ))
                },
            )
            .child(button(
                el("rescan"),
                "Check again",
                true,
                cx,
                act(|a, cx| a.load(cx)),
            ));
        let path_row = item("Custom path", None, cx)
            .child(
                div()
                    .w(px(280.))
                    .child(ui::input(&input, cx).id(el("path"))),
            )
            .child(button(el("save"), "Save", true, cx, move |_, _, cx| {
                let path = input.read(cx).value().to_string();
                Agents::global(cx).update(cx, |a, cx| a.set_cli(id, path, cx));
            }));
        let mut rows = vec![];
        if p.backend == Name::Acp {
            rows.push(adapter_row);
        }
        rows.extend([cli_row, path_row]);
        div()
            .flex()
            .flex_col()
            .child(
                labeled(el("name"), p.name)
                    .px(px(4.))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(card(None, None, rows, cx))
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Here, not when Settings opens: finding a CLI runs it.
        if !self.loaded {
            self.loaded = true;
            Agents::global(cx).update(cx, |a, cx| a.load(cx));
        }
        let agents = Agents::global(cx);
        let a = agents.read(cx);
        let catalog = match &a.catalog {
            Some(c) => format!("Adapters catalog {}.", c.serial),
            None => "The adapters catalog was not checked yet.".into(),
        };
        let check = if a.checking {
            "Checking…"
        } else {
            "Check for updates"
        };
        let about = "Each provider's own CLI, and the adapter an ACP provider runs through.";
        let mut page = layout("settings.agents", "Agents", about, None, cx);
        if let Some(e) = &a.error {
            page = page.child(labeled("agents.error", e.clone()).text_color(theme(cx).error));
        }
        for (i, r) in a.rows.iter().enumerate() {
            if a.config.shows(r.provider) {
                page = page.child(self.provider(i, r, a, cx));
            }
        }
        let catalog = item(
            labeled("agents.catalog", catalog),
            Some("Signed list of adapter releases".into()),
            cx,
        )
        .child(button(
            "agents.check",
            check,
            !a.checking,
            cx,
            act(|a, cx| a.check(cx)),
        ));
        page.child(card(Some("Adapters"), None, [catalog], cx))
    }
}
