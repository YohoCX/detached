//! A repo's docs: `docs/INDEX.md` and its module docs, the plans, the decisions and the contracts.
//! Read only; agents write docs, the host never does.

use std::fs;
use std::path::{Component, Path};

use crate::env::Env;
use crate::git::status::commits;
use crate::types::CommitSummary;

pub const INDEX: &str = "docs/INDEX.md";
const CONTRACTS: [&str; 3] = ["docs/KEYMAP.md", "docs/BACKLOG.md", "docs/TECHDEBT.md"];
/// A doc this short is answered whole.
const WHOLE: usize = 100;
const HITS: usize = 2;
const STOP: &[&str] = &[
    "the", "and", "for", "how", "what", "when", "does", "with", "from", "about", "this", "that",
    "work", "read", "are", "its", "into", "why", "where", "which",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Listed in the index.
    Module,
    Plan,
    Decision,
    Contract,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    /// Relative to the repo, with `/`.
    pub path: String,
    pub kind: Kind,
    pub title: String,
    /// The index line's "read when".
    pub when: Option<String>,
    /// The source paths its `covers:` front matter names, else its index line.
    pub covers: Vec<String>,
    pub lines: usize,
    /// Its text after the front matter.
    pub body: String,
    pub sections: Vec<Section>,
    /// The covered files' commits newer than the doc's own, newest first: it may be stale.
    pub newer: Vec<CommitSummary>,
    /// Why it could not be read.
    pub error: Option<String>,
}

/// A `## ` heading and its text up to the next one.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub heading: String,
    /// The heading's line, from 1.
    pub line: usize,
    pub text: String,
    /// The `path:line` places its text cites.
    pub cites: Vec<(String, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Docs {
    /// Whether the repo has `docs/INDEX.md`.
    pub index: bool,
    /// The index's docs in its order, the plans, the decisions newest first, the contracts.
    pub all: Vec<Doc>,
}

/// A doc a query routed to: one of its sections, or all of it (None) when it is short.
pub struct Hit<'a> {
    pub doc: &'a Doc,
    pub section: Option<&'a Section>,
}

/// A doc whose covered files a turn changed and the section that may need an update.
#[derive(Clone, Debug, PartialEq)]
pub struct Need {
    pub doc: String,
    pub section: &'static str,
    pub files: Vec<String>,
}

impl Docs {
    /// The docs with what makes them stale, from the repo's history.
    pub fn load(env: &Env, repo: &Path) -> Docs {
        let mut docs = Docs::read(repo);
        for d in (docs.all.iter_mut()).filter(|d| d.error.is_none() && !d.covers.is_empty()) {
            d.newer = newer(env, repo, d).unwrap_or_default();
        }
        docs
    }

    /// The docs as their files say, without staleness.
    pub fn read(repo: &Path) -> Docs {
        let file = |p: &str| {
            let f = repo.join(inside(repo, p)?);
            fs::read_to_string(f).map_err(|e| format!("{p} cannot be read: {e}"))
        };
        let mut all = vec![];
        let index = file(INDEX).ok();
        for (path, covers, when) in index.iter().flat_map(|i| i.lines().filter_map(entry)) {
            let path = inside(repo, &path).unwrap_or(path);
            let mut d = doc(path.clone(), Kind::Module, file(&path));
            if d.covers.is_empty() {
                d.covers = covers;
            }
            d.when = Some(when);
            all.push(d);
        }
        let names = |dir: &Path| {
            let mut n: Vec<String> = (fs::read_dir(dir).into_iter().flatten().flatten())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".md"))
                .collect();
            n.sort();
            n
        };
        for n in names(repo).into_iter().filter(|n| n.starts_with("PLAN")) {
            all.push(doc(n.clone(), Kind::Plan, file(&n)));
        }
        for n in names(&repo.join("docs/decisions")).into_iter().rev() {
            let p = format!("docs/decisions/{n}");
            all.push(doc(p.clone(), Kind::Decision, file(&p)));
        }
        for p in CONTRACTS.into_iter().filter(|p| repo.join(p).is_file()) {
            all.push(doc(p.into(), Kind::Contract, file(p)));
        }
        for d in all.iter_mut().filter(|d| d.error.is_none()) {
            match d.covers.iter().map(|c| inside(repo, c)).collect() {
                Ok(covers) => d.covers = covers,
                Err(e) => d.error = Some(e),
            }
        }
        Docs {
            index: index.is_some(),
            all,
        }
    }

    /// The docs whose index line's "read when" fits `query` best, at most two, each with the
    /// section that fits it best or whole when short.
    pub fn find(&self, query: &str) -> Vec<Hit<'_>> {
        let q = words(query);
        let has = |text: &str| words(text).iter().filter(|w| q.contains(w)).count();
        let mut scored: Vec<_> = (self.all.iter())
            .filter(|d| d.kind == Kind::Module && d.error.is_none())
            .map(|d| {
                let when = has(d.when.as_deref().unwrap_or_default());
                (2 * when + has(&d.path) + has(&d.covers.join(" ")), d)
            })
            .filter(|(score, _)| *score > 0)
            .collect();
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        let section = |doc: &Doc| {
            let mut best: Option<(usize, usize)> = None;
            for (i, s) in doc.sections.iter().enumerate() {
                let score = 2 * has(&s.heading) + has(&s.text);
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, i));
                }
            }
            best.map(|(_, i)| i).filter(|_| doc.lines >= WHOLE)
        };
        let hits = scored.into_iter().take(HITS).map(|(_, doc)| Hit {
            doc,
            section: section(doc).map(|i| &doc.sections[i]),
        });
        hits.collect()
    }

    /// The index's docs that cover a file of `changed`, unless the turn changed the doc too.
    pub fn may_need(&self, changed: &[String]) -> Vec<Need> {
        let module = |d: &&Doc| d.kind == Kind::Module && d.error.is_none();
        let needs = self.all.iter().filter(module).filter_map(|d| {
            if changed.contains(&d.path) {
                return None;
            }
            let files: Vec<String> = (changed.iter())
                .filter(|f| d.covers.iter().any(|c| covered(f, c)))
                .cloned()
                .collect();
            let section = match files.iter().all(|f| test(f)) {
                true => "Tests",
                false => "Flow",
            };
            (!files.is_empty()).then(|| Need {
                doc: d.path.clone(),
                section,
                files,
            })
        });
        needs.collect()
    }
}

/// An index line, `- <path> — <covers> — read when <trigger>.`, as path, covers and trigger.
fn entry(line: &str) -> Option<(String, Vec<String>, String)> {
    let mut parts = line.strip_prefix("- ")?.splitn(3, " — ");
    let (path, covers, when) = (parts.next()?, parts.next()?, parts.next()?);
    let when = when
        .trim()
        .strip_prefix("read when ")?
        .trim_end_matches('.');
    let covers = covers.split(',').map(|c| c.trim().trim_matches('`').into());
    Some((
        path.trim().trim_matches('`').into(),
        covers.filter(|c: &String| !c.is_empty()).collect(),
        when.into(),
    ))
}

fn doc(path: String, kind: Kind, text: Result<String, String>) -> Doc {
    let mut d = Doc {
        path,
        kind,
        title: String::new(),
        when: None,
        covers: vec![],
        lines: 0,
        body: String::new(),
        sections: vec![],
        newer: vec![],
        error: None,
    };
    let text = match text {
        Ok(t) => t,
        Err(e) => {
            d.error = Some(e);
            return d;
        }
    };
    let lines: Vec<&str> = text.lines().collect();
    d.lines = lines.len();
    let mut start = 0;
    if lines.first() == Some(&"---")
        && let Some(end) = lines[1..].iter().position(|l| *l == "---")
    {
        d.covers = covers(&lines[1..=end]);
        start = end + 2;
    }
    let body = &lines[start.min(lines.len())..];
    d.body = body.join("\n");
    d.title = body
        .iter()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or(&d.path)
        .trim()
        .into();
    let mut fenced = false;
    for (i, l) in body.iter().enumerate() {
        fenced ^= l.starts_with("```");
        match l.strip_prefix("## ") {
            Some(h) if !fenced => d.sections.push(Section {
                heading: h.trim().into(),
                line: start + i + 1,
                text: String::new(),
                cites: vec![],
            }),
            _ => {
                if let Some(s) = d.sections.last_mut() {
                    s.text.push_str(l);
                    s.text.push('\n');
                }
            }
        }
    }
    for s in &mut d.sections {
        s.text = s.text.trim().into();
        s.cites = cites(&s.text);
    }
    d
}

/// `covers:` in front matter, inline (`covers: a, b`) or as a `- path` list.
fn covers(front: &[&str]) -> Vec<String> {
    let Some(at) = front.iter().position(|l| l.starts_with("covers:")) else {
        return vec![];
    };
    let inline = front[at]["covers:".len()..].trim().trim_matches(['[', ']']);
    let items: Vec<&str> = match inline.is_empty() {
        true => (front[at + 1..].iter())
            .map_while(|l| l.trim().strip_prefix("- "))
            .collect(),
        false => inline.split(',').collect(),
    };
    let clean = |c: &str| {
        c.trim()
            .trim_matches(['`', '"', '\''])
            .trim_end_matches('/')
            .to_string()
    };
    items
        .into_iter()
        .map(clean)
        .filter(|c| !c.is_empty())
        .collect()
}

/// The `path:line` places in `text`.
fn cites(text: &str) -> Vec<(String, u32)> {
    let tokens = text.split(|c: char| c.is_whitespace() || "`()[],;".contains(c));
    let cite = |t: &str| {
        let (path, line) = t.rsplit_once(':')?;
        let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
        let ok = path.contains(['/', '.']) && !path.contains("://") && !path.starts_with('-');
        Some((path.to_string(), digits.parse().ok().filter(|_| ok)?))
    };
    let mut all: Vec<_> = tokens.filter_map(cite).collect();
    all.dedup();
    all
}

/// The commits to its covered files after the doc's own last commit; none before it has one.
fn newer(env: &Env, repo: &Path, d: &Doc) -> crate::error::Result<Vec<CommitSummary>> {
    // Literal pathspecs: a doc's paths are file names, never git patterns or magic.
    let literal = |p: &str| format!(":(literal){p}");
    let Some(own) = commits(env, repo, &["-1", "--", &literal(&d.path)])?.pop() else {
        return Ok(vec![]);
    };
    let mut args = vec![format!("{}..HEAD", own.sha), "--".into()];
    args.extend(d.covers.iter().map(|c| literal(c)));
    commits(
        env,
        repo,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

/// `path` relative to `repo`, normalized, when it stays there as a plain path: no `..`, no link,
/// no `.env*` part in any case (the agents' deny).
fn inside(repo: &Path, path: &str) -> Result<String, String> {
    let out = || format!("{path} is outside the repo, a link or denied, so it is skipped");
    let mut parts = vec![];
    for c in Path::new(path).components() {
        match c {
            Component::CurDir => continue,
            // Case-blind: `.ENV` opens `.env` on macOS and Windows.
            Component::Normal(n) if !n.to_string_lossy().to_lowercase().starts_with(".env") => {
                parts.push(n.to_string_lossy().into_owned());
            }
            _ => return Err(out()),
        }
        let link = fs::symlink_metadata(repo.join(parts.join("/")));
        if link.is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(out());
        }
    }
    match parts.is_empty() {
        true => Err(out()),
        false => Ok(parts.join("/")),
    }
}

fn covered(file: &str, cover: &str) -> bool {
    file == cover || file.strip_prefix(cover).is_some_and(|r| r.starts_with('/'))
}

fn test(file: &str) -> bool {
    file.contains("/tests/") || file.starts_with("tests/") || file.ends_with("tests.rs")
}

/// Lowercased words of three letters or more, cut to five so `watcher` meets `watch`.
fn words(s: &str) -> Vec<String> {
    let cut = |w: &str| w.to_lowercase().chars().take(5).collect::<String>();
    let all = s
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3);
    let mut w: Vec<String> = all
        .map(cut)
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect();
    w.sort();
    w.dedup();
    w
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    const INDEX_MD: &str = "# Index\n\n\
        - docs/watch.md — src/watch.rs — read when file events do not reach a repo's status.\n\
        - `docs/store.md` — src/store.rs, src/paths.rs — read when changing where shared items go.\n";

    fn module(covers: &str, sections: &[(&str, &str)], pad: usize) -> String {
        let mut s = format!("---\ncovers:\n  - {covers}\n---\n# {covers}\n");
        for (h, text) in sections {
            s += &format!("\n## {h}\n{text}\n");
        }
        s + &"filler\n".repeat(pad)
    }

    fn repo(sb: &Sandbox, short: bool) -> std::path::PathBuf {
        let r = sb.path("repo");
        let w = |p: &str, t: &str| {
            fs::create_dir_all(r.join(p).parent().unwrap()).unwrap();
            fs::write(r.join(p), t).unwrap();
        };
        let pad = if short { 0 } else { 120 };
        w(INDEX, INDEX_MD);
        w(
            "docs/watch.md",
            &module(
                "src/watch.rs",
                &[
                    ("Purpose", "Turns file events into reloads."),
                    ("Flow", "Each event is debounced, see `src/watch.rs:12`."),
                    ("Tests", "In `src/watch.rs`, for store paths."),
                ],
                pad,
            ),
        );
        w(
            "docs/store.md",
            &module(
                "src/store.rs",
                &[("Purpose", "Where the watcher's store goes.")],
                0,
            ),
        );
        w("src/watch.rs", "fn watch() {}\n");
        w(
            "PLAN-X.md",
            "---\ncovers: src, crates/x/\n---\n# Plan X\n\n## Steps\nOne.\n",
        );
        w("docs/decisions/2026-01-02-b.md", "# B\n");
        w("docs/decisions/2026-01-01-a.md", "# A\n");
        w("docs/KEYMAP.md", "# Keys\n");
        sb.git(&r, &["init", "-q"]);
        sb.git(&r, &["add", "."]);
        sb.git(&r, &["commit", "-qm", "docs"]);
        r
    }

    #[test]
    fn reads_the_index_s_docs_plans_decisions_by_date_and_contracts() {
        let sb = Sandbox::new();
        let docs = Docs::read(&repo(&sb, true));
        assert!(docs.index);
        let all: Vec<_> = docs.all.iter().map(|d| (d.path.as_str(), d.kind)).collect();
        assert_eq!(
            all,
            [
                ("docs/watch.md", Kind::Module),
                ("docs/store.md", Kind::Module),
                ("PLAN-X.md", Kind::Plan),
                ("docs/decisions/2026-01-02-b.md", Kind::Decision),
                ("docs/decisions/2026-01-01-a.md", Kind::Decision),
                ("docs/KEYMAP.md", Kind::Contract),
            ]
        );
        let watch = &docs.all[0];
        assert_eq!(watch.covers, ["src/watch.rs"]);
        assert_eq!(
            watch.when.as_deref(),
            Some("file events do not reach a repo's status")
        );
        assert_eq!(watch.title, "src/watch.rs");
        let flow = &watch.sections[1];
        assert_eq!((flow.heading.as_str(), flow.line), ("Flow", 10));
        assert_eq!(flow.cites, [("src/watch.rs".into(), 12)]);
        assert_eq!(docs.all[2].covers, ["src", "crates/x"]);
        assert_eq!(docs.all[2].title, "Plan X");
        assert!(!Docs::read(&sb.path("nowhere")).index);
    }

    #[test]
    fn routes_by_read_when_and_answers_one_section_per_hit_or_a_short_doc_whole() {
        let sb = Sandbox::new();
        let long = Docs::read(&repo(&sb, false));
        let hits = long.find("How does the watcher debounce file events?");
        let got: Vec<_> = hits
            .iter()
            .map(|h| (h.doc.path.as_str(), h.section.map(|s| s.heading.as_str())))
            .collect();
        // The store doc's body names the watcher, but only the watch doc's "read when" fits.
        assert_eq!(got, [("docs/watch.md", Some("Flow"))]);
        let hits = long.find("where do shared items go, and the watcher's file events");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].doc.path, "docs/watch.md");
        assert_eq!(hits[1].doc.path, "docs/store.md");
        assert!(hits[1].section.is_none(), "short: whole");
        assert!(long.find("nothing like it").is_empty());
        assert!(long.find("watcher").iter().all(|h| h.section.is_some()));
        fs::write(
            sb.path("repo/docs/watch.md"),
            module("src/watch.rs", &[("Flow", "Short.")], 0),
        )
        .unwrap();
        let short = Docs::read(&sb.path("repo"));
        assert!(short.find("watcher").iter().all(|h| h.section.is_none()));
    }

    #[test]
    fn a_doc_is_stale_after_a_newer_commit_to_a_covered_file() {
        let sb = Sandbox::new();
        let r = repo(&sb, true);
        let env = sb.state.env();
        let stale = |docs: &Docs, path: &str| {
            let d = docs.all.iter().find(|d| d.path == path).unwrap();
            d.newer
                .iter()
                .map(|c| c.subject.clone())
                .collect::<Vec<_>>()
        };
        let docs = Docs::load(&env, &r);
        assert!(stale(&docs, "docs/watch.md").is_empty());
        sb.commit(&r, "src/watch.rs", "fn watch() { moved }\n", "watch moved");
        sb.commit(&r, "src/other.rs", "x\n", "other");
        let docs = Docs::load(&env, &r);
        assert_eq!(stale(&docs, "docs/watch.md"), ["watch moved"]);
        assert!(stale(&docs, "docs/store.md").is_empty());
        assert_eq!(stale(&docs, "PLAN-X.md"), ["other", "watch moved"]);
        sb.commit(
            &r,
            "docs/watch.md",
            "---\ncovers: src/watch.rs\n---\n# W\n",
            "doc",
        );
        assert!(stale(&Docs::load(&env, &r), "docs/watch.md").is_empty());
    }

    #[test]
    fn covered_paths_reach_git_as_literal_names_never_patterns_or_magic() {
        let sb = Sandbox::new();
        let r = repo(&sb, true);
        let env = sb.state.env();
        let cover = |covers: &str| {
            let doc = format!("---\ncovers: {covers}\n---\n# W\n");
            sb.commit(&r, "docs/watch.md", &doc, "doc");
        };
        let newer = || {
            let d = Docs::load(&env, &r).all.remove(0);
            assert_eq!(d.error, None);
            d.newer.len()
        };
        fs::write(r.join("src/[ab].rs"), "x\n").unwrap();
        sb.git(&r, &["add", "."]);
        cover("src/[ab].rs");
        sb.commit(&r, "src/a.rs", "a\n", "a only");
        assert_eq!(newer(), 0, "`[ab]` is no pattern");
        sb.commit(&r, "src/[ab].rs", "y\n", "the literal file");
        assert_eq!(newer(), 1);
        cover("./:(exclude)src");
        sb.commit(&r, "README.md", "x\n", "outside src");
        assert_eq!(newer(), 0, "no magic");
    }

    #[test]
    fn a_change_to_a_covered_file_names_its_doc_unless_the_doc_changed_too() {
        let sb = Sandbox::new();
        let docs = Docs::read(&repo(&sb, true));
        let changed =
            |f: &[&str]| docs.may_need(&f.iter().map(|f| f.to_string()).collect::<Vec<_>>());
        assert_eq!(
            changed(&["src/watch.rs", "README.md"]),
            [Need {
                doc: "docs/watch.md".into(),
                section: "Flow",
                files: vec!["src/watch.rs".into()],
            }]
        );
        assert!(changed(&["src/watch.rs", "docs/watch.md"]).is_empty());
        assert!(changed(&["src/watch.rsx", "PLAN-X.md"]).is_empty());
        let r = sb.path("repo");
        fs::write(
            r.join("docs/watch.md"),
            "---\ncovers: ./src//, docs/../docs\n---\n",
        )
        .unwrap();
        let docs = Docs::read(&r);
        assert!(docs.all[0].error.is_some(), "`..` is refused");
        fs::write(r.join("docs/watch.md"), "---\ncovers: ./src//\n---\n").unwrap();
        let index = fs::read_to_string(r.join(INDEX)).unwrap();
        fs::write(
            r.join(INDEX),
            index.replace("- docs/watch.md", "- ./docs//watch.md"),
        )
        .unwrap();
        let docs = Docs::read(&r);
        assert_eq!(
            (docs.all[0].path.as_str(), &docs.all[0].covers[..]),
            ("docs/watch.md", &["src".to_string()][..])
        );
        let need = docs.may_need(&["src/a.rs".into()]);
        assert_eq!(need[0].files, ["src/a.rs"]);
    }

    #[test]
    fn a_path_out_of_the_repo_or_denied_is_skipped_with_its_reason() {
        let sb = Sandbox::new();
        let r = repo(&sb, true);
        fs::write(sb.path("secret.md"), "## Flow\nwatcher secret\n").unwrap();
        fs::write(r.join(".env"), "watcher secret\n").unwrap();
        let secret = sb.path("secret.md").display().to_string();
        let index = format!(
            "- ../secret.md — src — read when the watcher.\n\
             - {secret} — src — read when the watcher.\n\
             - docs/.env.md — src — read when the watcher.\n\
             - .ENV — src — read when the watcher.\n\
             - docs/.Env/x.md — src — read when the watcher.\n\
             - docs/store.md — ../secret.md — read when the watcher.\n"
        );
        fs::write(r.join("docs/store.md"), "# Store\n\n## Flow\nwatcher\n").unwrap();
        #[cfg(unix)]
        let index = {
            std::os::unix::fs::symlink(sb.path("secret.md"), r.join("docs/link.md")).unwrap();
            std::os::unix::fs::symlink("../.env", r.join("docs/setup.md")).unwrap();
            std::os::unix::fs::symlink("../docs", r.join("src/docs")).unwrap();
            index
                + "- docs/link.md — src — read when the watcher.\n\
                   - docs/setup.md — src — read when the watcher.\n\
                   - src/docs/store.md — src — read when the watcher.\n"
        };
        fs::write(r.join(INDEX), index).unwrap();
        let docs = Docs::read(&r);
        let modules: Vec<_> = docs.all.iter().filter(|d| d.kind == Kind::Module).collect();
        assert_eq!(modules.len(), if cfg!(unix) { 9 } else { 6 });
        for d in modules {
            let e = d
                .error
                .as_deref()
                .unwrap_or_else(|| panic!("{} was read", d.path));
            assert!(e.ends_with("so it is skipped"), "{e}");
            assert!(!d.body.contains("secret"), "{}", d.path);
        }
        assert!(docs.find("watcher secret").is_empty());
        assert!(docs.may_need(&["src/watch.rs".into()]).is_empty());
    }

    #[test]
    fn this_repo_s_docs_keep_the_layout_and_route_the_watcher() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let docs = Docs::read(&root);
        let modules: Vec<_> = docs.all.iter().filter(|d| d.kind == Kind::Module).collect();
        assert!(!modules.is_empty());
        let five = ["Purpose", "Public API", "Flow", "Gotchas", "Tests"];
        for d in modules {
            assert_eq!(d.error, None);
            let headings: Vec<_> = d.sections.iter().map(|s| s.heading.as_str()).collect();
            assert_eq!(headings, five, "{}", d.path);
            assert!(d.lines <= 150, "{} has {} lines", d.path, d.lines);
            for c in &d.covers {
                assert!(
                    root.join(c).exists(),
                    "{} covers {c}, which is not there",
                    d.path
                );
            }
        }
        let hits = docs.find("how does the watcher work?");
        assert_eq!(hits[0].doc.path, "docs/crates/core/src/watch.md");
    }
}
