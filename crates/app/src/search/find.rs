//! Find in Files: matches stream in grouped by file; Enter or a click opens one at its line.

use std::collections::HashMap;
use std::iter;
use std::ops::Range;
use std::time::Duration;

use detached_core::api;
use detached_core::types::{FindEvent, FindQuery};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, BoxShadow, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, MouseMoveEvent, ParentElement, Pixels, Point, Render, Role, ScrollStrategy,
    StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder, px, uniform_list,
};

use super::Search;
use crate::actions::{self, Group, Registration};
use crate::store::{Location, QueryId, Store};
use crate::ui::{self, radius, text, theme};
use crate::util::{base_name, dir_name};
use crate::{blocking, files};

const DEBOUNCE: Duration = Duration::from_millis(150);
const TRUNCATED: &str = "Results truncated at 10,000 matches. Narrow the search.";
/// Match case, Words and Regex: id, label, title.
const TOGGLES: [(&str, &str, &str); 3] = [
    ("find.case", "Cc", "Match case"),
    ("find.word", "W", "Words"),
    ("find.regex", ".*", "Regex"),
];

struct Hit {
    line: u64,
    text: String,
    ranges: Vec<(u32, u32)>,
}

struct File {
    repo: String,
    path: String,
    hits: Vec<Hit>,
}

/// A file's header or one of its matches, by index into `files` and its `hits`.
#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Hit(usize, usize),
}

/// The query, repos and rev a search runs over.
type Ask = (FindQuery, Vec<String>, Option<String>);

pub struct Find {
    open: bool,
    input: Entity<InputState>,
    glob: Entity<InputState>,
    toggles: [bool; 3],
    all: bool,
    /// The search running or waiting for the debounce.
    asked: Option<Ask>,
    wait: Option<Task<()>>,
    query: Option<QueryId>,
    run: Option<Task<()>>,
    running: bool,
    truncated: bool,
    error: Option<String>,
    /// The rev the rows were found at.
    rev: Option<String>,
    files: Vec<File>,
    /// `repo\0path` to its index in `files`.
    index: HashMap<String, usize>,
    rows: Vec<Row>,
    count: usize,
    sel: Option<usize>,
    scroll: UniformListScrollHandle,
    /// A move to the same place (content scrolled under a resting pointer) does not select.
    mouse: Option<Point<Pixels>>,
    _subscriptions: Vec<Subscription>,
    _action: Registration,
}

fn show(open: bool, cx: &mut App) {
    cx.global_mut::<Search>().find = open;
}

impl Find {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Text to find"));
        let glob =
            cx.new(|cx| InputState::new(window, cx).placeholder("File mask: *.ts, !*.test.ts"));
        let edit = |this: &mut Self, _, e: &InputEvent, cx: &mut Context<Self>| {
            if let InputEvent::Change = e {
                this.changed(cx);
            }
        };
        let subscriptions = vec![
            cx.observe_global_in::<Search>(window, Self::opened),
            cx.observe(&Store::global(cx), |this, _, cx| this.changed(cx)),
            cx.subscribe(&input, edit),
            cx.subscribe(&glob, edit),
        ];
        let action = actions::register(
            cx,
            "find.inFiles",
            "Find in Files",
            Group::Search,
            |_, cx| show(true, cx),
        );
        Find {
            open: false,
            input,
            glob,
            toggles: [false; 3],
            all: false,
            asked: None,
            wait: None,
            query: None,
            run: None,
            running: false,
            truncated: false,
            error: None,
            rev: None,
            files: vec![],
            index: HashMap::new(),
            rows: vec![],
            count: 0,
            sel: None,
            scroll: UniformListScrollHandle::new(),
            mouse: None,
            _subscriptions: subscriptions,
            _action: action,
        }
    }

    /// Opening selects the kept text and searches it again; closing drops the search.
    fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open = cx.global::<Search>().find;
        if open == self.open {
            return;
        }
        (self.open, self.asked, self.wait) = (open, None, None);
        if open {
            self.input.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            });
            self.changed(cx);
        } else {
            self.start(cx);
        }
    }

    /// The current repo at its tree ref, or every repo's working tree.
    fn ask(&self, cx: &App) -> Ask {
        let store = Store::global(cx).read(cx);
        let [case_sensitive, whole_word, regex] = self.toggles;
        let glob = self.glob.read(cx).value().trim().to_string();
        let q = FindQuery {
            text: self.input.read(cx).value().to_string(),
            case_sensitive,
            whole_word,
            regex,
            glob: (!glob.is_empty()).then_some(glob),
        };
        if self.all {
            return (
                q,
                store.repos.iter().map(|r| r.path.clone()).collect(),
                None,
            );
        }
        let repo = store.current();
        let rev = repo.and_then(|r| store.tree_ref(r)).map(String::from);
        (q, repo.into_iter().map(String::from).collect(), rev)
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        let ask = self.open.then(|| self.ask(cx));
        if ask.is_none() || ask == self.asked {
            return;
        }
        self.asked = ask;
        cx.notify();
        self.wait = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |this, cx| this.start(cx)).ok();
        }));
    }

    /// Drops the last search and its rows, then runs `asked` unless its text or repos are empty.
    fn start(&mut self, cx: &mut Context<Self>) {
        (self.query, self.run, self.running) = (None, None, false);
        (self.truncated, self.error) = (false, None);
        self.files.clear();
        self.index.clear();
        self.rows.clear();
        (self.count, self.sel, self.scroll) = (0, None, UniformListScrollHandle::new());
        cx.notify();
        let Some((q, repos, rev)) = self.asked.clone() else {
            return;
        };
        if q.text.is_empty() || repos.is_empty() {
            return;
        }
        let store = Store::global(cx).read(cx);
        let (query, core) = (store.query_id(), store.core().clone());
        let (id, at) = (query.id, rev.clone());
        let this = cx.weak_entity();
        self.run = Some(blocking::stream(
            cx,
            move |send| api::find_in_files(&core, &repos, at.as_deref(), &q, id, send),
            move |batch, cx| {
                this.update(cx, |this, cx| this.take(batch, cx)).ok();
            },
        ));
        (self.query, self.running, self.rev) = (Some(query), true, rev);
    }

    fn take(&mut self, batch: Vec<FindEvent>, cx: &mut Context<Self>) {
        let current = self.query.as_ref().map(|q| q.id);
        let mut found = false;
        for e in batch {
            match e {
                FindEvent::Match {
                    query_id,
                    repo,
                    path,
                    line,
                    text,
                    ranges,
                } if Some(query_id) == current => {
                    let files = &mut self.files;
                    let key = format!("{repo}\0{path}");
                    let f = *self.index.entry(key).or_insert_with(|| {
                        files.push(File {
                            repo,
                            path,
                            hits: vec![],
                        });
                        files.len() - 1
                    });
                    files[f].hits.push(Hit { line, text, ranges });
                    self.count += 1;
                    found = true;
                }
                FindEvent::Done {
                    query_id,
                    truncated,
                } if Some(query_id) == current => {
                    (self.running, self.truncated) = (false, truncated)
                }
                FindEvent::Error { query_id, error } if Some(query_id) == current => {
                    (self.running, self.error) = (false, Some(error))
                }
                _ => {}
            }
        }
        if found {
            self.rows = (self.files.iter().enumerate())
                .flat_map(|(f, file)| {
                    iter::once(Row::File(f))
                        .chain((0..file.hits.len()).map(move |h| Row::Hit(f, h)))
                })
                .collect();
            self.sel.get_or_insert(1);
        }
        cx.notify();
    }

    /// To the next match up or down, past headers; it stops at the ends.
    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        let hit = |i: &usize| matches!(self.rows.get(*i), Some(Row::Hit(..)));
        let next = match down {
            true => (self.sel.map_or(0, |s| s + 1)..self.rows.len()).find(hit),
            false => (0..self.sel.unwrap_or(0)).rev().find(hit),
        };
        if let Some(i) = next {
            self.sel = Some(i);
            self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn choose(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(&Row::Hit(f, h)) = self.rows.get(i) else {
            return;
        };
        let file = &self.files[f];
        let loc = Location {
            repo: file.repo.clone(),
            rev: self.rev.clone(),
            path: file.path.clone(),
            line: Some(file.hits[h].line as u32),
            doc: None,
        };
        show(false, cx);
        files::open(cx, loc);
    }

    fn row(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let (f, h) = match self.rows[i] {
            Row::File(f) => {
                let file = &self.files[f];
                let (name, path) = (base_name(&file.path).to_string(), file.path.clone());
                let count = match self.all {
                    true => {
                        let repo = Store::global(cx).read(cx).name_of(&file.repo);
                        format!("{repo} {}", file.hits.len())
                    }
                    false => file.hits.len().to_string(),
                };
                return div()
                    .id(("find.file", i))
                    .test_support()
                    .aria_label(name.clone())
                    .aria_description(count.clone())
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(24.))
                    .px(px(8.))
                    .whitespace_nowrap()
                    .child(div().font_weight(FontWeight::BOLD).child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(t.muted)
                            .child(dir_name(&file.path).to_string()),
                    )
                    .child(div().text_color(t.muted).child(count))
                    .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx))
                    .into_any_element();
            }
            Row::Hit(f, h) => (f, h),
        };
        let hit = &self.files[f].hits[h];
        let on = self.sel == Some(i);
        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        let runs = highlight(&hit.text, &hit.ranges);
        let last = runs.len().saturating_sub(1);
        // Each run is an element, so a hit is a marked node; only the last run truncates.
        let runs = runs.into_iter().enumerate().map(|(k, (run, is_hit))| {
            let el = div().child(run.to_string()).map(|d| match k == last {
                true => d.flex_1().min_w_0().truncate(),
                false => d.flex_none(),
            });
            match is_hit {
                true => el
                    .id(k)
                    .test_support()
                    .role(Role::Mark)
                    .aria_label(run.to_string())
                    .text_color(t.hit)
                    .font_weight(FontWeight::BOLD)
                    .into_any_element(),
                false => el.into_any_element(),
            }
        });
        div()
            .id(("find.match", i))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(hit.text.trim_start().to_string())
            .aria_selected(on)
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(24.))
            .pl(px(20.))
            .pr(px(8.))
            .rounded(radius::SEGMENT)
            .font_family(theme::mono(cx))
            .text_size(text::SMALL)
            .whitespace_nowrap()
            .when(on, |r| r.bg(t.selected).shadow(vec![ring]))
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                if this.mouse.replace(e.position) != Some(e.position) && this.sel != Some(i) {
                    this.sel = Some(i);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.choose(i, cx)))
            .child(
                div()
                    .flex_none()
                    .w(px(40.))
                    .text_right()
                    .text_color(t.muted)
                    .child(hit.line.to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .children(runs),
            )
            .into_any_element()
    }
}

/// `text` without its indentation, in runs that say whether they are a hit. `ranges` are byte
/// ranges, clipped to the indentation's end; one that does not fit the text is skipped.
fn highlight<'a>(text: &'a str, ranges: &[(u32, u32)]) -> Vec<(&'a str, bool)> {
    let mut pos = text.len() - text.trim_start().len();
    let mut out = vec![];
    for &(s, e) in ranges {
        let (from, to) = ((s as usize).max(pos), e as usize);
        let Some(hit) = text.get(from..to).filter(|h| !h.is_empty()) else {
            continue;
        };
        if from > pos {
            out.push((&text[pos..from], false));
        }
        out.push((hit, true));
        pos = to;
    }
    if pos < text.len() {
        out.push((&text[pos..], false));
    }
    out
}

impl Render for Find {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div();
        }
        let t = theme(cx);
        let store = Store::global(cx).read(cx);
        let scope = match (self.all, store.current()) {
            (true, _) => "All repositories".into(),
            (false, None) => "No repository".into(),
            (false, Some(r)) => {
                let name = store.name_of(r);
                store
                    .tree_ref(r)
                    .map_or(name.to_string(), |rev| format!("{name} @ {rev}"))
            }
        };
        let n = |k: usize, one, many| format!("{k} {}", if k == 1 { one } else { many });
        let status = match self.running {
            true => format!("Searching… {}", self.count),
            false if self.input.read(cx).value().is_empty() => String::new(),
            false => format!(
                "{} in {}",
                n(self.count, "match", "matches"),
                n(self.files.len(), "file", "files")
            ),
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .pt(px(12.))
            .pr(px(16.))
            .pb(px(6.))
            .pl(px(18.))
            .child(
                div()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Find in Files"),
            )
            .child(
                div()
                    .id("find.status")
                    .test_support()
                    .aria_label(status.clone())
                    .flex_1()
                    .text_color(t.muted)
                    .child(status),
            )
            .child(
                Checkbox::new("find.all")
                    .checked(self.all)
                    .label("All repositories")
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.all = *on;
                        this.changed(cx);
                    })),
            );
        let line = || {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .py(px(4.))
                .px(px(12.))
        };
        let input = Input::new(&self.input)
            .id("find.text")
            .flex_1()
            .h(px(40.))
            .px(px(12.))
            .font_family(theme::mono(cx))
            .text_size(text::TITLE)
            .rounded(radius::FIELD)
            .bg(t.input);
        let mono = theme::mono(cx);
        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        // Not focusable, so a click leaves focus in the input.
        let toggles = TOGGLES.iter().enumerate().map(|(k, &(id, label, title))| {
            let on = self.toggles[k];
            ui::pressable(id)
                .test_support()
                .role(Role::Button)
                .aria_label(title)
                .aria_toggled(on.into())
                .justify_center()
                .min_w(px(32.))
                .h(px(30.))
                .rounded(radius::ICON_BUTTON)
                .font_family(mono.clone())
                .font_weight(FontWeight::BOLD)
                .map(|b| match on {
                    true => b.bg(t.selected).text_color(t.fg).shadow(vec![ring.clone()]),
                    false => b.text_color(t.muted).hover(|s| s.bg(t.hover)),
                })
                .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggles[k] ^= true;
                    this.changed(cx);
                }))
                .child(label)
        });
        let glob = Input::new(&self.glob)
            .id("find.glob")
            .w(px(280.))
            .h(px(28.))
            .px(px(9.))
            .text_size(text::SMALL)
            .rounded(radius::ITEM)
            .bg(t.input);
        let scope = div()
            .id("find.scope")
            .test_support()
            .aria_label(scope.clone())
            .flex_1()
            .text_right()
            .truncate()
            .text_size(text::SMALL)
            .text_color(t.muted)
            .child(scope);
        let list = uniform_list(
            "find.list",
            self.rows.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|i| this.row(i, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .mt(px(6.))
        .py(px(4.))
        .px(px(8.))
        .border_t_1()
        .border_color(t.line)
        .bg(t.code);
        let note = match &self.error {
            Some(e) => Some((e.clone(), t.error)),
            None => self.truncated.then(|| (TRUNCATED.to_string(), t.warning)),
        };
        let note = note.map(|(note, color)| {
            div()
                .id("find.note")
                .test_support()
                .aria_label(note.clone())
                .py(px(8.))
                .px(px(16.))
                .border_t_1()
                .border_color(t.line)
                .text_size(text::SMALL)
                .text_color(color)
                .child(note)
        });
        let size = window.viewport_size();
        let dialog = div()
            .id("find")
            .test_support()
            .role(Role::Dialog)
            .aria_label("Find in Files")
            .flex()
            .flex_col()
            .h(px(580.).min(size.height * 0.8))
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                show(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, _, cx| {
                if let Some(i) = this.sel {
                    this.choose(i, cx);
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.step(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.step(true, cx);
                cx.stop_propagation();
            }))
            .child(head)
            .child(line().child(input).children(toggles))
            .child(line().child(glob).child(scope))
            .child(list)
            .children(note);
        ui::modal(
            px(860.).min(size.width * 0.92),
            0.1,
            radius::DIALOG,
            |_, cx| show(false, cx),
            dialog,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_maps_byte_ranges_to_runs_and_drops_the_indentation() {
        assert_eq!(
            highlight("  let x = y", &[(6, 7)]),
            [("let ", false), ("x", true), (" = y", false)]
        );
        // é is 2 bytes, 😀 is 4.
        assert_eq!(
            highlight("é😀ab", &[(6, 7)]),
            [("é😀", false), ("a", true), ("b", false)]
        );
        assert_eq!(highlight("ab", &[]), [("ab", false)]);
    }

    #[test]
    fn highlight_clips_a_range_in_the_indentation() {
        assert_eq!(
            highlight("    foo", &[(1, 6)]),
            [("fo", true), ("o", false)]
        );
        assert_eq!(highlight("    foo", &[(0, 2)]), [("foo", false)]);
    }
}
