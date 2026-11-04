use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// `DETACHED_CONFIG_DIR`, else `dev.wdevs.detached` in the OS config dir, where Tauri puts it.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("DETACHED_CONFIG_DIR") {
        return Ok(dir.into());
    }
    let base = dirs::config_dir().ok_or_else(|| Error::Other("No config directory".into()))?;
    Ok(in_base(&base))
}

/// Settings from the app's former identifier move over once.
fn in_base(base: &Path) -> PathBuf {
    let dir = base.join("dev.wdevs.detached");
    if !dir.exists() {
        let _ = std::fs::rename(base.join("dev.wdevs.workbench"), &dir);
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_former_dir_moves_over_once() {
        let base = tempfile::tempdir().unwrap();
        let old = base.path().join("dev.wdevs.workbench");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        let dir = in_base(base.path());
        assert!(dir.join("settings.json").is_file());
        std::fs::create_dir(&old).unwrap();
        assert_eq!(in_base(base.path()), dir);
        assert!(old.exists());
    }
}
