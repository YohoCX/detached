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
    Council {
        repo: String,
        task: String,
        round: String,
    },
    CouncilTask {
        repo: String,
        task: String,
    },
    CouncilStats {
        repo: String,
    },
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
    Log {
        repo: String,
    },
    Process {
        repo: String,
    },
    Tool {
        name: String,
        tab: String,
    },
    SessionOverview,
    NewTab,
    SessionArchive,
    PastChat {
        repo: String,
        provider: String,
        id: String,
        title: String,
    },
    /// A shell moved out of the Terminal app; after a restart, a new one in `cwd`.
    Terminal {
        id: u64,
        cwd: String,
    },
}

impl Doc {
    /// Every `kind`.
    pub const KINDS: [&str; 16] = [
        "file",
        "diff",
        "mr",
        "job",
        "chat",
        "log",
        "process",
        "tool",
        "session-overview",
        "new-tab",
        "session-archive",
        "past-chat",
        "terminal",
        "council",
        "council-task",
        "council-stats",
    ];

    pub fn repo(&self) -> Option<&str> {
        match self {
            Self::File(loc) => Some(&loc.repo),
            Self::Diff(doc) => Some(&doc.repo),
            Self::Terminal { cwd, .. } => Some(cwd),
            Self::Log { repo }
            | Self::Process { repo }
            | Self::Chat { repo, .. }
            | Self::Council { repo, .. }
            | Self::CouncilTask { repo, .. }
            | Self::CouncilStats { repo }
            | Self::Mr { repo, .. }
            | Self::Job { repo, .. }
            | Self::PastChat { repo, .. } => Some(repo),
            _ => None,
        }
    }

    /// The name the dock finds this document's behavior by.
    pub fn kind(&self) -> &'static str {
        match self {
            Doc::File(_) => "file",
            Doc::Council { .. } => "council",
            Doc::CouncilTask { .. } => "council-task",
            Doc::CouncilStats { .. } => "council-stats",
            Doc::Diff(_) => "diff",
            Doc::Mr { .. } => "mr",
            Doc::Job { .. } => "job",
            Doc::Chat { .. } => "chat",
            Doc::Log { .. } => "log",
            Doc::Process { .. } => "process",
            Doc::Tool { .. } => "tool",
            Doc::SessionOverview => "session-overview",
            Doc::NewTab => "new-tab",
            Doc::SessionArchive => "session-archive",
            Doc::PastChat { .. } => "past-chat",
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
