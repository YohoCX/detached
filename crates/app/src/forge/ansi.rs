//! Job log text to lines of styled spans: ANSI SGR, OSC 8 links, GitLab sections, GitHub groups.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    /// One of the theme's 16.
    Ansi(u8),
    Rgb(u8, u8, u8),
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Color::Ansi(n) => write!(f, "ansi-{n}"),
            Color::Rgb(r, g, b) => write!(f, "rgb({r},{g},{b})"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub href: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// A section's first line: `end` is the index of its last line, None while it is open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Head {
    pub end: Option<usize>,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub spans: Vec<Span>,
    pub depth: usize,
    pub head: Option<Head>,
}

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn c256(n: u8) -> Color {
    match n {
        0..16 => Color::Ansi(n),
        232.. => {
            let g = 8 + 10 * (n - 232);
            Color::Rgb(g, g, g)
        }
        _ => {
            let n = (n - 16) as usize;
            Color::Rgb(CUBE[n / 36], CUBE[n / 6 % 6], CUBE[n % 6])
        }
    }
}

fn sgr(codes: &str, s: &mut Style) {
    let p: Vec<u32> = codes
        .split([';', ':'])
        .map(|c| c.parse().unwrap_or(0))
        .collect();
    let at = |i: usize| p.get(i).map_or(0, |&v| v.min(255) as u8);
    let mut i = 0;
    while i < p.len() {
        match p[i] {
            0 => {
                *s = Style {
                    href: s.href.take(),
                    ..Style::default()
                }
            }
            1 => s.bold = true,
            2 => s.dim = true,
            3 => s.italic = true,
            4 => s.underline = true,
            22 => (s.bold, s.dim) = (false, false),
            23 => s.italic = false,
            24 => s.underline = false,
            39 => s.fg = None,
            49 => s.bg = None,
            n @ (30..=37 | 90..=97) => s.fg = Some(c256((n % 10) as u8 + 8 * (n >= 90) as u8)),
            n @ (40..=47 | 100..=107) => s.bg = Some(c256((n % 10) as u8 + 8 * (n >= 100) as u8)),
            n @ (38 | 48) => {
                let c = match p.get(i + 1) {
                    Some(5) => Some((c256(at(i + 2)), 2)),
                    Some(2) => Some((Color::Rgb(at(i + 2), at(i + 3), at(i + 4)), 4)),
                    _ => None,
                };
                if let Some((c, skip)) = c {
                    *if n == 38 { &mut s.fg } else { &mut s.bg } = Some(c);
                    i += skip;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// The length of the first character, 0 for none or a line break (a regex `.`).
fn dot(s: &str) -> usize {
    let c = s.chars().next();
    c.filter(|c| !matches!(c, '\r' | '\n' | '\u{2028}' | '\u{2029}'))
        .map_or(0, char::len_utf8)
}

/// GitLab's `section_start|end:<ts>:<name>[<options>]\r` and an optional `\e[0K` at the start of
/// `s`: its length, whether it starts a section, and whether that starts collapsed.
fn section(s: &str) -> Option<(usize, bool, bool)> {
    let r = s.strip_prefix("section_")?;
    let (start, r) = match r.strip_prefix("start:") {
        Some(r) => (true, r),
        None => (false, r.strip_prefix("end:")?),
    };
    let ts = r.bytes().take_while(u8::is_ascii_digit).count();
    let r = r[ts..].strip_prefix(':').filter(|_| ts > 0)?;
    let name = r
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || b"_.-".contains(b));
    let name = name.count();
    let mut r = &r[name..];
    let mut collapsed = false;
    if let Some(o) = r.strip_prefix('[') {
        let end = o.find(']')?;
        collapsed = o[..end].contains("collapsed=true");
        r = &o[end + 1..];
    }
    let r = r.strip_prefix('\r').filter(|_| name > 0)?;
    let r = r.strip_prefix("\x1b[0K").unwrap_or(r);
    Some((s.len() - r.len(), start, collapsed))
}

/// GitHub's `[<ts>Z ]##[group]` or `##[endgroup]` at the start of `s`: its length and whether it
/// opens a group.
fn group(s: &str) -> Option<(usize, bool)> {
    let ws = s.find(char::is_whitespace).unwrap_or(s.len());
    let ts = ws >= 2 && s[..ws].ends_with('Z') && s[ws..].starts_with(' ');
    let marker = |at: usize| {
        let r = &s[at..];
        let open = r.starts_with("##[group]");
        (open || r.starts_with("##[endgroup]")).then_some((at + if open { 9 } else { 12 }, open))
    };
    ts.then(|| marker(ws + 1)).flatten().or_else(|| marker(0))
}

/// Pads tabs to the next multiple of 8 columns, as `white-space: pre` shows them.
fn expand_tabs(spans: &mut [Span]) {
    let mut col = 0;
    for s in spans {
        let mut out = String::with_capacity(s.text.len());
        for c in s.text.chars() {
            if c == '\t' {
                let n = 8 - col % 8;
                out.extend(std::iter::repeat_n(' ', n));
                col += n;
            } else {
                out.push(c);
                col += 1;
            }
        }
        s.text = out;
    }
}

#[derive(Default)]
pub struct LogParser {
    pub lines: Vec<Line>,
    rest: String,
    style: Style,
    open: Vec<usize>,
    /// The line count, style and open sections before the shown partial last line, which the next
    /// push parses again.
    saved: Option<(usize, Style, Vec<usize>)>,
}

impl LogParser {
    pub fn push(&mut self, text: &str, done: bool) {
        if let Some((n, style, open)) = self.saved.take() {
            self.lines.truncate(n);
            (self.style, self.open) = (style, open);
        }
        let mut all = std::mem::take(&mut self.rest) + text;
        let rest = all.split_off(all.rfind('\n').map_or(0, |i| i + 1));
        for l in all.split_terminator('\n') {
            self.line(l);
        }
        if rest.is_empty() {
            return;
        }
        if !done {
            self.saved = Some((self.lines.len(), self.style.clone(), self.open.clone()));
        }
        self.line(&rest);
        if !done {
            self.rest = rest;
        }
    }

    fn close(&mut self) {
        let last = self.lines.len().saturating_sub(1);
        if let Some(h) = self.open.pop().and_then(|h| self.lines[h].head.as_mut()) {
            h.end = Some(last);
        }
    }

    fn line(&mut self, raw: &str) {
        let (mut head, mut marked) = (None, false);
        let (mut kept, mut rest) = (String::with_capacity(raw.len()), raw);
        while let Some(i) = rest.find("section_") {
            kept.push_str(&rest[..i]);
            rest = &rest[i..];
            let Some((n, start, collapsed)) = section(rest) else {
                kept.push_str("section_");
                rest = &rest["section_".len()..];
                continue;
            };
            marked = true;
            match start {
                true => {
                    head = Some(Head {
                        end: None,
                        open: !collapsed,
                    })
                }
                false => self.close(),
            }
            rest = &rest[n..];
        }
        kept.push_str(rest);
        let mut raw = kept.as_str();
        match group(raw) {
            Some((_, false)) => return self.close(),
            Some((n, true)) => {
                head = Some(Head {
                    end: None,
                    open: false,
                });
                raw = &raw[n..];
            }
            None => {}
        }

        let raw = raw.trim_end_matches('\r');
        let mut spans = vec![];
        let (b, mut at, mut i) = (raw.as_bytes(), 0, 0);
        while i < b.len() {
            if (b[i] >= 0x20 && b[i] != 0x7f) || b[i] == b'\t' {
                i += 1;
                continue;
            }
            self.text(&mut spans, &raw[at..i]);
            i += match b[i] {
                0x1b => self.escape(&raw[i..]),
                b'\r' => {
                    spans.clear();
                    1
                }
                _ => 1,
            };
            at = i;
        }
        self.text(&mut spans, &raw[at..]);
        if raw.contains('\t') {
            expand_tabs(&mut spans);
        }
        if marked && head.is_none() && spans.is_empty() {
            return;
        }
        self.lines.push(Line {
            spans,
            depth: self.open.len(),
            head,
        });
        if head.is_some() {
            self.open.push(self.lines.len() - 1);
        }
    }

    fn text(&self, spans: &mut Vec<Span>, text: &str) {
        if !text.is_empty() {
            let style = self.style.clone();
            spans.push(Span {
                text: text.into(),
                style,
            });
        }
    }

    /// Applies the escape at the start of `s` and returns its length: CSI, an OSC 8 link, another
    /// OSC, a charset, else the escape and the character after it.
    fn escape(&mut self, s: &str) -> usize {
        let b = s.as_bytes();
        match b.get(1) {
            Some(b'[') => {
                let params = b[2..].iter().take_while(|c| b"0123456789;:?".contains(c));
                let params = params.count();
                let mid = b[2 + params..]
                    .iter()
                    .take_while(|c| (b' '..=b'/').contains(c));
                let end = 2 + params + mid.count();
                if let Some(&f) = b.get(end).filter(|f| (b'@'..=b'~').contains(f)) {
                    if f == b'm' {
                        sgr(&s[2..2 + params], &mut self.style);
                    }
                    return end + 1;
                }
            }
            Some(b']') => {
                let k = 2 + b[2..]
                    .iter()
                    .take_while(|&&c| c != 0x07 && c != 0x1b)
                    .count();
                let len = match b.get(k) {
                    Some(0x07) => Some(k + 1),
                    Some(0x1b) if b.get(k + 1) == Some(&b'\\') => Some(k + 2),
                    _ => None,
                };
                if let Some(len) = len {
                    let link = s[2..k].strip_prefix("8;").and_then(|r| r.split_once(';'));
                    if let Some((_, uri)) = link {
                        let scheme = uri.get(..8).unwrap_or(uri).to_ascii_lowercase();
                        let web = scheme.starts_with("http://") || scheme.starts_with("https://");
                        self.style.href = web.then(|| uri.into());
                    }
                    return len;
                }
            }
            Some(b'(' | b')' | b'*' | b'+') if dot(&s[2..]) > 0 => return 2 + dot(&s[2..]),
            _ => {}
        }
        1 + dot(&s[1..])
    }
}

/// The indexes of the lines outside folded sections.
pub fn visible(lines: &[Line]) -> Vec<usize> {
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        out.push(i);
        i = match lines[i].head {
            Some(Head { end, open: false }) => end.unwrap_or(lines.len()),
            _ => i,
        } + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(chunks: &[&str]) -> Vec<Line> {
        let mut p = LogParser::default();
        for (i, c) in chunks.iter().enumerate() {
            p.push(c, i == chunks.len() - 1);
        }
        p.lines
    }

    fn texts<'a>(lines: impl IntoIterator<Item = &'a Line>) -> Vec<String> {
        let line = |l: &Line| l.spans.iter().map(|s| s.text.as_str()).collect();
        lines.into_iter().map(line).collect()
    }

    fn span(text: &str, style: Style) -> Span {
        Span {
            text: text.into(),
            style,
        }
    }

    fn fg(lines: &[Line], i: usize) -> Option<Color> {
        lines[i].spans[0].style.fg
    }

    #[test]
    fn turns_sgr_codes_into_span_styles_and_keeps_everything_else_as_text() {
        let l = parse(&[
            "\x1b[1;31mred\x1b[0m <img src=x onerror=alert(1)> \x1b[38;5;196mx\x1b[48;2;1;2;300my\x1b[22;39m.\n",
        ]);
        let (red, bg) = (Some(Color::Rgb(255, 0, 0)), Some(Color::Rgb(1, 2, 255)));
        let style = |fg, bg| Style {
            fg,
            bg,
            ..Style::default()
        };
        let bold = Style {
            bold: true,
            fg: Some(Color::Ansi(1)),
            ..Style::default()
        };
        assert_eq!(
            l[0].spans,
            [
                span("red", bold),
                span(" <img src=x onerror=alert(1)> ", Style::default()),
                span("x", style(red, None)),
                span("y", style(red, bg)),
                span(".", style(None, bg)),
            ]
        );
        assert_eq!(fg(&parse(&["\x1b[92mok\n"]), 0), Some(Color::Ansi(10)));
        let plain = parse(&["\x1b[244mg\n"]);
        assert_eq!(plain[0].spans, [span("g", Style::default())]);
        let gray = Some(Color::Rgb(128, 128, 128));
        assert_eq!(fg(&parse(&["\x1b[38;5;244mg\n"]), 0), gray);
    }

    #[test]
    fn carries_styles_across_lines_and_drops_other_escapes_and_control_characters() {
        let green = Style {
            fg: Some(Color::Ansi(2)),
            ..Style::default()
        };
        assert_eq!(
            parse(&["\x1b[32ma\nb\x1b[0m\n"])[1].spans,
            [span("b", green)]
        );
        let l = parse(&["\x1b[0K\x1b[2Ja\x07b\x1b]0;title\x07c\x1b(B\n"]);
        assert_eq!(texts(&l), ["abc"]);
    }

    #[test]
    fn shows_what_a_carriage_return_overwrote_last_and_strips_crlf() {
        assert_eq!(texts(&parse(&["10%\r50%\r100%\nend\r\n"])), ["100%", "end"]);
    }

    #[test]
    fn links_osc_8_hyperlinks_to_http_s_only() {
        let l = parse(&[
            "see \x1b]8;;https://ex.com/a\x1b\\here\x1b]8;;\x1b\\ and \x1b]8;;javascript:alert(1)\x07there\x1b]8;;\x07\n",
        ]);
        let link = Style {
            href: Some("https://ex.com/a".into()),
            ..Style::default()
        };
        let plain = Style::default;
        assert_eq!(
            l[0].spans,
            [
                span("see ", plain()),
                span("here", link),
                span(" and ", plain()),
                span("there", plain()),
            ]
        );
    }

    #[test]
    fn continues_a_partial_line_and_a_split_escape_in_the_next_chunk() {
        let mut p = LogParser::default();
        p.push("one\ntw\x1b[3", false);
        assert_eq!(texts(&p.lines), ["one", "tw3"]);
        p.push("1mo\nthree", false);
        assert_eq!(texts(&p.lines), ["one", "two", "three"]);
        assert_eq!(fg(&p.lines, 2), Some(Color::Ansi(1)));
        p.push("", true);
        assert_eq!(texts(&p.lines), ["one", "two", "three"]);
    }

    #[test]
    fn folds_gitlab_sections_nested_and_collapsed() {
        let s = |k: &str, name: &str, opts: &str| {
            format!("\x1b[0Ksection_{k}:1700000000:{name}{opts}\r\x1b[0K")
        };
        let chunks = [
            format!("{}Preparing\n", s("start", "prep", "")),
            "inside\n".into(),
            format!("{}Inner\n", s("start", "inner", "[collapsed=true]")),
            "deep\n".into(),
            format!(
                "{}{}{}Script\n",
                s("end", "inner", ""),
                s("end", "prep", ""),
                s("start", "script", "")
            ),
            "run\n".into(),
            format!("{}\n", s("end", "script", "")),
            "after\n".into(),
        ];
        let mut lines = parse(&chunks.each_ref().map(String::as_str));
        let all = [
            "Preparing",
            "inside",
            "Inner",
            "deep",
            "Script",
            "run",
            "after",
        ];
        assert_eq!(texts(&lines), all);
        let depths: Vec<_> = lines.iter().map(|l| l.depth).collect();
        assert_eq!(depths, [0, 1, 1, 2, 0, 1, 0]);
        let head = |end, open| {
            Some(Head {
                end: Some(end),
                open,
            })
        };
        let heads: Vec<_> = lines.iter().map(|l| l.head).collect();
        assert_eq!(
            heads,
            [
                head(3, true),
                None,
                head(3, false),
                None,
                head(5, true),
                None,
                None
            ]
        );
        let shown = |lines: &[Line]| texts(visible(lines).into_iter().map(|i| &lines[i]));
        let open = ["Preparing", "inside", "Inner", "Script", "run", "after"];
        assert_eq!(shown(&lines), open);
        lines[0].head.as_mut().unwrap().open = false;
        assert_eq!(shown(&lines), ["Preparing", "Script", "run", "after"]);
    }

    #[test]
    fn folds_github_groups_collapsed_and_an_unclosed_one_to_the_end() {
        let lines = parse(&[
            "2026-10-01T10:00:00.0000000Z ##[group]Run tests\n",
            "2026-10-01T10:00:01.0000000Z ok\n",
            "2026-10-01T10:00:02.0000000Z ##[endgroup]\n",
            "##[group]Post\n",
            "cleanup\n",
        ]);
        let all = [
            "Run tests",
            "2026-10-01T10:00:01.0000000Z ok",
            "Post",
            "cleanup",
        ];
        assert_eq!(texts(&lines), all);
        let head = |end| Some(Head { end, open: false });
        assert_eq!((lines[0].head, lines[2].head), (head(Some(1)), head(None)));
        let shown = visible(&lines).into_iter().map(|i| &lines[i]);
        assert_eq!(texts(shown), ["Run tests", "Post"]);
    }
}
