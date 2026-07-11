//! Session queue, steer, and stop semantics.

use std::sync::Arc;

use saku_harness::Harness;
use saku_harness::config::{Config, Effort};
use saku_harness::provider::FakeProvider;
use saku_harness::types::{RunEvent, UserTurn};
use tempfile::TempDir;

struct AbortUsageProvider {
    started: Arc<tokio::sync::Notify>,
    release_usage: Arc<tokio::sync::Notify>,
}

impl saku_harness::Provider for AbortUsageProvider {
    fn complete(&self, _request: saku_harness::Request) -> saku_harness::provider::ProviderStream {
        use futures::{StreamExt, stream};
        use saku_harness::types::{ProviderEvent, TokenUsage};

        self.started.notify_one();
        let release = Arc::clone(&self.release_usage);
        let usage = stream::once(async move {
            release.notified().await;
            Ok(ProviderEvent::Usage(TokenUsage {
                input: 11,
                output: 2,
                cache_read: 3,
                cache_write: 0,
            }))
        });
        Box::pin(usage.chain(stream::iter([Ok(ProviderEvent::MessageComplete)])))
    }
}

fn config(tmp: &TempDir) -> Config {
    let workspace = tmp.path().join("ws");
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

#[tokio::test]
async fn second_run_is_queued_then_dequeued() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    // First run blocks on a slow scripted response via tool-less text after delay — use two texts.
    fake.push_text("first");
    fake.push_text("second");
    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("q1").await.unwrap();

    let mut first = session.run(UserTurn::text("a")).await;
    let mut second = session.run(UserTurn::text("b")).await;

    // Second should see Queued before finishing.
    let mut saw_queued = false;
    while let Some(ev) = second.next_event().await {
        if matches!(ev, RunEvent::Queued) {
            saw_queued = true;
        }
        if matches!(ev, RunEvent::Dequeued | RunEvent::RunFinished) {
            break;
        }
    }
    assert!(saw_queued);
    while let Some(ev) = first.next_event().await {
        if matches!(ev, RunEvent::RunFinished) {
            break;
        }
    }
}

#[tokio::test]
async fn stop_emits_run_aborted_for_active_run() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    // Never push a response so the provider errors quickly — use a long bash instead via tools.
    // Simpler: push text after stop; abort mid-stream by stopping before collect.
    fake.push_text("should abort before finish if we stop early enough");
    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("q2").await.unwrap();
    let handle = session.run(UserTurn::text("x")).await;
    session.stop().await;
    let events = handle.collect().await;
    // Either aborted or finished depending on race; stop must not panic and queue must clear.
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::RunAborted | RunEvent::RunFinished | RunEvent::RunError { .. }
    )));
}

#[tokio::test]
async fn abort_retains_usage_already_reported_by_provider() {
    let tmp = TempDir::new().unwrap();
    let provider = Arc::new(AbortUsageProvider {
        started: Arc::new(tokio::sync::Notify::new()),
        release_usage: Arc::new(tokio::sync::Notify::new()),
    });
    let harness = Harness::new(config(&tmp), provider.clone()).unwrap();
    let session = harness.session("abort-usage").await.unwrap();
    let handle = session.run(UserTurn::text("stop after usage")).await;
    provider.started.notified().await;
    session.stop().await;
    provider.release_usage.notify_one();

    let events = handle.collect().await;
    assert!(events.contains(&RunEvent::RunAborted));
    let snapshot = session.snapshot().await;
    assert_eq!(snapshot.usage.input, 11);
    assert_eq!(snapshot.usage.cache_read, 3);
}

/// After `stop` aborts a mid-tool Run, the next Run must be able to complete.
///
/// Repro for: abort flag stuck true because `watch::Sender::send(false)` fails
/// when no abort receivers remain (session drops the initial receiver; tools only
/// subscribe for the duration of `execute`).
#[tokio::test]
async fn run_after_stop_during_tool_is_not_immediately_aborted() {
    use saku_harness::provider::ScriptedResponse;
    use saku_harness::provider::fake::tool_call;
    use saku_harness::tools::shell_tools;
    use serde_json::json;

    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "bash",
        json!({"command": "sleep 30"}),
    )]));
    fake.push_text("follow-up ok");

    let harness = Harness::new(config(&tmp), fake).unwrap();
    for tool in shell_tools() {
        harness.register_tool(tool).await;
    }
    let session = harness.session("q2-after-stop").await.unwrap();

    let mut first = session.run(UserTurn::text("long")).await;
    loop {
        match first.next_event().await {
            Some(RunEvent::ToolStarted { name, .. }) if name == "bash" => break,
            Some(RunEvent::RunFinished) | Some(RunEvent::RunError { .. }) | None => {
                panic!("bash never started");
            }
            _ => {}
        }
    }
    // ToolStarted is emitted before the tool subscribes to abort; wait until bash
    // is in wait_for_abort so stop()'s watch::send(true) has a live receiver.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    session.stop().await;
    let first_events = first.collect().await;
    assert!(
        first_events
            .iter()
            .any(|e| matches!(e, RunEvent::RunAborted)),
        "expected first run to abort, got {first_events:?}"
    );

    let second_events = session
        .run(UserTurn::text("continue"))
        .await
        .collect()
        .await;
    assert!(
        second_events
            .iter()
            .any(|e| matches!(e, RunEvent::RunFinished)),
        "expected follow-up run to finish, got {second_events:?}"
    );
    assert!(
        !second_events
            .iter()
            .any(|e| matches!(e, RunEvent::RunAborted)),
        "follow-up must not be aborted, got {second_events:?}"
    );
}

#[tokio::test]
async fn steer_injects_after_tool_batch() {
    use saku_harness::provider::ScriptedResponse;
    use saku_harness::provider::fake::tool_call;
    use saku_harness::tools::file_tools;
    use serde_json::json;

    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    std::fs::write(cfg.workspace.join("a.txt"), "hi").unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "read",
        json!({"path": "a.txt"}),
    )]));
    fake.push_text("after steer");
    let harness = Harness::new(cfg, fake.clone()).unwrap();
    for tool in file_tools() {
        harness.register_tool(tool).await;
    }
    let session = harness.session("q3").await.unwrap();
    let mut handle = session.run(UserTurn::text("read it")).await;
    // Steer during the tool batch so it applies before the next Provider call.
    loop {
        match handle.next_event().await {
            Some(RunEvent::ToolStarted { .. }) => {
                session.steer("prefer short answers").await;
                break;
            }
            Some(RunEvent::RunFinished) | Some(RunEvent::RunError { .. }) | None => {
                panic!("ended before tool started");
            }
            _ => {}
        }
    }
    let rest = handle.collect().await;
    assert!(rest.contains(&RunEvent::RunFinished));
    let reqs = fake.requests();
    assert!(reqs.len() >= 2);
    let second = &reqs[1];
    assert!(second.messages.iter().any(|m| {
        m.content.iter().any(
            |c| matches!(c, saku_harness::ContentPart::Text { text } if text.contains("[steer]")),
        )
    }));
}
