//! Skills from the folder list: the repo's stores, the repo's `.claude`, the user's `~/.claude` and
//! `~/.agents`, in that precedence.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::state::AppState;
use crate::store::{self, Kind, RepoStore, Scope};
use crate::{claude, proc, trust};

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
    /// Why it cannot be read: a special file inside, a store item that does not open or has a link
    /// out of its folder, a link inside one Windows would pass as a copy.
    pub error: Option<String>,
    /// The source of the earlier skill of the same name that loads instead.
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
        let get = proc::parent(&env);
        let home = claude::home(&get);
        let config: Config = core.get_settings().block(BLOCK).unwrap_or_default();
        let mut s = Skills::default();
        let mut folders = vec![];
        if let Some(repo) = repo {
            let store = RepoStore::open(core, repo.as_ref())?;
            for (source, scope, path) in [
                (
                    Source::Local,
                    Scope::Local,
                    store.local.join("local/skills"),
                ),
                (Source::Shared, Scope::Shared, store.shared.join("skills")),
            ] {
                s.folders.push(Folder {
                    source,
                    path: path.clone(),
                    on: true,
                });
                for (name, path) in entries(&path) {
                    match store.item(core, Kind::Skills, scope, &name) {
                        Ok(i) if path.join("SKILL.md").is_file() => {
                            s.add(source, name, path, Some(i.hash), i.trusted, None)
                        }
                        Ok(_) => {}
                        Err(e) => s.add(source, name, path, None, false, Some(e.to_string())),
                    }
                }
            }
            let project = trust::project(&env, repo.as_ref())?;
            let trusted = trust::trusted(&core.config_dir, repo, &project);
            let project = Path::new(repo).join(".claude");
            folders.push((Source::Project, Some(project), true, trusted));
        }
        folders.push((Source::User, claude::config_dir(&get, false), true, true));
        let agents = home.map(|h| h.join(".agents"));
        folders.push((Source::Agents, agents, config.agents, true));
        for (source, dir, on, trusted) in folders {
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
            for (name, path) in entries(&path) {
                if path.join("SKILL.md").is_file() {
                    let read = path.canonicalize().map_err(Error::from);
                    let error = read.and_then(|p| store::read(&p, true)).err();
                    let error = error.map(|e| e.to_string());
                    s.add(source, name, path, None, trusted, error);
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
        mut error: Option<String>,
    ) {
        let copied = !matches!(source, Source::Project | Source::User);
        if cfg!(windows) && copied && error.is_none() && linked(&path) {
            error = Some("A link inside: the plugin passes copies on Windows".into());
        }
        let by = self.all.iter().find(|s| s.name == name && s.loads());
        self.all.push(Skill {
            description: description(&path),
            overridden: by.map(|s| s.source),
            name,
            source,
            path,
            hash,
            trusted,
            error,
        });
    }

    /// The skills a session gets.
    pub fn active(&self) -> impl Iterator<Item = &Skill> {
        self.all.iter().filter(|s| s.loads())
    }
}

impl Skill {
    /// Trusted, readable and not overridden.
    pub fn loads(&self) -> bool {
        self.trusted && self.error.is_none() && self.overridden.is_none()
    }
}

/// The entries of `dir` by name.
fn entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut all: Vec<_> = (fs::read_dir(dir).into_iter().flatten().flatten())
        .filter(|e| e.file_name() != ".detached-trash")
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    all.sort();
    all
}

fn linked(dir: &Path) -> bool {
    let read = dir.canonicalize().map_err(Error::from);
    read.and_then(|p| store::read(&p, true))
        .is_ok_and(|(_, links)| !links.is_empty())
}

/// The SKILL.md front matter's `description`.
fn description(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let front = text.strip_prefix("---")?.split("\n---").next()?;
    let line = front.lines().find_map(|l| l.strip_prefix("description:"))?;
    Some(line.trim().trim_matches('"').to_string())
}

/// Copies the skill into `to`, its links as links; a store item that changed since it was accepted
/// is refused.
pub fn copy(skill: &Skill, to: &Path) -> Result<()> {
    let from = skill.path.canonicalize()?;
    let (files, links) = store::read(&from, skill.hash.is_none())?;
    if (skill.hash.as_ref()).is_some_and(|h| *h != store::hash(&files, &links)) {
        return Err(Error::Other(format!(
            "{} changed since it was accepted",
            skill.name
        )));
    }
    fs::create_dir_all(to.parent().unwrap())?;
    // Built aside and renamed in whole, so a failed copy leaves no partial skill behind.
    let tmp = tempfile::tempdir_in(to.parent().unwrap())?;
    for (rel, bytes) in files {
        let dest = tmp.path().join(rel);
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::write(dest, bytes)?;
    }
    for (rel, target) in links {
        // A store item's links are relative and inside it; any other relative link points back at
        // the original.
        let target = match skill.hash {
            Some(_) => target,
            None => from.join(&rel).parent().unwrap().join(target),
        };
        store::link(&target, &tmp.path().join(rel))?;
    }
    Ok(fs::rename(tmp.path(), to)?)
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
                ("y".into(), Source::Agents, true, None),
            ]
        );
        assert_eq!(
            active(&sb.state, &repo),
            [("x".into(), Source::Local), ("y".into(), Source::Agents)]
        );
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
            let copied = store::read(&sb.path(to).canonicalize().unwrap(), false);
            assert_eq!(copied.unwrap().0, files);
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

    #[cfg(unix)]
    #[test]
    fn links_load_as_the_cli_loads_them_and_a_store_skill_s_stay_inside_it() {
        use std::os::unix::fs::symlink;
        let (sb, repo) = setup();
        let store = RepoStore::open(&sb.state, repo.as_ref()).unwrap();
        put(&sb.path("LICENSE"), "license");
        let shared = store.shared.join("skills");
        for dir in ["abs", "empty", "in", "out"].map(|n| shared.join(n)) {
            put(&dir.join("SKILL.md"), "s");
        }
        symlink(shared.join("abs/SKILL.md"), shared.join("abs/more")).unwrap();
        fs::create_dir(shared.join("empty/data")).unwrap();
        symlink("data", shared.join("empty/alias")).unwrap();
        for dir in ["claude/skills/z", "home/.agents/skills/a"] {
            put(&sb.path(dir).join("SKILL.md"), "s");
        }
        for dir in ["home/.agents/skills/f", "home/.agents/skills/z"] {
            put(&sb.path(dir).join("SKILL.md"), "s");
        }
        symlink("SKILL.md", store.shared.join("skills/in/more")).unwrap();
        symlink(
            "../../../../LICENSE",
            store.shared.join("skills/out/LICENSE"),
        )
        .unwrap();
        symlink("../../../LICENSE", sb.path("claude/skills/z/LICENSE")).unwrap();
        symlink(
            "../../../../LICENSE",
            sb.path("home/.agents/skills/a/LICENSE"),
        )
        .unwrap();
        let pipe = std::ffi::CString::new(sb.path("home/.agents/skills/f/pipe").to_str().unwrap());
        // SAFETY: a NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(pipe.unwrap().as_ptr(), 0o600) }, 0);

        let s = Skills::load(&sb.state, Some(&repo)).unwrap();
        let errors: Vec<_> = (s.all.iter())
            .map(|s| (s.name.as_str(), s.error.as_deref().unwrap_or_default()))
            .collect();
        let out = "A link in an item must lead inside its folder by a relative path";
        let empty = "A link in an item cannot lead to an empty folder";
        let special = "Items cannot contain special files";
        let want = [
            ("abs", out),
            ("empty", empty),
            ("in", ""),
            ("out", out),
            ("z", ""),
        ];
        assert_eq!(
            errors,
            [&want[..], &[("a", ""), ("f", special), ("z", "")]].concat()
        );
        assert_eq!(
            active(&sb.state, &repo),
            [("z".into(), Source::User), ("a".into(), Source::Agents)]
        );
        let a = s.all.iter().find(|s| s.name == "a").unwrap();
        copy(a, &sb.path("plugin/a")).unwrap();
        assert_eq!(
            fs::read_to_string(sb.path("plugin/a/LICENSE")).unwrap(),
            "license"
        );
        put(&sb.path("plugin/taken/x"), "x");
        assert!(copy(a, &sb.path("plugin/taken")).is_err());
        let left = |dir: &str| entries(&sb.path(dir)).into_iter().map(|(n, _)| n);
        assert_eq!(left("plugin").collect::<Vec<_>>(), ["a", "taken"]);
        assert_eq!(left("plugin/taken").collect::<Vec<_>>(), ["x"]);

        let item = store
            .item(&sb.state, Kind::Skills, Scope::Shared, "in")
            .unwrap();
        store.accept(&sb.state, &item).unwrap();
        store.move_item(&sb.state, &item).unwrap();
        let s = Skills::load(&sb.state, Some(&repo)).unwrap();
        assert_eq!(names(&s)[0], ("in".into(), Source::Local, true, None));
        copy(&s.all[0], &sb.path("plugin/in")).unwrap();
        let link = fs::read_link(sb.path("plugin/in/more")).unwrap();
        assert_eq!(link, Path::new("SKILL.md"));
        let more = s.all[0].path.join("more");
        fs::remove_file(&more).unwrap();
        symlink("./SKILL.md", &more).unwrap();
        assert!(!Skills::load(&sb.state, Some(&repo)).unwrap().all[0].trusted);
    }
}
