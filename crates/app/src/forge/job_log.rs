use std::ops::Range;
use std::time::Duration;

use detached_core::api;
use detached_core::types::{Job, JobLogChunk, Project, Step};
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement,
    ListHorizontalSizingBehavior, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder, px, rgb, uniform_list,
};

use super::ansi::{Color, LogParser, Span, visible};
use super::pipelines::{Seen, job_key, status};
use super::{core, glyph};
use crate::blocking;
use crate::store::{Doc, Event, Store};
use crate::ui::theme::{Theme, mono};
use crate::ui::{self, Icon, text, theme};

const MB: usize = 1 << 20;

/// A job's log, followed while the job runs.
pub struct JobLog {
    doc: Doc,
    repo: String,
    project: Project,
    id: u64,
    name: String,
    parser: LogParser,
    /// The indexes in `parser.lines` outside folded sections.
    rows: Vec<usize>,
    /// The longest of `rows`, which sets the list's width.
    widest: usize,
    /// Its status and steps, from Pipelines, read again as the log grows.
    job: Option<Job>,
    following: bool,
    cut: bool,
    error: Option<String>,
    scroll: UniformListScrollHandle,
    /// Keeps the bottom in view; a fold toggle lets go until the next scroll.
    stick: bool,
    /// The scroll offset last rendered: a scroll since then pins or unpins.
    offset: Pixels,
    _fetch: Task<()>,
    _shown: Subscription,
}

impl JobLog {
    pub fn new(doc: Doc, cx: &mut Context<Self>) -> Self {
        let Doc::Job {
            repo,
            project,
            id,
            name,
        } = doc.clone()
        else {
            unreachable!("the shell's pages are MRs and job logs")
        };
        // Opening the shown job again starts it over, as it does after a retry.
        let shown = cx.subscribe(&Store::global(cx), |this: &mut Self, store, e, cx| {
            if matches!(e, Event::Shown) && store.read(cx).shown() == Some(&this.doc) {
                this.start(cx);
            }
        });
        let mut this = JobLog {
            doc,
            repo,
            project,
            id,
            name,
            parser: LogParser::default(),
            rows: vec![],
            widest: 0,
            job: None,
            following: true,
            cut: false,
            error: None,
            scroll: UniformListScrollHandle::new(),
            stick: true,
            offset: px(0.),
            _fetch: Task::ready(()),
            _shown: shown,
        };
        this.start(cx);
        this
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let key = job_key(&self.project, self.id);
        let known = cx.try_global::<Seen>().and_then(|s| s.0.get(&key).cloned());
        (self.parser, self.rows, self.widest) = (LogParser::default(), vec![], 0);
        (self.following, self.cut, self.error) = (true, false, None);
        (self.scroll, self.stick, self.offset) = (UniformListScrollHandle::new(), true, px(0.));
        self.job = known.as_ref().map(|k| k.0.clone());
        let pipeline = known.map(|k| k.1);
        let (core, repo, project, id) =
            (core(cx), self.repo.clone(), self.project.clone(), self.id);
        cx.notify();
        self._fetch = cx.spawn(async move |this, cx| {
            let (mut offset, mut raw) = (0, String::new());
            loop {
                let (c, r, p) = (core.clone(), repo.clone(), project.clone());
                let log = move || api::job_log(&c, &r, &p, id, offset);
                let chunk = match cx.update(|cx| blocking::run(cx, log)).await {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        let fail = |this: &mut Self, cx: &mut Context<Self>| {
                            (this.error, this.following) = (Some(e.to_string()), false);
                            cx.notify();
                        };
                        this.update(cx, fail).ok();
                        return;
                    }
                };
                offset = chunk.next;
                raw += &chunk.text;
                let cut = offset > 20 * MB as u64;
                let Ok(live) = this.update(cx, |this, cx| {
                    this.parse(&chunk, cut.then_some(raw.as_str()), cx)
                }) else {
                    return;
                };
                if cut {
                    return;
                }
                if let Some(pipeline) = pipeline.filter(|_| !live || chunk.complete) {
                    let (c, r, p) = (core.clone(), repo.clone(), project.clone());
                    let jobs = move || api::pipeline_jobs(&c, &r, &p, pipeline);
                    let stages = cx.update(|cx| blocking::run(cx, jobs)).await;
                    let mut jobs = stages.into_iter().flatten().flat_map(|s| s.jobs);
                    if let Some(j) = jobs.find(|j| j.id == id) {
                        let set = |this: &mut Self, cx: &mut Context<Self>| {
                            this.job = Some(j);
                            cx.notify();
                        };
                        this.update(cx, set).ok();
                    }
                }
                if chunk.complete {
                    return;
                }
                let github =
                    this.read_with(cx, |this, _| this.job.as_ref().is_some_and(|j| !j.live_log));
                let secs = match (github, offset < 2 * MB as u64) {
                    (Ok(true), _) => 5,
                    (_, true) => 3,
                    _ => 10,
                };
                cx.background_executor()
                    .timer(Duration::from_secs(secs))
                    .await;
            }
        });
    }

    /// Parses a chunk, or past 20 MB only the last ~1 MB of `raw`; says whether the log is live.
    fn parse(&mut self, chunk: &JobLogChunk, raw: Option<&str>, cx: &mut Context<Self>) -> bool {
        if let Some(raw) = raw {
            let from = raw.len().saturating_sub(MB);
            let line = raw.as_bytes()[from..].iter().position(|&b| b == b'\n');
            self.parser = LogParser::default();
            self.parser
                .push(&raw[line.map_or(0, |i| from + i + 1)..], true);
            (self.cut, self.following) = (true, false);
        } else {
            self.parser.push(&chunk.text, chunk.complete);
            self.following = !chunk.complete;
        }
        if raw.is_some() || !chunk.text.is_empty() {
            self.show();
        }
        cx.notify();
        self.job.as_ref().is_some_and(|j| j.live_log)
    }

    fn show(&mut self) {
        self.rows = visible(&self.parser.lines);
        let lines = &self.parser.lines;
        let width = |i: usize| {
            lines[i].depth * 2 + lines[i].spans.iter().map(|s| s.text.len()).sum::<usize>()
        };
        let widest = self.rows.iter().enumerate().max_by_key(|(_, i)| width(**i));
        self.widest = widest.map_or(0, |(k, _)| k);
        if self.stick {
            self.scroll.scroll_to_bottom();
        }
    }

    fn toggle(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(h) = &mut self.parser.lines[i].head {
            h.open = !h.open;
        }
        self.stick = false;
        self.show();
        cx.notify();
    }

    fn header(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let at = format!("{} #{}", self.project.path, self.id);
        let url = self
            .job
            .as_ref()
            .map(|j| j.web_url.clone())
            .filter(|u| !u.is_empty());
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(44.))
            .px(px(16.))
            .border_b_1()
            .border_color(t.line)
            .whitespace_nowrap()
            .children(self.job.as_ref().map(|j| status(&j.status, cx)))
            .child(div().font_weight(FontWeight::BOLD).child(self.name.clone()))
            .child(note("job.at", at, t.muted))
            .child(div().flex_1())
            .when(self.following, |d| {
                d.child(note("job.following", "Following…", t.muted))
            })
            .when(self.cut, |d| {
                d.child(note(
                    "job.cut",
                    "Over 20 MB: showing the end only",
                    t.warning,
                ))
            })
            .when_some(url, |d, url| {
                d.child(
                    ui::button("job.open", "Open in Browser", cx)
                        .test_support()
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
            })
    }

    fn line(&self, i: usize, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let l = &self.parser.lines[i];
        let text: String = l.spans.iter().map(|s| s.text.as_str()).collect();
        let spans = l.spans.iter().enumerate().map(|(k, s)| span(k, s, t));
        div()
            .id(("line", i))
            .test_support()
            .aria_label(text)
            .flex()
            .items_center()
            .h(px(18.))
            .pl(px(12. + l.depth as f32 * 16.))
            .pr(px(12.))
            .when_some(l.head, |d, h| {
                let chevron = if h.open { Icon::Down } else { Icon::Chevron };
                d.aria_expanded(h.open)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(i, cx)))
                    .child(
                        div()
                            .flex_none()
                            .w(px(14.))
                            .ml(px(-14.))
                            .text_color(t.muted)
                            .child(chevron.size(px(11.))),
                    )
            })
            .children(spans)
    }

    fn lines(&self, cx: &mut Context<Self>) -> AnyElement {
        let row = cx.processor(|this, range: Range<usize>, _, cx| {
            range
                .map(|k| this.line(this.rows[k], cx))
                .collect::<Vec<_>>()
        });
        uniform_list("job.lines", self.rows.len(), row)
            .track_scroll(&self.scroll)
            .with_width_from_item(Some(self.widest))
            .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
            .flex_1()
            .bg(theme(cx).code)
            .font_family(mono(cx))
            .text_size(text::SMALL)
            .line_height(px(18.))
            .whitespace_nowrap()
            .into_any_element()
    }

    fn steps(&self, steps: &[Step], cx: &Context<Self>) -> AnyElement {
        let note = "GitHub serves the log once the job has finished.";
        let step = |(i, s): (usize, &Step)| {
            div()
                .id(("step", i))
                .test_support()
                .aria_label(format!("{}{}", glyph(&s.status), s.name))
                .flex()
                .gap(px(8.))
                .child(status(&s.status, cx))
                .child(s.name.clone())
        };
        div()
            .id("job.steps")
            .test_support()
            .aria_label(note)
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(16.))
            .py(px(12.))
            .child(div().text_color(theme(cx).muted).child(note))
            .children(steps.iter().enumerate().map(step))
            .into_any_element()
    }
}

impl Render for JobLog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (at, max) = {
            let s = self.scroll.0.borrow();
            (s.base_handle.offset().y, s.base_handle.max_offset().y)
        };
        if at != self.offset {
            (self.offset, self.stick) = (at, -at >= max - px(4.));
        }
        let t = theme(cx);
        let github = self.job.as_ref().filter(|j| self.following && !j.live_log);
        let body = if let Some(e) = &self.error {
            note("job.error", e.clone(), t.error)
                .px(px(16.))
                .py(px(12.))
                .into_any_element()
        } else if let Some(j) = github {
            self.steps(&j.steps, cx)
        } else if self.rows.is_empty() {
            let empty = if self.following {
                "Waiting for output…"
            } else {
                "Empty log"
            };
            note("job.empty", empty, t.muted)
                .px(px(16.))
                .py(px(12.))
                .into_any_element()
        } else {
            self.lines(cx)
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.header(cx))
            .child(body)
    }
}

/// Text that tests find by its id and read by its label.
fn note(id: &'static str, text: impl Into<SharedString>, color: Hsla) -> impl IntoElement + Styled {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .text_color(color)
        .child(text)
}

/// A span's text in its style; a link opens in the browser. Its color is its description.
fn span(k: usize, s: &Span, t: &Theme) -> impl IntoElement + use<> {
    let st = &s.style;
    let hsla = |c: Color| match c {
        Color::Ansi(n) => t.ansi[n as usize],
        Color::Rgb(r, g, b) => rgb(u32::from_be_bytes([0, r, g, b])).into(),
    };
    let href = st.href.clone();
    div()
        .id(k)
        .test_support()
        .aria_label(s.text.clone())
        .when_some(st.fg, |d, c| {
            d.aria_description(c.to_string()).text_color(hsla(c))
        })
        .when_some(st.bg, |d, c| d.bg(hsla(c)))
        .when(st.bold, |d| d.font_weight(FontWeight::BOLD))
        .when(st.dim, |d| d.opacity(0.6))
        .when(st.italic, |d| d.italic())
        .when(st.underline, |d| d.underline())
        .when_some(href, |d, href| {
            d.underline()
                .when(st.fg.is_none(), |d| d.text_color(t.accent))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&href);
                })
        })
        .child(s.text.clone())
}
