//! `detached --session <repo> [prompt]` in debug builds: a chat session through `kernel::Session`
//! as lines on stdout, its asks and, with no prompt, its prompts read from stdin. Between turns
//! stdin also takes `/mode`, `/model` and `/stop <task id>`.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::sync::Arc;

use detached_core::adapters::CLAUDE;
use detached_core::backend::{Kind, Setting, Spec, Tool};
use detached_core::state::AppState;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{App, Entity, WindowingRequest};
use kernel::session::{self, Answer, End, Item, Session, State};

const USAGE: &str = "usage: detached --session <repo> [prompt] [--past | --resume <id> | --delete <id>]
stdin: prompts, y/a/n to an ask, /mode <default|acceptEdits|plan|auto>, /model <name>, /stop <task id>";
const MODES: &[&str] = &["default", "acceptEdits", "plan", "auto"];

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
    let (mut lines, mut eof, mut opened, mut due) = (VecDeque::new(), false, false, false);
    let (mut config, mut tasks) = (vec![], vec![]);
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
                if now.config != config {
                    config = now.config.clone();
                    let all = config.iter().map(|c| format!("{} {}", c.id, c.value));
                    println!("[config] {}", all.collect::<Vec<_>>().join(", "));
                }
                if now.tasks != tasks {
                    tasks = now.tasks.clone();
                    let all = tasks.iter().map(|t| format!("{} {}", t.id, t.description));
                    let all = all.collect::<Vec<_>>().join("; ");
                    println!("[tasks] {}", if all.is_empty() { "none" } else { &all });
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
                        // The limits come just after the turn's end.
                        due |= ended;
                        if due && !s.read(cx).limits_settled() {
                            return;
                        }
                        if std::mem::take(&mut due) {
                            report(s.read(cx));
                        }
                        while lines.front().is_some_and(|l| command(&s, l, cx)) {
                            lines.pop_front();
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

/// Runs `line` when it is one of the commands; false when it is a prompt.
fn command(s: &Entity<Session>, line: &str, cx: &mut App) -> bool {
    let Some((cmd, arg)) = line.trim().split_once(' ') else {
        return false;
    };
    let session = s.read(cx);
    let setting = match cmd {
        "/mode" if MODES.contains(&arg) => Ok(Setting::Mode(arg.into())),
        "/mode" => Err(format!("no mode {arg}")),
        "/model" if session.models.iter().any(|m| m.value == arg) => Ok(Setting::Model(arg.into())),
        "/model" => Err(format!("this session offers no model {arg}")),
        "/stop" => {
            session.stop_task(arg);
            return true;
        }
        _ => return false,
    };
    let done = setting.and_then(|set| s.update(cx, |s, _| s.set(set)).map_err(|e| e.to_string()));
    if let Err(e) = done {
        println!("[refused] {e}");
    }
    true
}

/// Answers the waiting ask with the next line: y once, a always, n reject; another line asks
/// again, and stdin's end cancels.
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
    let Some(item) = waiting else {
        return;
    };
    // Cyrillic у and а look like y and a and come from a Russian layout.
    let answer = loop {
        match lines.pop_front().as_deref().map(str::trim) {
            Some("y" | "у") => break Answer::Once,
            Some("a" | "а") => break Answer::Always,
            Some("n") => break Answer::Rejected,
            Some(other) => {
                println!(
                    "[ask] \"{}\": y once, a always, n reject",
                    other.escape_default()
                )
            }
            None if eof => break Answer::Gone,
            None => return,
        }
    };
    println!("[answer] {answer:?}");
    s.update(cx, |s, cx| s.answer(item, answer, cx));
}

/// What the session reports after a turn.
fn report(s: &Session) {
    let usage = s
        .usage
        .map_or("unknown".into(), |(u, n)| format!("{u} of {n} tokens"));
    let cost = s.cost.map_or("unknown".into(), |c| format!("${c:.4}"));
    println!("[usage] {usage}, cost {cost}");
    let now = detached_core::os::now() as u64 * 1000;
    let limits = s.limits.iter().flatten().map(|l| {
        let resets = l.resets.map(|ms| ms.saturating_sub(now) / 60_000);
        let resets = resets.map_or(String::new(), |m| format!(", resets in {m} min"));
        format!("{} {}%{resets}", l.name, l.used)
    });
    let limits: Vec<String> = limits.collect();
    match &s.limits {
        Some(_) if limits.is_empty() => println!("[limits] none"),
        Some(_) => println!("[limits] {}", limits.join("; ")),
        None => println!("[limits] unknown"),
    }
    match &s.account {
        Some(a) => println!(
            "[account] {} on {}",
            a.email.as_deref().unwrap_or("-"),
            a.plan
        ),
        None => println!("[account] unknown"),
    }
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
        Item::Docs { needs, .. } => {
            let docs: Vec<String> = needs
                .iter()
                .map(|n| format!("{} {}", n.doc, n.section))
                .collect();
            format!("[docs may need] {}", docs.join(", "))
        }
        Item::End { stop, ms, .. } => format!("[end] {stop:?} in {ms} ms"),
        Item::PastTurn(config) => {
            format!("[model] {}", config.model.as_deref().unwrap_or_default())
        }
        Item::Note(t) => format!("[note] {t}"),
        Item::Act(a) => format!("[act] {a:?}"),
        Item::Woken => "[woken] new messages".into(),
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
