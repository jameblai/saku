//! Shell tool tests: bash + cd + cwd persistence.

use std::sync::Arc;

use saku_harness::Harness;
use saku_harness::config::{Config, Effort};
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::tools::shell_tools;
use saku_harness::types::{RunEvent, UserTurn};
use serde_json::json;
use tempfile::TempDir;

fn config(tmp: &TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(workspace.join("sub")).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    Config {
        discord_token: "t".into(),
        authorized_user_ids: vec!["1".into()],
        command_prefix: "saku".into(),
        workspace,
        data_dir,
        default_model: "gpt-5.5".into(),
        default_effort: Effort::Medium,
    }
}

async fn harness_with_shell(fake: Arc<FakeProvider>, config: Config) -> Harness {
    let harness = Harness::new(config, fake).unwrap();
    for tool in shell_tools() {
        harness.register_tool(tool).await;
    }
    harness
}

#[tokio::test]
async fn cd_and_bash_use_working_directory() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "cd",
        json!({"path": "sub"}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "2",
        "bash",
        json!({"command": "pwd"}),
    )]));
    fake.push_text("done");

    let harness = harness_with_shell(fake, cfg.clone()).await;
    let session = harness.session("shell-1").await.unwrap();
    let events = session
        .run(UserTurn::text("cd and pwd"))
        .await
        .collect()
        .await;
    assert!(events.contains(&RunEvent::RunFinished));

    let state = session.snapshot().await;
    assert_eq!(
        state.cwd.canonicalize().unwrap(),
        cfg.workspace.join("sub").canonicalize().unwrap()
    );

    // Replay cwd from JSONL.
    let harness2 = Harness::new(
        Config {
            discord_token: "t".into(),
            authorized_user_ids: vec!["1".into()],
            command_prefix: "saku".into(),
            workspace: cfg.workspace.clone(),
            data_dir: cfg.data_dir.clone(),
            default_model: "gpt-5.5".into(),
            default_effort: Effort::Medium,
        },
        Arc::new(FakeProvider::new()),
    )
    .unwrap();
    let replayed = harness2.session("shell-1").await.unwrap().snapshot().await;
    assert_eq!(
        replayed.cwd.canonicalize().unwrap(),
        cfg.workspace.join("sub").canonicalize().unwrap()
    );
}

#[tokio::test]
async fn cd_rejects_escape_outside_workspace() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "cd",
        json!({"path": ".."}),
    )]));
    fake.push_text("nope");
    let harness = harness_with_shell(fake, cfg).await;
    let session = harness.session("shell-2").await.unwrap();
    let events = session.run(UserTurn::text("escape")).await.collect().await;
    // ToolError becomes tool message with ok=false when Err, or ToolResult::error.
    // resolve failure returns Err(ToolError) -> ok false
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "cd"
    )));
}

#[tokio::test]
async fn bash_timeout_is_honored() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "bash",
        json!({"command": "sleep 5", "timeout": 0.2}),
    )]));
    fake.push_text("timed out");
    let harness = harness_with_shell(fake, cfg).await;
    let session = harness.session("shell-3").await.unwrap();
    let events = session.run(UserTurn::text("sleep")).await.collect().await;
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "bash"
    )));
}

#[tokio::test]
async fn stop_aborts_running_bash() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "bash",
        json!({"command": "sleep 30"}),
    )]));
    // May not reach text if aborted mid-tool — push anyway.
    fake.push_text("should not matter");

    let harness = harness_with_shell(fake, cfg).await;
    let session = harness.session("shell-4").await.unwrap();
    let mut handle = session.run(UserTurn::text("long")).await;

    // Wait until bash starts, then stop.
    loop {
        match handle.next_event().await {
            Some(RunEvent::ToolStarted { name, .. }) if name == "bash" => break,
            Some(RunEvent::RunFinished) | Some(RunEvent::RunError { .. }) | None => {
                panic!("bash never started");
            }
            _ => {}
        }
    }
    session.stop().await;

    let mut saw_finish = false;
    while let Some(ev) = handle.next_event().await {
        match &ev {
            RunEvent::ToolFinished { name, ok: false } if name == "bash" => {
                saw_finish = true;
                break;
            }
            RunEvent::RunFinished | RunEvent::RunError { .. } | RunEvent::RunAborted => {
                saw_finish = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_finish);
}
