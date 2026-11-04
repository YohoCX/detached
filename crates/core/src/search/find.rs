//! Find in files: grep-searcher over the working tree, `git grep` at a rev.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::WalkState;
use ignore::overrides::OverrideBuilder;

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::arg;
use crate::git::files::{slashed, walker};
use crate::git::runner::{Kind, Opts, run};
use crate::state::Cancel;
use crate::types::{FindEvent, FindQuery};

const CAP: usize = 10_000;
/// Longer lines (minified code) are cut, so one match can't flood the UI.
const MAX_LINE: usize = 1000;

struct Find<'a> {
    query_id: u64,
    matcher: RegexMatcher,
    cancel: &'a Cancel,
    emit: &'a (dyn Fn(FindEvent) + Sync),
    count: AtomicUsize,
}

impl Find<'_> {
    fn stopped(&self) -> bool {
        self.cancel.load(Ordering::SeqCst) || self.count.load(Ordering::SeqCst) > CAP
    }

    /// Emits a match line; false once the cap is passed or the query is cancelled.
    fn hit(&self, repo: &str, path: &str, line: u64, bytes: &[u8]) -> bool {
        if self.cancel.load(Ordering::SeqCst) || self.count.fetch_add(1, Ordering::SeqCst) >= CAP {
            return false;
        }
        let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        if text.len() > MAX_LINE {
            let mut cut = MAX_LINE;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        let mut ranges = vec![];
        let _ = self.matcher.find_iter(text.as_bytes(), |m| {
            ranges.push((m.start() as u32, m.end() as u32));
            true
        });
        (self.emit)(FindEvent::Match {
            query_id: self.query_id,
            repo: repo.into(),
            path: path.into(),
            line,
            text,
            ranges,
        });
        true
    }
}

struct Lines<'a> {
    find: &'a Find<'a>,
    repo: &'a str,
    path: &'a str,
}

impl Sink for Lines<'_> {
    type Error = std::io::Error;

    fn matched(&mut self, _: &Searcher, m: &SinkMatch) -> std::io::Result<bool> {
        let line = m.line_number().unwrap_or(0);
        Ok(self.find.hit(self.repo, self.path, line, m.bytes()))
    }
}

fn globs(glob: Option<&str>) -> impl Iterator<Item = &str> {
    glob.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|g| !g.is_empty())
}

/// Streams matches over `emit`, then one `Done` (or `Error`).
pub fn find(
    env: &Env,
    repos: &[String],
    rev: Option<&str>,
    q: &FindQuery,
    query_id: u64,
    cancel: &Cancel,
    emit: &(dyn Fn(FindEvent) + Sync),
) {
    let r = (|| {
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(!q.case_sensitive)
            .word(q.whole_word)
            .fixed_strings(!q.regex)
            .line_terminator(Some(b'\n'))
            .build(&q.text)
            .map_err(|e| Error::Other(e.to_string()))?;
        let find = Find {
            query_id,
            matcher,
            cancel,
            emit,
            count: AtomicUsize::new(0),
        };
        for repo in repos {
            if find.stopped() {
                break;
            }
            match rev {
                None => worktree(&find, repo, q.glob.as_deref())?,
                Some(rev) => at_rev(env, &find, repo, rev, q)?,
            }
        }
        Ok(find.count.load(Ordering::SeqCst) > CAP)
    })();
    emit(match r {
        Ok(truncated) => FindEvent::Done {
            query_id,
            truncated,
        },
        Err(Error::Cancelled) => FindEvent::Done {
            query_id,
            truncated: false,
        },
        Err(e) => FindEvent::Error {
            query_id,
            error: e.to_string(),
        },
    });
}

fn worktree(find: &Find, repo: &str, glob: Option<&str>) -> Result<()> {
    let root = Path::new(repo);
    let mut walk = walker(root);
    let mut over = OverrideBuilder::new(root);
    for g in globs(glob) {
        over.add(g).map_err(|e| Error::Other(e.to_string()))?;
    }
    walk.overrides(over.build().map_err(|e| Error::Other(e.to_string()))?);
    walk.build_parallel().run(|| {
        let mut searcher = SearcherBuilder::new()
            .line_number(true)
            .binary_detection(BinaryDetection::quit(0))
            .build();
        Box::new(move |entry| {
            if find.stopped() {
                return WalkState::Quit;
            }
            let Ok(e) = entry else {
                return WalkState::Continue;
            };
            let rel = e.path().strip_prefix(root).ok().and_then(Path::to_str);
            if let (Some(path), true) = (rel, e.file_type().is_some_and(|t| t.is_file())) {
                let sink = Lines {
                    find,
                    repo,
                    path: &slashed(path),
                };
                let _ = searcher.search_path(&find.matcher, e.path(), sink);
            }
            WalkState::Continue
        })
    });
    Ok(())
}

fn at_rev(env: &Env, find: &Find, repo: &str, rev: &str, q: &FindQuery) -> Result<()> {
    // PCRE, close to the Rust regex syntax the working-tree search uses.
    let kind = if q.regex { "-P" } else { "-F" };
    let mut args = vec!["grep", "-n", "-z", "--full-name", "-I", "--no-color", kind];
    if !q.case_sensitive {
        args.push("-i");
    }
    if q.whole_word {
        args.push("-w");
    }
    args.extend(["-e", &q.text, arg(rev)?, "--"]);
    let specs: Vec<String> = globs(q.glob.as_deref())
        .map(|g| {
            g.strip_prefix('!')
                .map_or(g.into(), |x| format!(":(exclude){x}"))
        })
        .collect();
    args.extend(specs.iter().map(String::as_str));
    let prefix = format!("{rev}:");
    // `<rev>:<path>\0<line>\0<text>`
    let on_line = |l: &[u8]| {
        let mut f = l.splitn(3, |&b| b == 0);
        let (Some(name), Some(line), Some(text)) = (f.next(), f.next(), f.next()) else {
            return true;
        };
        let name = String::from_utf8_lossy(name);
        let path = name.strip_prefix(&prefix).unwrap_or(&name);
        let line = std::str::from_utf8(line)
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        find.hit(repo, path, line, text)
    };
    let opts = Opts {
        cancel: Some(find.cancel),
        lines: Some(&on_line),
        ..Default::default()
    };
    let o = run(env, Path::new(repo), &args, Kind::Read, opts)?;
    // 1 is no match.
    if o.code > 1 {
        o.ok()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use std::sync::Mutex;

    type Hit = (String, u64, Vec<(u32, u32)>);

    struct Repo {
        sb: Sandbox,
        path: String,
    }

    fn repo(files: &[(&str, &str)]) -> Repo {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        for (name, text) in files {
            if let Some(dir) = Path::new(name).parent() {
                std::fs::create_dir_all(work.join(dir)).unwrap();
            }
            std::fs::write(work.join(name), text).unwrap();
        }
        let path = work.to_str().unwrap().to_string();
        Repo { sb, path }
    }

    fn query(text: &str) -> FindQuery {
        FindQuery {
            text: text.into(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
            glob: None,
        }
    }

    /// `(path, line, ranges)` sorted, and the final event.
    fn search(
        r: &Repo,
        rev: Option<&str>,
        q: &FindQuery,
        cancel: &Cancel,
    ) -> (Vec<Hit>, FindEvent) {
        let events = Mutex::new(vec![]);
        let emit = |e| events.lock().unwrap().push(e);
        find(
            &r.sb.state.env(),
            std::slice::from_ref(&r.path),
            rev,
            q,
            7,
            cancel,
            &emit,
        );
        let mut events = events.into_inner().unwrap();
        let last = events.pop().unwrap();
        let mut hits: Vec<_> = events
            .into_iter()
            .map(|e| match e {
                FindEvent::Match {
                    path,
                    line,
                    ranges,
                    query_id: 7,
                    ..
                } => (path, line, ranges),
                e => panic!("{e:?}"),
            })
            .collect();
        hits.sort();
        (hits, last)
    }

    fn hits(r: &Repo, rev: Option<&str>, q: &FindQuery) -> Vec<Hit> {
        let (hits, last) = search(r, rev, q, &Cancel::default());
        assert_eq!(
            last,
            FindEvent::Done {
                query_id: 7,
                truncated: false
            }
        );
        hits
    }

    fn at(path: &str, line: u64, ranges: &[(u32, u32)]) -> Hit {
        (path.into(), line, ranges.to_vec())
    }

    #[test]
    fn literal_regex_case_and_whole_word() {
        let r = repo(&[
            ("a.txt", "a.b here\naxb there\n"),
            ("src/w.rs", "foo bar\nfoobar\nFOO\n"),
            ("bin", "a.b\0binary"),
        ]);
        assert_eq!(hits(&r, None, &query("a.b")), [at("a.txt", 1, &[(0, 3)])]);
        let re = FindQuery {
            regex: true,
            ..query("a.b")
        };
        assert_eq!(
            hits(&r, None, &re),
            [at("a.txt", 1, &[(0, 3)]), at("a.txt", 2, &[(0, 3)])]
        );
        let word = FindQuery {
            whole_word: true,
            ..query("foo")
        };
        assert_eq!(
            hits(&r, None, &word),
            [at("src/w.rs", 1, &[(0, 3)]), at("src/w.rs", 3, &[(0, 3)])]
        );
        let case = FindQuery {
            case_sensitive: true,
            ..query("FOO")
        };
        assert_eq!(hits(&r, None, &case), [at("src/w.rs", 3, &[(0, 3)])]);
        let bad = FindQuery {
            regex: true,
            ..query("(")
        };
        assert!(matches!(
            search(&r, None, &bad, &Cancel::default()).1,
            FindEvent::Error { .. }
        ));
    }

    #[test]
    fn glob_and_gitignore_limit_the_files() {
        let r = repo(&[
            (".gitignore", "out/\n"),
            ("out/x.rs", "needle\n"),
            ("src/a.rs", "needle needle\n"),
            ("src/b.ts", "needle\n"),
        ]);
        let all: Vec<_> = hits(&r, None, &query("needle"))
            .into_iter()
            .map(|h| h.0)
            .collect();
        assert_eq!(all, ["src/a.rs", "src/b.ts"]);
        let rs = FindQuery {
            glob: Some("*.rs".into()),
            ..query("needle")
        };
        assert_eq!(hits(&r, None, &rs), [at("src/a.rs", 1, &[(0, 6), (7, 13)])]);
        let not_rs = FindQuery {
            glob: Some("!*.rs".into()),
            ..query("needle")
        };
        assert_eq!(hits(&r, None, &not_rs)[0].0, "src/b.ts");
    }

    #[test]
    fn at_a_rev_through_git_grep() {
        let r = repo(&[]);
        let work = Path::new(&r.path);
        std::fs::create_dir(work.join("src")).unwrap();
        r.sb.commit(work, "src/x.rs", "let Needle = 1;\nneedles\n", "x");
        r.sb.commit(work, "y.md", "needle\n", "y");
        std::fs::write(work.join("src/x.rs"), "gone\n").unwrap();
        assert_eq!(
            hits(&r, Some("HEAD"), &query("needle")),
            [
                at("src/x.rs", 1, &[(4, 10)]),
                at("src/x.rs", 2, &[(0, 6)]),
                at("y.md", 1, &[(0, 6)])
            ]
        );
        let q = FindQuery {
            whole_word: true,
            case_sensitive: true,
            glob: Some("*.rs".into()),
            ..query("Needle")
        };
        assert_eq!(hits(&r, Some("HEAD"), &q), [at("src/x.rs", 1, &[(4, 10)])]);
        let q = FindQuery {
            glob: Some("!*.rs".into()),
            ..query("needle")
        };
        assert_eq!(hits(&r, Some("HEAD"), &q), [at("y.md", 1, &[(0, 6)])]);
    }

    #[test]
    fn stops_at_the_cap_and_says_so() {
        let many = "hit\n".repeat(CAP + 50);
        let r = repo(&[("many.txt", &many)]);
        r.sb.commit(Path::new(&r.path), "many.txt", &many, "many");
        for rev in [None, Some("HEAD")] {
            let (hits, last) = search(&r, rev, &query("hit"), &Cancel::default());
            assert_eq!(hits.len(), CAP);
            assert_eq!(
                last,
                FindEvent::Done {
                    query_id: 7,
                    truncated: true
                }
            );
        }
    }

    #[test]
    fn cancel_stops_early() {
        let files: Vec<(String, &str)> = (0..300)
            .map(|i| (format!("d{}/f{i}.txt", i % 10), "x\n"))
            .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let r = repo(&files);
        let cancel = Cancel::default();
        let events = Mutex::new(0);
        let emit = |_| {
            *events.lock().unwrap() += 1;
            cancel.store(true, Ordering::SeqCst);
        };
        find(
            &r.sb.state.env(),
            std::slice::from_ref(&r.path),
            None,
            &query("x"),
            1,
            &cancel,
            &emit,
        );
        let n = events.into_inner().unwrap();
        assert!(n < 50, "{n} events after cancel");
    }

    #[test]
    fn regex_means_the_same_in_the_tree_and_at_a_rev() {
        let r = repo(&[("a.txt", "v12 Plan\nnothing\nPLAN v3\n")]);
        r.sb.git(Path::new(&r.path), &["add", "."]);
        r.sb.git(Path::new(&r.path), &["commit", "-qm", "a"]);
        let q = FindQuery {
            regex: true,
            case_sensitive: true,
            ..query(r"(?i)plan\b|v\d+")
        };
        let tree = hits(&r, None, &q);
        assert_eq!(tree.len(), 2);
        assert_eq!(hits(&r, Some("HEAD"), &q), tree);
    }
}
