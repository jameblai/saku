//! Session Goal outer loop: evaluator continue/stop, max_runs, stop, replace,
//! and Session Store persistence (issue #50).

use std::sync::Arc;

use saku_harness::config::{Config, Effort};
use saku_harness::provider::FakeProvider;
use saku_harness::provider::fake::tool_call;
use saku_harness::types::UserTurn;
use saku_harness::{GoalDecision, Harness, MAX_GOAL_RUNS};
use serde_json::json;
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

fn goal_check(id: &str, met: bool, reason: &str) -> Vec<saku_harness::types::ToolCall> {
    vec![tool_call(
        id,
        "goal_check",
        json!({ "met": met, "reason": reason }),
    )]
}

#[tokio::test]
async fn evaluator_continues_until_met() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_text("working run 1");
    fake.push_tool_calls(goal_check("e1", false, "tests still failing"));
    fake.push_text("working run 2");
    fake.push_tool_calls(goal_check("e2", true, "all green"));
    let harness = Harness::new(config(&tmp), fake.clone()).unwrap();
    let session = harness.session("g1").await.unwrap();

    session.set_goal("cargo test green").await.unwrap();
    session
        .run(UserTurn::text("cargo test green"))
        .await
        .collect()
        .await;

    let d1 = session.evaluate_goal().await.unwrap();
    match d1 {
        GoalDecision::Continue { message, run } => {
            assert_eq!(run, 1);
            assert!(message.contains("cargo test green"));
            assert!(message.contains("tests still failing"));
        }
        other => panic!("expected Continue, got {other:?}"),
    }
    // Goal still active with the recorded reason.
    let goal = session.goal().await.unwrap();
    assert_eq!(goal.run_count, 1);
    assert_eq!(
        goal.last_evaluator_reason.as_deref(),
        Some("tests still failing")
    );

    session
        .run(UserTurn::text("continue"))
        .await
        .collect()
        .await;
    let d2 = session.evaluate_goal().await.unwrap();
    assert!(matches!(d2, GoalDecision::Achieved { run: 2, .. }));
    assert!(session.goal().await.is_none(), "achieved Goal must clear");
}

#[tokio::test]
async fn evaluator_uses_mini_model_low_effort_and_goal_check_tool() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(goal_check("e1", false, "not yet"));
    let harness = Harness::new(config(&tmp), fake.clone()).unwrap();
    let session = harness.session("g-model").await.unwrap();

    session.set_goal("ship it").await.unwrap();
    session.evaluate_goal().await.unwrap();

    let req = fake.last_request().expect("evaluator request");
    assert_eq!(req.model, "gpt-5.4-mini");
    assert_eq!(req.effort, Effort::Low);
    assert!(req.tools.iter().any(|t| t.name == "goal_check"));
    assert_eq!(req.tools.len(), 1, "evaluator sees only goal_check");
    assert!(req.system.contains("Goal Evaluator"));
    assert!(
        req.messages
            .iter()
            .any(|m| m.content.iter().any(|c| matches!(
                c,
                saku_harness::ContentPart::Text { text } if text.contains("ship it")
            ))),
        "condition included in evaluator prompt"
    );
}

#[tokio::test]
async fn max_runs_exhaustion_clears_goal() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    for i in 0..MAX_GOAL_RUNS {
        fake.push_tool_calls(goal_check(&format!("e{i}"), false, "still not met"));
    }
    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("g-max").await.unwrap();
    session.set_goal("impossible").await.unwrap();

    for run in 1..MAX_GOAL_RUNS {
        let d = session.evaluate_goal().await.unwrap();
        assert!(
            matches!(d, GoalDecision::Continue { run: r, .. } if r == run),
            "run {run} should Continue, got {d:?}"
        );
    }
    let last = session.evaluate_goal().await.unwrap();
    assert!(
        matches!(last, GoalDecision::Exhausted { run, .. } if run == MAX_GOAL_RUNS),
        "final run should Exhaust, got {last:?}"
    );
    assert!(session.goal().await.is_none(), "exhausted Goal must clear");
}

#[tokio::test]
async fn stop_clears_active_goal() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("g-stop").await.unwrap();

    session.set_goal("keep going").await.unwrap();
    assert!(session.goal().await.is_some());
    session.stop().await;
    assert!(session.goal().await.is_none(), "stop must clear Goal");

    // Persisted: a fresh Harness replays no active Goal.
    let harness2 = Harness::new(config_same(&tmp), Arc::new(FakeProvider::new())).unwrap();
    let reloaded = harness2.session("g-stop").await.unwrap();
    assert!(reloaded.goal().await.is_none());
}

#[tokio::test]
async fn new_goal_replaces_and_resets_run_count() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(goal_check("e1", false, "reason"));
    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("g-replace").await.unwrap();

    session.set_goal("goal a").await.unwrap();
    session.evaluate_goal().await.unwrap();
    assert_eq!(session.goal().await.unwrap().run_count, 1);

    session.set_goal("goal b").await.unwrap();
    let goal = session.goal().await.unwrap();
    assert_eq!(goal.condition, "goal b");
    assert_eq!(goal.run_count, 0, "replacement resets run count");
    assert!(goal.last_evaluator_reason.is_none());
}

#[tokio::test]
async fn goal_state_persists_across_reload() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_tool_calls(goal_check("e1", false, "one test failing"));
    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("g-persist").await.unwrap();
    session.set_goal("green build").await.unwrap();
    session.evaluate_goal().await.unwrap();

    // Fresh Harness over the same data dir replays the Goal from the Store.
    let harness2 = Harness::new(config_same(&tmp), Arc::new(FakeProvider::new())).unwrap();
    let reloaded = harness2.session("g-persist").await.unwrap();
    let goal = reloaded.goal().await.expect("Goal survives reload");
    assert_eq!(goal.condition, "green build");
    assert_eq!(goal.run_count, 1);
    assert_eq!(
        goal.last_evaluator_reason.as_deref(),
        Some("one test failing")
    );

    // sessions_with_active_goal surfaces it for auto-resume.
    let active = harness2.sessions_with_active_goal().await;
    assert!(active.contains(&"g-persist".to_string()));
}

/// Same config but pointing at the already-created dirs (no re-create needed).
fn config_same(tmp: &TempDir) -> Config {
    Config {
        discord_token: "t".into(),
        authorized_user_ids: vec!["1".into()],
        command_prefix: "saku".into(),
        workspace: tmp.path().join("ws"),
        data_dir: tmp.path().join("data"),
        default_model: "gpt-5.5".into(),
        default_effort: Effort::Medium,
        web_backend: "exa".into(),
        release_channel: saku_harness::ReleaseChannel::Stable,
    }
}
