//! Memory tool tests via Harness + fake Provider.

use std::sync::Arc;

use saku_harness::Harness;
use saku_harness::config::{Config, Effort};
use saku_harness::memory::{MEMORY_CHAR_LIMIT, memory_path, read_memory};
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::tools::{file_tools, memory_tools};
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

async fn harness_with_memory(fake: Arc<FakeProvider>, config: Config) -> Harness {
    let harness = Harness::new(config, fake).unwrap();
    for tool in memory_tools() {
        harness.register_tool(tool).await;
    }
    harness
}

async fn harness_with_files_and_memory(fake: Arc<FakeProvider>, config: Config) -> Harness {
    let harness = Harness::new(config, fake).unwrap();
    for tool in file_tools() {
        harness.register_tool(tool).await;
    }
    for tool in memory_tools() {
        harness.register_tool(tool).await;
    }
    harness
}

#[tokio::test]
async fn memory_tool_writes_and_injects_on_later_run() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let data_dir = cfg.data_dir.clone();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "memory",
        json!({"content": "prefers cargo; name is Ada"}),
    )]));
    fake.push_text("saved");

    let harness = harness_with_memory(fake.clone(), cfg).await;
    let session = harness.session("t-mem-write").await.unwrap();
    let events = session
        .run(UserTurn::text("remember my prefs"))
        .await
        .collect()
        .await;

    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: true } if name == "memory"
    )));
    assert_eq!(
        read_memory(&data_dir).unwrap(),
        "prefers cargo; name is Ada"
    );

    fake.push_text("ok");
    let _ = session.run(UserTurn::text("ping")).await.collect().await;
    let req = fake.last_request().expect("request recorded");
    assert!(req.system.contains("prefers cargo; name is Ada"));
}

#[tokio::test]
async fn memory_tool_rejects_over_cap_and_leaves_prior_unchanged() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let data_dir = cfg.data_dir.clone();
    std::fs::write(memory_path(&data_dir), "keep me").unwrap();

    let over: String = "x".repeat(MEMORY_CHAR_LIMIT + 1);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "memory",
        json!({"content": over}),
    )]));
    fake.push_text("noted");

    let harness = harness_with_memory(fake, cfg).await;
    let session = harness.session("t-mem-cap").await.unwrap();
    let events = session
        .run(UserTurn::text("remember too much"))
        .await
        .collect()
        .await;

    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "memory"
    )));
    assert_eq!(read_memory(&data_dir).unwrap(), "keep me");
}

#[tokio::test]
async fn memory_tool_empty_content_clears_memory() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let data_dir = cfg.data_dir.clone();
    std::fs::write(memory_path(&data_dir), "stale fact").unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "memory",
        json!({"content": ""}),
    )]));
    fake.push_text("cleared");

    let harness = harness_with_memory(fake.clone(), cfg).await;
    let session = harness.session("t-mem-clear").await.unwrap();
    let events = session
        .run(UserTurn::text("forget everything"))
        .await
        .collect()
        .await;

    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: true } if name == "memory"
    )));
    assert_eq!(read_memory(&data_dir).unwrap(), "");

    fake.push_text("ok");
    let _ = session.run(UserTurn::text("ping")).await.collect().await;
    let req = fake.last_request().expect("request recorded");
    assert!(req.system.contains("(empty)"));
}

#[tokio::test]
async fn file_tools_cannot_touch_memory_path() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let memory = memory_path(&cfg.data_dir);
    std::fs::write(&memory, "secret prefs").unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "read",
        json!({"path": memory.to_string_lossy()}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "2",
        "write",
        json!({"path": memory.to_string_lossy(), "content": "hijack"}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "3",
        "edit",
        json!({
            "path": memory.to_string_lossy(),
            "old_string": "secret",
            "new_string": "hijack"
        }),
    )]));
    fake.push_text("done");

    let harness = harness_with_files_and_memory(fake, cfg).await;
    let session = harness.session("t-mem-jail").await.unwrap();
    let events = session
        .run(UserTurn::text("try memory via files"))
        .await
        .collect()
        .await;

    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "read"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "write"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "edit"
    )));
    assert_eq!(std::fs::read_to_string(&memory).unwrap(), "secret prefs");
}
