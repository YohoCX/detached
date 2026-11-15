//! `detached --session <repo> [prompt]` in debug builds: a chat session through `kernel::Session`
//! as lines on stdout, its asks and, with no prompt, its prompts read from stdin.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::sync::Arc;

use detached_core::adapters::CLAUDE;
use detached_core::backend::{Kind, Spec, Tool};
use detached_core::state::AppState;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{App, Entity, WindowingRequest};
use kernel::session::{self, Answer, End, Item, Session, State};

const USAGE: &str =
    "usage: detached --session <repo> [prompt] [--past | --resume <id> | --delete <id>]";

enum Input {
    Changed,
    Line(Option<String>),
}

#[derive(Default)]
struct Run {
    prompt: Option<String>,
    past: bool,
    resume: Option<String>,
    delete: Option<String>,
}

pub fn run(core: Arc<AppState>, args: &[String]) -> ! {
    let Some((repo, rest)) = args.split_first() else {
        exit(USAGE, 2)
    };
    let mut run = Run::default();
    let mut rest = rest.iter();
    let id = |a: Option<&String>| match a {
        Some(id) if !id.starts_with("--") => Some(id.clone()),
        _ => exit(USAGE, 2),
    };
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--past" => run.past = true,
            "--resume" => run.resume = id(rest.next()),
            "--delete" => run.delete = id(rest.next()),
            // A mistyped flag would go to the real login as a prompt.
            f if f.starts_with("--") => exit(USAGE, 2),
            _ => run.prompt = Some(a.clone()),
        }
    }
    if [run.past, run.resume.is_some(), run.delete.is_some()]
        .iter()
        .filter(|&&b| b)
        .count()
        > 1
    {
        exit(USAGE, 2)
    }
    let repo = std::path::absolute(repo).map_or(repo.clone(), |p| p.display().to_string());
    let spec = Spec::new(&CLAUDE, &repo, Kind::Chat);
    let app = gpui_kit::application().with_windowing(WindowingRequest::Headless);
    app.run(move |cx: &mut App| {
        kernel::init(core, cx);
        if run.past {
            let past = session::past(spec, cx);
            cx.spawn(async move |_| match past.await {
                Ok(all) => {
                    for p in all {
                        let model = p.model.as_deref().unwrap_or("-");
                        let tag = p.tag.as_deref().unwrap_or("-");
                        println!("{}  {}  {model}  {tag}  {}", p.id, p.modified, p.title);
                    }
                    exit("", 0)
                }
                Err(e) => exit(&e.to_string(), 1),
            })
            .detach();
        } else if let Some(id) = run.delete.take() {
            let done = session::delete(spec, id.clone(), cx);
            cx.spawn(async move |_| match done.await {
                Ok(()) => exit(&format!("deleted {id}"), 0),
                Err(e) => exit(&e.to_string(), 1),
            })
            .detach();
        } else {
            chat(spec, run, cx);
        }
    });
    exit("", 0)
}

fn chat(spec: Spec, run: Run, cx: &mut App) {
    let s = Session::open(spec, run.resume.clone(), cx);
    let (tx, mut rx) = mpsc::unbounded();
    let changed = tx.clone();
    cx.observe(&s, move |_, _| {
        let _ = changed.unbounded_send(Input::Changed);
    })
    .detach();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let _ = tx.unbounded_send(Input::Line(line.ok()));
        }
        let _ = tx.unbounded_send(Input::Line(None));
    });
    let one_shot = run.prompt.is_some();
    let (mut prompt, mut feed) = (run.prompt, Feed::default());
    let (mut lines, mut eof, mut opened) = (VecDeque::new(), false, false);
    cx.spawn(async move |cx| {
        while let Some(input) = rx.next().await {
            match input {
                Input::Line(Some(l)) => lines.push_back(l),
                Input::Line(None) => eof = true,
                Input::Changed => {}
            }
            cx.update(|cx| {
                let now = s.read(cx);
                if let (false, Some(v)) = (opened, &now.version) {
                    opened = true;
                    println!("[session] {} on {v}", now.id.as_deref().unwrap_or("-"));
                }
                let state = now.state.clone();
                let ended = feed.print(now, state != State::Running);
                match state {
                    State::Starting => {}
                    State::Ended(end) => {
                        let code = i32::from(end != End::Closed);
                        exit(&format!("[ended] {end:?}"), code)
                    }
                    State::Running => answer(&s, &mut lines, eof, cx),
                    State::Idle => {
                        if ended {
                            report(s.read(cx));
                        }
                        let next = prompt
                            .take()
                            .or_else(|| if one_shot { None } else { lines.pop_front() });
                        match next {
                            Some(p) => s.update(cx, |s, cx| s.prompt(p, cx)),
                            None if one_shot || eof => s.update(cx, |s, cx| s.end(cx)),
                            None => {}
                        }
                    }
                }
            });
        }
    })
    .detach();
}

/// Answers the waiting ask with the next line: y once, a always, n reject, anything else or
/// stdin's end cancel.
fn answer(s: &Entity<Session>, lines: &mut VecDeque<String>, eof: bool, cx: &mut App) {
    let waiting = s.read(cx).items.iter().position(|i| {
        matches!(
            i,
            Item::Ask {
                answer: Answer::Waiting,
                ..
            }
        )
    });
    let Some(item) = waiting.filter(|_| eof || !lines.is_empty()) else {
        return;
    };
    let line = lines.pop_front().unwrap_or_default();
    let answer = match line.trim() {
        "y" => Answer::Once,
        "a" => Answer::Always,
        "n" => Answer::Rejected,
        _ => Answer::Gone,
    };
    s.update(cx, |s, cx| s.answer(item, answer, cx));
}

/// What the session reports after a turn.
fn report(s: &Session) {
    let usage = s
        .usage
        .map_or("unknown".into(), |(u, n)| format!("{u} of {n} tokens"));
    let cost = s.cost.map_or("unknown".into(), |c| format!("${c:.4}"));
    println!("[usage] {usage}, cost {cost}");
}

/// Prints the feed's items once each, the last one's text as it streams, and a tool call again
/// when its status moves.
#[derive(Default)]
struct Feed {
    at: usize,
    /// Bytes of `items[at]`'s text already printed.
    printed: usize,
    /// The status each printed tool call had.
    tools: HashMap<String, Option<String>>,
}

impl Feed {
    /// Prints what is new; true when that holds a turn's end.
    fn print(&mut self, s: &Session, settled: bool) -> bool {
        let mut out = std::io::stdout().lock();
        let mut ended = false;
        for item in &s.items[..self.at] {
            if let Item::Tool(t) = item
                && self.tools.get(&t.id).is_some_and(|was| *was != t.status)
            {
                self.tools.insert(t.id.clone(), t.status.clone());
                let _ = writeln!(out, "{}", line(item));
            }
        }
        while let Some(item) = s.items.get(self.at) {
            let last = self.at + 1 == s.items.len() && !settled;
            match item {
                Item::Text(t) | Item::Thought(t) => {
                    if self.printed == 0 && matches!(item, Item::Thought(_)) {
                        let _ = write!(out, "(thinking) ");
                    }
                    let _ = write!(out, "{}", &t[self.printed..]);
                    self.printed = t.len();
                    if last {
                        break;
                    }
                    let _ = writeln!(out);
                }
                Item::Ask { .. } => {
                    let _ = writeln!(out, "{}", line(item));
                }
                _ if last => break,
                _ => {
                    if let Item::Tool(t) = item {
                        self.tools.insert(t.id.clone(), t.status.clone());
                    }
                    ended |= matches!(item, Item::End { .. });
                    let _ = writeln!(out, "{}", line(item));
                }
            }
            (self.at, self.printed) = (self.at + 1, 0);
        }
        let _ = out.flush();
        ended
    }
}

fn line(item: &Item) -> String {
    let tool = |t: &Tool| {
        let kind = t.kind.as_deref().unwrap_or("tool");
        format!("{kind} {}", t.title.as_deref().unwrap_or_default())
    };
    match item {
        Item::User(t) => format!("> {t}"),
        Item::Tool(t) => {
            let status = t.status.as_deref().unwrap_or("-");
            let out = t
                .output
                .as_ref()
                .map(|o| format!("\n{o}"))
                .unwrap_or_default();
            format!("[tool] {} ({status}){out}", tool(t))
        }
        Item::Ask {
            tool: t,
            answer: Answer::Waiting,
            ..
        } => format!("[ask] {}: y once, a always, n reject", tool(t)),
        Item::Ask {
            tool: t, answer, ..
        } => format!("[ask] {}: {answer:?}", tool(t)),
        Item::Changes { files, .. } => {
            let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
            format!("[changes] {}", paths.join(", "))
        }
        Item::End { stop, ms } => format!("[end] {stop:?} in {ms} ms"),
        Item::Note(t) => format!("[note] {t}"),
        Item::Text(t) | Item::Thought(t) => t.clone(),
    }
}

fn exit(said: &str, code: i32) -> ! {
    if !said.is_empty() {
        println!("{said}");
    }
    let _ = std::io::stdout().flush();
    std::process::exit(code)
}
