//! File tool tests via Harness + fake Provider.

use std::sync::Arc;

use saku_harness::Harness;
use saku_harness::config::{Config, Effort};
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::tools::file_tools;
use saku_harness::types::{RunEvent, UserTurn};
use serde_json::json;
use tempfile::TempDir;

fn config(tmp: &TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    Config {
        discord_token: "t".into(),
        authorized_user_ids: vec!["1".into()],
        command_prefix: "saku".into(),
        workspace,
        data_dir,
        default_model: "gpt-5.5".into(),
        default_effort: Effort::Medium,
        web_backend: "exa".into(),
        release_channel: saku_harness::ReleaseChannel::Stable,
    }
}

async fn harness_with_files(fake: Arc<FakeProvider>, config: Config) -> Harness {
    let harness = Harness::new(config, fake).unwrap();
    for tool in file_tools() {
        harness.register_tool(tool).await;
    }
    harness
}

#[tokio::test]
async fn read_edit_write_happy_path() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let workspace = cfg.workspace.clone();
    std::fs::write(workspace.join("a.txt"), "hello").unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "read",
        json!({"path": "a.txt"}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "2",
        "edit",
        json!({"path": "a.txt", "old_string": "hello", "new_string": "hello world"}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "3",
        "write",
        json!({"path": "b.txt", "content": "new"}),
    )]));
    fake.push_text("done");

    let harness = harness_with_files(fake, cfg).await;
    let session = harness.session("t1").await.unwrap();
    let events = session
        .run(UserTurn::text("fix files"))
        .await
        .collect()
        .await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "read"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "edit"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "write"))
    );
    assert!(events.contains(&RunEvent::RunFinished));
    assert_eq!(
        std::fs::read_to_string(workspace.join("a.txt")).unwrap(),
        "hello world"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("b.txt")).unwrap(),
        "new"
    );
}

#[tokio::test]
async fn edit_without_read_fails() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    std::fs::write(cfg.workspace.join("a.txt"), "hello").unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "edit",
        json!({"path": "a.txt", "old_string": "hello", "new_string": "x"}),
    )]));
    fake.push_text("noted");

    let harness = harness_with_files(fake, cfg).await;
    let session = harness.session("t2").await.unwrap();
    let events = session.run(UserTurn::text("edit")).await.collect().await;
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "edit"
    )));
}

#[tokio::test]
async fn edit_fails_when_file_changed_since_read() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let file = cfg.workspace.join("a.txt");
    std::fs::write(&file, "hello").unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "read",
        json!({"path": "a.txt"}),
    )]));
    fake.push_text("read done");

    let harness = harness_with_files(fake.clone(), cfg).await;
    let session = harness.session("t3").await.unwrap();
    let _ = session.run(UserTurn::text("read")).await.collect().await;

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&file, "changed externally!").unwrap();

    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "2",
        "edit",
        json!({"path": "a.txt", "old_string": "changed", "new_string": "x"}),
    )]));
    fake.push_text("ok");
    let events = session.run(UserTurn::text("edit")).await.collect().await;
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "edit"
    )));
}

#[tokio::test]
async fn write_existing_without_snapshot_fails() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    std::fs::write(cfg.workspace.join("a.txt"), "old").unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "write",
        json!({"path": "a.txt", "content": "new"}),
    )]));
    fake.push_text("done");
    let harness = harness_with_files(fake, cfg).await;
    let session = harness.session("t4").await.unwrap();
    let events = session
        .run(UserTurn::text("overwrite"))
        .await
        .collect()
        .await;
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "write"
    )));
}
