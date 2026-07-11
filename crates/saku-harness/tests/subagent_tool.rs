use std::sync::Arc;

use saku_harness::Harness;
use saku_harness::config::{Config, Effort};
use saku_harness::provider::FakeProvider;
use saku_harness::provider::fake::tool_call;
use saku_harness::types::{ContentPart, Role, RunEvent, UserTurn};
use serde_json::json;
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
        web_backend: "exa".into(),
        release_channel: saku_harness::ReleaseChannel::Stable,
    }
}

#[tokio::test]
async fn explore_subagent_has_isolated_task_and_analysis_tools_and_returns_summary() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![tool_call(
        "child",
        "subagent",
        json!({"task": "map the code"}),
    )]);
    fake.push_text("The map summary");
    fake.push_text("Parent answer");
    let harness = Harness::new(test_config(&tmp), fake.clone()).unwrap();
    harness.register_default_tools().await;

    let session = harness.session("explore").await.unwrap();
    let events = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;
    assert!(events.contains(&RunEvent::RunFinished));

    let requests = fake.requests();
    let child = &requests[1];
    assert!(child.system.starts_with(&requests[0].system));
    assert_eq!(child.messages.len(), 1);
    assert_eq!(child.messages[0].role, Role::User);
    assert!(
        matches!(&child.messages[0].content[0], ContentPart::Text { text } if text == "map the code")
    );
    assert_eq!(
        child
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        ["read", "bash", "cd", "find", "grep", "ls"]
    );
    assert!(child.system.contains("analysis-only"));
    assert!(child.system.contains("do not retry through bash"));
    assert!(!child.tools.iter().any(|tool| tool.name == "write"));
    assert!(!child.tools.iter().any(|tool| tool.name == "edit"));
    assert!(!child.tools.iter().any(|tool| tool.name == "subagent"));
    assert!(requests[2].messages.iter().any(|message| {
        message.role == Role::Tool
            && matches!(&message.content[0], ContentPart::Text { text } if text == "The map summary")
    }));
}

#[tokio::test]
async fn edit_subagent_has_full_tools_without_subagent() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![tool_call(
        "child",
        "subagent",
        json!({"task": "make an edit", "mode": "edit"}),
    )]);
    fake.push_text("done");
    fake.push_text("ok");
    let harness = Harness::new(test_config(&tmp), fake.clone()).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("edit").await.unwrap();
    let _ = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    let requests = fake.requests();
    let names = requests[1]
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"bash"));
    assert!(names.contains(&"edit"));
    assert!(!names.contains(&"subagent"));
}

#[tokio::test]
async fn max_turns_returns_limit_notice_to_parent() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    std::fs::write(tmp.path().join("workspace/file.txt"), "hello").unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![tool_call(
        "child",
        "subagent",
        json!({"task": "loop", "max_turns": 1}),
    )]);
    fake.push_tool_calls(vec![tool_call("read", "read", json!({"path": "file.txt"}))]);
    fake.push_text("parent recovered");
    let harness = Harness::new(config, fake.clone()).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("limit").await.unwrap();
    let _ = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    let requests = fake.requests();
    assert!(requests[2].messages.iter().any(|message| {
        message.role == Role::Tool
            && matches!(&message.content[0], ContentPart::Text { text } if text.contains("max_turns=1"))
    }));
}

#[tokio::test]
async fn child_cd_is_local_to_the_subagent_and_applies_to_later_child_tools() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    std::fs::create_dir_all(config.workspace.join("nested")).unwrap();
    std::fs::write(config.workspace.join("nested/file.txt"), "nested content").unwrap();
    let workspace = config.workspace.clone();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![tool_call(
        "child",
        "subagent",
        json!({"task": "inspect nested"}),
    )]);
    fake.push_tool_calls(vec![tool_call("cd", "cd", json!({"path": "nested"}))]);
    fake.push_tool_calls(vec![tool_call("read", "read", json!({"path": "file.txt"}))]);
    fake.push_text("found it");
    fake.push_text("parent done");
    let harness = Harness::new(config, fake).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("child-cd").await.unwrap();
    let _ = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    assert_eq!(session.snapshot().await.cwd, workspace);
}

#[tokio::test]
async fn explore_child_can_run_bash_without_leaking_inner_messages_to_parent_transcript() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![tool_call(
        "child",
        "subagent",
        json!({"task": "inspect git state"}),
    )]);
    fake.push_tool_calls(vec![tool_call(
        "bash",
        "bash",
        json!({"command": "git status --short --branch 2>&1 || true"}),
    )]);
    fake.push_text("git inspection complete");
    fake.push_text("parent done");
    let harness = Harness::new(test_config(&tmp), fake.clone()).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("explore-bash").await.unwrap();
    let _ = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    let child_follow_up = &fake.requests()[2];
    assert!(child_follow_up.messages.iter().any(|message| {
        message.role == Role::Tool
            && matches!(&message.content[0], ContentPart::Text { text } if text.contains("not a git repository"))
    }));

    let parent = session.snapshot().await;
    assert_eq!(parent.messages.len(), 4);
    assert!(
        !parent
            .messages
            .iter()
            .any(|message| { message.tool_calls.iter().any(|call| call.name == "bash") })
    );
    assert!(parent.messages.iter().any(|message| {
        message.role == Role::Tool
            && matches!(&message.content[0], ContentPart::Text { text } if text == "git inspection complete")
    }));
}

#[tokio::test]
async fn parallel_edit_batch_is_rejected_before_children_start() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![
        tool_call("one", "subagent", json!({"task": "one", "mode": "edit"})),
        tool_call("two", "subagent", json!({"task": "two", "mode": "edit"})),
    ]);
    fake.push_text("handled");
    let harness = Harness::new(test_config(&tmp), fake.clone()).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("parallel-edit").await.unwrap();
    let _ = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    let errors = requests[1].messages.iter().filter(|message| {
        message.role == Role::Tool
            && matches!(&message.content[0], ContentPart::Text { text } if text.contains("explore-only"))
    }).count();
    assert_eq!(errors, 2);
}

#[tokio::test]
async fn explore_batch_starts_at_most_four_children_before_finishing_one() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    let calls = (1..=5)
        .map(|number| {
            tool_call(
                &format!("child-{number}"),
                "subagent",
                json!({"task": format!("task {number}")}),
            )
        })
        .collect();
    fake.push_tool_calls(calls);
    for number in 1..=5 {
        fake.push_text(format!("summary {number}"));
    }
    fake.push_text("parent done");
    let harness = Harness::new(test_config(&tmp), fake).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("parallel-explore").await.unwrap();
    let events = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    let lifecycle = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                RunEvent::ToolStarted { name, .. } | RunEvent::ToolFinished { name, .. }
                    if name == "subagent"
            )
        })
        .collect::<Vec<_>>();
    assert!(
        lifecycle[..4]
            .iter()
            .all(|event| matches!(event, RunEvent::ToolStarted { .. }))
    );
    assert!(matches!(lifecycle[4], RunEvent::ToolFinished { .. }));
    assert!(matches!(lifecycle[8], RunEvent::ToolStarted { .. }));
}

#[tokio::test]
async fn nested_subagent_call_returns_depth_error_to_child() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![tool_call(
        "child",
        "subagent",
        json!({"task": "try nesting"}),
    )]);
    fake.push_tool_calls(vec![tool_call(
        "nested",
        "subagent",
        json!({"task": "too deep"}),
    )]);
    fake.push_text("nesting was blocked");
    fake.push_text("parent done");
    let harness = Harness::new(test_config(&tmp), fake.clone()).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("nested").await.unwrap();
    let _ = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    assert!(fake.requests()[2].messages.iter().any(|message| {
        message.role == Role::Tool
            && matches!(&message.content[0], ContentPart::Text { text } if text.contains("maximum depth is 1"))
    }));
}

#[tokio::test]
async fn subagent_model_and_effort_can_override_the_session() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![tool_call(
        "child",
        "subagent",
        json!({
            "task": "use override",
            "model": "gpt-5.4-mini",
            "effort": "low"
        }),
    )]);
    fake.push_text("summary");
    fake.push_text("parent done");
    let harness = Harness::new(test_config(&tmp), fake.clone()).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("overrides").await.unwrap();
    let _ = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    let child = &fake.requests()[1];
    assert_eq!(child.model, "gpt-5.4-mini");
    assert_eq!(child.effort, Effort::Low);
}

#[tokio::test]
async fn subagent_normalizes_a_human_readable_model_name() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(vec![tool_call(
        "child",
        "subagent",
        json!({"task": "use mini", "model": "GPT 5.4 Mini"}),
    )]);
    fake.push_text("summary");
    fake.push_text("parent done");
    let harness = Harness::new(test_config(&tmp), fake.clone()).unwrap();
    harness.register_default_tools().await;
    let session = harness.session("normalized-model").await.unwrap();
    let _ = session
        .run(UserTurn::text("delegate"))
        .await
        .collect()
        .await;

    assert_eq!(fake.requests()[1].model, "gpt-5.4-mini");
}
