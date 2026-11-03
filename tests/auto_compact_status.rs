//! Actual store -> compaction -> mock HTTP -> checkpoint persistence coverage.
//! Synthetic isolated data only; no real provider, credential or GUI is used.

use popchat_windows::chat::{
    providers::{Provider, ProviderKind, ProviderStore},
    store::{
        ChatMessage, ChatRole, ChatStore, ConversationStore, SecretBackend, SecretStore,
        SendOptions, SendOutcome, ShortcutStore,
    },
};
use uuid::Uuid;

#[tokio::test]
async fn actual_send_prepares_model_budget_and_compacts_without_destroying_transcript() {
    // This integration-test executable has exactly one test: the environment
    // override cannot race another test in this process. Never load live data.
    let base = std::env::temp_dir().join("opencode");
    std::fs::create_dir_all(&base).unwrap();
    let directory = tempfile::tempdir_in(base).unwrap();
    let previous = std::env::var_os("POPCHAT_DATA_DIR");
    std::env::set_var("POPCHAT_DATA_DIR", directory.path());
    struct RestoreEnv(Option<std::ffi::OsString>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("POPCHAT_DATA_DIR", value),
                None => std::env::remove_var("POPCHAT_DATA_DIR"),
            }
        }
    }
    let _restore = RestoreEnv(previous);
    let secrets = SecretStore::with_backend(SecretBackend::File);
    let mut providers = ProviderStore::load(&secrets);
    let server = wiremock::MockServer::start().await;
    let provider = Provider::new(
        Uuid::new_v4(),
        "synthetic compact diagnostic",
        server.uri(),
        false,
        "fixture-model",
        ProviderKind::OpenAICompatible,
        None,
    );
    let provider_id = provider.id;
    providers.providers.push(provider);
    providers.select(provider_id, "fixture-model", None);
    let shortcuts = ShortcutStore { shortcuts: vec![] };
    let options = SendOptions {
        system_prompt: "SYSTEM_SENTINEL: preserve instructions".into(),
        web_search_enabled: false,
        ..Default::default()
    };
    let mut messages = Vec::new();
    for index in 0..40 {
        messages.push(ChatMessage::new(
            ChatRole::User,
            format!("OLD_USER_{index}: {}", "合成歷史 abc 123 ".repeat(200)),
        ));
        messages.push(ChatMessage::new(
            ChatRole::Assistant,
            format!(
                "OLD_ASSISTANT_{index}: {}",
                "synthetic response ".repeat(200)
            ),
        ));
    }
    let original = serde_json::to_value(&messages).unwrap();
    let mut baseline = None;
    for limit in [1_000_000, 1_000, 1] {
        providers
            .set_context_limit(provider_id, "fixture-model", Some(limit))
            .unwrap();
        // Confirm the preference really survives a reload before testing send.
        let mut reloaded = ProviderStore::load(&secrets);
        assert_eq!(reloaded.current_context_limit(), limit);
        let mut chat = ChatStore::new();
        chat.messages = messages.clone();
        let SendOutcome::Ready(outgoing) = chat.send(
            "LATEST_USER_SENTINEL",
            vec![],
            &mut reloaded,
            &shortcuts,
            &secrets,
            &options,
        ) else {
            panic!("synthetic send unexpectedly aborted");
        };
        assert_eq!(outgoing.history.len(), 82); // system + 80 old + latest
        assert_eq!(outgoing.context.limit, limit);
        assert_eq!(outgoing.context.provider_id, provider_id);
        let wire = serde_json::to_value(&outgoing.history).unwrap();
        assert_eq!(wire[0]["content"], options.system_prompt);
        for (index, message) in messages.iter().enumerate() {
            assert_eq!(wire[index + 1]["content"], message.text);
        }
        assert_eq!(wire[81]["content"], "LATEST_USER_SENTINEL");
        assert_eq!(
            serde_json::to_value(&chat.messages[..messages.len()]).unwrap(),
            original,
        );
        if let Some(expected) = &baseline {
            assert_eq!(&wire, expected, "budget changed outgoing history");
        } else {
            baseline = Some(wire);
        }
        println!(
            "SEND_BUDGET_PREPARED budget={limit} original_wire_messages={} transcript_preserved=true",
            outgoing.history.len(),
        );
        // Do not pass OutgoingTurn to a backend: no network or real API use.
    }

    use popchat_windows::chat::{client::ChatStreamEvent, compaction};
    use wiremock::{matchers::method, Mock, Request, ResponseTemplate};
    Mock::given(method("POST")).respond_with(|request:&Request| {
        let body:serde_json::Value=request.body_json().unwrap();
        let summary=body["messages"][0]["content"].as_str().unwrap_or_default().starts_with("將舊對話整理成");
        let answer=if summary {"Checkpoint: synthetic old facts kept."} else {"MOCK_FINAL_ANSWER"};
        ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string(format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",serde_json::json!({"choices":[{"delta":{"content":answer}}]}),serde_json::json!({"choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":10}})))
    }).mount(&server).await;
    providers
        .set_context_limit(provider_id, "fixture-model", Some(64_000))
        .unwrap();
    let mut chat = ChatStore::new();
    chat.messages = messages.clone();
    let SendOutcome::Ready(mut outgoing) = chat.send(
        "LATEST_USER_SENTINEL",
        vec![],
        &mut providers,
        &shortcuts,
        &secrets,
        &options,
    ) else {
        panic!("send failed")
    };
    let mut stream = compaction::run(
        outgoing.history,
        outgoing.config,
        outgoing.web_access,
        outgoing.context,
    );
    while let Some(event) = stream.recv().await {
        assert!(!matches!(&event, ChatStreamEvent::Error(_)), "{event:?}");
        chat.apply_event(&mut outgoing.turn, event, &mut providers);
    }
    chat.finish_turn(&outgoing.turn);
    assert!(chat.context.checkpoint.is_some());
    assert!(chat.context.measurement.is_some());
    assert_eq!(
        serde_json::to_value(&chat.messages[..messages.len()]).unwrap(),
        original
    );
    assert_eq!(chat.messages.last().unwrap().text, "MOCK_FINAL_ANSWER");
    let saved = ConversationStore::load(chat.conversation_id).unwrap();
    assert_eq!(saved.context, chat.context);
    assert_eq!(
        serde_json::to_value(&saved.messages[..messages.len()]).unwrap(),
        original
    );
    let resumed = ChatStore::new();
    assert_eq!(resumed.context, chat.context);
    let mut legacy = serde_json::to_value(&saved).unwrap();
    legacy.as_object_mut().unwrap().remove("context");
    let legacy: popchat_windows::chat::store::Conversation =
        serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.context, Default::default());
    chat.fork(messages[3].id);
    assert_eq!(chat.context, Default::default());
    chat.new_chat();
    assert_eq!(chat.context, Default::default());
    // A delayed event from an old/aborted turn cannot contaminate a new chat.
    chat.apply_event(
        &mut outgoing.turn,
        ChatStreamEvent::ContextUpdate(saved.context),
        &mut providers,
    );
    assert_eq!(chat.context, Default::default());

    // A checkpoint write failure rolls back context, keeps the raw transcript,
    // and surfaces a visible error instead of silently claiming durability.
    chat.messages = messages[..2].to_vec();
    let SendOutcome::Ready(mut outgoing) = chat.send(
        "write failure fixture",
        vec![],
        &mut providers,
        &shortcuts,
        &secrets,
        &options,
    ) else {
        panic!("send failed")
    };
    let file = directory
        .path()
        .join("conversations")
        .join(format!("{}.json", chat.conversation_id));
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let raw_before = serde_json::to_value(&chat.messages[..2]).unwrap();
    let mut state = compaction::ContextState::default();
    state.measurement = Some(compaction::Measurement {
        provider_id,
        model: "fixture-model".into(),
        request_messages: outgoing.history.len(),
        request_hash: compaction::fingerprint(&outgoing.history),
        usage: compaction::TokenUsage {
            input_tokens: 10,
            output_tokens: 1,
        },
    });
    chat.apply_event(
        &mut outgoing.turn,
        ChatStreamEvent::ContextUpdate(state),
        &mut providers,
    );
    assert_eq!(chat.context, Default::default());
    assert_eq!(
        serde_json::to_value(&chat.messages[..2]).unwrap(),
        raw_before
    );
    assert!(chat
        .messages
        .iter()
        .any(|m| m.role == ChatRole::Error && m.text.contains("未能保存")));
}
