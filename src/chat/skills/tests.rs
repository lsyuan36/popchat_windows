use super::*;
use crate::chat::{
    client::{self, ChatStreamEvent, ProviderConfig, ProviderKind, WireContent, WireMessage},
    mcp::McpTurn,
};
use uuid::Uuid;

fn fixture() -> (tempfile::TempDir, SkillStore) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("skills");
    fs::create_dir(&root).unwrap();
    let store = SkillStore::load_at(
        dir.path().join("settings.json"),
        vec![Source {
            name: "Test".into(),
            path: root,
            enabled: true,
            root_markdown: true,
        }],
    );
    (dir, store)
}
fn write_skill(store: &SkillStore, path: &str, name: &str, body: &str) {
    let file = store.sources()[0].path.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file,format!("---\nname: {name}\ndescription: |\n  Use for a synthetic test\n  with multiline YAML.\n---\n{body}")).unwrap();
}
fn enabled_fixture() -> (tempfile::TempDir, SkillStore, Skill) {
    let (dir, mut store) = fixture();
    write_skill(
        &store,
        "test/SKILL.md",
        "Test",
        "BODY_ONLY_LOADED_ON_DEMAND",
    );
    let skill = store.scan().skills.remove(0);
    store.set_enabled(&skill, true).unwrap();
    (dir, store, skill)
}
#[test]
fn defaults_match_official_and_legacy_paths_without_touching_them() {
    let home = std::env::current_dir().unwrap().join("not-real-home");
    let sources = default_sources(&home, None);
    assert_eq!(sources.len(), 4);
    assert_eq!(sources[0].path, home.join(".config/opencode/skills"));
    assert_eq!(sources[1].path, home.join(".claude/skills"));
    assert_eq!(sources[2].path, home.join(".agents/skills"));
    assert!(sources[3].name.contains("舊版"));
    let config = home.join("custom-config");
    assert_eq!(
        default_sources(&home, Some(&config))[0].path,
        config.join("opencode/skills")
    );
    assert!(!home.exists());
}
#[test]
fn discovery_real_yaml_duplicates_disabled_sources_and_author_opt_out() {
    let (_dir, mut store) = fixture();
    write_skill(&store, "one/SKILL.md", "Same", "one");
    write_skill(&store, "two/SKILL.md", "Same", "two");
    write_skill(&store, "root.md", "Root", "root");
    fs::write(
        store.sources()[0].path.join("bad.md"),
        "---\nname: [not, a, string]\ndescription: bad\n---\nignored",
    )
    .unwrap();
    fs::write(
        store.sources()[0].path.join("manual.md"),
        "---\nname: Manual\ndescription: manual only\ndisable-model-invocation: true\n---\nmanual",
    )
    .unwrap();
    let scan = store.scan();
    assert_eq!(scan.skills.len(), 4);
    assert!(!scan.notices.is_empty());
    assert!(scan.skills.iter().all(|s| !store.enabled(s)));
    let same = scan
        .skills
        .iter()
        .filter(|s| s.name == "Same")
        .collect::<Vec<_>>();
    assert_eq!(same.len(), 2);
    assert_ne!(same[0].id, same[1].id);
    assert!(same[0].description.contains("multiline YAML"));
    assert!(store
        .set_enabled(
            scan.skills.iter().find(|s| s.name == "Manual").unwrap(),
            true
        )
        .is_err());
    store.set_source_enabled(0, false).unwrap();
    assert!(store.scan().skills.is_empty());
}
#[test]
fn consent_persists_but_changes_disabling_and_removal_revoke_access() {
    let (dir, mut store, skill) = enabled_fixture();
    let reloaded = SkillStore::load_at(dir.path().join("settings.json"), vec![]);
    assert!(reloaded.enabled(&skill));
    assert_eq!(reloaded.for_turn().len(), 1);
    let mut stale = store.for_turn();
    write_skill(&store, "test/SKILL.md", "Test", "CHANGED");
    assert!(stale
        .execute("load_skill", &json!({"id":skill.id}).to_string())
        .is_err());
    assert!(!store.enabled(&store.scan().skills[0]));
    assert_eq!(store.for_turn().len(), 0);
    assert!(store.set_enabled(&skill, true).is_err());
    let changed = store.scan().skills.remove(0);
    store.set_enabled(&changed, true).unwrap();
    store.set_source_enabled(0, false).unwrap();
    assert_eq!(store.for_turn().len(), 0);
    store.set_source_enabled(0, true).unwrap();
    store.remove_source(0).unwrap();
    assert_eq!(store.for_turn().len(), 0);
    assert!(changed.file.exists());
}
#[test]
fn lazy_loading_scoped_references_and_scripts_never_execute() {
    let (_dir, store, skill) = enabled_fixture();
    let base = skill.file.parent().unwrap();
    fs::create_dir(base.join("references")).unwrap();
    fs::write(base.join("references/test.md"), "REFERENCE_ONLY_ON_DEMAND").unwrap();
    fs::write(base.join("run.ps1"), "throw 'must not run'").unwrap();
    fs::write(base.join(".env"), "PRIVATE=synthetic").unwrap();
    let mut turn = store.for_turn();
    let schema = serde_json::to_string(&turn.schemas()).unwrap();
    assert!(schema.contains("Test"));
    assert!(!schema.contains("BODY_ONLY"));
    assert!(!schema.contains("REFERENCE_ONLY"));
    assert!(turn
        .execute(
            "read_skill_file",
            &json!({"id":skill.id,"path":"references/test.md"}).to_string()
        )
        .is_err());
    let output = turn
        .execute("load_skill", &json!({"id":skill.id}).to_string())
        .unwrap();
    assert!(output.contains("BODY_ONLY_LOADED_ON_DEMAND"));
    assert!(turn
        .execute(
            "read_skill_file",
            &json!({"id":skill.id,"path":"references/test.md"}).to_string()
        )
        .unwrap()
        .contains("REFERENCE_ONLY_ON_DEMAND"));
    for path in [
        "../settings.json",
        "/etc/passwd",
        "C:/Windows/win.ini",
        "references\\test.md",
        "run.ps1",
        ".env",
        "references/../../settings.json",
        "test.md:secret",
        "//host/share/test.md",
    ] {
        assert!(
            turn.execute(
                "read_skill_file",
                &json!({"id":skill.id,"path":path}).to_string()
            )
            .is_err(),
            "must reject {path}"
        );
    }
    let mut fresh = store.for_turn();
    assert!(fresh
        .execute(
            "read_skill_file",
            &json!({"id":skill.id,"path":"references/test.md"}).to_string()
        )
        .is_err());
    assert!(fresh
        .execute(
            "load_skill",
            &json!({"id":skill.id,"extra":true}).to_string()
        )
        .is_err());
    assert!(fresh
        .execute("load_skill", &json!({"id":"not enabled"}).to_string())
        .is_err());
}
#[test]
fn bounded_files_depth_metadata_and_catalog() {
    let (_dir, mut store) = fixture();
    write_skill(&store, "large/SKILL.md", "Large", &"a".repeat(MAX_FILE));
    write_skill(&store, "a/b/c/d/e/f/SKILL.md", "Depth6", "yes");
    write_skill(&store, "a/b/c/d/e/f/g/SKILL.md", "Depth7", "no");
    let scan = store.scan();
    assert_eq!(scan.skills.len(), 1);
    assert_eq!(scan.skills[0].name, "Depth6");
    for n in 0..55 {
        write_skill(
            &store,
            &format!("n{n}/SKILL.md"),
            &format!("Skill{n}"),
            "body",
        );
    }
    let scan = store.scan();
    for skill in &scan.skills {
        store.set_enabled(skill, true).unwrap();
    }
    let mut turn = store.for_turn();
    assert_eq!(turn.len(), MAX_ACTIVE);
    let id = turn.skills[0].id.clone();
    for _ in 0..16 {
        turn.execute("load_skill", &json!({"id":id}).to_string())
            .unwrap();
    }
    assert!(turn
        .execute("load_skill", &json!({"id":id}).to_string())
        .is_err());
    assert!(serde_json::to_vec(&turn.schemas()).unwrap().len() < 32768);
}
#[test]
fn corrupt_settings_never_overwrite_and_read_only_scan_never_saves() {
    let (dir, store) = fixture();
    write_skill(&store, "test/SKILL.md", "Test", "body");
    assert_eq!(store.scan().skills.len(), 1);
    assert!(!dir.path().join("settings.json").exists());
    fs::write(dir.path().join("settings.json"), "USER_INVALID_CONTENT").unwrap();
    let mut broken =
        SkillStore::load_at(dir.path().join("settings.json"), store.sources().to_vec());
    assert!(broken.load_error.is_some());
    assert!(broken.scan().skills.is_empty());
    assert!(broken.set_source_enabled(0, false).is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        "USER_INVALID_CONTENT"
    );
}
#[test]
fn project_sources_are_explicit_and_do_not_modify_other_tool_settings() {
    let (dir, mut store) = fixture();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("config.toml"), "untouched").unwrap();
    store.add_source(project.clone(), true).unwrap();
    assert_eq!(store.sources().len(), 4);
    assert_eq!(
        fs::read_to_string(project.join("config.toml")).unwrap(),
        "untouched"
    );
    assert!(store.add_source(PathBuf::from("relative"), false).is_err());
    assert!(store
        .add_source(project.join("config.toml"), false)
        .is_err());
}
#[test]
fn root_markdown_has_no_sibling_reference_access() {
    let (_dir, mut store) = fixture();
    write_skill(&store, "root.md", "Root", "body");
    let skill = store.scan().skills.remove(0);
    store.set_enabled(&skill, true).unwrap();
    let mut turn = store.for_turn();
    turn.execute("load_skill", &json!({"id":skill.id}).to_string())
        .unwrap();
    assert!(turn
        .execute(
            "read_skill_file",
            &json!({"id":skill.id,"path":"root.md"}).to_string()
        )
        .is_err());
}

#[test]
fn symlink_and_junction_are_not_discovery_or_read_capabilities() {
    let (dir, store, skill) = enabled_fixture();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.md"), "OUTSIDE_SECRET").unwrap();
    let link = skill.file.parent().unwrap().join("linked");
    #[cfg(windows)]
    {
        // A directory junction does not require Windows developer mode/admin.
        let status = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(status.status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let mut turn = store.for_turn();
    turn.execute("load_skill", &json!({"id":skill.id}).to_string())
        .unwrap();
    assert!(turn
        .execute(
            "read_skill_file",
            &json!({"id":skill.id,"path":"linked/secret.md"}).to_string()
        )
        .is_err());
    assert!(store.scan().notices.iter().any(|n| n.contains("連結")));
    // Remove just our own junction, never a recursive delete through its target.
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
    assert!(outside.join("secret.md").exists());
}

#[tokio::test]
async fn shared_catalog_cancels_and_redacts_skill_metadata_body_and_references() {
    let (_dir, mut store) = fixture();
    let token = "synthetic-skill-\"secret\\tail";
    write_skill(
        &store,
        "test/SKILL.md",
        token,
        &format!("BODY_ONLY {token}"),
    );
    let skill = store.scan().skills.remove(0);
    store.set_enabled(&skill, true).unwrap();
    fs::write(
        skill.file.parent().unwrap().join("note.md"),
        format!("REFERENCE {token}"),
    )
    .unwrap();
    let service = crate::chat::mcp::Service {
        id: Uuid::new_v4(),
        draft: crate::chat::mcp::Draft {
            name: "No connection".into(),
            transport: crate::chat::mcp::Transport::Http {
                url: "https://example.com/mcp".into(),
            },
            token_env: None,
        },
        enabled: false,
        allow_private_http: false,
        secret_ref: None,
    };
    let turn = McpTurn::new(
        vec![(service, token.into())],
        false,
        Uuid::new_v4(),
        Uuid::new_v4(),
        |_| panic!("local skills never start MCP"),
    )
    .with_skills(store.for_turn());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut catalog = turn.open(&tx).await.unwrap();
    assert!(catalog.contains("load_skill"));
    assert!(!serde_json::to_string(catalog.schemas())
        .unwrap()
        .contains("synthetic-skill-"));
    let body = catalog
        .execute("load_skill", &json!({"id":skill.id}).to_string(), &tx)
        .await;
    assert!(body.contains("BODY_ONLY"));
    assert!(!body.contains("synthetic-skill-"));
    let reference = catalog
        .execute(
            "read_skill_file",
            &json!({"id":skill.id,"path":"note.md"}).to_string(),
            &tx,
        )
        .await;
    assert!(reference.contains("REFERENCE"));
    assert!(!reference.contains("synthetic-skill-"));
    drop(rx);
    assert!(catalog
        .execute("load_skill", &json!({"id":skill.id}).to_string(), &tx)
        .await
        .contains("取消"));
    assert!(turn.open(&tx).await.is_err());
}

#[tokio::test]
async fn compatible_chat_performs_real_lazy_skill_tool_roundtrip_without_system_injection() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use wiremock::{matchers::method, Mock, MockServer, Request, ResponseTemplate};
    let (_dir, store, skill) = enabled_fixture();
    fs::write(
        skill.file.parent().unwrap().join("note.md"),
        "ROUNDTRIP_REFERENCE",
    )
    .unwrap();
    let model = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let id = skill.id.clone();
    Mock::given(method("POST")).respond_with(move|request:&Request| {
        let body:Value=request.body_json().unwrap();let round=seen.fetch_add(1,Ordering::SeqCst);
        assert_eq!(body["messages"][0]["content"],"KEEP_CUSTOM_SYSTEM");
        assert!(!serde_json::to_string(&body["tools"]).unwrap().contains("BODY_ONLY"));
        assert!(body["tools"].as_array().unwrap().iter().any(|t|t["function"]["name"]=="load_skill"));
        let chunk=if round==0 {
            assert!(!serde_json::to_string(&body["messages"]).unwrap().contains("BODY_ONLY"));
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"skill_call","type":"function","function":{"name":"load_skill","arguments":json!({"id":id}).to_string()}}]},"finish_reason":"tool_calls"}]})
        }else if round==1 {
            assert!(serde_json::to_string(&body["messages"]).unwrap().contains("BODY_ONLY_LOADED_ON_DEMAND"));
            assert!(!serde_json::to_string(&body["messages"]).unwrap().contains("ROUNDTRIP_REFERENCE"));
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"reference_call","type":"function","function":{"name":"read_skill_file","arguments":json!({"id":id,"path":"note.md"}).to_string()}}]},"finish_reason":"tool_calls"}]})
        }else{
            assert!(serde_json::to_string(&body["messages"]).unwrap().contains("BODY_ONLY_LOADED_ON_DEMAND"));
            assert!(serde_json::to_string(&body["messages"]).unwrap().contains("ROUNDTRIP_REFERENCE"));
            json!({"choices":[{"delta":{"content":"SKILL_SUCCESS"},"finish_reason":"stop"}]})
        };
        ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
    }).mount(&model).await;
    let config = ProviderConfig {
        kind: ProviderKind::OpenAICompatible,
        base_url: model.uri(),
        api_key: "fake".into(),
        model: "fixture".into(),
        reasoning_effort: None,
    };
    let turn = McpTurn::new(vec![], false, Uuid::new_v4(), Uuid::new_v4(), |_| {
        panic!("no permission needed outside opted-in skill")
    })
    .with_skill_store(store);
    let mut rx = client::run_with_mcp(
        vec![
            WireMessage::new(
                "system",
                Some(WireContent::Text("KEEP_CUSTOM_SYSTEM".into())),
                None,
                None,
            ),
            WireMessage::new(
                "user",
                Some(WireContent::Text("use test skill".into())),
                None,
                None,
            ),
        ],
        config,
        None,
        Some(turn),
    );
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(event) = rx.recv().await {
            match event {
                ChatStreamEvent::Done(text) => return text,
                ChatStreamEvent::Error(error) => panic!("{error}"),
                _ => {}
            }
        }
        panic!("no answer")
    })
    .await
    .unwrap();
    assert_eq!(result, "SKILL_SUCCESS");
    assert_eq!(count.load(Ordering::SeqCst), 3);
}
