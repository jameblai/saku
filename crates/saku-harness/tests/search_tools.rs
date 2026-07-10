//! Search tool tests with a small fixture tree.

use std::sync::Arc;

use saku_harness::Harness;
use saku_harness::config::{Config, Effort};
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::tools::search_tools;
use saku_harness::types::{ContentPart, RunEvent, UserTurn};
use serde_json::json;
use tempfile::TempDir;

fn config(tmp: &TempDir) -> Config {
    let workspace = tmp.path().join("workspace");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(
        workspace.join("src/main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();
    std::fs::write(workspace.join("README.md"), "hello saku\n").unwrap();
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

async fn harness_with_search(fake: Arc<FakeProvider>, config: Config) -> Harness {
    let harness = Harness::new(config, fake).unwrap();
    for tool in search_tools(Arc::clone(harness.index())) {
        harness.register_tool(tool).await;
    }
    harness
}

fn tool_text(events: &[RunEvent], tool: &str) -> Option<String> {
    // Tool results are in session messages; assert via finished ok + second round text.
    let _ = tool;
    let _ = events;
    None
}

#[tokio::test]
async fn find_grep_ls_via_fake_provider() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "find",
        json!({"pattern": "main"}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "2",
        "grep",
        json!({"pattern": "println"}),
    )]));
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "3",
        "ls",
        json!({"path": "src"}),
    )]));
    fake.push_text("done");

    let harness = harness_with_search(fake, cfg).await;
    let session = harness.session("search-1").await.unwrap();
    let events = session.run(UserTurn::text("search")).await.collect().await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "find"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "grep"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "ls"))
    );
    assert!(events.contains(&RunEvent::RunFinished));

    let state = session.snapshot().await;
    let find_out = state
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("1"))
        .expect("find result");
    let find_text = match &find_out.content[0] {
        ContentPart::Text { text } => text,
        _ => panic!("expected text"),
    };
    assert!(find_text.contains("main.rs"), "find output: {find_text}");

    let grep_out = state
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("2"))
        .expect("grep result");
    let grep_text = match &grep_out.content[0] {
        ContentPart::Text { text } => text,
        _ => panic!("expected text"),
    };
    assert!(grep_text.contains("println"), "grep output: {grep_text}");

    let ls_out = state
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("3"))
        .expect("ls result");
    let ls_text = match &ls_out.content[0] {
        ContentPart::Text { text } => text,
        _ => panic!("expected text"),
    };
    assert!(ls_text.contains("lib.rs"));
    assert!(ls_text.contains("main.rs"));
    let _ = tool_text;
}

#[tokio::test]
async fn find_rejects_wildcard_only_pattern() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "find",
        json!({"pattern": "**/*"}),
    )]));
    fake.push_text("ok");
    let harness = harness_with_search(fake, cfg).await;
    let session = harness.session("search-2").await.unwrap();
    let events = session.run(UserTurn::text("bad")).await.collect().await;
    assert!(events.iter().any(|e| matches!(
        e,
        RunEvent::ToolFinished { name, ok: false } if name == "find"
    )));
}
