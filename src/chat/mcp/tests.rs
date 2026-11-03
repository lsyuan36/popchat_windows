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
        secret_ref: None,
    }
}
fn tools() -> Value {
    json!([{"name":"echo","description":"Untrusted test description","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}])
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
    assert!(turn.open(&tx).await.is_err());
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
            let name=body["tools"][0]["name"].as_str().unwrap();assert_eq!(body["tools"][0]["type"],"function");
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
    assert_eq!(rounds.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn tool_call_requires_explicit_approval_and_denial_executes_nothing() {
    for approved in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = server(calls.clone()).await;
        let (tx, _rx) = mpsc::unbounded_channel();
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
        request.respond.send(approved).unwrap();
        let result = handle.await.unwrap();
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
        matches!(rx.try_recv().unwrap(),ChatStreamEvent::McpProposal(d) if d.name=="Draft only")
    );
    let malicious = json!({"name":"bad","enabled":true,"transport":{"type":"http","url":"https://example.com/mcp"}});
    assert!(catalog
        .execute("propose_mcp_service", &malicious.to_string(), &tx)
        .await
        .starts_with("ERROR:"));
    assert!(rx.try_recv().is_err());
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
