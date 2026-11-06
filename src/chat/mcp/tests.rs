use super::*;
use crate::chat::{
    client::{self, ChatStreamEvent, ProviderConfig, ProviderKind, WireContent, WireMessage},
    store::SecretStore,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::mpsc;
use uuid::Uuid;
use wiremock::{matchers::method, Mock, MockServer, Request, ResponseTemplate};

fn draft(url: &str) -> Draft {
    Draft {
        name: "Fixture MCP".into(),
        transport: Transport::Http { url: url.into() },
        token_env: None,
    }
}
fn service(url: &str) -> Service {
    Service {
        id: Uuid::new_v4(),
        draft: draft(url),
        enabled: true,
        allow_private_http: false,
        secret_ref: None,
    }
}
fn tools() -> Value {
    json!([{"name":"echo","description":"Untrusted test description","inputSchema":{"type":"object","properties":{"text":{"type":"string"},"optional_note":{"type":"string"}},"required":["text"]}}])
}

fn control_events(rx: &mut mpsc::UnboundedReceiver<ChatStreamEvent>) -> Vec<ChatStreamEvent> {
    std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|event| !matches!(event, ChatStreamEvent::ToolActivity(_)))
        .collect()
}

#[test]
fn private_http_requires_a_separate_persisted_destination_approval() {
    for url in [
        "http://10.0.0.12:2918/v1/mcp",
        "http://172.16.0.9/mcp",
        "http://192.168.1.9/mcp",
        "http://[fd00::1]/mcp",
    ] {
        let mut service = service(url);
        service.draft.validate().unwrap();
        assert!(service.draft.requires_private_http_approval());
        assert!(service.validate_connection().is_err());
        service.allow_private_http = true;
        service.validate_connection().unwrap();
    }
    for url in [
        "http://8.8.8.8/mcp",
        "http://public.example/mcp",
        "http://169.254.169.254/mcp",
    ] {
        assert!(draft(url).validate().is_err());
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.json");
    let secret_path = dir.path().join("secrets.json");
    let mut store = McpStore::load_at(path.clone());
    let mut secrets = SecretStore::isolated_file_for_tests(secret_path.clone());
    let candidate = draft("http://10.0.0.12/mcp");
    assert!(store
        .save(
            None,
            candidate.clone(),
            true,
            Some("synthetic-private-http-password"),
            false,
            &mut secrets
        )
        .is_err());
    assert!(!path.exists());
    assert!(!secret_path.exists());
    let id = store
        .save_with_http_permission(
            None,
            candidate,
            true,
            Some("synthetic-private-http-password"),
            false,
            true,
            &mut secrets,
        )
        .unwrap();
    assert!(McpStore::load_at(path.clone()).servers[0].allow_private_http);
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("synthetic-private-http-password"));
    let before = std::fs::read(&path).unwrap();
    assert!(store
        .save(
            Some(id),
            draft("http://10.0.0.13/mcp"),
            true,
            None,
            false,
            &mut secrets
        )
        .is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn saved_mcp_credentials_are_redacted_from_wire_history_without_rewriting_originals() {
    let token = "synthetic-previously-pasted-password";
    let turn = McpTurn::new(
        vec![(service("https://example.com/mcp"), token.into())],
        false,
        Uuid::new_v4(),
        Uuid::new_v4(),
        |_| panic!("redaction cannot execute"),
    );
    let original = vec![
        WireMessage::new(
            "user",
            Some(WireContent::Text(format!("old input included {token}"))),
            None,
            None,
        ),
        WireMessage::new(
            "user",
            Some(WireContent::Parts(vec![client::WirePart::text(format!(
                "part {token}"
            ))])),
            None,
            None,
        ),
    ];
    let mut outgoing = original.clone();
    turn.redact_history(&mut outgoing);
    assert!(!serde_json::to_string(&outgoing).unwrap().contains(token));
    assert!(serde_json::to_string(&original).unwrap().contains(token));
}

#[tokio::test]
async fn confirmed_local_setup_initializes_lists_and_calls_capabilities_with_a_local_only_token() {
    let mcp = MockServer::start().await;
    let sequence = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = sequence.clone();
    Mock::given(method("POST")).respond_with(move|req:&Request| {
        assert_eq!(req.headers.get("authorization").unwrap(),"Bearer synthetic-local-capabilities-password");
        let body:Value=req.body_json().unwrap();let method=body["method"].as_str().unwrap();seen.lock().unwrap().push(method.into());
        let result=match method {
            "initialize"=>json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}),
            "notifications/initialized"=>return ResponseTemplate::new(202),
            "tools/list"=>json!({"tools":[{"name":"armor_capabilities","description":"Synthetic capability fixture","inputSchema":{"type":"object","properties":{}}}]}),
            "tools/call"=>{assert_eq!(body["params"]["name"],"armor_capabilities");json!({"content":[{"type":"text","text":"synthetic capability report"}],"structuredContent":{"ocr_ready":true,"speech_ready":false}})},
            _=>panic!("unexpected method {method}"),
        };
        ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
    }).mount(&mcp).await;
    let setup = parse_chat_setup(&super::onboarding_fixture(
        &mcp.uri(),
        "synthetic-local-capabilities-password",
    ))
    .unwrap()
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut store = McpStore::load_at(dir.path().join("mcp.json"));
    let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
    let request = setup.request;
    store
        .save(
            None,
            setup.draft,
            true,
            Some(&setup.token),
            false,
            &mut secrets,
        )
        .unwrap();
    let model = MockServer::start().await;
    let rounds = Arc::new(AtomicUsize::new(0));
    let count = rounds.clone();
    Mock::given(method("POST")).respond_with(move|req:&Request| {
        let body:Value=req.body_json().unwrap();assert!(!body.to_string().contains("synthetic-local-capabilities-password"));
        let event=if count.fetch_add(1,Ordering::SeqCst)==0 {
            let tool=body["tools"].as_array().unwrap().iter().find(|tool|tool["function"]["description"].as_str().unwrap_or_default().contains("工具 armor_capabilities")).unwrap();
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"capabilities-1","function":{"name":tool["function"]["name"],"arguments":"{}"}}]}}]})
        } else {
            let output=body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap();assert!(output.contains("\"ocr_ready\":true"));assert!(output.contains("\"speech_ready\":false"));
            json!({"choices":[{"delta":{"content":"工具清單：armor_capabilities。測試結果：OCR 就緒，語音未就緒。"}}]})
        };
        ResponseTemplate::new(200).set_body_raw(format!("data: {event}\n\ndata: [DONE]\n\n"),"text/event-stream")
    }).mount(&model).await;
    let turn = McpTurn::from_store(&store, &secrets, Uuid::new_v4(), Uuid::new_v4(), |r| {
        assert_eq!(r.action, ApprovalAction::ToolCall);
        assert_eq!(r.tool, "armor_capabilities");
        r.respond.send(true).unwrap();
    })
    .unwrap()
    .unwrap();
    let config = ProviderConfig {
        base_url: model.uri(),
        api_key: String::new(),
        model: "fixture".into(),
        reasoning_effort: None,
        kind: ProviderKind::OpenAICompatible,
    };
    let mut rx = client::run_with_mcp(
        vec![WireMessage::new(
            "user",
            Some(WireContent::Text(request)),
            None,
            None,
        )],
        config,
        None,
        Some(turn),
    );
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut events = vec![];
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        events
    })
    .await
    .unwrap();
    assert_eq!(
        events.last(),
        Some(&ChatStreamEvent::Done(
            "工具清單：armor_capabilities。測試結果：OCR 就緒，語音未就緒。".into()
        ))
    );
    assert_eq!(rounds.load(Ordering::SeqCst), 2);
    assert_eq!(
        *sequence.lock().unwrap(),
        vec![
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/call"
        ]
    );
}

#[tokio::test]
async fn management_tools_are_available_without_services_and_do_not_change_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.json");
    let store = McpStore::load_at(path.clone());
    let secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
    let turn = McpTurn::from_store(&store, &secrets, Uuid::new_v4(), Uuid::new_v4(), |_| {
        panic!("local status/settings must not execute anything")
    })
    .unwrap()
    .unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut catalog = turn.open(&tx).await.unwrap();
    assert_eq!(catalog.schemas().len(), 3);
    for name in ["mcp_status", "open_mcp_settings", "test_mcp_service"] {
        assert!(catalog.contains(name));
    }
    assert!(!catalog.contains("propose_mcp_service"));
    let status: Value =
        serde_json::from_str(&catalog.execute("mcp_status", "{}", &tx).await).unwrap();
    assert_eq!(status["services"], json!([]));
    assert_eq!(status["config_readable"], true);
    assert_eq!(status["chat_proposals_enabled"], false);
    assert!(catalog
        .execute("open_mcp_settings", "{}", &tx)
        .await
        .contains("未保存"));
    assert_eq!(
        control_events(&mut rx),
        vec![ChatStreamEvent::McpOpenSettings(None)]
    );
    for (name, args) in [
        ("mcp_status", json!({"token":"must-not-be-accepted"})),
        ("open_mcp_settings", json!({"url":"https://example.com"})),
        ("test_mcp_service", json!({"service_id":Uuid::new_v4()})),
    ] {
        assert!(catalog
            .execute(name, &args.to_string(), &tx)
            .await
            .starts_with("ERROR:"));
    }
    assert!(control_events(&mut rx).is_empty());
    assert!(!path.exists());
    assert!(!dir.path().join("secrets.json").exists());
}

#[tokio::test]
async fn corrupt_config_still_exposes_read_only_diagnostics_and_never_repairs_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.json");
    std::fs::write(&path, b"invalid original config").unwrap();
    let store = McpStore::load_at(path.clone());
    let secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
    let turn = McpTurn::from_store(&store, &secrets, Uuid::new_v4(), Uuid::new_v4(), |_| {
        panic!("corrupt settings must not connect")
    })
    .unwrap()
    .unwrap();
    let (tx, _) = mpsc::unbounded_channel();
    let mut catalog = turn.open(&tx).await.unwrap();
    let status: Value =
        serde_json::from_str(&catalog.execute("mcp_status", "{}", &tx).await).unwrap();
    assert_eq!(status["config_readable"], false);
    assert_eq!(std::fs::read(path).unwrap(), b"invalid original config");
}

#[tokio::test]
async fn disabled_service_test_connects_only_after_approval_and_never_enables_or_calls_tools() {
    for approved in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = server(calls.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
        let mut store = McpStore::load_at(path.clone());
        let id = store
            .save(
                None,
                draft(&server.uri()),
                false,
                Some("synthetic-test-auth-only"),
                false,
                &mut secrets,
            )
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        let (approval_tx, mut approval_rx) = mpsc::unbounded_channel();
        let (tx, _rx) = mpsc::unbounded_channel();
        let turn =
            McpTurn::from_store(&store, &secrets, Uuid::new_v4(), Uuid::new_v4(), move |r| {
                let _ = approval_tx.send(r);
            })
            .unwrap()
            .unwrap();
        let mut catalog = turn.open(&tx).await.unwrap();
        let status = catalog.execute("mcp_status", "{}", &tx).await;
        assert!(!status.contains("synthetic-test-auth-only"));
        assert!(!status.contains("secret_ref"));
        assert_eq!(server.received_requests().await.unwrap().len(), 0);
        let task = tokio::spawn(async move {
            let result = catalog
                .execute(
                    "test_mcp_service",
                    &json!({"service_id":id}).to_string(),
                    &tx,
                )
                .await;
            (catalog, tx, result)
        });
        let request = approval_rx.recv().await.unwrap();
        assert_eq!(request.action, ApprovalAction::ConnectionTest);
        assert!(!request.service.enabled);
        assert_eq!(server.received_requests().await.unwrap().len(), 0);
        request.respond.send(approved).unwrap();
        let (mut catalog, tx, result) = task.await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!store.servers[0].enabled);
        if approved {
            let result: Value = serde_json::from_str(&result).unwrap();
            assert_eq!(result["connected"], true);
            assert_eq!(result["tool_names"], json!(["echo"]));
            assert_eq!(result["settings_changed"], false);
            for req in server.received_requests().await.unwrap() {
                assert_eq!(
                    req.headers.get("authorization").unwrap(),
                    "Bearer synthetic-test-auth-only"
                );
            }
        } else {
            assert!(result.contains("拒絕"));
            assert_eq!(server.received_requests().await.unwrap().len(), 0);
            assert!(catalog
                .execute(
                    "test_mcp_service",
                    &json!({"service_id":id}).to_string(),
                    &tx
                )
                .await
                .contains("不再詢問"));
            assert!(approval_rx.try_recv().is_err());
        }
        assert_eq!(
            catalog.schemas().len(),
            3,
            "test cannot enable tools in the current turn"
        );
    }
}

#[tokio::test]
async fn cancelling_connection_test_while_waiting_never_contacts_service() {
    let server = server(Arc::new(AtomicUsize::new(0))).await;
    let mut saved = service(&server.uri());
    saved.enabled = false;
    let id = saved.id;
    let (approval_tx, mut approval_rx) = mpsc::unbounded_channel();
    let (tx, _) = mpsc::unbounded_channel();
    let turn = McpTurn::new(
        vec![(saved, String::new())],
        false,
        Uuid::new_v4(),
        Uuid::new_v4(),
        move |r| {
            let _ = approval_tx.send(r);
        },
    );
    let mut catalog = turn.open(&tx).await.unwrap();
    let task = tokio::spawn(async move {
        catalog
            .execute(
                "test_mcp_service",
                &json!({"service_id":id}).to_string(),
                &tx,
            )
            .await
    });
    let mut request = approval_rx.recv().await.unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(std::time::Duration::from_secs(2), request.respond.closed())
        .await
        .unwrap();
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn failed_discovery_does_not_hide_management_tools_or_leak_server_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("synthetic-private-error"))
        .mount(&server)
        .await;
    let turn = McpTurn::new(
        vec![(service(&server.uri()), "synthetic-private-token".into())],
        false,
        Uuid::new_v4(),
        Uuid::new_v4(),
        |_| panic!("discovery cannot execute"),
    );
    let (tx, _) = mpsc::unbounded_channel();
    let mut catalog = turn.open(&tx).await.unwrap();
    assert_eq!(catalog.schemas().len(), 3);
    let status = catalog.execute("mcp_status", "{}", &tx).await;
    assert!(status.contains("\"discovery_failed\":true"));
    assert!(!status.contains("synthetic-private"));
}

#[tokio::test]
async fn actual_openai_chat_can_check_status_open_settings_and_request_approved_test() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mcp = server(calls.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let mut store = McpStore::load_at(dir.path().join("mcp.json"));
    let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
    let id = store
        .save(
            None,
            draft(&mcp.uri()),
            false,
            Some("synthetic-mcp-auth-not-for-model"),
            false,
            &mut secrets,
        )
        .unwrap();
    let model = MockServer::start().await;
    let rounds = Arc::new(AtomicUsize::new(0));
    let count = rounds.clone();
    Mock::given(method("POST")).respond_with(move|req:&Request| {
        let body:Value=req.body_json().unwrap();assert!(!body.to_string().contains("synthetic-mcp-auth-not-for-model"));
        let schemas=body["tools"].as_array().unwrap();for name in ["mcp_status","open_mcp_settings","test_mcp_service"] {assert!(schemas.iter().any(|t|t["function"]["name"]==name));}
        let round=count.fetch_add(1,Ordering::SeqCst);
        let (name,args)=match round {
            0=>("mcp_status",json!({})),
            1=>{let result:Value=serde_json::from_str(body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap()).unwrap();assert_eq!(result["services"][0]["enabled"],false);("open_mcp_settings",json!({"service_id":id}))},
            2=>("test_mcp_service",json!({"service_id":id})),
            3=>{let output=body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap();assert!(output.contains("\"connected\":true"));("",json!({}))},
            _=>panic!("unexpected model retry")
        };
        let event=if round==3 {json!({"choices":[{"delta":{"content":"已經你確認完成連線測試，服務仍停用。"}}]})} else {json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":format!("manage-{round}"),"function":{"name":name,"arguments":args.to_string()}}]}}]})};
        ResponseTemplate::new(200).set_body_raw(format!("data: {event}\n\ndata: [DONE]\n\n"),"text/event-stream")
    }).mount(&model).await;
    let turn = McpTurn::from_store(&store, &secrets, Uuid::new_v4(), Uuid::new_v4(), |r| {
        assert_eq!(r.action, ApprovalAction::ConnectionTest);
        r.respond.send(true).unwrap();
    })
    .unwrap()
    .unwrap();
    let config = ProviderConfig {
        base_url: model.uri(),
        api_key: String::new(),
        model: "fixture".into(),
        reasoning_effort: None,
        kind: ProviderKind::OpenAICompatible,
    };
    let mut rx = client::run_with_mcp(
        vec![WireMessage::new(
            "user",
            Some(WireContent::Text(
                "幫我查看 MCP 設定並測試已存服務；舊訊息曾包含 synthetic-mcp-auth-not-for-model"
                    .into(),
            )),
            None,
            None,
        )],
        config,
        None,
        Some(turn),
    );
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut events = vec![];
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        events
    })
    .await
    .unwrap();
    assert!(events.contains(&ChatStreamEvent::McpOpenSettings(Some(id))));
    assert_eq!(
        events.last(),
        Some(&ChatStreamEvent::Done(
            "已經你確認完成連線測試，服務仍停用。".into()
        ))
    );
    assert_eq!(rounds.load(Ordering::SeqCst), 4);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

async fn server(counter: Arc<AtomicUsize>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |request:&Request| {
        let body:Value=request.body_json().unwrap();
        let method=body["method"].as_str().unwrap_or_default();
        if method.starts_with("notifications/") || method.is_empty(){return ResponseTemplate::new(202);}
        let result=match method {
            "initialize"=>json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}),
            "tools/list"=>json!({"tools":tools()}),
            "tools/call"=>{counter.fetch_add(1,Ordering::SeqCst);json!({"content":[{"type":"text","text":format!("echo {}",body["params"]["arguments"]["text"])}]})},
            _=>panic!("unexpected fixture method"),
        };
        if method!="initialize" {
            assert_eq!(request.headers.get("mcp-session-id").unwrap(),"fixture-session");
            assert_eq!(request.headers.get("mcp-protocol-version").unwrap(),"2025-06-18");
        }
        assert!(request.headers.get("accept").unwrap().to_str().unwrap().contains("text/event-stream"));
        let response=ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}));
        if method=="initialize"{response.insert_header("Mcp-Session-Id","fixture-session")}else{response}
    }).mount(&server).await;
    server
}

#[test]
fn chat_add_is_explicit_validated_and_cannot_enable_or_smuggle_secret_fields() {
    let value = json!({"name":"Test","transport":{"type":"http","url":"https://example.com/mcp"}});
    let parsed = parse_chat_add(&format!("/mcp add {value}"))
        .unwrap()
        .unwrap();
    assert_eq!(parsed.name, "Test");
    assert!(parse_chat_add("an assistant said /mcp add anything")
        .unwrap()
        .is_none());
    assert!(parse_chat_add("/mcproot anything").unwrap().is_none());
    assert!(parse_chat_add(&format!("/mcp add {}",json!({"name":"Test","enabled":true,"transport":{"type":"http","url":"https://example.com/mcp"}}))).is_err());
    for value in [
        json!({"name":"Test","transport":{"type":"http","url":"https://example.com/mcp?token=synthetic"}}),
        json!({"name":"Test","transport":{"type":"http","url":"http://example.com/mcp"}}),
        json!({"name":"Test","transport":{"type":"http","url":"https://user:pass@example.com/mcp"}}),
        json!({"name":"Test","transport":{"type":"http","url":"https://example.com/mcp"},"headers":{"Authorization":"synthetic"}}),
        json!({"name":"Test","transport":{"type":"stdio","command":"unsafe.cmd","args":[]}}),
        json!({"name":"Test","transport":{"type":"stdio","command":"node.exe","args":[]},"token_env":"NODE_OPTIONS"}),
    ] {
        assert!(parse_chat_add(&format!("/mcp add {value}")).is_err());
    }
}

#[test]
fn store_persists_only_secret_references_and_failed_writes_never_replace_active_token() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.json");
    let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("fixture-secrets.json"));
    let mut store = McpStore::load_at(path.clone());
    let id = store
        .save(
            None,
            draft("https://example.com/mcp"),
            false,
            Some("synthetic-first-token"),
            false,
            &mut secrets,
        )
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&before).contains("synthetic-first-token"));
    assert!(!store.servers[0].enabled);
    let loaded = McpStore::load_at(path.clone());
    assert_eq!(loaded.servers, store.servers);
    assert_eq!(
        loaded.servers[0].token(&secrets).unwrap(),
        "synthetic-first-token"
    );
    assert!(
        store
            .save(
                None,
                draft("https://example.com/other"),
                true,
                None,
                false,
                &mut secrets
            )
            .is_err(),
        "same name must not overwrite"
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(store
        .save(
            Some(id),
            draft("https://example.com/updated"),
            true,
            Some("synthetic-replacement-token"),
            false,
            &mut secrets
        )
        .is_err());
    assert_eq!(
        store.servers[0].token(&secrets).unwrap(),
        "synthetic-first-token"
    );
    assert!(
        !std::fs::read_to_string(dir.path().join("fixture-secrets.json"))
            .unwrap()
            .contains("synthetic-replacement-token")
    );
}

#[test]
fn malformed_mcp_store_is_retained_and_cannot_be_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.json");
    std::fs::write(&path, b"user file not JSON").unwrap();
    let mut store = McpStore::load_at(path.clone());
    let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
    assert!(store.load_error.is_some());
    assert!(store
        .save(
            None,
            draft("https://example.com/mcp"),
            true,
            Some("synthetic"),
            false,
            &mut secrets
        )
        .is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"user file not JSON");
    assert!(!dir.path().join("secrets.json").exists());
}

#[tokio::test]
async fn http_lifecycle_negotiates_session_version_and_calls_tool_without_provider_credentials() {
    let calls = Arc::new(AtomicUsize::new(0));
    let server = server(calls.clone()).await;
    let mut session = Session::connect(&service(&server.uri()), "mcp-only-synthetic-token".into())
        .await
        .unwrap();
    assert_eq!(session.tools().await.unwrap().len(), 1);
    let result = session.call("echo", json!({"text":"hello"})).await.unwrap();
    assert!(result["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("hello"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    for request in server.received_requests().await.unwrap() {
        assert_eq!(
            request.headers.get("authorization").unwrap(),
            "Bearer mcp-only-synthetic-token"
        );
    }
}

#[test]
fn long_tokens_are_chunked_and_corrupt_or_missing_fragments_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
    let mut store = McpStore::load_at(dir.path().join("mcp.json"));
    let token = "synthetic-long-token-".repeat(500);
    store
        .save(
            None,
            draft("https://example.com/mcp"),
            false,
            Some(&token),
            false,
            &mut secrets,
        )
        .unwrap();
    let service = &store.servers[0];
    assert_eq!(service.token(&secrets).unwrap(), token);
    let id = service.secret_ref.unwrap();
    let header: Value =
        serde_json::from_str(&secrets.get(&format!("mcp-token-{id}")).unwrap()).unwrap();
    let count = header["chunks"].as_u64().unwrap();
    assert!(count > 1);
    for index in 0..count {
        assert!(
            secrets
                .get(&format!("mcp-token-{id}-{index}"))
                .unwrap()
                .encode_utf16()
                .count()
                <= 1000
        );
    }
    secrets.delete(&format!("mcp-token-{id}-0")).unwrap();
    assert!(service.token(&secrets).is_err());
}

#[tokio::test]
async fn server_metadata_cannot_leak_its_auth_token_to_the_model() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(|req:&Request| {
        let value:Value=req.body_json().unwrap();
        let result=match value["method"].as_str() {
            Some("initialize")=>json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}),
            Some("tools/list")=>json!({"tools":[{"name":"echo","description":"contains synthetic-private-token","inputSchema":{"type":"object"}}]}),
            _=>return ResponseTemplate::new(202),
        };
        ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":value["id"],"result":result}))
    }).mount(&server).await;
    let turn = McpTurn::new(
        vec![(service(&server.uri()), "synthetic-private-token".into())],
        false,
        Uuid::new_v4(),
        Uuid::new_v4(),
        |_| panic!("discovery must not execute tool"),
    );
    let (tx, _) = mpsc::unbounded_channel();
    let mut catalog = turn.open(&tx).await.unwrap();
    assert_eq!(
        catalog.schemas().len(),
        3,
        "only local management tools, no leaked remote schema"
    );
    let status = catalog.execute("mcp_status", "{}", &tx).await;
    assert!(status.contains("\"discovery_failed\":true"));
    assert!(!status.contains("synthetic-private-token"));
}

#[test]
fn credentials_are_not_serialized_as_draft_fields_and_nested_results_are_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp.json");
    let mut secrets = SecretStore::isolated_file_for_tests(dir.path().join("secrets.json"));
    let mut store = McpStore::load_at(path.clone());
    let mut candidate = draft("https://example.com/mcp");
    candidate.name = "MCP synthetic-private-token".into();
    assert!(store
        .save(
            None,
            candidate,
            false,
            Some("synthetic-private-token"),
            false,
            &mut secrets
        )
        .is_err());
    assert!(!path.exists());
    assert!(!dir.path().join("secrets.json").exists());
    let token = "synthetic\"quoted\\token";
    let mut value = json!({});
    value[token] = json!([{"nested":format!("data {token}")}]);
    assert!(super::config::contains_secret(&value, token));
    super::config::redact_secret(&mut value, token);
    assert!(!super::config::contains_secret(&value, token));
    assert!(value.to_string().contains("REDACTED"));
}

#[tokio::test]
async fn dropping_turn_while_awaiting_approval_never_executes_the_pending_tool() {
    let calls = Arc::new(AtomicUsize::new(0));
    let server = server(calls.clone()).await;
    let (tx, _rx) = mpsc::unbounded_channel();
    let (approval_tx, mut approval_rx) = mpsc::unbounded_channel();
    let turn = McpTurn::new(
        vec![(service(&server.uri()), String::new())],
        false,
        Uuid::new_v4(),
        Uuid::new_v4(),
        move |r| {
            let _ = approval_tx.send(r);
        },
    );
    let mut catalog = turn.open(&tx).await.unwrap();
    let name = catalog.schemas()[0]["function"]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let task = tokio::spawn(async move {
        catalog
            .execute(&name, r#"{"text":"must not execute"}"#, &tx)
            .await
    });
    let mut approval = approval_rx.recv().await.unwrap();
    task.abort();
    let _ = task.await;
    tokio::time::timeout(std::time::Duration::from_secs(2), approval.respond.closed())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn responses_actual_tool_loop_supports_mcp_without_native_credentials_or_web_tools() {
    use crate::chat::{
        chatgpt_auth::{TokenStore, Tokens},
        responses::ResponsesBackend,
    };
    struct MemoryTokens;
    impl TokenStore for MemoryTokens {
        fn load(&self) -> Option<Tokens> {
            Some(Tokens {
                access_token: "synthetic-model-access".into(),
                refresh_token: String::new(),
                id_token: String::new(),
                account_id: "synthetic-account".into(),
                expires_at: u64::MAX,
                email: None,
                plan: None,
            })
        }
        fn save(&self, _: &Tokens) -> anyhow::Result<()> {
            panic!("must not refresh test token")
        }
        fn delete(&self) -> anyhow::Result<()> {
            panic!("must not delete test token")
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mcp = server(calls.clone()).await;
    let model = MockServer::start().await;
    let rounds = Arc::new(AtomicUsize::new(0));
    let count = rounds.clone();
    Mock::given(method("POST")).respond_with(move |req:&Request| {
        let body:Value=req.body_json().unwrap();let round=count.fetch_add(1,Ordering::SeqCst);
        let events=if round==0 {
            assert!(body["tools"].as_array().unwrap().iter().all(|t|t["strict"]==false));
            for name in ["mcp_status","open_mcp_settings","test_mcp_service"] {assert!(body["tools"].as_array().unwrap().iter().any(|t|t["name"]==name));}
            vec![json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"mcp-status-call","name":"mcp_status","arguments":"{}"}}),json!({"type":"response.completed"})]
        } else if round==1 {
            let last=body["input"].as_array().unwrap().last().unwrap();assert_eq!(last["call_id"],"mcp-status-call");
            let status:Value=serde_json::from_str(last["output"].as_str().unwrap()).unwrap();assert_eq!(status["services"][0]["enabled"],true);assert_eq!(status["services"][0]["tools_available_this_turn"],true);
            let name=body["tools"][0]["name"].as_str().unwrap();assert_eq!(body["tools"][0]["type"],"function");
            assert_eq!(body["tools"][0]["parameters"]["required"],json!(["text"]));assert_eq!(body["tools"][0]["parameters"]["properties"]["optional_note"]["type"],"string");
            assert!(body["tools"].as_array().unwrap().iter().all(|t|t["strict"]==false));
            vec![json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"mcp-response-call","name":name,"arguments":"{\"text\":\"responses echo\"}"}}),json!({"type":"response.completed"})]
        } else {
            let last=body["input"].as_array().unwrap().last().unwrap();assert_eq!(last["type"],"function_call_output");assert_eq!(last["call_id"],"mcp-response-call");assert!(last["output"].as_str().unwrap().contains("responses echo"));
            vec![json!({"type":"response.output_text.delta","delta":"Responses MCP answer"}),json!({"type":"response.completed"})]
        };
        ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string(events.iter().map(|e|format!("data: {e}\n\n")).collect::<String>())
    }).mount(&model).await;
    let backend = ResponsesBackend::with_endpoints(
        Arc::new(MemoryTokens),
        model.uri(),
        format!("{}/unused-refresh", model.uri()),
    );
    let turn = McpTurn::new(
        vec![(service(&mcp.uri()), String::new())],
        false,
        Uuid::new_v4(),
        Uuid::new_v4(),
        |r| {
            r.respond.send(true).unwrap();
        },
    );
    let config = ProviderConfig {
        base_url: String::new(),
        api_key: String::new(),
        model: "fixture".into(),
        reasoning_effort: None,
        kind: ProviderKind::ChatGPT,
    };
    let mut rx = backend.start_with_mcp(
        vec![WireMessage::new(
            "user",
            Some(WireContent::Text("use echo".into())),
            None,
            None,
        )],
        config,
        None,
        Some(turn),
    );
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut all = vec![];
        while let Some(e) = rx.recv().await {
            all.push(e);
        }
        all
    })
    .await
    .unwrap();
    assert_eq!(
        events.last(),
        Some(&ChatStreamEvent::Done("Responses MCP answer".into()))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(rounds.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn tool_call_requires_explicit_approval_and_denial_executes_nothing() {
    use crate::chat::activity::{ActivityKind, ActivityState};
    for approved in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = server(calls.clone()).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (approval_tx, mut approval_rx) = mpsc::unbounded_channel();
        let service = service(&server.uri());
        let id = service.id;
        let turn = McpTurn::new(
            vec![(service, String::new())],
            false,
            Uuid::new_v4(),
            Uuid::new_v4(),
            move |r| {
                let _ = approval_tx.send(r);
            },
        );
        let mut catalog = turn.open(&tx).await.unwrap();
        let mut discovery = vec![];
        while let Ok(ChatStreamEvent::ToolActivity(activity)) = rx.try_recv() {
            discovery.push(activity);
        }
        assert_eq!(discovery.len(), 2);
        assert_eq!(discovery[0].id, discovery[1].id);
        assert_eq!(discovery[1].kind, ActivityKind::McpDiscovery);
        assert_eq!(discovery[1].state, ActivityState::Succeeded);
        assert!(discovery[1]
            .detail
            .as_deref()
            .unwrap()
            .contains("尚未執行工具"));
        let name = catalog.schemas()[0]["function"]["name"]
            .as_str()
            .unwrap()
            .to_string();
        let handle = tokio::spawn(async move {
            catalog
                .execute(&name, r#"{"text":"reviewed arguments"}"#, &tx)
                .await
        });
        let request = approval_rx.recv().await.unwrap();
        assert_eq!(request.service.id, id);
        assert_eq!(request.tool, "echo");
        assert_eq!(request.arguments["text"], "reviewed arguments");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let mut lifecycle = vec![];
        while let Ok(ChatStreamEvent::ToolActivity(activity)) = rx.try_recv() {
            assert_eq!(activity.label, "MCP／Fixture MCP／echo");
            lifecycle.push(activity);
        }
        assert_eq!(
            lifecycle.last().unwrap().state,
            ActivityState::WaitingApproval
        );
        request.respond.send(approved).unwrap();
        let result = handle.await.unwrap();
        while let Ok(ChatStreamEvent::ToolActivity(activity)) = rx.try_recv() {
            lifecycle.push(activity);
        }
        assert!(lifecycle.iter().all(|a| a.id == lifecycle[0].id));
        assert_eq!(
            lifecycle.last().unwrap().state,
            if approved {
                ActivityState::Succeeded
            } else {
                ActivityState::Denied
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(approved));
        assert!(if approved {
            result.contains("reviewed arguments")
        } else {
            result.contains("拒絕")
        });
    }
}

#[tokio::test]
async fn conversational_proposal_only_emits_review_event_and_never_connects() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let turn = McpTurn::new(vec![], true, Uuid::new_v4(), Uuid::new_v4(), |_| {
        panic!("proposal must not ask to execute tool")
    });
    let mut catalog = turn.open(&tx).await.unwrap();
    assert_eq!(
        catalog.schemas()[0]["function"]["name"],
        "propose_mcp_service"
    );
    let value = json!({"name":"Draft only","transport":{"type":"stdio","command":"this-program-must-never-run.exe","args":[]}});
    let result = catalog
        .execute("propose_mcp_service", &value.to_string(), &tx)
        .await;
    assert!(result.contains("尚未儲存"));
    assert!(
        matches!(control_events(&mut rx).as_slice(), [ChatStreamEvent::McpProposal(d)] if d.name == "Draft only")
    );
    let malicious = json!({"name":"bad","enabled":true,"transport":{"type":"http","url":"https://example.com/mcp"}});
    assert!(catalog
        .execute("propose_mcp_service", &malicious.to_string(), &tx)
        .await
        .starts_with("ERROR:"));
    assert!(control_events(&mut rx).is_empty());
}

#[tokio::test]
async fn json_sse_multi_line_response_and_server_requests_are_handled_without_sampling() {
    let server = MockServer::start().await;
    let replies = Arc::new(AtomicUsize::new(0));
    let count = replies.clone();
    Mock::given(method("POST")).respond_with(move |req:&Request| {
        let value:Value=req.body_json().unwrap();
        match value["method"].as_str() {
            Some("initialize")=>ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":value["id"],"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}})),
            Some("notifications/initialized")=>ResponseTemplate::new(202),
            Some("tools/list")=>{
                let reply=serde_json::to_string_pretty(&json!({"jsonrpc":"2.0","id":value["id"],"result":{"tools":tools()}})).unwrap();
                let data=reply.lines().map(|line|format!("data: {line}\n")).collect::<String>();
                ResponseTemplate::new(200).set_body_raw(format!("data: {}\n\n{data}\n",json!({"jsonrpc":"2.0","id":"sampling-request","method":"sampling/createMessage","params":{}})),"text/event-stream")
            },
            None=>{assert_eq!(value["id"],"sampling-request");assert_eq!(value["error"]["code"],-32601);count.fetch_add(1,Ordering::SeqCst);ResponseTemplate::new(202)},
            _=>ResponseTemplate::new(202),
        }
    }).mount(&server).await;
    let mut session = Session::connect(&service(&server.uri()), String::new())
        .await
        .unwrap();
    assert_eq!(session.tools().await.unwrap().len(), 1);
    assert_eq!(replies.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn expired_session_redirect_and_oversized_response_never_replay_a_tool() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(move |req:&Request| {
        let value:Value=req.body_json().unwrap();
        match value["method"].as_str() {
            Some("initialize")=>ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":value["id"],"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}}}})),
            Some("tools/call")=>{count.fetch_add(1,Ordering::SeqCst);ResponseTemplate::new(404)},
            _=>ResponseTemplate::new(202),
        }
    }).mount(&server).await;
    let mut session = Session::connect(&service(&server.uri()), String::new())
        .await
        .unwrap();
    assert!(session.call("echo", json!({})).await.is_err());
    assert!(session.call("echo", json!({})).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let target = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&target)
        .await;
    let redirect = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", target.uri()))
        .mount(&redirect)
        .await;
    assert!(
        Session::connect(&service(&redirect.uri()), "synthetic-auth".into())
            .await
            .is_err()
    );
    let huge = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string("x".repeat(1024 * 1024 + 1)),
        )
        .mount(&huge)
        .await;
    assert!(Session::connect(&service(&huge.uri()), String::new())
        .await
        .is_err());
}

#[tokio::test]
async fn natural_language_model_proposal_round_trip_never_contacts_the_proposed_server() {
    let target = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&target)
        .await;
    let proposed =
        json!({"name":"User requested notes","transport":{"type":"http","url":target.uri()}});
    let model = MockServer::start().await;
    let rounds = Arc::new(AtomicUsize::new(0));
    let counter = rounds.clone();
    let expected = proposed.clone();
    Mock::given(method("POST")).respond_with(move |req:&Request| {
        let body:Value=req.body_json().unwrap();
        let event=if counter.fetch_add(1,Ordering::SeqCst)==0 {
            assert_eq!(body["tools"][0]["function"]["name"],"propose_mcp_service");
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"proposal-1","function":{"name":"propose_mcp_service","arguments":expected.to_string()}}]}}]})
        } else {
            let last=body["messages"].as_array().unwrap().last().unwrap();assert_eq!(last["role"],"tool");assert!(last["content"].as_str().unwrap().contains("尚未儲存"));
            json!({"choices":[{"delta":{"content":"草稿待你確認，尚未連線。"}}]})
        };
        ResponseTemplate::new(200).set_body_raw(format!("data: {event}\n\ndata: [DONE]\n\n"),"text/event-stream")
    }).mount(&model).await;
    let turn = McpTurn::new(vec![], true, Uuid::new_v4(), Uuid::new_v4(), |_| {
        panic!("proposal must not execute")
    });
    let config = ProviderConfig {
        base_url: model.uri(),
        api_key: String::new(),
        model: "fixture".into(),
        reasoning_effort: None,
        kind: ProviderKind::OpenAICompatible,
    };
    let mut rx = client::run_with_mcp(
        vec![WireMessage::new(
            "user",
            Some(WireContent::Text("請幫我新增這個筆記 MCP 服務".into())),
            None,
            None,
        )],
        config,
        None,
        Some(turn),
    );
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut all = vec![];
        while let Some(event) = rx.recv().await {
            all.push(event);
        }
        all
    })
    .await
    .unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e,ChatStreamEvent::McpProposal(d) if d.name=="User requested notes")));
    assert_eq!(
        events.last(),
        Some(&ChatStreamEvent::Done("草稿待你確認，尚未連線。".into()))
    );
    assert_eq!(rounds.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn openai_actual_tool_loop_routes_mcp_results_back_to_model_after_approval() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mcp = server(calls.clone()).await;
    let model = MockServer::start().await;
    let rounds = Arc::new(AtomicUsize::new(0));
    let count = rounds.clone();
    Mock::given(method("POST")).respond_with(move |req:&Request| {
        let body:Value=req.body_json().unwrap();let round=count.fetch_add(1,Ordering::SeqCst);
        let event=if round==0 {
            let name=body["tools"][0]["function"]["name"].as_str().unwrap();
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"mcp-call-1","function":{"name":name,"arguments":"{\"text\":\"hello from model\"}"}}]}}]})
        } else {
            let last=body["messages"].as_array().unwrap().last().unwrap();assert_eq!(last["role"],"tool");assert_eq!(last["tool_call_id"],"mcp-call-1");assert!(last["content"].as_str().unwrap().contains("hello from model"));
            json!({"choices":[{"delta":{"content":"MCP final answer"}}]})
        };
        ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string(format!("data: {event}\n\ndata: [DONE]\n\n"))
    }).mount(&model).await;
    let turn = McpTurn::new(
        vec![(service(&mcp.uri()), String::new())],
        false,
        Uuid::new_v4(),
        Uuid::new_v4(),
        |request| {
            request.respond.send(true).unwrap();
        },
    );
    let config = ProviderConfig {
        base_url: model.uri(),
        api_key: "model-only-token".into(),
        model: "fixture".into(),
        reasoning_effort: None,
        kind: ProviderKind::OpenAICompatible,
    };
    let mut rx = client::run_with_mcp(
        vec![WireMessage::new(
            "user",
            Some(WireContent::Text("use echo".into())),
            None,
            None,
        )],
        config,
        None,
        Some(turn),
    );
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut all = vec![];
        while let Some(e) = rx.recv().await {
            all.push(e);
        }
        all
    })
    .await
    .unwrap();
    assert_eq!(
        events.last(),
        Some(&ChatStreamEvent::Done("MCP final answer".into()))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(rounds.load(Ordering::SeqCst), 2);
    for req in mcp.received_requests().await.unwrap() {
        assert!(req.headers.get("authorization").is_none());
    }
}

#[cfg(windows)]
#[tokio::test]
async fn real_stdio_protocol_env_token_and_owned_process_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fixture.ps1");
    std::fs::write(&script,r#"while ($null -ne ($line = [Console]::ReadLine())) {
 $request = $line | ConvertFrom-Json
 if ($request.method -eq 'notifications/initialized') { continue }
 $result = switch ($request.method) {
 'initialize' { @{ protocolVersion='2025-11-25'; capabilities=@{tools=@{}} } }
 'tools/list' { @{ tools=@(@{name='echo';inputSchema=@{type='object';properties=@{}}}) } }
 'tools/call' { @{ content=@(@{type='text';text='fixture response'}); structuredContent=@{pid=$PID;token=[Environment]::GetEnvironmentVariable('MCP_FIXTURE_TOKEN')} } }
 default { @{} }
 }
 [Console]::WriteLine((@{jsonrpc='2.0';id=$request.id;result=$result} | ConvertTo-Json -Depth 12 -Compress))
}"#).unwrap();
    let service = Service {
        id: Uuid::new_v4(),
        draft: Draft {
            name: "stdio fixture".into(),
            transport: Transport::Stdio {
                command: "powershell.exe".into(),
                args: vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-ExecutionPolicy".into(),
                    "Bypass".into(),
                    "-File".into(),
                    script.to_string_lossy().into_owned(),
                ],
            },
            token_env: Some("MCP_FIXTURE_TOKEN".into()),
        },
        enabled: true,
        allow_private_http: false,
        secret_ref: None,
    };
    let mut session = Session::connect(&service, "synthetic-stdio-token".into())
        .await
        .unwrap();
    assert_eq!(session.tools().await.unwrap().len(), 1);
    let result = session.call("echo", json!({})).await.unwrap();
    assert_eq!(
        result["structuredContent"]["token"],
        "synthetic-stdio-token"
    );
    let pid = result["structuredContent"]["pid"].as_u64().unwrap() as u32;
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
        fn CloseHandle(handle: isize) -> i32;
    }
    let handle = unsafe { OpenProcess(0x0010_0000, 0, pid) };
    assert_ne!(handle, 0);
    drop(session);
    assert_eq!(
        unsafe { WaitForSingleObject(handle as *mut _, 5000) },
        0,
        "owned MCP child survived session drop"
    );
    unsafe {
        CloseHandle(handle);
    }
}
