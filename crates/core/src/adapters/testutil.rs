//! A catalog and adapter archives on localhost, signed with a test key, for the adapters' tests.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};

use super::{Artifact, CLAUDE, Entry, target};
use crate::update::testutil::{Server, keys};

pub type Signer = Box<dyn Fn(&[u8]) -> String>;

pub fn other_signer() -> Signer {
    Box::new(keys().1)
}

pub struct Fixture {
    pub server: Server,
    pub key: String,
    pub sign: Signer,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    pub fn new() -> Fixture {
        let (key, sign) = keys();
        Fixture {
            server: Server::empty(),
            key,
            sign: Box::new(sign),
        }
    }

    pub fn url(&self) -> String {
        format!("{}/adapters.json", self.server.base)
    }

    /// Serves a catalog of `entries` with `serial`, signed by the fixture's key.
    pub fn publish(&self, serial: u64, entries: &[Entry]) {
        self.publish_signed_by(&self.sign, serial, entries);
    }

    pub fn publish_signed_by(&self, sign: &Signer, serial: u64, entries: &[Entry]) {
        let json = serde_json::json!({"serial": serial, "adapters": entries}).to_string();
        self.server
            .put("/adapters.json.minisig", sign(json.as_bytes()));
        self.server.put("/adapters.json", json);
    }

    /// An entry published long ago whose archive holds the adapter and a license, for this
    /// platform.
    pub fn entry(&self, name: &str, version: &str) -> Entry {
        self.entry_with(name, version, &[])
    }

    /// `entry` with `extra` in its archive.
    pub fn entry_with(&self, name: &str, version: &str, extra: &[Member]) -> Entry {
        let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        let mut members = vec![
            Member::file(&exe, b"#!/bin/sh\n"),
            Member::file("licenses/LICENSE", b"MIT"),
        ];
        members.extend(extra.iter().cloned());
        self.entry_of(name, version, archive(&members))
    }

    /// An entry like `entry`'s, served from the archive `bytes`.
    pub fn entry_of(&self, name: &str, version: &str, bytes: Vec<u8>) -> Entry {
        let path = format!("/{name}-{version}.tar.gz");
        let artifact = Artifact {
            url: format!("{}{path}", self.server.base),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        self.server.put(&path, bytes);
        Entry {
            name: name.into(),
            version: version.parse().unwrap(),
            published: "2026-01-01".parse().unwrap(),
            min_app: "0.0.0".parse().unwrap(),
            max_app: None,
            min_cli: None,
            blocked_cli: vec![],
            artifacts: HashMap::from([(target(), artifact)]),
        }
    }
}

#[derive(Clone)]
pub struct Member {
    path: String,
    data: Vec<u8>,
    link: Option<String>,
}

impl Member {
    pub fn file(path: &str, data: &[u8]) -> Member {
        Member {
            path: path.into(),
            data: data.into(),
            link: None,
        }
    }

    pub fn link(path: &str, to: &str) -> Member {
        Member {
            path: path.into(),
            data: vec![],
            link: Some(to.into()),
        }
    }
}

/// A `.tar.gz` of `members`, their names written raw so `..` and absolute paths get in.
fn archive(members: &[Member]) -> Vec<u8> {
    let mut tar = tar::Builder::new(GzEncoder::new(vec![], Compression::fast()));
    for m in members {
        let mut head = tar::Header::new_old();
        head.as_old_mut().name[..m.path.len()].copy_from_slice(m.path.as_bytes());
        match &m.link {
            Some(to) => {
                head.set_entry_type(tar::EntryType::Symlink);
                head.as_old_mut().linkname[..to.len()].copy_from_slice(to.as_bytes());
            }
            None => head.set_entry_type(tar::EntryType::Regular),
        }
        head.set_size(m.data.len() as u64);
        head.set_mode(0o755);
        head.set_cksum();
        tar.append(&head, &m.data[..]).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

/// A stand-in for the provider's CLI in `dir` that prints `version`.
pub fn fake_cli(dir: &Path, version: &str) -> PathBuf {
    #[cfg(unix)]
    let (cli, text) = (
        dir.join(CLAUDE.cli),
        format!("#!/bin/sh\necho '{version} (claude)'\n"),
    );
    #[cfg(windows)]
    let (cli, text) = (
        dir.join(format!("{}.cmd", CLAUDE.cli)),
        format!("@echo {version} (claude)\r\n"),
    );
    std::fs::write(&cli, text).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&cli, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    cli
}
