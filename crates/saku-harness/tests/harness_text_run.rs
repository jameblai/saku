//! Harness integration tests: fake Provider + temp Workspace/Data Dir.

use std::sync::Arc;

use saku_harness::config::{Config, Effort};
use saku_harness::memory::memory_path;
use saku_harness::provider::FakeProvider;
use saku_harness::types::{RunEvent, UserTurn};
use saku_harness::Harness;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    Config {
        discord_token: "test".into(),
        authorized_user_ids: vec!["1".into()],
        command_prefix: "saku".into(),
        workspace,
        data_dir,
        default_model: "gpt-5.5".into(),
        default_effort: Effort::Medium,
    }
}

#[tokio::test]
async fn text_only_run_streams_events_and_persists_transcript() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push_text("Hello from Saku");
    let harness = Harness::new(config, fake.clone()).unwrap();

    let session = harness.session("thread-1").await.unwrap();
    let events = session.run(UserTurn::text("hi")).await.collect().await;

    assert!(events.contains(&RunEvent::RunStarted));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::TextDelta { text } if text.contains("Hello") || text.contains("Saku") || text.contains("from")))
    );
    assert!(events.contains(&RunEvent::AssistantFinished));
    assert!(events.contains(&RunEvent::RunFinished));

    let state = session.snapshot().await;
    assert_eq!(state.messages.len(), 2);
    assert!(
        state.messages[1]
            .content
            .iter()
            .any(|c| matches!(c, saku_harness::ContentPart::Text { text } if text == "Hello from Saku"))
    );

    // Replay from disk into a fresh Harness.
    let config2 = Config {
        discord_token: "test".into(),
        authorized_user_ids: vec!["1".into()],
        command_prefix: "saku".into(),
        workspace: tmp.path().join("workspace"),
        data_dir: tmp.path().join("data"),
        default_model: "gpt-5.5".into(),
        default_effort: Effort::Medium,
    };
    let harness2 = Harness::new(config2, Arc::new(FakeProvider::new())).unwrap();
    let session2 = harness2.session("thread-1").await.unwrap();
    let replayed = session2.snapshot().await;
    assert_eq!(replayed.messages.len(), 2);
}

#[tokio::test]
async fn system_prompt_includes_memory() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    std::fs::write(memory_path(&config.data_dir), "prefers concise answers").unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_text("ok");
    let harness = Harness::new(config, fake.clone()).unwrap();
    let session = harness.session("t-mem").await.unwrap();
    let _ = session.run(UserTurn::text("ping")).await.collect().await;

    let req = fake.last_request().expect("request recorded");
    assert!(req.system.contains("prefers concise answers"));
    assert!(req.system.contains("Workspace:"));
    assert!(req.system.contains("Working Directory:"));
}

#[tokio::test]
async fn provider_error_emits_run_error() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_error("boom");
    let harness = Harness::new(test_config(&tmp), fake).unwrap();
    let session = harness.session("t-err").await.unwrap();
    let events = session.run(UserTurn::text("x")).await.collect().await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::RunError { message } if message.contains("boom")))
    );
    assert!(!events.contains(&RunEvent::RunFinished));
}
