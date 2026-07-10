//! Background Process tools: bg_start / bg_list / bg_logs / bg_stop (ADR 0019).

use std::sync::Arc;
use std::time::Duration;

use saku_harness::Harness;
use saku_harness::config::{Config, Effort};
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::tools::background_tools;
use saku_harness::types::{ContentPart, RunEvent, UserTurn};
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

async fn harness_with_bg(fake: Arc<FakeProvider>, config: Config) -> Harness {
    let harness = Harness::new(config, fake).unwrap();
    for tool in background_tools() {
        harness.register_tool(tool).await;
    }
    harness
}

fn last_tool_text(messages: &[saku_harness::types::Message]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == saku_harness::types::Role::Tool)
        .and_then(|m| m.content.first())
        .and_then(|c| match c {
            ContentPart::Text { text } => Some(text.clone()),
            _ => None,
        })
        .expect("tool result text")
}

fn parse_pid(text: &str) -> u32 {
    text.lines()
        .find_map(|line| {
            line.strip_prefix("pid: ")
                .and_then(|s| s.trim().parse().ok())
        })
        .unwrap_or_else(|| panic!("no pid in: {text}"))
}

fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn wait_for_logs(session: &saku_harness::Session, pid: u32, needle: &str) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let logs = session.bg_logs_text(pid, Some(50)).await.unwrap();
        if logs.contains(needle) {
            return logs;
        }
        if tokio::time::Instant::now() >= deadline {
            return logs;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn bg_start_returns_pid_and_survives_run() {
    let _lock = bg_test_lock().await;
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "bg_start",
        json!({"command": "sleep 60", "settle": 0.05}),
    )]));
    fake.push_text("done");

    let harness = harness_with_bg(fake, config(&tmp)).await;
    let session = harness.session("bg-1").await.unwrap();
    let events = session
        .run(UserTurn::text("start sleep"))
        .await
        .collect()
        .await;
    assert!(events.contains(&RunEvent::RunFinished));
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: true } if name == "bg_start"
    )));

    let pid = parse_pid(&last_tool_text(&session.snapshot().await.messages));
    assert!(
        process_alive(pid),
        "background process {pid} should still be alive after RunFinished"
    );

    let _ = session.bg_stop(Some(pid)).await;
}

#[tokio::test]
async fn bg_start_enforces_running_cap_of_five() {
    let _lock = bg_test_lock().await;
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    for i in 1..=5 {
        fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
            &format!("s{i}"),
            "bg_start",
            json!({"command": "sleep 60", "settle": 0.02}),
        )]));
    }
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "s6",
        "bg_start",
        json!({"command": "sleep 60", "settle": 0.02}),
    )]));
    fake.push_text("done");

    let harness = harness_with_bg(fake, config(&tmp)).await;
    let session = harness.session("bg-cap").await.unwrap();
    let events = session.run(UserTurn::text("cap")).await.collect().await;
    assert!(events.contains(&RunEvent::RunFinished));

    let starts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            RunEvent::ToolFinished { name, ok } if name == "bg_start" => Some(*ok),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 6);
    assert_eq!(&starts[..5], &[true, true, true, true, true]);
    assert!(!starts[5]);

    let _ = session.bg_stop(None).await;
}

#[tokio::test]
async fn session_stop_does_not_kill_background_process() {
    let _lock = bg_test_lock().await;
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "bg_start",
        json!({"command": "sleep 60", "settle": 0.05}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "2",
        "bg_start",
        json!({"command": "sleep 60", "settle": 2.0}),
    )]));
    fake.push_text("should abort");

    let harness = harness_with_bg(fake, config(&tmp)).await;
    let session = harness.session("bg-stop-run").await.unwrap();
    let mut handle = session.run(UserTurn::text("long")).await;

    let pid;
    loop {
        match handle.next_event().await {
            Some(RunEvent::ToolFinished { name, ok: true }) if name == "bg_start" => {
                pid = parse_pid(&last_tool_text(&session.snapshot().await.messages));
                break;
            }
            Some(RunEvent::RunFinished) | Some(RunEvent::RunError { .. }) | None => {
                panic!("first bg_start never finished");
            }
            _ => {}
        }
    }
    assert!(process_alive(pid));

    session.stop().await;
    let _ = handle.collect().await;

    assert!(
        process_alive(pid),
        "saku stop / Session::stop must not kill Background Processes"
    );
    let _ = session.bg_stop(Some(pid)).await;
}

#[tokio::test]
async fn bg_list_logs_stop_and_exited_code() {
    let _lock = bg_test_lock().await;
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "bg_start",
        json!({"command": "bash -c 'echo hello-bg; sleep 60'", "settle": 0.2}),
    )]));
    fake.push_text("started");

    let harness = harness_with_bg(fake, config(&tmp)).await;
    let session = harness.session("bg-ctrl").await.unwrap();
    let events = session.run(UserTurn::text("start")).await.collect().await;
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: true } if name == "bg_start"
    )));
    let pid = parse_pid(&last_tool_text(&session.snapshot().await.messages));

    let list = session.bg_list_text().await;
    assert!(list.contains(&pid.to_string()));
    assert!(list.to_lowercase().contains("running"));

    let logs = wait_for_logs(&session, pid, "hello-bg").await;
    assert!(
        logs.contains("hello-bg"),
        "expected hello-bg in logs, got: {logs}"
    );

    let stop = session.bg_stop(Some(pid)).await.unwrap();
    assert!(stop.contains(&pid.to_string()));

    tokio::time::sleep(Duration::from_millis(300)).await;
    let list_after = session.bg_list_text().await;
    assert!(list_after.contains(&pid.to_string()));
    assert!(
        list_after.to_lowercase().contains("exited"),
        "exited entry should remain listable: {list_after}"
    );
    assert!(
        list_after.contains("exited (") && list_after.contains(')'),
        "exited entry should include exit code: {list_after}"
    );
    assert!(!process_alive(pid));
}

/// Remap this process's stdin to a pipe so inherited stdin is observably non-null.
/// Restores the previous stdin fd on drop.
///
/// Holds [`STDIN_REMAP_LOCK`] for the remap lifetime so parallel tests do not observe
/// a temporarily swapped stdin.
struct RemapStdinToPipe {
    saved_fd: i32,
    write_end: i32,
    _lock: std::sync::MutexGuard<'static, ()>,
}

static STDIN_REMAP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Serialize integration tests that spawn real Background Processes so CI
/// runners do not starve their stdout reader tasks under full-suite load.
static BG_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn bg_test_lock() -> tokio::sync::MutexGuard<'static, ()> {
    BG_TEST_LOCK.lock().await
}

impl RemapStdinToPipe {
    fn new() -> Self {
        let lock = STDIN_REMAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let mut fds = [0i32; 2];
            assert_eq!(libc::pipe(fds.as_mut_ptr()), 0, "pipe");
            let saved = libc::dup(libc::STDIN_FILENO);
            assert!(saved >= 0, "dup stdin");
            assert_eq!(
                libc::dup2(fds[0], libc::STDIN_FILENO),
                0,
                "dup2 pipe->stdin"
            );
            libc::close(fds[0]);
            Self {
                saved_fd: saved,
                write_end: fds[1],
                _lock: lock,
            }
        }
    }
}

impl Drop for RemapStdinToPipe {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::dup2(self.saved_fd, libc::STDIN_FILENO);
            libc::close(self.saved_fd);
            libc::close(self.write_end);
        }
    }
}

#[tokio::test]
async fn bg_start_uses_null_stdin() {
    let _lock = bg_test_lock().await;
    // Without Stdio::null(), the child would inherit this pipe and fail the assertion.
    let _stdin = RemapStdinToPipe::new();

    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "bg_start",
        json!({
            "command": "bash -c 'echo STDIN:$(readlink /proc/self/fd/0); IFS= read -r _ || echo READ_EOF; sleep 30'",
            "settle": 0.3
        }),
    )]));
    fake.push_text("done");

    let harness = harness_with_bg(fake, config(&tmp)).await;
    let session = harness.session("bg-stdin").await.unwrap();
    let _ = session.run(UserTurn::text("start")).await.collect().await;
    let pid = parse_pid(&last_tool_text(&session.snapshot().await.messages));

    let logs = wait_for_logs(&session, pid, "READ_EOF").await;
    assert!(
        logs.contains("STDIN:/dev/null"),
        "Background Process stdin must be /dev/null (not inherited), got: {logs}"
    );
    assert!(
        logs.contains("READ_EOF"),
        "null stdin should yield EOF on read, got: {logs}"
    );
    assert!(process_alive(pid));

    session.bg_stop(Some(pid)).await.unwrap();
}

#[tokio::test]
async fn bg_stop_kills_process_group_children() {
    let _lock = bg_test_lock().await;
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "bg_start",
        json!({
            "command": "bash -c 'sleep 120 & echo CHILD:$!; wait'",
            "settle": 0.25
        }),
    )]));
    fake.push_text("started");

    let harness = harness_with_bg(fake, config(&tmp)).await;
    let session = harness.session("bg-pg").await.unwrap();
    let _ = session.run(UserTurn::text("pg")).await.collect().await;
    let leader = parse_pid(&last_tool_text(&session.snapshot().await.messages));

    let logs = wait_for_logs(&session, leader, "CHILD:").await;
    let child_pid: u32 = logs
        .lines()
        .find_map(|l| l.strip_prefix("CHILD:").and_then(|s| s.trim().parse().ok()))
        .unwrap_or_else(|| panic!("no CHILD pid in logs: {logs}"));

    assert!(process_alive(leader));
    assert!(
        process_alive(child_pid),
        "child should be alive before stop"
    );

    session.bg_stop(Some(leader)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(!process_alive(leader));
    assert!(
        !process_alive(child_pid),
        "bg_stop must kill the process group (no orphaned children)"
    );
}

#[tokio::test]
async fn bg_list_logs_stop_tools_work_in_runs() {
    let _lock = bg_test_lock().await;
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "a",
        "bg_start",
        json!({"command": "bash -c 'echo tool-log; sleep 30'", "settle": 0.2}),
    )]));
    fake.push_text("started");

    let harness = harness_with_bg(fake.clone(), config(&tmp)).await;
    let session = harness.session("bg-tools-run").await.unwrap();
    let _ = session.run(UserTurn::text("start")).await.collect().await;
    let pid = parse_pid(&last_tool_text(&session.snapshot().await.messages));

    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "b",
        "bg_list",
        json!({}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "c",
        "bg_logs",
        json!({"pid": pid, "lines": 50}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "d",
        "bg_stop",
        json!({"pid": pid}),
    )]));
    fake.push_text("done");

    let logs = wait_for_logs(&session, pid, "tool-log").await;
    assert!(
        logs.contains("tool-log"),
        "expected tool-log before tool Run: {logs}"
    );

    let events = session
        .run(UserTurn::text("list logs stop"))
        .await
        .collect()
        .await;
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: true } if name == "bg_list"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: true } if name == "bg_logs"
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: true } if name == "bg_stop"
    )));
}
