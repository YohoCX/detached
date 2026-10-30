//! Lane assignment for the log graph, one row at a time in `--date-order`.

use crate::types::{Edge, LogRow};

struct Lane {
    /// The commit this lane runs down to.
    sha: String,
    color: u32,
}

#[derive(Default)]
pub struct Graph {
    lanes: Vec<Option<Lane>>,
    colors: u32,
}

fn edge(from: usize, to: usize, color: u32) -> Edge {
    Edge {
        from: from as u32,
        to: to as u32,
        color,
    }
}

impl Graph {
    /// Sets `row`'s dot and edges, then points the lanes at its parents.
    pub fn place(&mut self, row: &mut LogRow) {
        let waits_for = |l: &Option<Lane>, sha: &str| l.as_ref().is_some_and(|l| l.sha == sha);
        let col = match self.lanes.iter().position(|l| waits_for(l, &row.sha)) {
            Some(i) => i,
            None => self.free(),
        };
        let color = match &self.lanes[col] {
            Some(l) => l.color,
            None => self.color(),
        };
        let mut passing = vec![];
        row.top = vec![];
        for (i, l) in self.lanes.iter_mut().enumerate() {
            let Some(lane) = l else { continue };
            if lane.sha == row.sha {
                row.top.push(edge(i, col, lane.color));
                *l = None;
            } else {
                row.top.push(edge(i, i, lane.color));
                passing.push(edge(i, i, lane.color));
            }
        }
        let mut targets = vec![];
        for (n, p) in row.parents.iter().enumerate() {
            let j = match self.lanes.iter().position(|l| waits_for(l, p)) {
                Some(j) if n > 0 => j,
                _ => {
                    let (j, color) = if n == 0 {
                        (col, color)
                    } else {
                        (self.free(), self.color())
                    };
                    self.lanes[j] = Some(Lane {
                        sha: p.clone(),
                        color,
                    });
                    j
                }
            };
            targets.push(edge(col, j, self.lanes[j].as_ref().unwrap().color));
        }
        row.bottom = [passing, targets].concat();
        (row.column, row.color) = (col as u32, color);
        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
    }

    /// The leftmost empty column.
    fn free(&mut self) -> usize {
        self.lanes
            .iter()
            .position(Option::is_none)
            .unwrap_or_else(|| {
                self.lanes.push(None);
                self.lanes.len() - 1
            })
    }

    fn color(&mut self) -> u32 {
        self.colors += 1;
        self.colors - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Places `(sha, parents)` rows and renders each as `column@color | top | bottom`, edges `from>to@color`.
    fn graph(rows: &[(&str, &[&str])]) -> Vec<String> {
        let mut g = Graph::default();
        let edges = |e: &[Edge]| {
            e.iter()
                .map(|e| format!("{}>{}@{}", e.from, e.to, e.color))
                .collect::<Vec<_>>()
                .join(" ")
        };
        rows.iter()
            .map(|(sha, parents)| {
                let mut r = row(sha, parents);
                g.place(&mut r);
                format!(
                    "{}@{} | {} | {}",
                    r.column,
                    r.color,
                    edges(&r.top),
                    edges(&r.bottom)
                )
            })
            .collect()
    }

    fn row(sha: &str, parents: &[&str]) -> LogRow {
        LogRow {
            sha: sha.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: String::new(),
            email: String::new(),
            time: 0,
            refs: vec![],
            subject: String::new(),
            column: 0,
            color: 0,
            top: vec![],
            bottom: vec![],
        }
    }

    #[test]
    fn linear() {
        assert_eq!(
            graph(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]),
            ["0@0 |  | 0>0@0", "0@0 | 0>0@0 | 0>0@0", "0@0 | 0>0@0 | ",]
        );
    }

    #[test]
    fn branch_and_merge() {
        // d merges c into b; both fork from a.
        assert_eq!(
            graph(&[("d", &["b", "c"]), ("c", &["a"]), ("b", &["a"]), ("a", &[])]),
            [
                "0@0 |  | 0>0@0 0>1@1",
                "1@1 | 0>0@0 1>1@1 | 0>0@0 1>1@1",
                "0@0 | 0>0@0 1>1@1 | 1>1@1 0>0@0",
                "0@0 | 0>0@0 1>0@1 | ",
            ]
        );
    }

    #[test]
    fn two_tips_fork_from_one_commit() {
        assert_eq!(
            graph(&[("y", &["a"]), ("x", &["a"]), ("a", &[])]),
            [
                "0@0 |  | 0>0@0",
                "1@1 | 0>0@0 | 0>0@0 1>1@1",
                "0@0 | 0>0@0 1>0@1 | ",
            ]
        );
    }

    #[test]
    fn octopus() {
        assert_eq!(
            graph(&[
                ("m", &["a", "b", "c"]),
                ("c", &["r"]),
                ("b", &["r"]),
                ("a", &["r"]),
                ("r", &[])
            ]),
            [
                "0@0 |  | 0>0@0 0>1@1 0>2@2",
                "2@2 | 0>0@0 1>1@1 2>2@2 | 0>0@0 1>1@1 2>2@2",
                "1@1 | 0>0@0 1>1@1 2>2@2 | 0>0@0 2>2@2 1>1@1",
                "0@0 | 0>0@0 1>1@1 2>2@2 | 1>1@1 2>2@2 0>0@0",
                "0@0 | 0>0@0 1>0@1 2>0@2 | ",
            ]
        );
    }

    #[test]
    fn criss_cross() {
        // m1 and m2 each merge x and y; their second parents reuse the lanes already waiting.
        assert_eq!(
            graph(&[
                ("t", &["m1", "m2"]),
                ("m2", &["y", "x"]),
                ("m1", &["x", "y"]),
                ("y", &["r"]),
                ("x", &["r"]),
                ("r", &[])
            ]),
            [
                "0@0 |  | 0>0@0 0>1@1",
                "1@1 | 0>0@0 1>1@1 | 0>0@0 1>1@1 1>2@2",
                "0@0 | 0>0@0 1>1@1 2>2@2 | 1>1@1 2>2@2 0>0@0 0>1@1",
                "1@1 | 0>0@0 1>1@1 2>2@2 | 0>0@0 2>2@2 1>1@1",
                "0@0 | 0>0@0 1>1@1 2>0@2 | 1>1@1 0>0@0",
                "0@0 | 0>0@0 1>0@1 | ",
            ]
        );
    }

    #[test]
    fn freed_columns_are_reused_and_colors_stay() {
        // b's lane ends at a; the later tip z takes column 1 again with a new color.
        assert_eq!(
            graph(&[
                ("c", &["a", "b"]),
                ("b", &["a"]),
                ("a", &["s"]),
                ("z", &["s"]),
                ("s", &[])
            ]),
            [
                "0@0 |  | 0>0@0 0>1@1",
                "1@1 | 0>0@0 1>1@1 | 0>0@0 1>1@1",
                "0@0 | 0>0@0 1>0@1 | 0>0@0",
                "1@2 | 0>0@0 | 0>0@0 1>1@2",
                "0@0 | 0>0@0 1>0@2 | ",
            ]
        );
    }

    #[test]
    fn a_missing_parent_runs_to_the_last_row() {
        // `gone` never arrives (truncation, a shallow clone), so its lane passes every later row.
        assert_eq!(
            graph(&[("m", &["b", "gone"]), ("b", &["a"]), ("a", &[])]),
            [
                "0@0 |  | 0>0@0 0>1@1",
                "0@0 | 0>0@0 1>1@1 | 1>1@1 0>0@0",
                "0@0 | 0>0@0 1>1@1 | 1>1@1",
            ]
        );
    }
}
