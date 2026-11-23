//! The Stats page: the one change the numbers suggest and the dispositions by role, lens, model,
//! mode and verifier.

use std::time::Instant;

use detached_core::council::stats::{self, By, Row, Stats};
use gpui_kit::{Div, relative};

use super::live::tokens;
use super::*;

pub(super) struct StatsDocs;
impl DocKind for StatsDocs {
    fn category(&self, _: &Doc) -> &'static str {
        "chats"
    }
    fn view(&self, doc: &mut Doc, read_only: bool, _: &mut Window, cx: &mut App) -> AnyView {
        let Doc::CouncilStats { repo } = doc else {
            unreachable!()
        };
        cx.new(|cx| StatsView {
            rounds: Interest::view(kernel::council::rounds_view(cx, repo, read_only), cx),
            at: None,
            stats: Stats::new(),
        })
        .into()
    }
    fn id(&self, doc: &Doc) -> SharedString {
        let Doc::CouncilStats { repo } = doc else {
            unreachable!()
        };
        format!("doc.council-stats.{repo}").into()
    }
    fn title(&self, _: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        "Council stats".into()
    }
}

pub fn open(repo: &str, cx: &mut App) {
    let doc = Doc::CouncilStats { repo: repo.into() };
    Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx));
}

struct StatsView {
    rounds: Interest<Rounds>,
    /// When the rounds the numbers come from loaded.
    at: Option<Instant>,
    stats: Stats,
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
            let id = SharedString::from(format!("council.stats.{slug}.{key}"));
            table = table.child(line(cells).id(id).test_support().aria_label(label));
        }
        Some(table)
    }
}

impl Render for StatsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let resource = self.rounds.read(cx);
        if resource.at != self.at {
            if let Some(r) = &resource.value {
                self.stats = stats::stats(&r.records);
            }
            self.at = resource.at;
        }
        let error = resource.error.clone();
        let say = stats::suggest(&self.stats).unwrap_or_else(|| {
            "No change to suggest: none has five findings or more behind it.".into()
        });
        let tables = [By::Role, By::Lens, By::Model, By::Mode, By::Verifier];
        let tables: Vec<Div> = tables.iter().filter_map(|b| self.table(*b, cx)).collect();
        div()
            .id("council.stats")
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
                    .id("council.stats.suggestion")
                    .test_support()
                    .aria_label(say.clone())
                    .child(say),
            )
            .child(div().pt(GAP).text_size(text::HEADING).child("Dispositions"))
            .when(tables.is_empty(), |d| {
                d.child(ui::placeholder("No dispositions yet", cx))
            })
            .children(tables)
    }
}
