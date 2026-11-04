//! Both sides of a file as one unified patch, for the native diff view.

use std::path::Path;

use imara_diff::{Algorithm, Diff, InternedInput, Token};

use crate::error::Result;
use crate::git::diff::file_diff;
use crate::state::AppState;
use crate::types::{Content, FileDiff, Side};

#[derive(Debug, Clone, PartialEq)]
pub enum FilePatch {
    /// The whole file as one hunk of context, in the form the kit's `DiffFile::parse` reads.
    Patch(String),
    /// Both sides hold this text: an unmodified file, a pure rename or a mode-only change.
    Unchanged(String),
    /// The sides as `file_diff` reads them, when they are not two texts: either is binary, too
    /// large or a submodule, the right one is conflicted, or both are missing.
    Sides(FileDiff),
}

pub fn file_patch(
    state: &AppState,
    repo: &Path,
    path: &str,
    orig_path: Option<&str>,
    left: &Side,
    right: &Side,
) -> Result<FilePatch> {
    fn text(c: &Content) -> Option<&str> {
        match c {
            Content::Text { text } | Content::Conflicted { text } => Some(text),
            _ => None,
        }
    }
    let d = file_diff(state, repo, path, orig_path, left, right)?;
    let (a, b) = (text(&d.left), text(&d.right));
    let diffable = !matches!(d.right, Content::Conflicted { .. })
        && (a.is_some() || d.left == Content::Missing)
        && (b.is_some() || d.right == Content::Missing)
        && (a.is_some() || b.is_some());
    if !diffable {
        return Ok(FilePatch::Sides(d));
    }
    Ok(match (a, b) {
        (Some(a), Some(b)) if a == b => FilePatch::Unchanged(b.into()),
        _ => FilePatch::Patch(unified(orig_path.unwrap_or(path), path, a, b)),
    })
}

/// `a` and `b` are the old and new text, `None` for a missing side.
fn unified(from: &str, to: &str, a: Option<&str>, b: Option<&str>) -> String {
    let input = InternedInput::new(a.unwrap_or_default(), b.unwrap_or_default());
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    diff.postprocess_lines(&input);
    let mut s = String::with_capacity(a.map_or(0, str::len) + b.map_or(0, str::len) + 256);
    s += &format!("diff --git {} {}\n", quote("a/", from), quote("b/", to));
    if a.is_some() && b.is_some() && from != to {
        s += &format!(
            "rename from {}\nrename to {}\n",
            quote("", from),
            quote("", to)
        );
    }
    let name =
        |side: Option<&str>, prefix, path| side.map_or("/dev/null".into(), |_| quote(prefix, path));
    s += &format!("--- {}\n+++ {}\n", name(a, "a/", from), name(b, "b/", to));
    let (n, m) = (input.before.len(), input.after.len());
    if n + m > 0 {
        s += &format!(
            "@@ -{},{n} +{},{m} @@\n",
            usize::from(n > 0),
            usize::from(m > 0)
        );
    }
    let mut push = |tag: char, tokens: &[Token]| {
        for &t in tokens {
            let line = input.interner[t];
            s.push(tag);
            s.push_str(line);
            if !line.ends_with('\n') {
                s.push_str("\n\\ No newline at end of file\n");
            }
        }
    };
    let mut at = 0;
    for h in diff.hunks() {
        let (start, end) = (h.before.start as usize, h.before.end as usize);
        push(' ', &input.before[at..start]);
        push('-', &input.before[start..end]);
        push(
            '+',
            &input.after[h.after.start as usize..h.after.end as usize],
        );
        at = end;
    }
    push(' ', &input.before[at..]);
    s
}

/// Quotes a path the way git does when it holds a quote, a backslash or a control character.
fn quote(prefix: &str, path: &str) -> String {
    let p = format!("{prefix}{path}");
    if !p.contains(|c: char| c == '"' || c == '\\' || c.is_ascii_control()) {
        return p;
    }
    let mut q = String::from("\"");
    for c in p.chars() {
        match c {
            '"' | '\\' => q += &format!("\\{c}"),
            c if c.is_ascii_control() => q += &format!("\\{:03o}", c as u32),
            c => q.push(c),
        }
    }
    q + "\""
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::runner::{Kind, Opts, run};
    use crate::git::testutil::Sandbox;

    fn patch(p: FilePatch) -> String {
        match p {
            FilePatch::Patch(p) => p,
            p => panic!("not a patch: {p:?}"),
        }
    }

    /// The old and new text a patch's hunk spells out.
    fn sides(patch: &str) -> (String, String) {
        let (mut a, mut b) = (String::new(), String::new());
        let mut last = ' ';
        let body = patch
            .split_inclusive('\n')
            .skip_while(|l| !l.starts_with("@@ "));
        for line in body.skip(1) {
            if line == "\\ No newline at end of file\n" {
                if last != '+' {
                    a.pop();
                }
                if last != '-' {
                    b.pop();
                }
                continue;
            }
            last = line.chars().next().unwrap();
            if last != '+' {
                a += &line[1..];
            }
            if last != '-' {
                b += &line[1..];
            }
        }
        (a, b)
    }

    #[test]
    fn modified_added_and_deleted() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.commit(&work, "f", "one\ntwo\nthree\nfour\n", "f");
        std::fs::write(work.join("f"), "one\n2\nthree\nfour\nfive\n").unwrap();
        std::fs::write(work.join("new"), "a\nb\n").unwrap();
        std::fs::write(work.join("empty"), "").unwrap();
        let p = |path| file_patch(&sb.state, &work, path, None, &Side::Head, &Side::Worktree);

        assert_eq!(
            patch(p("f").unwrap()),
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,4 +1,5 @@\n \
             one\n-two\n+2\n three\n four\n+five\n"
        );
        assert_eq!(
            patch(p("new").unwrap()),
            "diff --git a/new b/new\n--- /dev/null\n+++ b/new\n@@ -0,0 +1,2 @@\n+a\n+b\n"
        );
        assert_eq!(
            patch(p("empty").unwrap()),
            "diff --git a/empty b/empty\n--- /dev/null\n+++ b/empty\n"
        );
        std::fs::remove_file(work.join("f")).unwrap();
        assert_eq!(
            patch(p("f").unwrap()),
            "diff --git a/f b/f\n--- a/f\n+++ /dev/null\n@@ -1,4 +0,0 @@\n\
             -one\n-two\n-three\n-four\n"
        );
    }

    #[test]
    fn renames_with_and_without_edits_and_equal_sides() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let p = |path, orig, right| file_patch(&sb.state, &work, path, orig, &Side::Head, right);
        assert_eq!(
            p("f", None, &Side::Worktree).unwrap(),
            FilePatch::Unchanged("initial\n".into())
        );
        sb.git(&work, &["update-index", "--chmod=+x", "f"]);
        assert_eq!(
            p("f", None, &Side::Index).unwrap(),
            FilePatch::Unchanged("initial\n".into())
        );

        sb.git(&work, &["mv", "f", "g"]);
        assert_eq!(
            p("g", Some("f"), &Side::Index).unwrap(),
            FilePatch::Unchanged("initial\n".into())
        );
        std::fs::write(work.join("g"), "initial\nmore\n").unwrap();
        assert_eq!(
            patch(p("g", Some("f"), &Side::Worktree).unwrap()),
            "diff --git a/f b/g\nrename from f\nrename to g\n--- a/f\n+++ b/g\n\
             @@ -1,1 +1,2 @@\n initial\n+more\n"
        );
    }

    #[test]
    fn crlf_and_no_trailing_newline() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.commit(&work, "f", "a\r\nb\r\nend", "crlf");
        let p = |text: &str| {
            std::fs::write(work.join("f"), text).unwrap();
            let d = file_patch(&sb.state, &work, "f", None, &Side::Head, &Side::Worktree);
            patch(d.unwrap())
        };

        let head = "diff --git a/f b/f\n--- a/f\n+++ b/f\n";
        assert_eq!(
            p("a\r\nB\r\nend"),
            format!(
                "{head}@@ -1,3 +1,3 @@\n a\r\n-b\r\n+B\r\n end\n\\ No newline at end of file\n"
            )
        );
        assert_eq!(
            p("a\r\nb\r\nend\n"),
            format!(
                "{head}@@ -1,3 +1,3 @@\n a\r\n b\r\n-end\n\\ No newline at end of file\n+end\n"
            )
        );
        assert_eq!(
            p("a\nb\r\nend"),
            format!("{head}@@ -1,3 +1,3 @@\n-a\r\n+a\n b\r\n end\n\\ No newline at end of file\n")
        );
        for text in ["a\r\nb\r\nother", "x", "", "a\r\nb\r\nend\r\n\r\n"] {
            assert_eq!(sides(&p(text)), ("a\r\nb\r\nend".into(), text.into()));
        }
    }

    #[test]
    fn a_five_thousand_line_file_is_one_hunk_of_context() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let old: String = (1..=5000).map(|i| format!("line {i}\n")).collect();
        sb.commit(&work, "big", &old, "big");
        let new = old
            .replace("line 10\n", "line ten\n")
            .replace("line 2500\n", "")
            .replace("line 4999\n", "line 4999\nadded\n");
        std::fs::write(work.join("big"), &new).unwrap();

        let p =
            patch(file_patch(&sb.state, &work, "big", None, &Side::Head, &Side::Worktree).unwrap());
        assert!(p.contains("\n@@ -1,5000 +1,5000 @@\n"));
        assert_eq!(p.matches("\n@@ ").count(), 1);
        let changed: Vec<&str> = p.lines().skip(4).filter(|l| !l.starts_with(' ')).collect();
        assert_eq!(changed, ["-line 10", "+line ten", "-line 2500", "+added"]);
        assert_eq!(sides(&p), (old, new));
    }

    #[test]
    fn sides_that_are_not_text() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        std::fs::write(work.join("bin"), b"abc\0def").unwrap();
        sb.git(&work, &["add", "bin"]);
        sb.git(&work, &["commit", "-qm", "bin"]);
        std::fs::write(work.join("bin"), "text now\n").unwrap();
        let p = |path, left, right| file_patch(&sb.state, &work, path, None, left, right).unwrap();
        let sides = |left, right| FilePatch::Sides(FileDiff { left, right });
        let text = || Content::Text {
            text: "text now\n".into(),
        };

        assert_eq!(
            p("bin", &Side::Head, &Side::Worktree),
            sides(Content::Binary, text())
        );
        assert_eq!(
            p("bin", &Side::Worktree, &Side::Index),
            sides(text(), Content::Binary)
        );
        assert_eq!(
            p("nope", &Side::Head, &Side::Worktree),
            sides(Content::Missing, Content::Missing)
        );

        sb.git(&work, &["checkout", "--", "bin"]);
        sb.git(&work, &["switch", "-qc", "side"]);
        sb.commit(&work, "f", "side\n", "side");
        sb.git(&work, &["switch", "-q", "main"]);
        sb.commit(&work, "f", "main\n", "main");
        run(
            &sb.state.env(),
            &work,
            &["merge", "side"],
            Kind::Write,
            Opts::default(),
        )
        .unwrap();
        match p("f", &Side::Head, &Side::Worktree) {
            FilePatch::Sides(FileDiff {
                left: Content::Text { .. },
                right: Content::Conflicted { text },
            }) => assert!(text.contains("<<<<<<<")),
            p => panic!("{p:?}"),
        }
    }

    #[test]
    fn paths_are_quoted_like_git() {
        assert_eq!(quote("a/", "dir/file name.rs"), "a/dir/file name.rs");
        assert_eq!(quote("b/", "naïve"), "b/naïve");
        assert_eq!(quote("a/", "x\ty\"z\\"), "\"a/x\\011y\\\"z\\\\\"");
        assert_eq!(quote("", "line\nbreak"), "\"line\\012break\"");
    }
}
