use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// `DETACHED_CONFIG_DIR`, else `com.w-devs.detached` in the OS config dir.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("DETACHED_CONFIG_DIR") {
        return Ok(dir.into());
    }
    let base = dirs::config_dir().ok_or_else(|| Error::Other("No config directory".into()))?;
    Ok(in_base(&base))
}

/// Settings under a former identifier move over once, the newest first.
fn in_base(base: &Path) -> PathBuf {
    let dir = base.join("com.w-devs.detached");
    for old in ["dev.wdevs.detached", "dev.wdevs.workbench"] {
        if !dir.exists() {
            let _ = std::fs::rename(base.join(old), &dir);
        }
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_former_dir_moves_over_once() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("dev.wdevs.workbench")).unwrap();
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
