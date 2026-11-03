//! Dependency-free Windows PE resources. Cargo discovers this file automatically.
#[path = "src/app_icon.rs"]
mod app_icon;

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    for path in [
        "build.rs",
        "Cargo.toml",
        "src/app_icon.rs",
        "assets/app-icon/app.ico",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    for name in [
        "PATH",
        "WindowsSdkDir",
        "WindowsSDKVersion",
        "WindowsSdkVerBinPath",
        "WindowsSdkBinPath",
        "ProgramFiles(x86)",
        "ProgramFiles",
        "HOST",
        "CARGO_CFG_TARGET_OS",
        "CARGO_CFG_TARGET_ENV",
        "CARGO_PKG_VERSION",
        "CARGO_PKG_VERSION_MAJOR",
        "CARGO_PKG_VERSION_MINOR",
        "CARGO_PKG_VERSION_PATCH",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ENV").as_deref(),
        Ok("msvc"),
        "PopChat's Windows SDK resource build requires a Windows MSVC target"
    );
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let icon = manifest.join("assets/app-icon/app.ico");
    assert_eq!(
        fs::read(&icon).expect("read app icon"),
        app_icon::ico_bytes(),
        "assets/app-icon/app.ico is stale; regenerate it with src/app_icon.rs::ico_bytes()"
    );
    let compiler = find_rc().expect(
        "Windows SDK rc.exe not found. Install the Windows SDK / C++ build tools, or put rc.exe on PATH (WindowsSdkDir is also supported)."
    );
    let version = ["MAJOR", "MINOR", "PATCH"].map(|part| {
        env::var(format!("CARGO_PKG_VERSION_{part}"))
            .expect("Cargo package version component")
            .parse::<u16>()
            .expect("Windows version components must fit in u16")
    });
    let version_string = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    for (binary, description) in [
        ("popchat_windows", "PopChat"),
        ("popchat_updater", "PopChat Update Helper"),
    ] {
        let source = out.join(format!("{binary}.rc"));
        let resource = out.join(format!("{binary}.res"));
        fs::write(
            &source,
            resource_script(binary, description, &icon, version, &version_string),
        )
        .expect("write Windows resource source to OUT_DIR");
        let result = Command::new(&compiler)
            .current_dir(&out)
            .arg("/nologo")
            .arg("/c65001")
            .arg("/fo")
            .arg(&resource)
            .arg(&source)
            .output()
            .unwrap_or_else(|error| panic!("start {}: {error}", compiler.display()));
        assert!(
            result.status.success(),
            "{} failed for {}:\n{}\n{}",
            compiler.display(),
            source.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        // Never attach resources to the library, examples or test harnesses.
        println!("cargo:rustc-link-arg-bin={binary}={}", resource.display());
    }
}

fn rc_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn resource_script(
    binary: &str,
    description: &str,
    icon: &Path,
    version: [u16; 3],
    full_version: &str,
) -> String {
    let [major, minor, patch] = version;
    // Use absolute forward-slash paths: rc.exe need not share Cargo's CWD,
    // and spaces/non-ASCII checkout directories remain quoted UTF-8 strings.
    let icon = rc_string(&icon.to_string_lossy().replace('\\', "/"));
    let full_version = rc_string(full_version);
    format!(
        r#"// Generated in OUT_DIR; numeric constants avoid Windows header dependencies.
LANGUAGE 9, 1
1 ICON "{icon}"
1 VERSIONINFO
 FILEVERSION {major},{minor},{patch},0
 PRODUCTVERSION {major},{minor},{patch},0
 FILEFLAGSMASK 0x3fL
 FILEFLAGS 0x0L
 FILEOS 0x40004L
 FILETYPE 0x1L
 FILESUBTYPE 0x0L
BEGIN
 BLOCK "StringFileInfo"
 BEGIN
  BLOCK "040904B0"
  BEGIN
   VALUE "CompanyName", "Evis Lai\0"
   VALUE "FileDescription", "{description}\0"
   VALUE "FileVersion", "{full_version}\0"
   VALUE "InternalName", "{binary}\0"
   VALUE "LegalCopyright", "2026 Le Chen; Windows port 2026 Evis Lai\0"
   VALUE "OriginalFilename", "{binary}.exe\0"
   VALUE "ProductName", "PopChat\0"
   VALUE "ProductVersion", "{full_version}\0"
  END
 END
 BLOCK "VarFileInfo"
 BEGIN
  VALUE "Translation", 0x0409, 1200
 END
END
"#
    )
}

fn find_rc() -> Option<PathBuf> {
    if let Some(paths) = env::var_os("PATH") {
        for directory in env::split_paths(&paths) {
            let candidate = directory.join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let host = env::var("HOST").unwrap_or_default();
    let architectures = if host.starts_with("aarch64") {
        ["arm64", "x64", "x86"]
    } else {
        ["x64", "x86", "arm64"]
    };
    for name in ["WindowsSdkVerBinPath", "WindowsSdkBinPath"] {
        if let Some(root) = env::var_os(name) {
            if let Some(compiler) = rc_in_bin(&PathBuf::from(root), &architectures) {
                return Some(compiler);
            }
        }
    }
    let mut roots = Vec::new();
    if let Some(root) = env::var_os("WindowsSdkDir") {
        roots.push(PathBuf::from(root));
    }
    for name in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(root) = env::var_os(name) {
            let root = PathBuf::from(root).join("Windows Kits/10");
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    for root in roots {
        let bin = root.join("bin");
        if let Ok(version) = env::var("WindowsSDKVersion") {
            if let Some(compiler) =
                rc_in_bin(&bin.join(version.trim_matches(['\\', '/'])), &architectures)
            {
                return Some(compiler);
            }
        }
        let mut versions: Vec<_> = fs::read_dir(&bin)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && sdk_version(path).is_some())
            .collect();
        versions.sort_by_key(|path| std::cmp::Reverse(sdk_version(path)));
        for version in versions {
            if let Some(compiler) = rc_in_bin(&version, &architectures) {
                return Some(compiler);
            }
        }
        if let Some(compiler) = rc_in_bin(&bin, &architectures) {
            return Some(compiler);
        }
    }
    None
}

fn rc_in_bin(bin: &Path, architectures: &[&str]) -> Option<PathBuf> {
    architectures
        .iter()
        .map(|arch| bin.join(arch).join("rc.exe"))
        .chain(std::iter::once(bin.join("rc.exe")))
        .find(|candidate| candidate.is_file())
}

fn sdk_version(path: &Path) -> Option<Vec<u32>> {
    let name = path.file_name()?.to_str()?;
    if !name.starts_with("10.") {
        return None;
    }
    name.split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_metadata_tracks_cargo_version_and_needs_no_headers() {
        let resource = resource_script(
            "popchat_windows",
            "PopChat",
            Path::new("C:/a b/app.ico"),
            [0, 2, 1],
            "0.2.1",
        );
        for text in [
            "FILEVERSION 0,2,1,0",
            "PRODUCTVERSION 0,2,1,0",
            "Evis Lai",
            "0.2.1",
            "popchat_windows.exe",
            "1 ICON \"C:/a b/app.ico\"",
        ] {
            assert!(resource.contains(text), "missing {text}");
        }
        assert!(!resource.contains("#include"));
    }

    #[test]
    fn sdk_versions_are_numeric_and_non_versions_are_ignored() {
        assert!(sdk_version(Path::new("10.0.26100.0")) > sdk_version(Path::new("10.0.9999.0")));
        assert_eq!(sdk_version(Path::new("x64")), None);
        assert_eq!(sdk_version(Path::new("10.not-a-version")), None);
    }
}
