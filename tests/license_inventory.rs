//! Check notice integrity without building, installing or launching the app.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path};

#[test]
fn dependency_notices_preserve_the_877_entry_portable_contract() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = repo.join("assets/licenses/dependencies");
    let index: Value =
        serde_json::from_slice(&std::fs::read(root.join("index.json")).unwrap()).unwrap();
    let mut files = HashSet::new();
    let mut packages = HashSet::new();
    let mut bundled = 0;
    for entry in index["packages"].as_array().unwrap() {
        assert!(packages.insert(format!(
            "{}@{}",
            entry["name"].as_str().unwrap(),
            entry["version"].as_str().unwrap()
        )));
        let paths = entry["files"].as_array().unwrap();
        if paths.is_empty() {
            let notice = &entry["bundled_notice"];
            assert_eq!(notice["path"], "THIRD_PARTY_NOTICES.md");
            let data = std::fs::read(repo.join("THIRD_PARTY_NOTICES.md")).unwrap();
            assert_eq!(
                format!("{:x}", Sha256::digest(&data)),
                notice["sha256"].as_str().unwrap()
            );
            let text = std::str::from_utf8(&data).unwrap();
            assert!(text.contains(&format!("#### {}", notice["section"].as_str().unwrap())));
            assert_eq!(entry["license"], "MIT");
            bundled += 1;
        } else {
            for path in paths {
                let path = path.as_str().unwrap();
                assert!(files.insert(path.to_owned()), "duplicate {path}");
                assert!(
                    !path.contains("..") && !path.contains(':') && !path.starts_with(['/', '\\'])
                );
                let data = std::fs::read(root.join(path)).unwrap();
                assert_eq!(
                    format!("{:x}", Sha256::digest(&data)),
                    entry["file_sha256"][path].as_str().unwrap(),
                    "{path}"
                );
            }
        }
    }
    assert!(packages.contains("serde_yaml_ng@0.10.0"));
    assert!(packages.contains("unsafe-libyaml@0.2.11"));
    assert_eq!(bundled, 2);
    // Two binaries, four root notices/docs, three Font Awesome docs, fourteen
    // icons, three font notices and the license index, plus dependency files.
    assert_eq!(27 + files.len(), 877);
}
