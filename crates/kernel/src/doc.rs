//! The documents tabs show, as plain data.

use detached_core::types::{Project, Side};
use serde::{Deserialize, Serialize};

/// A file at `rev` (None is the working tree), or with `doc` set an MR page or job log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub repo: String,
    pub rev: Option<String>,
    pub path: String,
    pub line: Option<u32>,
    pub doc: Option<DocRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DocRef {
    Mr {
        repo: String,
        number: u64,
    },
    Job {
        repo: String,
        project: Project,
        id: u64,
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffDoc {
    pub repo: String,
    pub path: String,
    pub orig_path: Option<String>,
    pub left: Side,
    pub right: Side,
}

/// A document in a tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Doc {
    File(Location),
    Diff(DiffDoc),
    Mr {
        repo: String,
        number: u64,
    },
    Job {
        repo: String,
        project: Project,
        id: u64,
        name: String,
    },
    /// The `n`th chat open on `repo`.
    Chat {
        repo: String,
        #[serde(default)]
        n: u32,
    },
    /// A shell moved out of the Terminal app; after a restart, a new one in `cwd`.
    Terminal {
        id: u64,
        cwd: String,
    },
}

impl Doc {
    /// Every `kind`.
    pub const KINDS: [&str; 6] = ["file", "diff", "mr", "job", "chat", "terminal"];

    /// The name the dock finds this document's behavior by.
    pub fn kind(&self) -> &'static str {
        match self {
            Doc::File(_) => "file",
            Doc::Diff(_) => "diff",
            Doc::Mr { .. } => "mr",
            Doc::Job { .. } => "job",
            Doc::Chat { .. } => "chat",
            Doc::Terminal { .. } => "terminal",
        }
    }
}

impl From<Location> for Doc {
    fn from(mut loc: Location) -> Doc {
        match loc.doc.take() {
            Some(DocRef::Mr { repo, number }) => Doc::Mr { repo, number },
            Some(DocRef::Job {
                repo,
                project,
                id,
                name,
            }) => Doc::Job {
                repo,
                project,
                id,
                name,
            },
            None => Doc::File(loc),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_location_with_a_doc_shows_the_doc() {
        let mr = DocRef::Mr {
            repo: "/r".into(),
            number: 7,
        };
        let loc = Location {
            repo: "/r".into(),
            rev: None,
            path: String::new(),
            line: None,
            doc: Some(mr),
        };
        assert_eq!(
            Doc::from(loc),
            Doc::Mr {
                repo: "/r".into(),
                number: 7
            }
        );
    }
}
