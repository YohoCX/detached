//! Agent adapters from the signed catalog `adapters.json`, installed under
//! `data/adapters/<name>/<version>/` with the version before kept for rollback, and the check of the
//! user's own CLI they run.

use std::collections::{BTreeMap, HashMap};
use std::env::consts::EXE_SUFFIX;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Days, NaiveDate};
pub use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backend;
use crate::env::{Env, which};
use crate::error::{Error, Result};
use crate::state::Cancel;
use crate::update::{self, Progress};

#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

/// The asset of the rolling `adapters` release that adapters.yml publishes.
pub const CATALOG_URL: &str =
    "https://github.com/YohoCX/detached/releases/download/adapters/adapters.json";
/// The catalog key's public half, which `make adapters-sign` prints when it makes the key; not the
/// update key.
pub const CATALOG_KEY: &str = "";
/// Days after its `published` date before a version installs on its own.
pub const COOL_DOWN: u64 = 3;

/// Settings' block for the agents.
pub const BLOCK: &str = "agents";

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Config {
    /// A CLI's path by provider id, in place of the one on PATH.
    #[serde(default)]
    pub cli: BTreeMap<String, String>,
    /// Codex is listed, and its adapter installs, only once this is on.
    #[serde(default)]
    pub codex: bool,
}

impl Config {
    pub fn shows(&self, provider: &Provider) -> bool {
        provider.id != CODEX.id || self.codex
    }
}

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub backend: backend::Name,
    pub adapter: &'static str,
    pub cli: &'static str,
    /// A floor; the catalog entry in use may raise it.
    pub min_cli: Version,
    /// Run in a terminal tab, the CLI's name first.
    pub login: &'static [&'static str],
    /// The vendor's installer as typed in a shell: Unix, then Windows.
    pub installer: Option<[&'static str; 2]>,
    /// Names the CLI to the adapter.
    pub cli_var: &'static str,
    /// Passed from the app's env when set: the login, cloud and proxy variables the provider
    /// documents.
    pub env: &'static [&'static str],
    /// The files a session loads only once their items are trusted, as globs from a tree's root.
    pub trust: &'static [&'static str],
}

pub static CLAUDE: Provider = claude(backend::Name::Direct);

/// Claude's manifest on `backend`; the adapter's fields serve the ACP one until it goes.
pub(crate) const fn claude(backend: backend::Name) -> Provider {
    Provider {
        id: "claude",
        name: "Claude Agent",
        backend,
        adapter: "claude-agent-acp",
        cli: "claude",
        min_cli: Version::new(2, 1, 322),
        login: &["claude", "/login"],
        installer: Some([
            "curl -fsSL https://claude.ai/install.sh | bash",
            "irm https://claude.ai/install.ps1 | iex",
        ]),
        cli_var: "CLAUDE_CODE_EXECUTABLE",
        env: &[
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_TMPDIR",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "AWS_PROFILE",
            "AWS_REGION",
            "CLOUD_ML_REGION",
            "ANTHROPIC_VERTEX_PROJECT_ID",
            "CLAUDE_CODE_USE_FOUNDRY",
            "ANTHROPIC_FOUNDRY_RESOURCE",
            "ANTHROPIC_FOUNDRY_BASE_URL",
            "ANTHROPIC_FOUNDRY_API_KEY",
            "ANTHROPIC_FOUNDRY_AUTH_TOKEN",
            "HTTPS_PROXY",
            "NO_PROXY",
            "NODE_EXTRA_CA_CERTS",
        ],
        trust: &[
            ".claude/settings.json",
            ".claude/settings.local.json",
            ".claude/skills/*/SKILL.md",
            ".claude/commands/**",
            ".claude/agents/**",
            ".mcp.json",
        ],
    }
}

pub static CODEX: Provider = Provider {
    id: "codex",
    name: "Codex",
    backend: backend::Name::Acp,
    adapter: "codex-acp",
    cli: "codex",
    min_cli: Version::new(0, 166, 0),
    login: &["codex", "login"],
    installer: None,
    cli_var: "CODEX_PATH",
    env: &["CODEX_HOME"],
    trust: &[".codex/config.toml", ".agents/skills/*/SKILL.md"],
};

pub static PROVIDERS: [&Provider; 2] = [&CLAUDE, &CODEX];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Catalog {
    /// Rises with every signing; an older catalog is refused.
    pub serial: u64,
    pub adapters: Vec<Entry>,
    /// The CLIs the host drives without an adapter, Claude's.
    #[serde(default)]
    pub cli: Vec<CliRule>,
}

/// A directly driven CLI's floor and its blocked versions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CliRule {
    pub name: String,
    #[serde(default)]
    pub min_cli: Option<Version>,
    #[serde(default)]
    pub blocked_cli: Vec<Version>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Entry {
    pub name: String,
    pub version: Version,
    pub published: NaiveDate,
    pub min_app: Version,
    #[serde(default)]
    pub max_app: Option<Version>,
    #[serde(default)]
    pub min_cli: Option<Version>,
    #[serde(default)]
    pub blocked_cli: Vec<Version>,
    /// By `target()`.
    pub artifacts: HashMap<String, Artifact>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Artifact {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

impl Catalog {
    /// `name`'s entries this app and platform can run, newest first.
    pub fn entries(&self, name: &str, app: &Version) -> Vec<&Entry> {
        let target = target();
        let mut entries: Vec<_> = (self.adapters.iter())
            .filter(|e| e.name == name && e.min_app <= *app)
            .filter(|e| e.max_app.as_ref().is_none_or(|max| app <= max))
            .filter(|e| e.artifacts.contains_key(&target))
            .collect();
        entries.sort_by(|a, b| b.version.cmp(&a.version));
        entries
    }

    pub fn entry(&self, name: &str, version: &Version) -> Option<&Entry> {
        let mut all = self.adapters.iter();
        all.find(|e| e.name == name && e.version == *version)
    }

    /// What a check installs on its own for an installed adapter: the newest entry out of its
    /// cool-down, when it is newer than both kept versions, so a rollback is not undone.
    pub fn auto(
        &self,
        name: &str,
        app: &Version,
        kept: &Installed,
        today: NaiveDate,
    ) -> Option<&Entry> {
        kept.current.as_ref()?;
        let settled = (self.entries(name, app).into_iter())
            .find(|e| e.published + Days::new(COOL_DOWN) <= today)?;
        let newer = |v: &Option<Version>| v.as_ref().is_none_or(|v| settled.version > *v);
        (newer(&kept.current) && newer(&kept.previous)).then_some(settled)
    }
}

/// This build's version, which an entry's `min_app` and `max_app` bound.
pub fn app_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate's version is semver")
}

pub fn today() -> NaiveDate {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    DateTime::from_timestamp(secs.as_secs() as i64, 0)
        .unwrap_or_default()
        .date_naive()
}

/// The key of this machine's build in an entry's artifacts: `macos-aarch64`, `windows-x86_64`,
/// `linux-x86_64-gnu`.
pub fn target() -> String {
    let platform = update::platform();
    match cfg!(target_os = "linux") {
        true if cfg!(target_env = "musl") => format!("{platform}-musl"),
        true => format!("{platform}-gnu"),
        false => platform,
    }
}

/// The catalog at `url` once its signature by `key` holds and its serial is not below the last one
/// accepted, which it then replaces in `data`.
pub fn fetch(url: &str, key: &str, data: &Path) -> Result<Catalog> {
    if key.is_empty() {
        return Err(Error::Other(
            "This build has no adapters catalog key".into(),
        ));
    }
    let json = update::get(url)?;
    let signature = update::get(&format!("{url}.minisig"))?;
    update::verify(&json, &String::from_utf8_lossy(&signature), key)?;
    let catalog: Catalog = serde_json::from_slice(&json)?;
    if let Some(last) = stored(data).filter(|c| c.serial > catalog.serial) {
        return Err(Error::Other(format!(
            "The adapters catalog's serial {} is older than {}",
            catalog.serial, last.serial
        )));
    }
    let dir = data.join("adapters");
    fs::create_dir_all(&dir)?;
    crate::store::atomic(&dir.join("catalog.json"), &json)?;
    Ok(catalog)
}

/// The last catalog `fetch` accepted.
pub fn stored(data: &Path) -> Option<Catalog> {
    let json = fs::read(data.join("adapters/catalog.json")).ok()?;
    serde_json::from_slice(&json).ok()
}

/// The version in use and the one kept for rollback.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Installed {
    pub current: Option<Version>,
    pub previous: Option<Version>,
}

fn folder(data: &Path, name: &str) -> PathBuf {
    data.join("adapters").join(name)
}

pub fn installed(data: &Path, name: &str) -> Installed {
    let json = fs::read(folder(data, name).join("installed.json"));
    json.ok()
        .and_then(|j| serde_json::from_slice(&j).ok())
        .unwrap_or_default()
}

/// The adapter's executable in its version's folder.
pub fn exe(data: &Path, name: &str, version: &Version) -> PathBuf {
    let dir = folder(data, name).join(version.to_string());
    dir.join(format!("{name}{EXE_SUFFIX}"))
}

/// Installs `entry` as the current version, the one before kept, older ones removed. The archive is
/// checked against the entry's size and sha256 before it is unpacked, and the folder goes in by a
/// rename; a failure leaves nothing.
pub fn install(
    data: &Path,
    entry: &Entry,
    cancel: &Cancel,
    progress: &dyn Fn(Progress),
) -> Result<Installed> {
    let name = &entry.name;
    if !PROVIDERS.iter().any(|p| p.adapter == name) {
        return Err(Error::Other(format!("{name} is not an agent adapter")));
    }
    let artifact = entry.artifacts.get(&target()).ok_or_else(|| {
        Error::Other(format!(
            "{name} {} has no build for {}",
            entry.version,
            target()
        ))
    })?;
    let dir = folder(data, name);
    fs::create_dir_all(&dir)?;
    let to = dir.join(entry.version.to_string());
    if !to.is_dir() {
        let mut file = tempfile::tempfile_in(&dir)?;
        download(artifact, &mut file, cancel, progress)?;
        file.rewind()?;
        let unpacked = tempfile::Builder::new()
            .prefix(".install")
            .tempdir_in(&dir)?;
        unpack(file, unpacked.path())?;
        let exe = format!("{name}{EXE_SUFFIX}");
        if !unpacked.path().join(&exe).is_file() {
            return Err(Error::Other(format!("The archive has no {exe}")));
        }
        fs::rename(unpacked.path(), &to)?;
    }
    let mut kept = installed(data, name);
    if kept.current.as_ref() != Some(&entry.version) {
        kept.previous = kept.current.replace(entry.version.clone());
    }
    save(&dir, &kept)?;
    prune(&dir, &kept);
    Ok(kept)
}

/// Makes the kept version current again, and the current one the kept.
pub fn rollback(data: &Path, name: &str) -> Result<Installed> {
    let mut kept = installed(data, name);
    let Some(previous) = kept.previous.take() else {
        return Err(Error::Other(format!(
            "{name} has no version to roll back to"
        )));
    };
    kept.previous = kept.current.replace(previous);
    save(&folder(data, name), &kept)?;
    Ok(kept)
}

fn save(dir: &Path, kept: &Installed) -> Result<()> {
    crate::store::atomic(&dir.join("installed.json"), &serde_json::to_vec(kept)?)
}

/// Removes all but the kept versions, a quit's leftovers too; on Windows a version a session still
/// runs stays until the next install.
fn prune(dir: &Path, kept: &Installed) {
    let keep: Vec<_> = [&kept.current, &kept.previous]
        .into_iter()
        .flatten()
        .map(|v| v.to_string())
        .collect();
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name != "installed.json" && !keep.contains(&name) {
            let _ = fs::remove_dir_all(e.path()).or_else(|_| fs::remove_file(e.path()));
        }
    }
}

fn download(
    artifact: &Artifact,
    file: &mut File,
    cancel: &Cancel,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    let url = &artifact.url;
    let mut response = update::call(url)?;
    let mut body = response.body_mut().as_reader();
    let (mut hash, mut received, mut chunk) = (Sha256::new(), 0, vec![0; 1 << 16]);
    let wrong = |what: &str| Err(Error::Other(format!("{url}: wrong {what}")));
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let n = body.read(&mut chunk).map_err(|e| update::fail(url, &e))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > artifact.size {
            return wrong("size");
        }
        hash.update(&chunk[..n]);
        file.write_all(&chunk[..n])?;
        progress(Progress::Received(received, Some(artifact.size)));
    }
    progress(Progress::Verifying);
    if received != artifact.size {
        return wrong("size");
    }
    if format!("{:x}", hash.finalize()) != artifact.sha256.to_ascii_lowercase() {
        return wrong("sha256");
    }
    Ok(())
}

/// Unpacks a `.tar.gz` into `to`, refusing any entry that is not a plain file or folder inside it,
/// and any provider's CLI, which Detached never ships.
fn unpack(file: File, to: &Path) -> Result<()> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let kind = entry.header().entry_type();
        let inside =
            (path.components()).all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        if !inside || !(kind.is_file() || kind.is_dir()) {
            return Err(Error::Other(format!(
                "The archive's {} leaves its folder",
                path.display()
            )));
        }
        let file = path.file_name().unwrap_or_default();
        if kind.is_file() && PROVIDERS.iter().any(|p| *file == *cli_file(p)) {
            return Err(Error::Other(format!(
                "The archive ships {}",
                path.display()
            )));
        }
        entry.unpack_in(to)?;
    }
    Ok(())
}

fn cli_file(p: &Provider) -> String {
    format!("{}{EXE_SUFFIX}", p.cli)
}

/// What the user's CLI's version allows.
#[derive(Clone, Debug, PartialEq)]
pub enum Cli {
    Ok,
    /// Below this version: the provider's floor or the entry's `min_cli`.
    Old(Version),
    /// In the entry's `blocked_cli`: a chat warns, an isolated participant refuses.
    Blocked,
}

/// Checks the CLI's `version` against `entry`, the catalog's entry of the adapter in use.
pub fn check_cli(provider: &Provider, entry: Option<&Entry>, version: &Version) -> Cli {
    let blocked = entry.map_or(&[][..], |e| &e.blocked_cli);
    check(
        provider,
        entry.and_then(|e| e.min_cli.as_ref()),
        blocked,
        version,
    )
}

/// Checks a CLI the host drives itself against the catalog's `cli` list.
pub fn check_own_cli(provider: &Provider, catalog: Option<&Catalog>, version: &Version) -> Cli {
    let rule = catalog.and_then(|c| c.cli.iter().find(|r| r.name == provider.cli));
    let blocked = rule.map_or(&[][..], |r| &r.blocked_cli);
    check(
        provider,
        rule.and_then(|r| r.min_cli.as_ref()),
        blocked,
        version,
    )
}

fn check(provider: &Provider, raised: Option<&Version>, blocked: &[Version], v: &Version) -> Cli {
    let need = raised.filter(|r| **r > provider.min_cli);
    let need = need.unwrap_or(&provider.min_cli).clone();
    if *v < need {
        Cli::Old(need)
    } else if blocked.contains(v) {
        Cli::Blocked
    } else {
        Cli::Ok
    }
}

/// The provider's CLI: the path the settings name, else the first on PATH, else where its
/// installer puts it.
pub fn find_cli(provider: &Provider, setting: Option<&str>, env: &Env) -> Option<PathBuf> {
    if let Some(path) = setting.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(path)).filter(|p| p.is_file());
    }
    let home = env.vars.iter().find(|(k, _)| k == "HOME");
    let home = home.map(|(_, v)| PathBuf::from(v)).or_else(dirs::home_dir);
    which(provider.cli, &env.path).or_else(|| {
        let local = home?.join(".local/bin").join(cli_file(provider));
        local.is_file().then_some(local)
    })
}

/// The version `exe --version` prints.
pub fn cli_version(exe: &Path, env: &Env) -> Result<Version> {
    let out = crate::os::group(&mut Command::new(exe))
        .arg("--version")
        .env("PATH", &env.path)
        .envs(env.vars.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .output()?;
    parse_version(&String::from_utf8_lossy(&out.stdout))
        .ok_or_else(|| Error::Other(format!("{} printed no version", exe.display())))
}

fn parse_version(text: &str) -> Option<Version> {
    text.split_whitespace().find_map(|w| Version::parse(w).ok())
}

/// The login as a line to type into the terminal's shell, with the CLI found in place of its name.
pub fn login(provider: &Provider, cli: &Path) -> String {
    let quote = |s: &str| match cfg!(windows) {
        true => format!("'{}'", s.replace('\'', "''")),
        false => format!("'{}'", s.replace('\'', r"'\''")),
    };
    let mut line = quote(&cli.to_string_lossy());
    if cfg!(windows) {
        line.insert_str(0, "& ");
    }
    for arg in &provider.login[1..] {
        line.push(' ');
        line.push_str(arg);
    }
    line
}

/// The provider's installer as a line for this OS's shell.
pub fn installer(provider: &Provider) -> Option<&'static str> {
    provider.installer.map(|i| i[usize::from(cfg!(windows))])
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::testutil::{self, Fixture};
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn data() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn fetched(f: &Fixture, data: &Path) -> Result<Catalog> {
        fetch(&f.url(), &f.key, data)
    }

    fn left(data: &Path, name: &str) -> Vec<String> {
        let dir = fs::read_dir(folder(data, name)).into_iter().flatten();
        let mut names: Vec<_> = (dir.flatten())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn put(data: &Path, entry: &Entry) -> Result<Installed> {
        install(data, entry, &Cancel::default(), &|_| {})
    }

    #[test]
    fn a_signed_catalog_lists_this_platforms_newest_entry_first() {
        let f = Fixture::new();
        f.publish(
            1,
            &[
                f.entry("claude-agent-acp", "0.1.0"),
                f.entry("claude-agent-acp", "0.2.0"),
            ],
        );
        let d = data();
        let catalog = fetched(&f, d.path()).unwrap();
        let entries = catalog.entries(CLAUDE.adapter, &app_version());
        let versions: Vec<_> = entries.iter().map(|e| e.version.to_string()).collect();
        assert_eq!(versions, ["0.2.0", "0.1.0"]);
        assert_eq!(stored(d.path()).unwrap().serial, 1);
    }

    #[test]
    fn the_committed_catalog_parses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../adapters/adapters.json");
        serde_json::from_slice::<Catalog>(&fs::read(path).unwrap()).unwrap();
    }

    #[test]
    fn a_bad_signature_or_an_older_serial_is_refused_and_the_stored_catalog_stays() {
        let f = Fixture::new();
        let d = data();
        f.publish(5, &[]);
        fetched(&f, d.path()).unwrap();
        let stored_json = fs::read(d.path().join("adapters/catalog.json")).unwrap();
        f.publish(4, &[f.entry("claude-agent-acp", "0.1.0")]);
        let err = fetched(&f, d.path()).unwrap_err().to_string();
        assert!(err.contains("older than 5"), "{err}");
        f.publish_signed_by(&testutil::other_signer(), 6, &[]);
        let err = fetched(&f, d.path()).unwrap_err().to_string();
        assert!(err.starts_with("signature: "), "{err}");
        assert!(fetch(&f.url(), "", d.path()).is_err());
        let after = fs::read(d.path().join("adapters/catalog.json")).unwrap();
        assert_eq!(after, stored_json);
        f.publish(5, &[]);
        fetched(&f, d.path()).unwrap();
    }

    #[test]
    fn a_wrong_size_or_sha256_is_refused_and_leaves_nothing() {
        let f = Fixture::new();
        let d = data();
        let good = f.entry("claude-agent-acp", "0.1.0");
        let mut entry = good.clone();
        let a = entry.artifacts.get_mut(&target()).unwrap();
        a.size -= 1;
        let err = put(d.path(), &entry).unwrap_err().to_string();
        assert!(err.ends_with("wrong size"), "{err}");
        entry.artifacts.get_mut(&target()).unwrap().size += 2;
        assert!(
            put(d.path(), &entry)
                .unwrap_err()
                .to_string()
                .ends_with("wrong size")
        );
        entry = good.clone();
        entry.artifacts.get_mut(&target()).unwrap().sha256 = "0".repeat(64);
        let err = put(d.path(), &entry).unwrap_err().to_string();
        assert!(err.ends_with("wrong sha256"), "{err}");
        assert!(left(d.path(), CLAUDE.adapter).is_empty());
        assert_eq!(installed(d.path(), CLAUDE.adapter), Installed::default());
        put(d.path(), &good).unwrap();
    }

    #[test]
    fn an_entry_leaving_its_folder_is_refused_and_leaves_nothing() {
        let f = Fixture::new();
        let d = data();
        let evil = [
            testutil::Member::file("../escaped", b"x"),
            testutil::Member::file("/detached-escaped", b"x"),
            testutil::Member::link("licenses", "/etc"),
        ];
        for member in evil {
            let entry = f.entry_with("claude-agent-acp", "0.1.0", &[member]);
            let err = put(d.path(), &entry).unwrap_err().to_string();
            assert!(err.ends_with("leaves its folder"), "{err}");
            assert!(left(d.path(), CLAUDE.adapter).is_empty());
        }
    }

    #[test]
    fn no_claude_cli_is_downloaded_or_shipped() {
        let f = Fixture::new();
        let d = data();
        let cli = cli_file(&CLAUDE);
        let entry = f.entry_with(
            "claude-agent-acp",
            "0.1.0",
            &[testutil::Member::file(&format!("bin/{cli}"), b"cli")],
        );
        let err = put(d.path(), &entry).unwrap_err().to_string();
        assert!(err.contains("ships bin/"), "{err}");
        let entry = f.entry("claude", "2.1.400");
        assert!(
            put(d.path(), &entry)
                .unwrap_err()
                .to_string()
                .contains("not an agent adapter")
        );
        assert!(!d.path().join("adapters/claude").exists());
        assert!(left(d.path(), CLAUDE.adapter).is_empty());
    }

    #[test]
    fn a_version_inside_its_cool_down_does_not_auto_install() {
        let f = Fixture::new();
        let mut new = f.entry("claude-agent-acp", "0.2.0");
        new.published = day("2026-11-14");
        let catalog = Catalog {
            cli: vec![],
            serial: 1,
            adapters: vec![f.entry("claude-agent-acp", "0.1.0"), new],
        };
        let kept = Installed {
            current: Some(v("0.0.9")),
            previous: None,
        };
        let pick = |today| catalog.auto(CLAUDE.adapter, &app_version(), &kept, day(today));
        assert_eq!(pick("2026-11-16").unwrap().version, v("0.1.0"));
        assert_eq!(pick("2026-11-17").unwrap().version, v("0.2.0"));
        let none = Installed::default();
        assert!(
            catalog
                .auto(CLAUDE.adapter, &app_version(), &none, day("2026-12-01"))
                .is_none()
        );
    }

    #[test]
    fn an_entry_outside_min_app_and_max_app_is_ignored() {
        let f = Fixture::new();
        let app = v("0.5.0");
        let with = |version: &str, min: &str, max: Option<&str>| {
            let mut e = f.entry("claude-agent-acp", version);
            (e.min_app, e.max_app) = (v(min), max.map(v));
            e
        };
        let catalog = Catalog {
            cli: vec![],
            serial: 1,
            adapters: vec![
                with("0.4.0", "0.6.0", None),
                with("0.3.0", "0.1.0", Some("0.4.9")),
                with("0.2.0", "0.5.0", Some("0.5.0")),
                f.entry("codex-acp", "9.0.0"),
            ],
        };
        let found: Vec<_> = (catalog.entries(CLAUDE.adapter, &app).iter())
            .map(|e| e.version.to_string())
            .collect();
        assert_eq!(found, ["0.2.0"]);
        let mut other = f.entry("claude-agent-acp", "0.9.0");
        other.artifacts =
            HashMap::from([("plan9-mips".into(), other.artifacts[&target()].clone())]);
        let catalog = Catalog {
            cli: vec![],
            serial: 1,
            adapters: vec![other],
        };
        assert!(catalog.entries(CLAUDE.adapter, &app).is_empty());
    }

    #[test]
    fn two_versions_sit_side_by_side_and_rollback_points_at_the_kept_one() {
        let f = Fixture::new();
        let d = data();
        let name = CLAUDE.adapter;
        let [one, two, three] = ["0.1.0", "0.2.0", "0.3.0"].map(|x| f.entry(name, x));
        let seen = RefCell::new(vec![]);
        install(d.path(), &one, &Cancel::default(), &|p| {
            seen.borrow_mut().push(p)
        })
        .unwrap();
        assert_eq!(seen.borrow().last(), Some(&Progress::Verifying));
        put(d.path(), &two).unwrap();
        assert_eq!(left(d.path(), name), ["0.1.0", "0.2.0", "installed.json"]);
        assert!(exe(d.path(), name, &v("0.1.0")).is_file());
        let kept = rollback(d.path(), name).unwrap();
        assert_eq!(kept.current, Some(v("0.1.0")));
        assert_eq!(kept.previous, Some(v("0.2.0")));
        assert_eq!(installed(d.path(), name), kept);
        assert!(exe(d.path(), name, kept.current.as_ref().unwrap()).is_file());
        let catalog = Catalog {
            cli: vec![],
            serial: 1,
            adapters: vec![one.clone(), two.clone()],
        };
        let late = day("2030-01-01");
        assert!(catalog.auto(name, &app_version(), &kept, late).is_none());
        put(d.path(), &three).unwrap();
        assert_eq!(left(d.path(), name), ["0.1.0", "0.3.0", "installed.json"]);
        assert!(rollback(d.path(), name).is_ok());
        assert!(rollback(d.path(), CODEX.adapter).is_err());
    }

    #[test]
    fn a_cancelled_install_leaves_nothing() {
        let f = Fixture::new();
        let d = data();
        let cancel = Cancel::new(true.into());
        let entry = f.entry(CLAUDE.adapter, "0.1.0");
        let done = install(d.path(), &entry, &cancel, &|_| {});
        assert!(matches!(done, Err(Error::Cancelled)));
        assert!(left(d.path(), CLAUDE.adapter).is_empty());
    }

    #[test]
    fn the_min_cli_check_refuses_a_cli_below_the_entry_or_the_floor_and_a_blocked_one() {
        let f = Fixture::new();
        let mut entry = f.entry(CLAUDE.adapter, "0.1.0");
        let check = |e: Option<&Entry>, cli| check_cli(&CLAUDE, e, &v(cli));
        assert_eq!(check(None, "2.1.321"), Cli::Old(v("2.1.322")));
        assert_eq!(check(None, "2.1.322"), Cli::Ok);
        entry.min_cli = Some(v("2.1.300"));
        assert_eq!(check(Some(&entry), "2.1.321"), Cli::Old(v("2.1.322")));
        entry.min_cli = Some(v("2.1.330"));
        assert_eq!(check(Some(&entry), "2.1.329"), Cli::Old(v("2.1.330")));
        entry.blocked_cli = vec![v("2.1.331")];
        assert_eq!(check(Some(&entry), "2.1.331"), Cli::Blocked);
        assert_eq!(check(Some(&entry), "2.1.332"), Cli::Ok);
    }

    #[test]
    fn a_cli_version_is_the_first_word_that_parses() {
        assert_eq!(
            parse_version("2.1.322 (Claude Agent)\n"),
            Some(v("2.1.322"))
        );
        assert_eq!(parse_version("codex-cli 0.166.0"), Some(v("0.166.0")));
        assert_eq!(parse_version("command not found"), None);
    }

    #[test]
    fn the_login_runs_the_found_cli_with_the_manifests_arguments() {
        let line = login(&CLAUDE, Path::new("/a b/it's/claude"));
        match cfg!(windows) {
            true => assert_eq!(line, "& '/a b/it''s/claude' /login"),
            false => assert_eq!(line, r"'/a b/it'\''s/claude' /login"),
        }
        assert!(login(&CODEX, Path::new("codex")).ends_with("'codex' login"));
    }

    #[test]
    fn a_cli_path_in_the_settings_wins_and_a_missing_one_finds_nothing() {
        let d = data();
        let bin = d.path().join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(bin.join(cli_file(&CLAUDE)), "").unwrap();
        let mine = d.path().join("mine");
        fs::write(&mine, "").unwrap();
        let env = Env {
            path: bin.to_string_lossy().into(),
            vars: vec![("HOME".into(), d.path().to_string_lossy().into())],
            ..Env::default()
        };
        assert_eq!(
            find_cli(&CLAUDE, None, &env),
            Some(bin.join(cli_file(&CLAUDE)))
        );
        assert_eq!(find_cli(&CLAUDE, mine.to_str(), &env), Some(mine));
        assert_eq!(find_cli(&CLAUDE, Some("/nowhere/claude"), &env), None);
        assert_eq!(find_cli(&CODEX, None, &env), None);
        let local = d.path().join(".local/bin");
        fs::create_dir_all(&local).unwrap();
        fs::write(local.join(cli_file(&CODEX)), "").unwrap();
        assert_eq!(
            find_cli(&CODEX, None, &env),
            Some(local.join(cli_file(&CODEX)))
        );
    }

    const LOGINS: [&str; 3] = [
        ".claude/.credentials.json",
        ".claude.json",
        ".codex/auth.json",
    ];

    /// The host's whole adapter flow, which `the_host_never_opens_the_login` runs in a child under a
    /// temp HOME.
    #[test]
    #[ignore = "run by the_host_never_opens_the_login"]
    fn login_flow() {
        let f = Fixture::new();
        let d = data();
        f.publish(1, &[f.entry(CLAUDE.adapter, "0.1.0")]);
        let catalog = fetched(&f, d.path()).unwrap();
        let entry = catalog.entries(CLAUDE.adapter, &app_version())[0];
        put(d.path(), entry).unwrap();
        let cli = testutil::fake_cli(d.path(), "2.1.400");
        let env = Env::default();
        let found = find_cli(&CLAUDE, cli.to_str(), &env).unwrap();
        let version = cli_version(&found, &env).unwrap();
        assert_eq!(check_cli(&CLAUDE, Some(entry), &version), Cli::Ok);
        assert!(login(&CLAUDE, &found).ends_with("/login"));
    }

    /// Whether `login_flow` passed, with its output when it did not.
    fn run_login_flow(home: &Path) -> std::result::Result<(), String> {
        let out = Command::new(std::env::current_exe().unwrap())
            .args([
                "adapters::tests::login_flow",
                "--exact",
                "--ignored",
                "--quiet",
            ])
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
            .env("CODEX_HOME", home.join(".codex"))
            .output()
            .unwrap();
        match out.status.success() {
            true => Ok(()),
            false => Err(String::from_utf8_lossy(&out.stdout).into()),
        }
    }

    /// A writer blocked on each login FIFO wakes only when a reader opens it.
    #[cfg(unix)]
    #[test]
    fn the_host_never_opens_the_login() {
        use std::os::unix::fs::OpenOptionsExt;
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let home = data();
        let (opened, done) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let fifos: Vec<_> = LOGINS.iter().map(|l| home.path().join(l)).collect();
        let writers: Vec<_> = (fifos.iter().cloned())
            .map(|fifo| {
                fs::create_dir_all(fifo.parent().unwrap()).unwrap();
                let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
                // SAFETY: a NUL-terminated path.
                assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
                let (opened, done) = (opened.clone(), done.clone());
                std::thread::spawn(move || {
                    let _writer = fs::OpenOptions::new().write(true).open(&fifo);
                    if !done.load(Ordering::SeqCst) {
                        opened.store(true, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        let flow = run_login_flow(home.path());
        done.store(true, Ordering::SeqCst);
        let release = |fifo: &PathBuf| {
            let mut o = fs::OpenOptions::new();
            o.read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(fifo)
                .unwrap()
        };
        let readers: Vec<_> = fifos.iter().map(release).collect();
        writers.into_iter().for_each(|w| w.join().unwrap());
        drop(readers);
        flow.unwrap();
        assert!(!opened.load(Ordering::SeqCst), "the host opened a login");
    }

    /// Each login file is held with no sharing, so any open by the host fails the flow.
    #[cfg(windows)]
    #[test]
    fn the_host_never_opens_the_login() {
        use std::os::windows::fs::OpenOptionsExt;
        let home = data();
        let held: Vec<_> = (LOGINS.iter())
            .map(|l| {
                let file = home.path().join(l);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                let mut o = fs::OpenOptions::new();
                o.read(true).write(true).create(true).share_mode(0);
                let held = o.open(&file).unwrap();
                assert!(fs::read(&file).is_err(), "the share mode holds");
                held
            })
            .collect();
        run_login_flow(home.path()).unwrap();
        drop(held);
    }
}
