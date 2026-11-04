//! Offline lock-policy checks. Each immutable upstream action.yml was reviewed
//! for `runs.using: node24`; no CI test contacts GitHub or executes JavaScript.
use serde_yaml_ng::Value;
use std::collections::BTreeMap;

#[test]
fn all_javascript_actions_use_reviewed_immutable_node24_versions() {
    let workflow: Value =
        serde_yaml_ng::from_str(include_str!("../.github/workflows/windows-build.yml")).unwrap();
    let steps = workflow["jobs"]["windows-x64"]["steps"]
        .as_sequence()
        .unwrap();
    // Upstream release tags were dereferenced, then each SHA's action.yml was
    // fetched and its native runtime verified. Never use floating major tags.
    let reviewed = [
        (
            "actions/checkout",
            "3d3c42e5aac5ba805825da76410c181273ba90b1",
        ), // v7.0.1
        ("actions/cache", "55cc8345863c7cc4c66a329aec7e433d2d1c52a9"), // v6.1.0
        (
            "actions/upload-artifact",
            "cf430e030ddbb5b0abf93d22962f4752f3646cd9",
        ), // v7.0.2
    ];
    let mut counts = BTreeMap::new();
    for step in steps {
        let Some(uses) = step["uses"].as_str() else {
            continue;
        };
        let (action, sha) = uses.split_once('@').unwrap();
        let expected = reviewed
            .iter()
            .find(|(name, _)| *name == action)
            .expect("review the runtime of newly added actions")
            .1;
        assert_eq!(sha, expected);
        assert_eq!(sha.len(), 40);
        assert!(sha.bytes().all(|c| c.is_ascii_hexdigit()));
        *counts.entry(action).or_insert(0) += 1;
        if action == "actions/checkout" {
            assert_eq!(step["with"]["persist-credentials"], Value::Bool(false));
        }
        if action == "actions/upload-artifact" {
            assert_eq!(
                step["with"]["archive"],
                Value::Bool(true),
                "keep the existing multi-file artifact contract"
            );
            assert_eq!(step["with"]["if-no-files-found"].as_str(), Some("error"));
            assert_eq!(step["with"]["compression-level"].as_i64(), Some(0));
        }
    }
    assert_eq!(counts.get("actions/checkout"), Some(&1));
    assert_eq!(counts.get("actions/cache"), Some(&1));
    assert_eq!(counts.get("actions/upload-artifact"), Some(&2));
}
