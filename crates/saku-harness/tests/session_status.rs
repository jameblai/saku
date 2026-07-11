//! Session status: Run count, usage persistence, run state.

use std::io::Write;
use std::sync::Arc;

use saku_harness::config::{Config, Effort};
use saku_harness::provider::FakeProvider;
use saku_harness::status::RunState;
use saku_harness::types::{RunEvent, TokenUsage, UsageSource, UserTurn};
use saku_harness::{CODEX_PROVIDER_ID, Harness, format_status};
use tempfile::TempDir;

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
async fn run_count_and_usage_persist_across_replay() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_text_with_usage(
        "hi",
        TokenUsage {
            input: 100,
            output: 20,
            cache_read: 50,
            cache_write: 0,
        },
    );
    let harness = Harness::new(config(&tmp), fake.clone()).unwrap();
    let session = harness.session("status-1").await.unwrap();
    let events = session.run(UserTurn::text("hello")).await.collect().await;
    assert!(events.iter().any(|e| matches!(e, RunEvent::RunFinished)));

    let snap = session.snapshot().await;
    assert_eq!(snap.run_count, 1);
    assert_eq!(snap.usage.input, 100);
    assert_eq!(snap.usage.output, 20);
    assert_eq!(snap.usage.cache_read, 50);
    assert!(snap.estimated_cost_usd > 0.0);
    assert_eq!(snap.last_prompt_tokens, Some(150));
    assert_eq!(snap.usage_by_source.run.tokens, snap.usage);
    assert_eq!(snap.usage_records[0].source, UsageSource::Run);
    assert_eq!(snap.usage_records[0].model, "gpt-5.5");
    let jsonl = std::fs::read_to_string(tmp.path().join("data/sessions/status-1.jsonl")).unwrap();
    assert!(!jsonl.contains("cost_usd"));

    // Drop in-memory session map by creating a new Harness on the same data dir.
    let harness2 = Harness::new(config(&tmp), Arc::new(FakeProvider::new())).unwrap();
    let session2 = harness2.session("status-1").await.unwrap();
    let snap2 = session2.snapshot().await;
    assert_eq!(snap2.run_count, 1);
    assert_eq!(snap2.usage.input, 100);
    assert_eq!(snap2.usage.output, 20);
    assert_eq!(snap2.usage.cache_read, 50);
    assert!((snap2.estimated_cost_usd - snap.estimated_cost_usd).abs() < 1e-9);
    assert_eq!(snap2.usage_by_source, snap.usage_by_source);
    assert_eq!(snap2.usage_records, snap.usage_records);
}

#[tokio::test]
async fn usage_without_source_replays_as_run_with_the_model_active_at_that_point() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let harness = Harness::new(cfg.clone(), Arc::new(FakeProvider::new())).unwrap();
    let _ = harness.session("legacy-usage").await.unwrap();
    let path = cfg.data_dir.join("sessions/legacy-usage.jsonl");
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, r#"{{"type":"model_change","model":"gpt-5.4-mini"}}"#).unwrap();
    writeln!(
        file,
        r#"{{"type":"usage","input":7,"output":2,"cache_read":1,"cache_write":0}}"#
    )
    .unwrap();
    writeln!(
        file,
        r#"{{"type":"usage","input":99,"output":1,"cache_read":0,"cache_write":0,"source":"subagent","model":"gpt-5.4-mini"}}"#
    )
    .unwrap();

    let replay = Harness::new(cfg, Arc::new(FakeProvider::new())).unwrap();
    let snapshot = replay
        .session("legacy-usage")
        .await
        .unwrap()
        .snapshot()
        .await;
    assert_eq!(snapshot.usage_records[0].source, UsageSource::Run);
    assert_eq!(snapshot.usage_records[0].model, "gpt-5.4-mini");
    assert!(
        (snapshot.usage_by_source.run.estimated_cost_usd - 0.000014325).abs() < 1e-12,
        "cost must be derived from current rates"
    );
    assert_eq!(snapshot.usage_by_source.run.tokens.input, 7);
    assert_eq!(snapshot.usage_by_source.subagent.tokens.input, 99);
    assert_eq!(snapshot.last_prompt_tokens, Some(8));
}

#[tokio::test]
async fn aborted_run_still_increments_run_count() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_text("should abort");
    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("status-abort").await.unwrap();
    let handle = session.run(UserTurn::text("x")).await;
    session.stop().await;
    let _ = handle.collect().await;
    let snap = session.snapshot().await;
    assert_eq!(snap.run_count, 1);
}

#[tokio::test]
async fn run_state_reports_queued_depth() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_text("first");
    fake.push_text("second");
    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("status-q").await.unwrap();
    let mut first = session.run(UserTurn::text("a")).await;
    let _second = session.run(UserTurn::text("b")).await;
    // While first is active and second queued:
    let state = session.run_state().await;
    assert!(matches!(
        state,
        RunState::Running { waiting: 1 }
            | RunState::Running { waiting: 0 }
            | RunState::Queued { .. }
    ));
    while let Some(ev) = first.next_event().await {
        if matches!(ev, RunEvent::RunFinished) {
            break;
        }
    }
}

#[tokio::test]
async fn status_report_formats_session_and_config() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_text_with_usage(
        "ok",
        TokenUsage {
            input: 1_000,
            output: 0,
            cache_read: 0,
            cache_write: 0,
        },
    );
    let cfg = config(&tmp);
    let harness = Harness::new(cfg.clone(), fake).unwrap();
    let session = harness.session("status-fmt").await.unwrap();
    let _ = session.run(UserTurn::text("hi")).await.collect().await;
    let report = session
        .status_report(
            &cfg.default_model,
            cfg.default_effort,
            CODEX_PROVIDER_ID,
            None,
            Some("offline".into()),
            saku_harness::WebBackendStatus::NoCredential,
        )
        .await;
    let text = format_status(&report);
    assert!(text.contains("Runs: 1"));
    assert!(text.contains("Background: 0 running, 0 exited"));
    assert!(text.contains("Tokens: input 1000 / output 0 / cache read 0 / cache write 0"));
    assert!(text.contains("Provider: `codex`"));
    assert!(text.contains("Plan Usage unavailable: offline"));
    assert!(text.contains("Context:"));
    assert!(text.contains("**Tools**"));
    assert!(text.contains("**Web Backend**"));
    assert!(text.contains("Backend: `exa`"));
    assert!(text.contains("no Credential"));
}

#[tokio::test]
async fn status_report_lists_registered_tools_in_order() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    let cfg = config(&tmp);
    let harness = Harness::new(cfg.clone(), fake).unwrap();
    for tool in saku_harness::file_tools() {
        harness.register_tool(tool).await;
    }
    for tool in saku_harness::shell_tools() {
        harness.register_tool(tool).await;
    }
    let session = harness.session("status-tools").await.unwrap();
    let report = session
        .status_report(
            &cfg.default_model,
            cfg.default_effort,
            CODEX_PROVIDER_ID,
            None,
            None,
            saku_harness::WebBackendStatus::NoCredential,
        )
        .await;
    assert_eq!(
        report.tool_names,
        vec!["read", "edit", "write", "bash", "cd"]
    );
    let text = format_status(&report);
    assert!(text.contains("`read` `edit` `write` `bash` `cd`"));
}

#[tokio::test]
async fn status_report_lists_project_context_paths() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    std::fs::write(cfg.workspace.join("AGENTS.md"), "ws norms").unwrap();
    let fake = Arc::new(FakeProvider::new());
    let harness = Harness::new(cfg.clone(), fake).unwrap();
    let session = harness.session("status-agents").await.unwrap();
    let report = session
        .status_report(
            &cfg.default_model,
            cfg.default_effort,
            CODEX_PROVIDER_ID,
            None,
            None,
            saku_harness::WebBackendStatus::NoCredential,
        )
        .await;
    assert_eq!(
        report.project_context_paths,
        vec![cfg.workspace.join("AGENTS.md")]
    );
    let text = format_status(&report);
    assert!(text.contains(&format!(
        "- `{}`",
        cfg.workspace.join("AGENTS.md").display()
    )));
}
