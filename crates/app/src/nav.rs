use gpui_kit::{App, Global};

use crate::store::{Doc, Location, Store};

/// Back/forward over files, MR pages and job logs; recent files. In memory only.
#[derive(Default)]
pub struct Nav {
    /// Files only, newest first, `line` dropped.
    pub recent: Vec<Location>,
    history: Vec<Location>,
    index: usize,
}

impl Global for Nav {}

impl Nav {
    /// Recent files (50, deduped) and history (100, forward entries dropped); the current entry is not pushed twice.
    pub fn visit(&mut self, loc: &Location) {
        if loc.doc.is_none() {
            let same = |r: &Location| r.repo == loc.repo && r.rev == loc.rev && r.path == loc.path;
            self.recent.retain(|r| !same(r));
            self.recent.insert(
                0,
                Location {
                    line: None,
                    ..loc.clone()
                },
            );
            self.recent.truncate(50);
        }
        if self.history.get(self.index) == Some(loc) {
            return;
        }
        self.history.truncate(self.index + 1);
        self.history.push(loc.clone());
        if self.history.len() > 100 {
            self.history.remove(0);
        }
        self.index = self.history.len() - 1;
    }

    pub fn back(&mut self) -> Option<Location> {
        self.index = self.index.checked_sub(1)?;
        self.history.get(self.index).cloned()
    }

    pub fn forward(&mut self) -> Option<Location> {
        let loc = self.history.get(self.index + 1).cloned()?;
        self.index += 1;
        Some(loc)
    }
}

/// Records `loc` and shows it.
pub fn open(cx: &mut App, loc: Location) {
    cx.default_global::<Nav>().visit(&loc);
    show(loc, cx);
}

pub fn back(cx: &mut App) {
    if let Some(loc) = cx.default_global::<Nav>().back() {
        show(loc, cx);
    }
}

pub fn forward(cx: &mut App) {
    if let Some(loc) = cx.default_global::<Nav>().forward() {
        show(loc, cx);
    }
}

fn show(loc: Location, cx: &mut App) {
    Store::global(cx).update(cx, |s, cx| s.show(Doc::from(loc), cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(path: &str, line: Option<u32>) -> Location {
        Location {
            repo: "/r".into(),
            rev: None,
            path: path.into(),
            line,
            doc: None,
        }
    }

    fn paths(v: &[Location]) -> Vec<&str> {
        v.iter().map(|l| l.path.as_str()).collect()
    }

    #[test]
    fn keeps_the_last_50_recent_files_most_recent_first_deduped() {
        let mut n = Nav::default();
        for i in 0..60 {
            n.visit(&loc(&format!("f{i}"), None));
        }
        n.visit(&loc("f55", Some(3)));
        assert_eq!(n.recent.len(), 50);
        assert_eq!(n.recent[0], loc("f55", None));
        assert_eq!(n.recent.iter().filter(|r| r.path == "f55").count(), 1);
        assert_eq!(n.recent[1].path, "f59");
        n.visit(&Location {
            rev: Some("main".into()),
            ..loc("f59", None)
        });
        assert_eq!(n.recent[0].rev.as_deref(), Some("main"));
        assert_eq!(n.recent[1].rev, None);
    }

    #[test]
    fn caps_history_at_100() {
        let mut n = Nav::default();
        for i in 0..120 {
            n.visit(&loc(&format!("f{i}"), None));
        }
        assert_eq!(n.history.len(), 100);
        assert_eq!(n.history[0].path, "f20");
        assert_eq!(n.index, 99);
    }

    #[test]
    fn walks_back_and_forward_and_a_new_visit_drops_the_forward_entries() {
        let mut n = Nav::default();
        for p in ["a", "b", "c"] {
            n.visit(&loc(p, None));
        }
        assert_eq!(n.back().unwrap().path, "b");
        assert_eq!(n.back().unwrap().path, "a");
        assert!(n.back().is_none());
        assert_eq!(n.forward().unwrap().path, "b");
        n.visit(&loc("d", None));
        assert!(n.forward().is_none());
        assert_eq!(paths(&n.history), ["a", "b", "d"]);
    }

    #[test]
    fn does_not_push_the_current_location_twice_but_a_new_line_counts() {
        let mut n = Nav::default();
        n.visit(&loc("a", Some(1)));
        n.visit(&loc("a", Some(1)));
        n.visit(&loc("a", Some(9)));
        let lines: Vec<_> = n.history.iter().map(|l| l.line).collect();
        assert_eq!(lines, [Some(1), Some(9)]);
    }

    #[test]
    fn an_empty_history_goes_nowhere() {
        let mut n = Nav::default();
        assert!(n.back().is_none());
        assert!(n.forward().is_none());
    }
}
