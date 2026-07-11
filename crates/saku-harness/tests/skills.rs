//! Skills: discovery in System Prompt, `$skill-name` expansion, read allowlist.

use std::sync::Arc;

use saku_harness::config::{Config, Effort};
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::tools::file_tools;
use saku_harness::types::{RunEvent, UserTurn};
use saku_harness::{Harness, format_status};
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

fn empty_global(tmp: &TempDir) -> std::path::PathBuf {
    let dir = tmp.path().join("empty-global-skills");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_project_skill(workspace: &std::path::Path, name: &str, body: &str) {
    let dir = workspace.join(".agents").join("skills").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), body).unwrap();
}

#[tokio::test]
async fn system_prompt_lists_project_skills() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    write_project_skill(
        &cfg.workspace,
        "triage",
        "---\nname: triage\ndescription: Triage GitHub issues\n---\n\nTriage steps.\n",
    );
    write_project_skill(
        &cfg.workspace,
        "secret",
        "---\nname: secret\ndescription: Explicit only\ndisable-model-invocation: true\n---\n\nSecret.\n",
    );

    let fake = Arc::new(FakeProvider::new());
    fake.push_text("ok");
    let harness = Harness::with_global_skills_dir(cfg, fake.clone(), empty_global(&tmp)).unwrap();
    let session = harness.session("skills-prompt").await.unwrap();
    let _ = session.run(UserTurn::text("hi")).await.collect().await;

    let req = fake.last_request().expect("request");
    assert!(req.system.contains("<available_skills>"));
    assert!(req.system.contains("<name>triage</name>"));
    assert!(req.system.contains("Triage GitHub issues"));
    assert!(!req.system.contains("<name>secret</name>"));
}

#[tokio::test]
async fn dollar_skill_name_expands_into_user_message() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    write_project_skill(
        &cfg.workspace,
        "triage",
        "---\nname: triage\ndescription: Triage issues\n---\n\nFollow triage protocol.\n",
    );

    let fake = Arc::new(FakeProvider::new());
    fake.push_text("ok");
    let harness = Harness::with_global_skills_dir(cfg, fake.clone(), empty_global(&tmp)).unwrap();
    let session = harness.session("skills-expand").await.unwrap();
    let _ = session
        .run(UserTurn::text("$triage handle #42"))
        .await
        .collect()
        .await;

    let req = fake.last_request().expect("request");
    let user = &req.messages[0];
    let text = user
        .content
        .iter()
        .find_map(|c| match c {
            saku_harness::ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .expect("user text");
    assert!(text.contains("<skill name=\"triage\""));
    assert!(text.contains("Follow triage protocol."));
    assert!(text.contains("handle #42"));
}

#[tokio::test]
async fn read_tool_can_load_skill_outside_workspace_via_allowlist() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let global = tmp.path().join("global-skills");
    let outside = global.join("triage");
    std::fs::create_dir_all(&outside).unwrap();
    let skill_md = outside.join("SKILL.md");
    std::fs::write(
        &skill_md,
        "---\nname: triage\ndescription: Triage\n---\n\nBody.\n",
    )
    .unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "read",
        json!({"path": skill_md.to_string_lossy()}),
    )]));
    fake.push_text("done");

    let harness = Harness::with_global_skills_dir(cfg, fake, global).unwrap();
    for tool in file_tools() {
        harness.register_tool(tool).await;
    }
    let session = harness.session("skills-read").await.unwrap();
    let events = session
        .run(UserTurn::text("read skill"))
        .await
        .collect()
        .await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "read")),
        "read should succeed for allowlisted skill path: {events:?}"
    );
}

#[tokio::test]
async fn status_lists_discovered_skill_names() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    write_project_skill(
        &cfg.workspace,
        "triage",
        "---\nname: triage\ndescription: Triage\n---\n\nBody.\n",
    );
    let harness = Harness::with_global_skills_dir(
        cfg.clone(),
        Arc::new(FakeProvider::new()),
        empty_global(&tmp),
    )
    .unwrap();
    let session = harness.session("skills-status").await.unwrap();
    let report = session
        .status_report(
            &cfg.default_model,
            cfg.default_effort,
            "codex",
            None,
            None,
            saku_harness::WebBackendStatus::NoCredential,
        )
        .await;
    assert_eq!(report.skill_names, vec!["triage".to_string()]);
    let text = format_status(&report);
    assert!(text.contains("**Skills**"));
    assert!(text.contains("1: `triage`"));
}
