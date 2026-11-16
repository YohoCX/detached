//! Skills from the folder list: the repo's stores, the repo's `.claude`, the user's `~/.claude` and
//! `~/.agents`, in that precedence.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::{self, Kind, RepoStore, Scope};
use crate::{proc, trust};

pub const BLOCK: &str = "skills";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Whether `~/.agents/skills` is read.
    pub agents: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { agents: true }
    }
}

/// Where a skill comes from, by precedence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Local,
    Shared,
    Project,
    User,
    Agents,
}

impl Source {
    pub fn title(self) -> &'static str {
        match self {
            Source::Local => "local",
            Source::Shared => "shared",
            Source::Project => "the repo's .claude",
            Source::User => "~/.claude",
            Source::Agents => "~/.agents",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub source: Source,
    pub path: PathBuf,
    pub on: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skill {
    pub name: String,
    pub source: Source,
    /// Its folder.
    pub path: PathBuf,
    pub description: Option<String>,
    /// A store item's accepted hash, which its copy must still have.
    pub hash: Option<String>,
    pub trusted: bool,
    /// The source of the skill of the same name that loads instead.
    pub overridden: Option<Source>,
}

#[derive(Clone, Debug, Default)]
pub struct Skills {
    pub folders: Vec<Folder>,
    pub all: Vec<Skill>,
}

impl Skills {
    /// The folder list and every skill in it; `repo` adds the repo's folders.
    pub fn load(core: &AppState, repo: Option<&str>) -> Result<Skills> {
        let env = core.env();
        let var = |k: &str| {
            proc::parent(&env)(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).or_else(dirs::home_dir);
        let claude = var("CLAUDE_CONFIG_DIR").or_else(|| Some(home.clone()?.join(".claude")));
        let config: Config = core.get_settings().block(BLOCK).unwrap_or_default();
        let mut s = Skills::default();
        let mut personal = vec![];
        if let Some(repo) = repo {
            let store = RepoStore::open(core, repo.as_ref())?;
            for (source, path) in [
                (Source::Local, store.local.join("local/skills")),
                (Source::Shared, store.shared.join("skills")),
            ] {
                let on = true;
                s.folders.push(Folder { source, path, on });
            }
            for item in store.list(core, Kind::Skills)? {
                let source = match item.scope {
                    Scope::Local => Source::Local,
                    _ => Source::Shared,
                };
                if item.path.join("SKILL.md").is_file() {
                    s.add(source, item.name, item.path, Some(item.hash), item.trusted);
                }
            }
            let project = trust::project(&env, repo.as_ref())?;
            let trusted = trust::trusted(&core.config_dir, repo, &project);
            let project_dir = Path::new(repo).join(".claude");
            personal.push((Source::Project, Some(project_dir), true, trusted));
        }
        personal.push((Source::User, claude, true, true));
        let agents = home.map(|h| h.join(".agents"));
        personal.push((Source::Agents, agents, config.agents, true));
        for (source, dir, on, trusted) in personal {
            let Some(path) = dir.map(|d| d.join("skills")) else {
                continue;
            };
            s.folders.push(Folder {
                source,
                path: path.clone(),
                on,
            });
            if !on {
                continue;
            }
            let mut names: Vec<_> = fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .collect();
            names.sort_by_key(|e| e.file_name());
            for e in names {
                let (name, path) = (e.file_name().to_string_lossy().into_owned(), e.path());
                if path.join("SKILL.md").is_file() {
                    s.add(source, name, path, None, trusted);
                }
            }
        }
        Ok(s)
    }

    fn add(
        &mut self,
        source: Source,
        name: String,
        path: PathBuf,
        hash: Option<String>,
        trusted: bool,
    ) {
        let overridden = self.all.iter().find(|s| s.name == name).map(|s| s.source);
        self.all.push(Skill {
            description: description(&path),
            name,
            source,
            path,
            hash,
            trusted,
            overridden,
        });
    }

    /// The skills a session gets: trusted, and not overridden.
    pub fn active(&self) -> impl Iterator<Item = &Skill> {
        self.all
            .iter()
            .filter(|s| s.trusted && s.overridden.is_none())
    }
}

/// The SKILL.md front matter's `description`.
fn description(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let front = text.strip_prefix("---")?.split("\n---").next()?;
    let line = front.lines().find_map(|l| l.strip_prefix("description:"))?;
    Some(line.trim().trim_matches('"').to_string())
}

/// Copies the skill's files into `to`; a store item that changed since it was accepted is refused.
pub fn copy(skill: &Skill, to: &Path) -> Result<()> {
    let files = store::read(&skill.path.canonicalize()?)?;
    if skill
        .hash
        .as_ref()
        .is_some_and(|h| *h != store::hash(&files))
    {
        return Err(Error::Other(format!(
            "{} changed since it was accepted",
            skill.name
        )));
    }
    for (rel, bytes) in files {
        let dest = to.join(rel);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(dest, bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use crate::store::Files;

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn skill(text: &str) -> Files {
        [("SKILL.md".into(), text.as_bytes().to_vec())].into()
    }

    /// A sandbox with a repo, its Claude config dir its own.
    fn setup() -> (Sandbox, String) {
        let mut sb = Sandbox::new();
        let claude = sb.path("claude").display().to_string();
        let env = sb.state.env.get_mut().unwrap();
        env.vars.push(("CLAUDE_CONFIG_DIR".into(), claude));
        let repo = sb.clone_pair("repo").display().to_string();
        (sb, repo)
    }

    fn names(s: &Skills) -> Vec<(String, Source, bool, Option<Source>)> {
        let all = s.all.iter();
        all.map(|s| (s.name.clone(), s.source, s.trusted, s.overridden))
            .collect()
    }

    fn active(core: &AppState, repo: &str) -> Vec<(String, Source)> {
        let s = Skills::load(core, Some(repo)).unwrap();
        s.active().map(|s| (s.name.clone(), s.source)).collect()
    }

    #[test]
    fn agents_skills_are_read_when_present_unless_turned_off() {
        let (sb, repo) = setup();
        assert_eq!(active(&sb.state, &repo), []);
        put(
            &sb.path("home/.agents/skills/lint/SKILL.md"),
            "---\nname: lint\ndescription: \"Lints it\"\n---\nLint.",
        );
        put(&sb.path("home/.agents/skills/notes/README.md"), "no skill");
        let s = Skills::load(&sb.state, None).unwrap();
        assert_eq!(names(&s), [("lint".into(), Source::Agents, true, None)]);
        assert_eq!(s.all[0].description.as_deref(), Some("Lints it"));
        sb.state.set_block(BLOCK, Config { agents: false }).unwrap();
        let s = Skills::load(&sb.state, Some(&repo)).unwrap();
        assert!(s.all.is_empty());
        let agents = s
            .folders
            .iter()
            .find(|f| f.source == Source::Agents)
            .unwrap();
        assert_eq!(
            (&agents.path, agents.on),
            (&sb.path("home/.agents/skills"), false)
        );
    }

    #[test]
    fn the_repo_s_skills_override_the_user_s_and_its_claude_folder_loads_once_trusted() {
        let (sb, repo) = setup();
        let store = RepoStore::open(&sb.state, repo.as_ref()).unwrap();
        store
            .write(&sb.state, Kind::Skills, "x", &skill("local"))
            .unwrap();
        for dir in [
            "claude/skills/x",
            "home/.agents/skills/x",
            "home/.agents/skills/y",
        ] {
            put(&sb.path(dir).join("SKILL.md"), "s");
        }
        let grants = "---\nallowed-tools: Bash\n---\n";
        put(&sb.path("repo/.claude/skills/y/SKILL.md"), grants);
        let s = Skills::load(&sb.state, Some(&repo)).unwrap();
        assert_eq!(
            names(&s),
            [
                ("x".into(), Source::Local, true, None),
                ("y".into(), Source::Project, false, None),
                ("x".into(), Source::User, true, Some(Source::Local)),
                ("x".into(), Source::Agents, true, Some(Source::Local)),
                ("y".into(), Source::Agents, true, Some(Source::Project)),
            ]
        );
        assert_eq!(active(&sb.state, &repo), [("x".into(), Source::Local)]);
        let project = trust::project(&sb.state.env(), repo.as_ref()).unwrap();
        trust::trust(&sb.state.config_dir, &repo, &project).unwrap();
        assert_eq!(
            active(&sb.state, &repo),
            [("x".into(), Source::Local), ("y".into(), Source::Project)]
        );
    }

    #[test]
    fn share_then_make_local_round_trips_a_skill_and_its_copy() {
        let (sb, repo) = setup();
        let store = RepoStore::open(&sb.state, repo.as_ref()).unwrap();
        let files: Files = [
            ("SKILL.md".into(), b"check".to_vec()),
            ("scripts/run".into(), b"echo".to_vec()),
        ]
        .into();
        store
            .write(&sb.state, Kind::Skills, "check", &files)
            .unwrap();
        let moved = |source, to: &str| {
            let s = Skills::load(&sb.state, Some(&repo)).unwrap();
            assert_eq!(names(&s), [("check".into(), source, true, None)]);
            copy(&s.all[0], &sb.path(to)).unwrap();
            let copied = store::read(&sb.path(to).canonicalize().unwrap());
            assert_eq!(copied.unwrap(), files);
            let item = store.list(&sb.state, Kind::Skills).unwrap().remove(0);
            store.move_item(&sb.state, &item).unwrap();
        };
        moved(Source::Local, "a");
        sb.git(&store.repo, &["add", ".detached"]);
        sb.git(&store.repo, &["commit", "-qm", "share"]);
        moved(Source::Shared, "b");
        let s = Skills::load(&sb.state, Some(&repo)).unwrap();
        assert_eq!(names(&s), [("check".into(), Source::Local, true, None)]);
    }

    #[test]
    fn a_foreign_local_skill_asks_once_and_a_changed_shared_one_is_not_copied() {
        let (sb, repo) = setup();
        let store = RepoStore::open(&sb.state, repo.as_ref()).unwrap();
        let local = store
            .path(&sb.state, Kind::Skills, Scope::Local, "foreign")
            .unwrap();
        put(&local.join("SKILL.md"), "foreign");
        put(&store.shared.join("skills/team/SKILL.md"), "team");
        assert_eq!(active(&sb.state, &repo), []);
        for item in store.list(&sb.state, Kind::Skills).unwrap() {
            store.accept(&sb.state, &item).unwrap();
        }
        let s = Skills::load(&sb.state, Some(&repo)).unwrap();
        assert_eq!(
            names(&s),
            [
                ("foreign".into(), Source::Local, true, None),
                ("team".into(), Source::Shared, true, None),
            ]
        );
        put(&store.shared.join("skills/team/SKILL.md"), "changed");
        assert!(copy(&s.all[1], &sb.path("copy")).is_err());
        assert_eq!(
            active(&sb.state, &repo),
            [("foreign".into(), Source::Local)]
        );
    }
}
