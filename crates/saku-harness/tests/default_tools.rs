//! Smoke test: Harness::register_default_tools registers the core Tool set.
//!
//! Fresh test Data Dir has no Web Backend Credential, so web Tools are not
//! registered (credential-gated).

use std::sync::Arc;

use saku_harness::Harness;
use saku_harness::WebBackendStatus;
use saku_harness::config::{Config, Effort};
use saku_harness::provider::{CODEX_PROVIDER_ID, FakeProvider};
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
async fn register_default_tools_registers_core_set() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    let harness = Harness::new(cfg.clone(), fake).unwrap();
    harness.register_default_tools().await;

    let session = harness.session("default-tools").await.unwrap();
    let report = session
        .status_report(
            &cfg.default_model,
            cfg.default_effort,
            CODEX_PROVIDER_ID,
            None,
            None,
            WebBackendStatus::NoCredential,
        )
        .await;

    let expected_core = [
        "read", "edit", "write", "memory", "bash", "cd", "bg_start", "bg_list", "bg_logs",
        "bg_stop", "find", "grep", "ls", "subagent",
    ];
    assert_eq!(report.tool_names, expected_core);
    // No Exa Credential in this test Data Dir → web Tools stay unregistered.
    assert!(!report.tool_names.iter().any(|n| n.starts_with("web_")));
}
