//! Markdown on the kit's TextView: repo files with their images and links, MR text, mermaid.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use base64::Engine;
use detached_core::api;
use detached_core::state::AppState;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use futures::channel::oneshot;
use gpui_kit::base::markdown_ast::Node;
use gpui_kit::base::{MarkdownNode, TestSupportExt, TextView, TextViewState, TextViewStyle};
use gpui_kit::{
    AnyElement, App, AppContext, ClipboardItem, Context, Entity, FontWeight, HighlightStyle, Image,
    ImageFormat, ImageSource, InteractiveElement, IntoElement, ListOffset, ParentElement, Pixels,
    Render, SharedString, SharedUri, StatefulInteractiveElement, StyleRefinement, Styled, Task,
    WeakEntity, Window, div, img, px, relative,
};
use markdown::ParseOptions;
use percent_encoding::percent_decode_str;
use url::Url;

use super::api::{Links, Location, open};
use super::code::fence_language;
use crate::ui::theme::{self, doc, radius, text, theme};
use crate::ui::{self, Icon};
use kernel::blocking;

type Diagram = Result<Arc<Image>, SharedString>;

pub struct Markdown {
    links: Links,
    breaks: bool,
    state: Entity<TextViewState>,
    /// By URL as written; None while loading, or when it is no image.
    images: Arc<HashMap<String, Option<Arc<Image>>>>,
    want: UnboundedSender<String>,
    /// By source, with whether it was drawn dark.
    diagrams: HashMap<String, (bool, Diagram)>,
    /// Draws under way, by source and appearance.
    drawing: HashSet<(String, bool)>,
    /// A file's side padding, which centers its column.
    pad: Pixels,
    /// A long text's pieces still to show, last first, and the length shown so far.
    rest: Vec<String>,
    fed: usize,
    _prepare: Task<()>,
    _images: Task<()>,
}

impl Markdown {
    /// `breaks` renders a lone newline as a line break, as MR text does.
    pub fn new(
        text: impl Into<SharedString>,
        links: Links,
        breaks: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let (want, mut wanted) = unbounded();
        let images = cx.spawn(async move |this, cx| {
            while let Some(url) = wanted.next().await {
                if this.update(cx, |m, cx| m.load(url, cx)).is_err() {
                    break;
                }
            }
        });
        let mut md = Markdown {
            links,
            breaks,
            state: cx.new(|cx| TextViewState::markdown("", cx)),
            images: Arc::default(),
            want,
            diagrams: HashMap::new(),
            drawing: HashSet::new(),
            pad: doc::PAD_X,
            rest: vec![],
            fed: 0,
            _prepare: Task::ready(()),
            _images: images,
        };
        md.set_text(text.into(), cx);
        md
    }

    /// Shows new text in place; a long one is prepared off the UI thread.
    pub(super) fn set_text(&mut self, text: SharedString, cx: &mut Context<Self>) {
        let (links, breaks) = (self.links.clone(), self.breaks);
        if text.len() < PIECE {
            self._prepare = Task::ready(());
            let text = prepare(&text, &links, breaks);
            return self.show(vec![text], cx);
        }
        let job = blocking::run(cx, move || pieces(prepare(&text, &links, breaks)));
        self._prepare = cx.spawn(async move |this, cx| {
            let pieces = job.await;
            this.update(cx, |m, cx| m.show(pieces, cx)).ok();
        });
    }

    /// A first text goes in a piece a frame, as the kit lays out a whole text in one
    /// (https://github.com/longbridge/gpui-kit/issues/3695); a new one replaces it at once.
    fn show(&mut self, mut pieces: Vec<String>, cx: &mut Context<Self>) {
        let text = match self.fed {
            0 => pieces.remove(0),
            _ => pieces.drain(..).collect(),
        };
        pieces.reverse();
        (self.rest, self.fed) = (pieces, text.len());
        self.state.update(cx, |s, cx| s.set_text(&text, cx));
    }

    /// The next piece, once the last one is parsed.
    fn feed(&mut self, cx: &mut Context<Self>) {
        let shown = self.state.read(cx).rendered_text().source().len();
        if shown != self.fed {
            return;
        }
        if let Some(piece) = self.rest.pop() {
            self.fed += piece.len();
            self.state.update(cx, |s, cx| s.push_str(&piece, cx));
        }
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.read(cx).focus_handle().clone().focus(window, cx);
    }

    fn load(&mut self, url: String, cx: &mut Context<Self>) {
        if self.images.contains_key(&url) {
            return;
        }
        Arc::make_mut(&mut self.images).insert(url.clone(), None);
        let Some(from) = image_url(&self.links, &url) else {
            return;
        };
        let job = if from.to_ascii_lowercase().starts_with("https:") {
            web_image(cx, from)
        } else {
            let core = kernel::core(cx).clone();
            let links = self.links.clone();
            blocking::run(cx, move || fetch(&core, &links, &from))
        };
        cx.spawn(async move |this, cx| {
            let image = job.await;
            this.update(cx, |m, cx| {
                Arc::make_mut(&mut m.images).insert(url, image);
                m.state.update(cx, |s, cx| s.invalidate_inline_layout(cx));
            })
            .ok();
        })
        .detach();
    }

    fn draw(&mut self, source: String, dark: bool, cx: &mut Context<Self>) {
        if !self.drawing.insert((source.clone(), dark)) {
            return;
        }
        let src = source.clone();
        let job = blocking::run(cx, move || mermaid(&src, dark));
        cx.spawn(async move |this, cx| {
            let diagram = job.await;
            this.update(cx, |m, cx| {
                m.drawing.remove(&(source.clone(), dark));
                m.diagrams.insert(source, (dark, diagram));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn click(&mut self, url: &str, cx: &mut Context<Self>) {
        let url = resolve_url(url, &self.links);
        if is_web(&url) {
            cx.open_url(&url);
        } else if let Some(anchor) = url.strip_prefix('#') {
            let id = percent_decode_str(anchor).decode_utf8_lossy();
            self.state.update(cx, |s, cx| {
                let text = s.rendered_text();
                let Some((block, range)) = heading(text.source(), &id) else {
                    return;
                };
                // Puts the heading at the top, as a browser does; reveal_range alone scrolls
                // only as far as its line.
                let top = ListOffset {
                    item_ix: block,
                    offset_in_item: px(0.),
                };
                s.list_state().scroll_to(top);
                if let Some(range) = text.range_for_source(range) {
                    s.reveal_range(range, cx).ok();
                }
                cx.notify();
            });
        } else if let Links::Repo { repo, rev, path } = &self.links
            && !url.is_empty()
            && !has_scheme(&url)
        {
            let loc = Location {
                repo: repo.clone(),
                rev: rev.clone(),
                path: repo_path(&url, path),
                line: None,
                doc: None,
            };
            open(cx, loc);
        }
    }

    /// Images as the rules allow, asking for each one not loaded yet.
    fn image_source(&self) -> impl Fn(&SharedUri) -> ImageSource + Send + Sync + 'static {
        let (images, want, links) = (self.images.clone(), self.want.clone(), self.links.clone());
        move |url| {
            let none = || ImageSource::Custom(Arc::new(|_, _| None));
            match images.get(url.as_ref()) {
                Some(Some(image)) => ImageSource::Image(image.clone()),
                Some(None) => none(),
                None => {
                    if image_url(&links, url).is_some() {
                        want.unbounded_send(url.to_string()).ok();
                    }
                    none()
                }
            }
        }
    }
}

impl Render for Markdown {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.feed(cx);
        let file = matches!(self.links, Links::Repo { .. });
        let this = cx.weak_entity();
        let click = this.clone();
        let view = TextView::new(&self.state)
            .style(style(file, cx))
            .image_source(self.image_source())
            // In place of the kit's language label, which Monaco's colorized blocks did not have.
            .code_block_actions(|block, _, cx| copy_button(block.code(), cx))
            .on_link_click(move |url, _, _, cx| {
                click.update(cx, |m, cx| m.click(url, cx)).ok();
            })
            .markdown_block_parser(|node, _| mermaid_block(node))
            .markdown_block_renderer("mermaid", move |node, _, cx| diagram(&this, node, cx));
        let root = div().id("markdown").test_support();
        if !file {
            return root.child(view.text_size(text::BODY)).into_any_element();
        }
        // The column is centered by padding, so the scrollbar stays at the edge.
        let list = self.state.read(cx).list_state().clone();
        let width = list.viewport_bounds().size.width + self.pad * 2.;
        self.pad = ((width - doc::WIDTH) / 2.).max(px(0.)) + doc::PAD_X;
        root.size_full()
            .bg(theme::theme(cx).code)
            .on_scroll(move |e, window, _| {
                if !list.viewport_bounds().contains(&e.position) {
                    list.scroll_by(-e.delta.pixel_delta(px(20.)).y);
                    window.refresh();
                }
            })
            .child(
                view.scrollable(true)
                    .size_full()
                    .pt(doc::PAD_TOP)
                    .pb(doc::PAD_BOTTOM)
                    .px(self.pad)
                    .text_size(doc::BODY)
                    .line_height(relative(doc::LINE_HEIGHT)),
            )
            .into_any_element()
    }
}

fn copy_button(code: SharedString, cx: &App) -> impl IntoElement + use<> {
    let id = SharedString::from(format!("md.copy.{}", code.len()));
    ui::titled(ui::icon_button(id, Icon::Copy.size(px(12.)), cx), "Copy")
        .text_color(theme(cx).muted)
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()))
        })
}

fn mermaid_block(node: &Node) -> Option<MarkdownNode> {
    match node {
        Node::Code(c) if c.lang.as_deref() == Some("mermaid") => {
            Some(MarkdownNode::new("mermaid", c.value.clone()).text(c.value.clone()))
        }
        _ => None,
    }
}

/// A mermaid block: the diagram, or its source until drawn and with the error when it cannot be.
fn diagram(this: &WeakEntity<Markdown>, node: &MarkdownNode, cx: &mut App) -> AnyElement {
    let source = node.data::<String>().cloned().unwrap_or_default();
    let dark = theme::is_dark(cx);
    let drawn = this
        .upgrade()
        .and_then(|m| m.read(cx).diagrams.get(&source).cloned());
    if drawn.as_ref().is_none_or(|d| d.0 != dark) {
        let (this, source) = (this.clone(), source.clone());
        cx.defer(move |cx| {
            this.update(cx, |m, cx| m.draw(source, dark, cx)).ok();
        });
    }
    let t = theme::theme(cx);
    let pre = div()
        .id("md.mermaid.source")
        .test_support()
        .aria_label(source.clone())
        .p(px(10.))
        .rounded(radius::ITEM)
        .bg(t.hover)
        .font_family(theme::mono(cx))
        .text_size(text::SMALL)
        .child(source);
    match drawn {
        Some((dark, Ok(image))) => div()
            .id("md.mermaid")
            .test_support()
            .aria_label("Diagram")
            .aria_description(if dark { "dark" } else { "light" })
            .flex()
            .justify_center()
            .child(img(image).max_w(relative(1.)))
            .into_any_element(),
        Some((_, Err(e))) => div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(pre)
            .child(
                div()
                    .id("md.mermaid.error")
                    .test_support()
                    .aria_label(e.clone())
                    .text_color(t.error)
                    .font_family(theme::mono(cx))
                    .text_size(text::SMALL)
                    .child(e),
            )
            .into_any_element(),
        None => pre.into_any_element(),
    }
}

fn mermaid(source: &str, dark: bool) -> Diagram {
    use mermaid_rs_renderer::{RenderOptions, Theme};
    let theme = if dark {
        Theme::dark()
    } else {
        Theme::mermaid_default()
    };
    let theme = Theme {
        background: "none".into(),
        ..theme
    };
    let options = RenderOptions {
        theme,
        ..RenderOptions::default()
    };
    let svg = mermaid_rs_renderer::render_strict(source, options).map_err(|e| e.to_string())?;
    Ok(Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        svg.into_bytes(),
    )))
}

/// Files in a 14px column with rules under h1 and h2; MR text stays compact.
fn style(file: bool, cx: &App) -> TextViewStyle {
    let t = theme::theme(cx);
    let (h1, h2, h3, code) = match file {
        true => (doc::H1, doc::H2, doc::H3, doc::CODE),
        false => (text::TITLE, text::TITLE, text::TITLE, text::SMALL),
    };
    let rule = t.line;
    TextViewStyle::default()
        .with_foreground(t.fg)
        .with_muted_foreground(t.muted)
        .with_link(t.accent)
        .with_selection(t.text_selection)
        .with_code_background(t.hover)
        .with_border(t.line)
        .with_dark(theme::is_dark(cx))
        .with_heading(move |level| {
            let size = [h1, h2, h3].get(level as usize - 1).copied();
            let s = StyleRefinement::default()
                .text_size(size.unwrap_or(text::TITLE))
                .font_weight(FontWeight::EXTRA_BOLD);
            match file && level <= 2 {
                true => s.pb(px(6.)).border_b_1().border_color(rule),
                false => s,
            }
        })
        .with_inline_code(HighlightStyle {
            background_color: Some(t.hover),
            ..HighlightStyle::default()
        })
        .with_code_block(StyleRefinement::default().rounded(radius::ITEM))
        .with_inline_code_style(
            StyleRefinement::default()
                .text_size(code)
                .rounded(radius::LETTER)
                .px(px(4.)),
        )
        .with_blockquote_style(
            StyleRefinement::default()
                .border_l(px(3.))
                .border_color(t.line)
                .pl(px(10.)),
        )
}

/// Raw HTML whose content a browser does not show, but the kit would as text.
const HIDDEN: &[&str] = &[
    "script", "style", "iframe", "noscript", "template", "svg", "math", "title", "object", "video",
    "audio", "noembed", "noframes", "xmp",
];

/// The text the kit parses: fence languages by the viewer's names, images the rules show as
/// links made links, `HIDDEN` elements dropped, and with `breaks` a lone newline outside code
/// made a hard break.
fn prepare(text: &str, links: &Links, breaks: bool) -> String {
    let Ok(root) = markdown::to_mdast(text, &ParseOptions::gfm()) else {
        return text.into();
    };
    let mut defs = HashMap::new();
    visit(&root, &mut |n| {
        if let Node::Definition(d) = n {
            defs.entry(d.identifier.clone()).or_insert(d.url.clone());
        }
    });
    let (mut edits, mut code) = (vec![], vec![]);
    visit(&root, &mut |n| {
        let Some(p) = n.position() else { return };
        let span = p.start.offset..p.end.offset;
        if let Node::Code(_) | Node::InlineCode(_) = n {
            code.push(span.clone());
        }
        match n {
            Node::Text(_) if breaks => {
                let lines = text[span.clone()].match_indices('\n');
                edits.extend(lines.map(|(i, _)| {
                    let at = span.start + i - text[..span.start + i].ends_with('\r') as usize;
                    (at..at, "\\".to_string())
                }));
            }
            Node::Code(c) => edits.extend(fence(text, span, c.lang.as_deref())),
            Node::Image(i) => edits.extend(image_link(links, &i.url, &i.alt, span)),
            Node::ImageReference(r) => {
                let url = defs.get(&r.identifier);
                edits.extend(url.and_then(|url| image_link(links, url, &r.alt, span)));
            }
            _ => {}
        }
    });
    edits.extend(hidden(text, &code).map(|r| (r, String::new())));
    edits.sort_by_key(|e: &(Range<usize>, String)| e.0.start);
    let (mut out, mut at) = (String::with_capacity(text.len()), 0);
    // An edit inside a dropped element goes with it.
    for (range, with) in edits {
        if range.start < at {
            continue;
        }
        out.push_str(&text[at..range.start]);
        out.push_str(&with);
        at = range.end;
    }
    out + &text[at..]
}

/// `HIDDEN` elements outside code, from their start tag to the end of their end tag.
fn hidden<'a>(text: &'a str, code: &'a [Range<usize>]) -> impl Iterator<Item = Range<usize>> + 'a {
    let lower = text.to_ascii_lowercase();
    let mut at = 0;
    std::iter::from_fn(move || {
        loop {
            let start = at + lower[at..].find('<')?;
            at = start + 1;
            let rest = &lower[at..];
            let Some(tag) = HIDDEN.iter().find(|t| {
                rest.starts_with(**t) && rest[t.len()..].starts_with([' ', '\t', '\n', '/', '>'])
            }) else {
                continue;
            };
            let open_end = at + rest.find('>')? + 1;
            if code.iter().any(|c| c.contains(&start)) {
                continue;
            }
            let close = match lower[..open_end].ends_with("/>") {
                true => Some(open_end),
                false => lower[open_end..].find(&format!("</{tag}")).and_then(|i| {
                    lower[open_end + i..]
                        .find('>')
                        .map(|j| open_end + i + j + 1)
                }),
            };
            if let Some(end) = close {
                at = end;
                return Some(start..end);
            }
        }
    })
}

/// About how much text the kit lays out in a frame.
const PIECE: usize = 4 << 10;

/// `text` cut before top-level blocks into pieces of about `PIECE` bytes, or whole when it has
/// definitions, as the kit resolves references only within a piece.
fn pieces(text: String) -> Vec<String> {
    let Ok(root) = markdown::to_mdast(&text, &ParseOptions::gfm()) else {
        return vec![text];
    };
    let mut defs = false;
    visit(&root, &mut |n| {
        defs |= matches!(n, Node::Definition(_) | Node::FootnoteDefinition(_))
    });
    if defs {
        return vec![text];
    }
    let starts = root.children().into_iter().flatten();
    let starts = starts.filter_map(|n| n.position().map(|p| p.start.offset));
    let (mut pieces, mut at) = (vec![], 0);
    for start in starts {
        if start - at >= PIECE {
            pieces.push(text[at..start].to_string());
            at = start;
        }
    }
    pieces.push(text[at..].to_string());
    pieces
}

fn visit(node: &Node, f: &mut impl FnMut(&Node)) {
    f(node);
    for child in node.children().into_iter().flatten() {
        visit(child, f);
    }
}

/// The fence's language renamed to the one the viewer highlights.
fn fence(text: &str, span: Range<usize>, lang: Option<&str>) -> Option<(Range<usize>, String)> {
    let lang = lang?;
    let name = fence_language(lang).filter(|name| *name != lang)?;
    let line = text[span.clone()].lines().next()?;
    let at = line.find(|c: char| !matches!(c, ' ' | '\t' | '`' | '~'))?;
    let start = span.start + at;
    line[at..]
        .starts_with(lang)
        .then(|| (start..start + lang.len(), name.to_string()))
}

/// An image without a URL goes; one the rules do not load becomes a link, labeled with its alt
/// text or its URL.
fn image_link(
    links: &Links,
    url: &str,
    alt: &str,
    span: Range<usize>,
) -> Option<(Range<usize>, String)> {
    if url.is_empty() {
        return Some((span, String::new()));
    }
    if image_url(links, url).is_some() {
        return None;
    }
    let href = resolve_url(url, links);
    let label = if alt.is_empty() { &href } else { alt };
    let label: String = label
        .chars()
        .flat_map(|c| {
            c.is_ascii_punctuation()
                .then_some('\\')
                .into_iter()
                .chain([c])
        })
        .collect();
    let href = href.replace('<', "%3C").replace('>', "%3E");
    Some((span, format!("[{label}](<{href}>)")))
}

/// Where an image loads from: a data or https URL, or a path in the repo; None when it does not.
fn image_url(links: &Links, url: &str) -> Option<String> {
    let secure = |u: &str| {
        let u = u.to_ascii_lowercase();
        u.starts_with("https:") || u.starts_with("data:")
    };
    match links {
        Links::Repo { path, .. } if !has_scheme(url) => Some(repo_path(url, path)),
        Links::Repo { .. } => secure(url).then(|| url.into()),
        Links::Forge {
            project_url,
            gitlab,
            ..
        } => {
            let url = resolve_url(url, links);
            let upload = format!("{}/uploads/", project_url.trim_end_matches('/'));
            (secure(&url) && !(*gitlab && url.starts_with(&upload))).then_some(url)
        }
    }
}

/// A data or repo image.
fn fetch(core: &AppState, links: &Links, from: &str) -> Option<Arc<Image>> {
    if from.to_ascii_lowercase().starts_with("data:") {
        return data_image(from);
    }
    let Links::Repo { repo, rev, .. } = links else {
        return None;
    };
    data_image(&api::read_image(core, repo, rev.as_deref(), from).ok()??)
}

/// Web images load a few at a time on threads of their own, so a slow host cannot hold core's
/// pool.
fn web_image(cx: &App, url: String) -> Task<Option<Arc<Image>>> {
    type Job = (String, oneshot::Sender<Option<Arc<Image>>>);
    static QUEUE: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10)));
        let agent = ureq::Agent::new_with_config(config.build());
        for _ in 0..4 {
            let (rx, agent) = (rx.clone(), agent.clone());
            let work = move || {
                loop {
                    let job = rx.lock().map(|rx| rx.recv());
                    let Ok(Ok((url, tx))) = job else { return };
                    tx.send(get(&agent, &url)).ok();
                }
            };
            thread::Builder::new()
                .name("images".into())
                .spawn(work)
                .ok();
        }
        tx
    });
    let (tx, rx) = oneshot::channel();
    queue.send((url, tx)).ok();
    cx.foreground_executor()
        .spawn(async move { rx.await.ok().flatten() })
}

fn get(agent: &ureq::Agent, url: &str) -> Option<Arc<Image>> {
    let mut response = agent.get(url).call().ok()?;
    let mime = response.body().mime_type()?.to_string();
    let bytes = response
        .body_mut()
        .with_config()
        .limit(5 << 20)
        .read_to_vec()
        .ok()?;
    Some(Arc::new(Image::from_bytes(
        ImageFormat::from_mime_type(&mime)?,
        bytes,
    )))
}

/// A base64 or percent-encoded data URL's image.
fn data_image(url: &str) -> Option<Arc<Image>> {
    let (head, data) = url.get(5..)?.split_once(',')?;
    let bytes = match head.strip_suffix(";base64") {
        Some(_) => base64::engine::general_purpose::STANDARD
            .decode(data)
            .ok()?,
        None => percent_decode_str(data).collect(),
    };
    let mime = head.split(';').next()?;
    Some(Arc::new(Image::from_bytes(
        ImageFormat::from_mime_type(mime)?,
        bytes,
    )))
}

fn is_web(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.starts_with("http://") || url.starts_with("https://")
}

fn has_scheme(url: &str) -> bool {
    let scheme = url.split_once(':').map(|(s, _)| s);
    url.starts_with("//")
        || scheme.is_some_and(|s| {
            s.starts_with(|c: char| c.is_ascii_alphabetic())
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        })
}

/// A relative or root-relative link in the file at `from` as a repo path, without query and
/// anchor.
fn repo_path(url: &str, from: &str) -> String {
    let joined = Url::parse(&format!("file:///{from}")).and_then(|base| base.join(url));
    let path = joined.map(|u| u.path().to_string()).unwrap_or_default();
    let path = path.strip_prefix('/').unwrap_or(&path);
    percent_decode_str(path).decode_utf8_lossy().into_owned()
}

/// GitLab uploads under the project, other root-relative URLs under the base URL, relative ones
/// against the project; repo files keep theirs.
fn resolve_url(url: &str, links: &Links) -> String {
    let Links::Forge {
        project_url,
        base_url,
        gitlab,
    } = links
    else {
        return url.into();
    };
    let project = project_url.trim_end_matches('/');
    if has_scheme(url) || url.starts_with('#') {
        return url.into();
    }
    if url.starts_with('/') {
        let uploads = *gitlab && url.starts_with("/uploads/");
        return format!("{}{url}", if uploads { project } else { base_url });
    }
    let joined = Url::parse(&format!("{project}/")).and_then(|base| base.join(url));
    joined.map_or_else(|_| url.into(), String::from)
}

/// GitHub's anchor for a heading.
fn slug(text: &str) -> String {
    let kept = text.trim().to_lowercase();
    let kept = kept
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || "_-".contains(*c));
    kept.map(|c| if c.is_whitespace() { '-' } else { c })
        .collect()
}

/// The first heading whose slug is `id`: its top-level block, a row of the kit's list, and its
/// source range.
fn heading(source: &str, id: &str) -> Option<(usize, Range<usize>)> {
    // The kit's options, so the blocks are its rows.
    let mut options = ParseOptions::gfm();
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    let root = markdown::to_mdast(source, &options).ok()?;
    root.children()?.iter().enumerate().find_map(|(i, block)| {
        let mut found = None;
        visit(block, &mut |n| {
            if let Node::Heading(_) = n
                && found.is_none()
                && slug(&n.to_string()) == id
            {
                found = n.position().map(|p| (i, p.start.offset..p.end.offset));
            }
        });
        found
    })
}

pub(super) fn is_markdown(path: &str) -> bool {
    let path = path.to_lowercase();
    path.ends_with(".md") || path.ends_with(".markdown")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    fn repo(path: &str) -> Links {
        Links::Repo {
            repo: "/r".into(),
            rev: None,
            path: path.into(),
        }
    }

    fn gitlab() -> Links {
        Links::Forge {
            project_url: "https://git.example.com/gitlab/group/app".into(),
            base_url: "https://git.example.com/gitlab".into(),
            gitlab: true,
        }
    }

    fn github() -> Links {
        Links::Forge {
            project_url: "https://github.com/owner/app".into(),
            base_url: "https://github.com".into(),
            gitlab: false,
        }
    }

    /// The text the kit renders for `md`, as plain copy has it.
    fn rendered(md: &str, cx: &mut TestAppContext) -> String {
        cx.update(gpui_kit::init);
        let state = cx.new(|cx| TextViewState::markdown(md, cx));
        cx.run_until_parked();
        state.read_with(cx, |s, _| s.rendered_text().as_str().to_string())
    }

    #[test]
    fn marks_code_languages_and_mermaid_blocks() {
        let md = "```ts title\nconst a = \"<b>\"\n```\n\n```mermaid\ngraph TD; A-->B\n```\n\n```\nplain\n```";
        let p = prepare(md, &repo("a.md"), false);
        assert_eq!(p, md.replace("```ts", "```typescript"));
        let root = markdown::to_mdast(&p, &ParseOptions::gfm()).unwrap();
        let blocks = root.children().unwrap().iter().filter_map(mermaid_block);
        let blocks: Vec<_> = blocks.map(|n| n.as_text().to_string()).collect();
        assert_eq!(blocks, ["graph TD; A-->B"]);
        let tags = [
            "ts",
            "TypeScript",
            "shell",
            "py",
            "Dockerfile",
            "mermaid",
            "",
        ];
        assert_eq!(
            tags.map(fence_language),
            [
                Some("typescript"),
                Some("typescript"),
                Some("bash"),
                Some("python"),
                Some("dockerfile"),
                None,
                None
            ]
        );
        assert!(mermaid("graph TD; A-->B", true).is_ok());
        let error = mermaid("graph TD; A-->", false).unwrap_err();
        assert!(error.contains("invalid flowchart edge syntax"), "{error}");
    }

    #[gpui_kit::test]
    fn keeps_task_list_checkboxes_and_drops_other_inputs(cx: &mut TestAppContext) {
        let md = "- [x] done\n- [ ] todo\n\n<input type=\"text\" value=\"x\">";
        assert_eq!(prepare(md, &repo("a.md"), false), md);
        let mut boxes = vec![];
        let root = markdown::to_mdast(md, &ParseOptions::gfm()).unwrap();
        visit(&root, &mut |n| {
            if let Node::ListItem(item) = n {
                boxes.push(item.checked);
            }
        });
        assert_eq!(boxes, [Some(true), Some(false)], "the kit draws these");
        let text = rendered(md, cx);
        assert_eq!(text.trim_end(), "done\ntodo", "the boxes are no text");
    }

    #[test]
    fn loads_relative_images_from_the_repo_and_links_insecure_ones() {
        let md = "![a](img/a.png) ![b](/b.svg) ![c](https://x/c.png) ![d](http://x/d.png) ![e](../e.gif)";
        let links = repo("docs/README.md");
        let want = md.replace("![d](http://x/d.png)", "[d](<http://x/d.png>)");
        assert_eq!(prepare(md, &links, false), want);
        let urls = [
            "img/a.png",
            "/b.svg",
            "https://x/c.png",
            "http://x/d.png",
            "../e.gif",
        ];
        let urls = urls.map(|u| image_url(&links, u));
        let want = ["docs/img/a.png", "b.svg", "https://x/c.png", "", "e.gif"];
        assert_eq!(urls, want.map(|u| (!u.is_empty()).then(|| u.to_string())));
        assert_eq!(
            prepare("![](http://x/a_b.png) ![]()", &links, false),
            r"[http\:\/\/x\/a\_b\.png](<http://x/a_b.png>) "
        );
        let svg = data_image("data:image/svg+xml;base64,PHN2Zy8+").unwrap();
        assert_eq!(
            (svg.format, &svg.bytes[..]),
            (ImageFormat::Svg, &b"<svg/>"[..])
        );
        let svg = data_image("data:image/svg+xml,%3Csvg%2F%3E").unwrap();
        assert_eq!(&svg.bytes[..], b"<svg/>");
    }

    #[test]
    fn a_repo_image_loads_through_read_image_in_the_worktree_and_at_a_rev() {
        let sb = detached_core::git::testutil::Sandbox::new();
        let r = sb.path("r");
        std::fs::create_dir_all(r.join("docs/img")).unwrap();
        sb.git(&r, &["init", "-q"]);
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
        sb.commit(&r, "docs/img/logo.svg", svg, "logo");
        for rev in [None, Some("main".to_string())] {
            let links = Links::Repo {
                repo: r.display().to_string(),
                rev,
                path: "docs/guide.md".into(),
            };
            let from = image_url(&links, "img/logo.svg").unwrap();
            let image = fetch(&sb.state, &links, &from).unwrap();
            assert_eq!(
                (image.format, &image.bytes[..]),
                (ImageFormat::Svg, svg.as_bytes())
            );
        }
    }

    #[test]
    fn resolves_repo_links_and_heading_anchors() {
        assert_eq!(repo_path("b.md#part", "docs/a.md"), "docs/b.md");
        assert_eq!(
            repo_path("../src/x%20y.ts?plain=1", "docs/a.md"),
            "src/x y.ts"
        );
        assert_eq!(repo_path("/LICENSE", "docs/deep/a.md"), "LICENSE");
        assert_eq!(slug(" Install & Run: v2_beta "), "install--run-v2_beta");
        assert_eq!(slug("Привет, мир"), "привет-мир");
        let md = "# Guide\n\ntext\n\n## Install & Run\n";
        assert_eq!(heading(md, "install--run"), Some((2, 15..31)));
        assert_eq!(heading(md, "nope"), None);
    }

    #[gpui_kit::test]
    fn strips_script_handlers_style_forms_and_frames(cx: &mut TestAppContext) {
        let md = [
            r#"<script>alert(1)</script><img src="https://x/a.png" onerror="alert(1)">"#,
            r#"<p style="color:red" onclick="alert(1)">hi</p><style>body{}</style>"#,
            r#"<form action="https://x"><input name="a"><button>go</button></form><iframe src="https://x">frame</iframe>"#,
            r#"<map><area href="https://x"></map><a href="javascript:alert(1)">js</a> and <script>alert(2)</script><svg/>"#,
        ]
        .join("\n");
        let p = prepare(&md, &github(), true);
        assert!(
            !p.contains("<script") && !p.contains("<style") && !p.contains("<iframe"),
            "{p}"
        );
        assert!(p.contains(r#"<img src="https://x/a.png""#), "{p}");
        assert!(image_url(&github(), "https://x/a.png").is_some());
        let text = rendered(&p, cx);
        for gone in ["alert", "body", "frame"] {
            assert!(!text.contains(gone), "{gone} in {text:?}");
        }
        assert!(text.contains("hi"), "{text:?}");
        assert!(!is_web(&resolve_url("javascript:alert(1)", &github())));
    }

    #[test]
    fn resolves_links_against_the_project_and_the_base_url() {
        let cases = [
            (
                "/uploads/abc/x.png",
                gitlab(),
                "https://git.example.com/gitlab/group/app/uploads/abc/x.png",
            ),
            (
                "/group/other",
                gitlab(),
                "https://git.example.com/gitlab/group/other",
            ),
            (
                "docs/a.md",
                gitlab(),
                "https://git.example.com/gitlab/group/app/docs/a.md",
            ),
            ("../b", github(), "https://github.com/owner/b"),
            ("/uploads/x", github(), "https://github.com/uploads/x"),
            ("https://a.b/c", gitlab(), "https://a.b/c"),
            ("#top", gitlab(), "#top"),
        ];
        for (url, links, want) in cases {
            assert_eq!(resolve_url(url, &links), want, "{url}");
        }
    }

    #[test]
    fn shows_gitlab_uploads_and_non_https_images_as_links() {
        let md = "![shot](/uploads/abc/x.png) ![plain](http://x/y.png) ![ok](https://x/z.png)";
        assert_eq!(
            prepare(md, &gitlab(), true),
            "[shot](<https://git.example.com/gitlab/group/app/uploads/abc/x.png>) [plain](<http://x/y.png>) ![ok](https://x/z.png)"
        );
        assert_eq!(
            resolve_url("src/a.ts", &gitlab()),
            "https://git.example.com/gitlab/group/app/src/a.ts"
        );
    }

    #[test]
    fn keeps_a_long_text_with_definitions_in_one_piece() {
        let body = "[a] and [^1]\n\n".to_string() + &"para\n\n".repeat(PIECE);
        assert!(pieces(body.clone()).len() > 1);
        for def in ["[a]: https://x/a", "[^1]: note"] {
            assert_eq!(pieces(format!("{body}{def}\n")).len(), 1, "{def}");
        }
    }

    #[gpui_kit::test]
    fn a_short_text_drops_a_long_one_still_preparing(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.executor().allow_parking();
        let long = "para\n\n".repeat(PIECE);
        let m = cx.new(|cx| Markdown::new(long.clone(), repo("a.md"), false, cx));
        m.update(cx, |m, cx| m.set_text("short".into(), cx));
        // Queued later and longer, so its prepare ends after the first one's.
        let probe = cx.new(|cx| Markdown::new(long.repeat(2), repo("a.md"), false, cx));
        while probe.read_with(cx, |p, _| p.fed == 0) {
            let tick = cx.executor().timer(Duration::from_millis(10));
            cx.foreground_executor().block_test(tick);
        }
        cx.run_until_parked();
        assert_eq!(m.read_with(cx, |m, _| m.fed), "short".len());
    }

    #[test]
    fn breaks_turn_lone_newlines_outside_code_into_hard_breaks() {
        let md = "a\nb\r\nc\n\n```\nd\ne\n```\n\n`f\ng` h";
        let want = "a\\\nb\\\r\nc\n\n```\nd\ne\n```\n\n`f\ng` h";
        assert_eq!(prepare(md, &github(), true), want);
        assert_eq!(prepare(md, &repo("a.md"), false), md);
    }
}
