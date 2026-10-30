//! Optional isolated profile for UI smoke tests and portable instances.

pub fn isolated_profile() -> Option<std::path::PathBuf> {
    std::env::var_os("POPCHAT_DATA_DIR")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}

pub fn config_dir() -> Option<std::path::PathBuf> {
    isolated_profile().or_else(|| {
        directories::ProjectDirs::from("com", "PopChat", "PopChat")
            .map(|dirs| dirs.config_dir().to_owned())
    })
}

/// Same-directory atomic replacement, matching Foundation's `.atomic` writes.
/// A failed serialization/write/rename must not truncate the previous profile.
pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("missing parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".popchat-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn atomic_replacement_and_failure_never_leave_partial_targets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        super::atomic_write(&path, b"old").unwrap();
        super::atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"new");
        let protected = dir.path().join("directory");
        std::fs::create_dir(&protected).unwrap();
        std::fs::write(protected.join("keep"), b"untouched").unwrap();
        assert!(super::atomic_write(&protected, b"cannot replace nonempty directory").is_err());
        assert_eq!(std::fs::read(protected.join("keep")).unwrap(), b"untouched");
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    }
}
