use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub const IDENTIFIER: &str = "com.w-devs.detached";

/// `DETACHED_CONFIG_DIR`, else `com.w-devs.detached` in the OS config dir.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = override_dir("DETACHED_CONFIG_DIR")? {
        return Ok(dir);
    }
    let base = dirs::config_dir().ok_or_else(|| Error::Other("No config directory".into()))?;
    Ok(in_base(&base))
}

pub(crate) fn override_dir(key: &str) -> Result<Option<PathBuf>> {
    match std::env::var_os(key) {
        Some(dir) if dir.is_empty() => Err(Error::Other(format!("{key} cannot be empty"))),
        dir => Ok(dir.map(PathBuf::from)),
    }
}

/// Settings under the former identifier move over once.
pub(crate) fn in_base(base: &Path) -> PathBuf {
    let dir = base.join(IDENTIFIER);
    if !dir.exists() {
        let _ = std::fs::rename(base.join("dev.wdevs.detached"), &dir);
    }
    dir
}

pub(crate) fn directories(path: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut dirs = vec![];
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_former_dir_moves_over_once() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join("dev.wdevs.detached");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        let dir = in_base(base.path());
        assert!(dir.join("settings.json").is_file());
        std::fs::create_dir(&old).unwrap();
        assert_eq!(in_base(base.path()), dir);
        assert!(old.exists());
    }
}
