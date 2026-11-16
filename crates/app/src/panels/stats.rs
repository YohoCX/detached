//! The Stats page: the one change the numbers suggest, the dispositions by role, lens, model, mode
//! and verifier, the corpus's passes, and the button that runs a mode over the corpus.

use std::time::Instant;

use detached_core::panel::corpus::{self, Pass};
use detached_core::panel::stats::{self, By, Row, Stats};
use gpui_kit::{Div, EntityId, relative};
use kernel::panel::corpus::Corpus;

use super::live::tokens;
use super::*;

pub(super) struct StatsDocs;
impl DocKind for StatsDocs {
    fn view(&self, doc: &mut Doc, _: &mut Window, cx: &mut App) -> AnyView {
        let Doc::PanelStats { repo } = doc else {
            unreachable!()
        };
        let repo = repo.clone();
        cx.new(|cx| StatsView {
            rounds: Interest::view(kernel::panel::rounds(cx, &repo), cx),
            repo,
            gate: true,
            at: None,
            stats: Stats::new(),
            passes: vec![],
            corpus: None,
            refused: None,
        })
        .into()
    }
    fn id(&self, doc: &Doc) -> SharedString {
        let Doc::PanelStats { repo } = doc else {
            unreachable!()
        };
        format!("doc.panel-stats.{repo}").into()
    }
    fn title(&self, _: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        "Panel stats".into()
    }
}

pub fn open(repo: &str, cx: &mut App) {
    let doc = Doc::PanelStats { repo: repo.into() };
    Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx));
}

struct StatsView {
    repo: String,
    rounds: Interest<Rounds>,
    gate: bool,
    /// When the rounds the numbers come from loaded.
    at: Option<Instant>,
    stats: Stats,
    passes: Vec<Pass>,
    corpus: Option<(EntityId, Subscription)>,
    /// Why Run was refused.
    refused: Option<String>,
}

fn pct(a: usize, n: usize) -> String {
    match n {
        0 => "–".into(),
        n => format!("{}%", a * 100 / n),
    }
}

fn cell(text: impl Into<SharedString>, w: f32) -> Div {
    div().w(relative(w)).min_w_0().truncate().child(text.into())
}

const COLUMNS: [f32; 8] = [0.2, 0.08, 0.08, 0.26, 0.08, 0.08, 0.1, 0.12];

impl StatsView {
    fn table(&self, by: By, cx: &App) -> Option<Div> {
        let rows: Vec<(&String, &Row)> = (self.stats.iter())
            .filter(|((b, _), _)| *b == by)
            .map(|((_, k), r)| (k, r))
            .collect();
        if rows.is_empty() {
            return None;
        }
        let (title, raised) = match by {
            By::Role => ("By role", "raised"),
            By::Lens => ("By lens", "raised"),
            By::Model => ("By model", "raised"),
            By::Mode => ("By mode", "raised"),
            By::Verifier => ("By verifier", "judged"),
        };
        let line = |cells: [String; 8]| {
            div()
                .flex()
                .gap(GAP)
                .min_w_0()
                .children(cells.into_iter().zip(COLUMNS).map(|(t, w)| cell(t, w)))
        };
        let head = [
            title,
            raised,
            "fixed",
            "dismissed",
            "deferred",
            "open",
            "set aside",
            "per fixed",
        ];
        let slug = format!("{by:?}").to_lowercase();
        let mut table = div().flex().flex_col().min_w_0().child(
            line(head.map(String::from))
                .text_size(text::SMALL)
                .text_color(theme(cx).muted),
        );
        for (key, r) in rows {
            let why = r
                .reason()
                .map_or(String::new(), |w| format!(", most often \"{w}\""));
            let dismissed = match r.false_confirms {
                0 => format!("{}{why}", pct(r.dismissed, r.raised)),
                n => format!(
                    "{}, {n} confirmed by verify{why}",
                    pct(r.dismissed, r.raised)
                ),
            };
            let cells = [
                key.clone(),
                r.raised.to_string(),
                pct(r.fixed, r.raised),
                dismissed,
                r.deferred.to_string(),
                r.open().to_string(),
                r.aside.to_string(),
                r.per_fixed().map_or("–".into(), tokens),
            ];
            let label = format!(
                "{key} · {} {raised} · {} fixed · {} dismissed · {} per fixed",
                cells[1], cells[2], cells[3], cells[7]
            );
            let id = SharedString::from(format!("panel.stats.{slug}.{key}"));
            table = table.child(line(cells).id(id).test_support().aria_label(label));
        }
        Some(table)
    }

    /// The corpus's run going on, or the button that starts one behind its estimate.
    fn corpus(&mut self, cx: &mut Context<Self>) -> Div {
        let mut out = div().flex().flex_col().gap(GAP).min_w_0();
        let c = kernel::panel::corpus::current(cx);
        if let Some(c) = &c
            && self.corpus.as_ref().map(|w| w.0) != Some(c.entity_id())
        {
            let watch = cx.observe(c, |_, _, cx| cx.notify());
            self.corpus = Some((c.entity_id(), watch));
        }
        match c.as_ref().map(|c| c.read(cx)) {
            Some(c) if c.ended.is_none() => out.child(self.running(c, cx)),
            last => {
                let last = last.filter(|c| c.repo == self.repo);
                let mode = if self.gate { "gate" } else { "review" };
                let n = corpus::subjects().len();
                let rounds = self.rounds.read(cx).value.as_ref();
                let estimate = rounds.map(|r| kernel::panel::corpus::estimate(r, mode, n));
                if let Some(c) = last {
                    let text = format!(
                        "Last corpus run: {}, {} of {} subjects, {} tokens",
                        c.ended.clone().unwrap_or_default(),
                        c.finished,
                        c.total,
                        tokens(c.spent)
                    );
                    out = out.child(
                        div()
                            .id("panel.stats.last")
                            .test_support()
                            .aria_label(text.clone())
                            .child(text),
                    );
                }
                let me = cx.entity().downgrade();
                let pick = ui::segmented(
                    "panel.stats.mode",
                    &[(false, "Review"), (true, "Gate")],
                    self.gate,
                    move |gate, _, cx| {
                        me.update(cx, |v, cx| {
                            v.gate = gate;
                            cx.notify();
                        })
                        .ok();
                    },
                    cx,
                );
                let run = estimate.map(|e| {
                    let label = format!(
                        "Run the {mode} over the corpus's {n} subjects: about {} tokens, its budget",
                        tokens(e)
                    );
                    ui::button("panel.stats.corpus", label.clone(), cx)
                        .test_support()
                        .aria_label(label)
                        .on_click(cx.listener(move |v, _, _, cx| {
                            let all = corpus::subjects();
                            let started =
                                kernel::panel::corpus::start(v.repo.clone(), mode.into(), e, all, cx);
                            v.refused = started.err().map(|e| e.to_string());
                            cx.notify();
                        }))
                });
                let refused = self.refused.clone().map(|e| {
                    div()
                        .id("panel.stats.refused")
                        .test_support()
                        .aria_label(e.clone())
                        .text_color(theme(cx).error)
                        .child(e)
                });
                out.children(refused).child(
                    div()
                        .flex()
                        .gap(GAP)
                        .items_center()
                        .child(pick)
                        .children(run),
                )
            }
        }
    }

    fn running(&self, c: &Corpus, cx: &App) -> impl IntoElement {
        let (name, run) = match &c.current {
            Some((branch, p)) => {
                let p = p.read(cx);
                let name = corpus::of(branch).map_or(branch.as_str(), |x| x.1);
                let run = (p.repo.clone(), p.task.clone(), p.run.clone(), p.tokens(cx));
                (name.to_string(), Some(run))
            }
            None => ("preparing".into(), None),
        };
        let spent = c.spent + run.as_ref().map_or(0, |r| r.3);
        let elsewhere = match c.repo == self.repo {
            true => String::new(),
            false => format!(" on {}", c.repo),
        };
        let text = format!(
            "Corpus{elsewhere}: {} of {}, {name}, {} of {} tokens",
            (c.finished + 1).min(c.total),
            c.total,
            tokens(spent),
            tokens(c.budget)
        );
        let this = kernel::panel::corpus::current(cx);
        div()
            .flex()
            .gap(GAP)
            .items_center()
            .min_w_0()
            .child(
                div()
                    .id("panel.stats.progress")
                    .test_support()
                    .aria_label(text.clone())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .cursor_pointer()
                    .child(text)
                    .when_some(run, |d, (repo, task, round, _)| {
                        d.on_click(move |_, _, cx| {
                            let doc = Doc::Panel {
                                repo: repo.clone(),
                                task: task.clone(),
                                round: round.clone(),
                            };
                            Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx));
                        })
                    }),
            )
            .child(
                ui::button("panel.stats.cancel", "Cancel", cx)
                    .test_support()
                    .on_click(move |_, _, cx| {
                        if let Some(c) = &this {
                            c.update(cx, |c, cx| c.cancel(cx));
                        }
                    }),
            )
    }

    fn passes(&self, cx: &App) -> Div {
        let mut out = div().flex().flex_col().min_w_0();
        for (i, p) in self.passes.iter().enumerate() {
            let text = format!(
                "{}: caught {} of {} bugs, blocked {} of {} clean, {} of {} runs done, {} tokens",
                p.pass,
                p.caught,
                p.bugs,
                p.blocked,
                p.clean,
                p.done,
                p.started,
                tokens(p.tokens)
            );
            out = out.child(
                div()
                    .id(("panel.stats.pass", i))
                    .test_support()
                    .aria_label(text.clone())
                    .truncate()
                    .child(text),
            );
            for (model, v) in &p.verifiers {
                let text = format!(
                    "{model}: catch {} of {} ({}), false confirms {} of {} ({})",
                    v.caught,
                    v.bugs,
                    pct(v.caught, v.bugs),
                    v.false_confirms,
                    v.clean,
                    pct(v.false_confirms, v.clean)
                );
                out = out.child(
                    div()
                        .id(SharedString::from(format!("panel.stats.pass.{i}.{model}")))
                        .test_support()
                        .aria_label(text.clone())
                        .pl(GAP)
                        .text_color(theme(cx).muted)
                        .truncate()
                        .child(text),
                );
            }
        }
        out
    }
}

impl Render for StatsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let resource = self.rounds.read(cx);
        if resource.at != self.at {
            if let Some(r) = &resource.value {
                self.stats = stats::stats(&r.records);
                self.passes = corpus::calibrate(&r.records, &corpus::subjects());
            }
            self.at = resource.at;
        }
        let error = resource.error.clone();
        let say = stats::suggest(&self.stats, &self.passes).unwrap_or_else(|| {
            "No change to suggest: none has five findings or more behind it.".into()
        });
        let heading = |t: &'static str| div().pt(GAP).text_size(text::HEADING).child(t);
        let tables = [By::Role, By::Lens, By::Model, By::Mode, By::Verifier];
        let tables: Vec<Div> = tables.iter().filter_map(|b| self.table(*b, cx)).collect();
        let corpus = self.corpus(cx);
        div()
            .id("panel.stats")
            .test_support()
            .flex()
            .flex_col()
            .gap(GAP)
            .size_full()
            .min_w_0()
            .p(GAP)
            .overflow_y_scroll()
            .when_some(error, |d, e| {
                d.child(div().text_color(theme(cx).error).child(e))
            })
            .child(
                div()
                    .id("panel.stats.suggestion")
                    .test_support()
                    .aria_label(say.clone())
                    .child(say),
            )
            .child(heading("Calibration"))
            .child(corpus)
            .child(self.passes(cx))
            .child(heading("Dispositions"))
            .when(tables.is_empty(), |d| {
                d.child(ui::placeholder("No dispositions yet", cx))
            })
            .children(tables)
    }
}
