//! Log, graph and commit details.

mod api;
mod graph;
mod log;

pub use api::*;
pub use log::Log;

use detached_core::types::LogRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chip {
    Head,
    Branch,
    Tag,
}

/// `HEAD -> main` and a detached `HEAD` mark the current commit; `tag: v1` is a tag.
fn chip(r: &str) -> (Chip, &str) {
    if let Some(branch) = r.strip_prefix("HEAD -> ") {
        (Chip::Head, branch)
    } else if r == "HEAD" {
        (Chip::Head, r)
    } else if let Some(tag) = r.strip_prefix("tag: ") {
        (Chip::Tag, tag)
    } else {
        (Chip::Branch, r)
    }
}

/// Lanes a row needs: its dot and every edge crossing it.
fn lanes(r: &LogRow) -> usize {
    let edges = r.top.iter().chain(&r.bottom).flat_map(|e| [e.from, e.to]);
    edges.fold(r.column, u32::max) as usize + 1
}

#[cfg(test)]
mod tests {
    use detached_core::types::Edge;

    use super::*;

    fn row(column: u32, top: &[(u32, u32)], bottom: &[(u32, u32)]) -> LogRow {
        let edges = |e: &[(u32, u32)]| {
            e.iter()
                .map(|&(from, to)| Edge { from, to, color: 0 })
                .collect()
        };
        LogRow {
            sha: String::new(),
            parents: vec![],
            author: String::new(),
            email: String::new(),
            time: 0,
            refs: vec![],
            subject: String::new(),
            column,
            color: 0,
            top: edges(top),
            bottom: edges(bottom),
        }
    }

    #[test]
    fn reads_ref_labels() {
        let chips = ["HEAD -> main", "HEAD", "tag: v1", "origin/main"].map(chip);
        assert_eq!(
            chips,
            [
                (Chip::Head, "main"),
                (Chip::Head, "HEAD"),
                (Chip::Tag, "v1"),
                (Chip::Branch, "origin/main"),
            ]
        );
    }

    #[test]
    fn counts_the_lanes_of_the_dot_and_the_edges_crossing_a_row() {
        assert_eq!(lanes(&row(0, &[], &[])), 1);
        assert_eq!(lanes(&row(1, &[(0, 0)], &[])), 2);
        assert_eq!(lanes(&row(0, &[(2, 0)], &[(0, 0), (0, 3)])), 4);
    }
}
