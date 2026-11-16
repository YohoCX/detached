//! Modes and roles: a file each, TOML front matter between `+++` lines and instructions after it.
//! Built-in ones ship in the binary; a repo's or the owner's of the same name overrides them.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::{Files, Kind, RepoStore};

const BUILTIN: &[(&str, &str)] = &[
    ("review", include_str!("builtin/review.md")),
    ("gate", include_str!("builtin/gate.md")),
    ("bugs", include_str!("builtin/bugs.md")),
    ("architecture", include_str!("builtin/architecture.md")),
    ("docs", include_str!("builtin/docs.md")),
    ("adversarial", include_str!("builtin/adversarial.md")),
    ("verifier", include_str!("builtin/verifier.md")),
    ("merge", include_str!("builtin/merge.md")),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Find,
    Merge,
    Verify,
    Gate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seat {
    pub role: String,
    pub model: String,
    #[serde(default = "claude")]
    pub provider: String,
    #[serde(default = "high")]
    pub effort: String,
}

fn claude() -> String {
    "claude".into()
}

fn high() -> String {
    "high".into()
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct Head {
    graph: Vec<Stage>,
    #[serde(default)]
    seat: Vec<Seat>,
    merge: Option<String>,
    #[serde(default)]
    verifiers: Vec<String>,
    #[serde(default)]
    block: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    pub name: String,
    pub graph: Vec<Stage>,
    pub roster: Vec<Seat>,
    /// The merge participant's model.
    pub merge: Option<String>,
    /// Verifier models, in order of preference.
    pub verifiers: Vec<String>,
    /// The severities a verified finding blocks the gate with.
    pub block: Vec<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Role {
    pub name: String,
    pub lenses: Vec<String>,
    pub text: String,
}

#[derive(Deserialize)]
struct RoleHead {
    #[serde(default)]
    lenses: Vec<String>,
}

/// `text`'s front matter and the rest.
fn split(text: &str) -> Result<(&str, &str)> {
    let bad =
        || Error::Other("A mode or role file starts with front matter between +++ lines".into());
    let rest = text.strip_prefix("+++").ok_or_else(bad)?;
    let end = rest.find("\n+++\n").ok_or_else(bad)?;
    Ok((&rest[..end], rest[end + 5..].trim()))
}

pub fn parse_mode(name: &str, text: &str) -> Result<Mode> {
    let (head, text) = split(text)?;
    let h: Head = toml::from_str(head).map_err(|e| Error::Other(format!("{name}: {e}")))?;
    if h.graph.first() != Some(&Stage::Find) || h.seat.is_empty() {
        return Err(Error::Other(format!(
            "{name}: a mode starts with find and has seats"
        )));
    }
    Ok(Mode {
        name: name.into(),
        graph: h.graph,
        roster: h.seat,
        merge: h.merge,
        verifiers: h.verifiers,
        block: h.block,
        text: text.into(),
    })
}

pub fn parse_role(name: &str, text: &str) -> Result<Role> {
    let (head, text) = split(text)?;
    let h: RoleHead = toml::from_str(head).map_err(|e| Error::Other(format!("{name}: {e}")))?;
    Ok(Role {
        name: name.into(),
        lenses: h.lenses,
        text: text.into(),
    })
}

/// The file `name` of `kind` as the repo's store resolves it, the built-in one last; a repo item not
/// accepted yet is an error.
fn file(core: &AppState, repo: &Path, kind: Kind, name: &str) -> Result<String> {
    let builtin = BUILTIN.iter().find(|(n, _)| *n == name);
    let builtin: Option<Files> =
        builtin.map(|(_, t)| [(Default::default(), t.as_bytes().to_vec())].into());
    let store = RepoStore::open(core, repo)?;
    let files = store.load(core, kind, &format!("{name}.md"), builtin)?;
    let files = files.ok_or_else(|| {
        Error::Other(format!(
            "{name} waits for you to accept it on the Sharing page"
        ))
    })?;
    let bytes = files.into_values().next().unwrap_or_default();
    String::from_utf8(bytes).map_err(|_| Error::Other(format!("{name} is not UTF-8")))
}

pub fn mode(core: &AppState, repo: &Path, name: &str) -> Result<Mode> {
    parse_mode(name, &file(core, repo, Kind::Modes, name)?)
}

pub fn role(core: &AppState, repo: &Path, name: &str) -> Result<Role> {
    parse_role(name, &file(core, repo, Kind::Roles, name)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_modes_and_roles_parse() {
        for (name, text) in BUILTIN {
            match *name {
                "review" | "gate" => _ = parse_mode(name, text).unwrap(),
                _ => _ = parse_role(name, text).unwrap(),
            }
        }
        let gate = parse_mode("gate", BUILTIN[1].1).unwrap();
        assert_eq!(gate.graph.last(), Some(&Stage::Gate));
        assert_eq!(gate.roster.len(), 2);
        assert_eq!(gate.block, ["critical", "major"]);
        assert!(parse_mode("x", "no front matter").is_err());
    }

    #[test]
    fn a_repo_mode_of_the_same_name_overrides_once_accepted() {
        let sb = crate::git::testutil::Sandbox::new();
        let repo = sb.path("r");
        std::fs::create_dir(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "a", "a", "init");
        let text =
            "+++\ngraph = [\"find\"]\n[[seat]]\nrole = \"bugs\"\nmodel = \"haiku\"\n+++\nMine.";
        let store = RepoStore::open(&sb.state, &repo).unwrap();
        let files: Files = [(Default::default(), text.as_bytes().to_vec())].into();
        store
            .write(&sb.state, Kind::Modes, "review.md", &files)
            .unwrap();
        let m = mode(&sb.state, &repo, "review").unwrap();
        assert_eq!(
            (m.roster[0].model.as_str(), m.text.as_str()),
            ("haiku", "Mine.")
        );
        assert_eq!(
            role(&sb.state, &repo, "bugs").unwrap().lenses,
            ["bugs", "impl"]
        );
    }
}
