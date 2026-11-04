use detached_core::types::FileChange;

pub fn base_name(p: &str) -> &str {
    p.rsplit_once('/').map_or(p, |(_, b)| b)
}

pub fn dir_name(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The last segment of a disk path, which may use `\` on Windows.
pub fn folder_name(p: &str) -> &str {
    p.split(['/', '\\']).rfind(|s| !s.is_empty()).unwrap_or(p)
}

/// By path, not `RepoInfo::folder`: a repo listed once may sit in folders of several workspaces.
pub fn in_folder(path: &str, folder: &str) -> bool {
    path.strip_prefix(folder)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '\\']))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Conflict,
    Untracked,
    Added,
    Deleted,
    Renamed,
    Modified,
}

pub fn status_of(c: &FileChange) -> Status {
    let either = |x| c.index == x || c.worktree == x;
    if c.conflicted {
        Status::Conflict
    } else if c.index == '?' {
        Status::Untracked
    } else if either('A') {
        Status::Added
    } else if either('D') {
        Status::Deleted
    } else if matches!(c.index, 'R' | 'C') {
        Status::Renamed
    } else {
        Status::Modified
    }
}

pub fn letter(s: Status) -> &'static str {
    match s {
        Status::Modified => "M",
        Status::Renamed => "R",
        Status::Added => "A",
        Status::Deleted => "D",
        Status::Untracked => "U",
        Status::Conflict => "C",
    }
}

/// Unix seconds as local "Nov 4, 2026, 3:05 PM".
pub fn when(t: i64) -> String {
    let tm = detached_core::os::local(t);
    stamp(
        tm.tm_year + 1900,
        tm.tm_mon,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
    )
}

fn stamp(year: i32, month: i32, day: i32, hour: i32, min: i32) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS[month.clamp(0, 11) as usize];
    let (h12, pm) = ((hour + 11) % 12 + 1, hour >= 12);
    let ampm = if pm { "PM" } else { "AM" };
    format!("{month} {day}, {year}, {h12}:{min:02} {ampm}")
}

/// Matching for short lists (actions, branches, recent files); files are ranked in core.
pub mod fuzzy {
    pub struct Hit<'a, T> {
        pub item: &'a T,
        /// Char indices of the matched characters in the item's text.
        pub indices: Vec<usize>,
    }

    /// Case-insensitive subsequence match; tighter and earlier matches score higher.
    fn score(query: &str, text: &str) -> Option<(f64, Vec<usize>)> {
        let q: Vec<char> = query
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        let mut indices = vec![];
        for (i, c) in text.chars().enumerate() {
            match q.get(indices.len()) {
                Some(&want) if c.to_lowercase().next() == Some(want) => indices.push(i),
                None => break,
                _ => {}
            }
        }
        if indices.len() < q.len() {
            return None;
        }
        let (Some(&first), Some(&last)) = (indices.first(), indices.last()) else {
            return Some((0.0, indices));
        };
        Some((first as f64 - last as f64 - first as f64 / 100.0, indices))
    }

    /// Matching items, best first; an empty query keeps them all in order.
    pub fn filter<'a, T>(
        items: &'a [T],
        query: &str,
        text: impl Fn(&T) -> &str,
    ) -> Vec<Hit<'a, T>> {
        let mut out: Vec<_> = items
            .iter()
            .filter_map(|item| {
                score(query, text(item)).map(|(s, indices)| (s, Hit { item, indices }))
            })
            .collect();
        out.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.into_iter().map(|(_, hit)| hit).collect()
    }

    /// `text` split into runs, each marked whether its chars are at `indices`.
    pub fn segments(text: &str, indices: &[usize]) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = vec![];
        for (i, c) in text.chars().enumerate() {
            let hit = indices.contains(&i);
            match out.last_mut() {
                Some((run, h)) if *h == hit => run.push(c),
                _ => out.push((c.to_string(), hit)),
            }
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn matches_subsequences_case_insensitively() {
            assert_eq!(score("gtf", "Go to File").unwrap().1, [0, 3, 6]);
            assert!(score("xyz", "Go to File").is_none());
            assert_eq!(score("", "anything"), Some((0.0, vec![])));
        }

        #[test]
        fn ranks_tighter_matches_first_and_keeps_order_for_an_empty_query() {
            let items = ["feature/login", "fix-logging", "main"];
            let names = |q| {
                filter(&items, q, |s| s)
                    .iter()
                    .map(|h| *h.item)
                    .collect::<Vec<_>>()
            };
            assert_eq!(names("log"), ["fix-logging", "feature/login"]);
            assert_eq!(names(""), items);
            assert_eq!(*filter(&["a-x-b-c", "abc"], "abc", |s| s)[0].item, "abc");
        }

        #[test]
        fn splits_text_into_highlight_runs() {
            assert_eq!(
                segments("main", &[0, 1]),
                [("ma".to_string(), true), ("in".to_string(), false)]
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(index: char, worktree: char) -> FileChange {
        FileChange {
            path: "p".into(),
            orig_path: None,
            index,
            worktree,
            conflicted: false,
        }
    }

    #[test]
    fn names_the_status() {
        let s: Vec<_> = [('A', 'M'), ('.', 'D'), ('R', '.'), ('M', 'M')]
            .map(|(i, w)| status_of(&change(i, w)))
            .into();
        assert_eq!(
            s,
            [
                Status::Added,
                Status::Deleted,
                Status::Renamed,
                Status::Modified
            ]
        );
        assert_eq!(status_of(&change('?', '?')), Status::Untracked);
        let conflict = FileChange {
            conflicted: true,
            ..change('U', 'U')
        };
        assert_eq!(letter(status_of(&conflict)), "C");
        assert_eq!(letter(Status::Untracked), "U");
    }

    #[test]
    fn names_paths() {
        assert_eq!(base_name("src/lib/a.ts"), "a.ts");
        assert_eq!(base_name("a.ts"), "a.ts");
        assert_eq!(dir_name("src/lib/a.ts"), "src/lib");
        assert_eq!(dir_name("a.ts"), "");
        assert_eq!(folder_name("/Users/me/dev/"), "dev");
        assert!(in_folder("/dev/a", "/dev/a"));
        assert!(in_folder("/dev/a/b", "/dev/a"));
        assert!(!in_folder("/dev/ab", "/dev/a"));
        assert_eq!(folder_name(r"C:\Users\me\dev\"), "dev");
        assert!(in_folder(r"C:\dev\a\b", r"C:\dev\a"));
        assert!(!in_folder(r"C:\dev\ab", r"C:\dev\a"));
    }

    #[test]
    fn formats_times() {
        assert_eq!(stamp(2026, 10, 4, 15, 5), "Nov 4, 2026, 3:05 PM");
        assert_eq!(stamp(2026, 0, 1, 0, 30), "Jan 1, 2026, 12:30 AM");
        assert_eq!(stamp(2026, 0, 1, 12, 0), "Jan 1, 2026, 12:00 PM");
        assert!(when(0).contains("19"));
    }
}
