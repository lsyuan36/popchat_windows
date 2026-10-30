//! Windows semantic equivalent of SMAppService's per-user login item.
//! Only the exact PopChat value is read/written; no scheduled task or elevation.

/// Record `AppDelegate.hasLaunchedBefore` in this profile, including login-item
/// launches. Call only in the primary process, before deciding launch visibility.
/// Returns true exactly once for a newly created profile marker.
pub fn record_launch() -> anyhow::Result<bool> {
    record_launch_in(&crate::chat::store::app_data_dir())
}

fn record_launch_in(profile: &std::path::Path) -> anyhow::Result<bool> {
    use anyhow::Context;
    std::fs::create_dir_all(profile).context("建立啟動狀態資料夾失敗")?;
    // Existence is the state: create_new is atomic, needs no partially written
    // JSON repair, and cannot overwrite another process's first-launch record.
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(profile.join("hasLaunchedBefore"))
    {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).context("記錄首次啟動狀態失敗"),
    }
}

/// `AppDelegate.presentLaunchUI`: only the first non-login launch presents the
/// panel automatically. An explicit Windows `--settings` request always wins.
/// The one-time hint itself should use `first && !login_item`, not this override.
pub fn should_present_launch(first: bool, login_item: bool, settings_requested: bool) -> bool {
    settings_requested || (first && !login_item)
}

pub fn launch_command(executable: &std::path::Path) -> anyhow::Result<String> {
    let path = executable
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("執行檔路徑不是有效 Unicode"))?;
    anyhow::ensure!(!path.contains('"'), "執行檔路徑包含引號");
    Ok(format!("\"{path}\" --login-item"))
}

#[cfg(windows)]
pub fn enabled() -> anyhow::Result<bool> {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_READ},
        RegKey,
    };
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let key = match root.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        KEY_READ,
    ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    match key.get_value::<String, _>("PopChat") {
        Ok(value) => Ok(value == launch_command(&std::env::current_exe()?)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
pub fn set_enabled(enable: bool) -> anyhow::Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = root.create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
    if enable {
        key.set_value("PopChat", &launch_command(&std::env::current_exe()?)?)?;
    } else {
        match key.delete_value("PopChat") {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn enabled() -> anyhow::Result<bool> {
    Ok(false)
}

#[cfg(not(windows))]
pub fn set_enabled(_: bool) -> anyhow::Result<()> {
    anyhow::bail!("僅支援 Windows 登入啟動")
}

#[cfg(test)]
mod tests {
    #[test]
    fn launch_marker_is_persistent_and_profile_local() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        assert!(super::record_launch_in(&first).unwrap());
        assert!(!super::record_launch_in(&first).unwrap());
        assert!(super::record_launch_in(&second).unwrap());
        assert!(!super::record_launch_in(&second).unwrap());
        assert!(first.join("hasLaunchedBefore").is_file());
    }

    #[test]
    fn launch_marker_errors_do_not_pretend_to_be_a_repeat_launch() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("not-a-directory");
        std::fs::write(&file, b"keep").unwrap();
        assert!(super::record_launch_in(&file).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"keep");
    }

    #[test]
    fn concurrent_launch_record_has_exactly_one_first_launch() {
        let root = tempfile::tempdir().unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let profile = root.path().to_owned();
                std::thread::spawn(move || super::record_launch_in(&profile).unwrap())
            })
            .collect();
        let first_count = threads
            .into_iter()
            .map(|thread| usize::from(thread.join().unwrap()))
            .sum::<usize>();
        assert_eq!(first_count, 1);
    }

    #[test]
    fn launch_visibility_matches_first_run_login_and_settings_policy() {
        for first in [false, true] {
            for login_item in [false, true] {
                assert_eq!(
                    super::should_present_launch(first, login_item, false),
                    first && !login_item
                );
                assert!(super::should_present_launch(first, login_item, true));
            }
        }
        // A first login launch still consumes the one-time onboarding state.
        let root = tempfile::tempdir().unwrap();
        let first = super::record_launch_in(root.path()).unwrap();
        assert!(!super::should_present_launch(first, true, false));
        assert!(!super::record_launch_in(root.path()).unwrap());
    }

    #[test]
    fn login_launch_quotes_executable_and_marks_background_launch() {
        assert_eq!(
            super::launch_command(std::path::Path::new(
                "C:\\Program Files\\PopChat\\popchat.exe"
            ))
            .unwrap(),
            "\"C:\\Program Files\\PopChat\\popchat.exe\" --login-item"
        );
        assert!(super::launch_command(std::path::Path::new("bad\"path")).is_err());
    }
}
