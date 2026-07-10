//! Compaction JSONL persistence and replay.

use std::sync::Arc;

use saku_harness::compaction::{compact_messages, default_local_summarize, DEFAULT_KEEP_RECENT};
use saku_harness::config::{Config, Effort};
use saku_harness::provider::FakeProvider;
use saku_harness::types::Message;
use saku_harness::{Harness, SessionStore};
use tempfile::TempDir;

#[test]
fn compaction_entry_replays_into_summary_plus_tail() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();

    let store = SessionStore::open(&data_dir).unwrap();
    let state = store
        .load_or_create("thread-c", &workspace, "gpt-5.5", Effort::Medium)
        .unwrap();
    assert!(state.messages.is_empty());

    let mut messages = Vec::new();
    for i in 0..20 {
        let msg = Message::user_text(format!("old {i}"));
        store.append_message("thread-c", &msg).unwrap();
        messages.push(msg);
    }
    let compacted = compact_messages(&messages, DEFAULT_KEEP_RECENT, default_local_summarize).unwrap();
    store
        .append_compaction("thread-c", &compacted.summary)
        .unwrap();
    for msg in &compacted.kept_messages {
        if msg
            .content
            .first()
            .and_then(|c| match c {
                saku_harness::ContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .is_some_and(|t| t.starts_with("[compaction summary"))
        {
            continue;
        }
        store.append_message("thread-c", msg).unwrap();
    }

    let replayed = store.replay("thread-c").unwrap();
    assert!(
        replayed.messages.first().is_some_and(|m| {
            matches!(
                m.content.first(),
                Some(saku_harness::ContentPart::Text { text }) if text.contains("compaction summary")
            )
        }),
        "expected summary message after replay"
    );
    assert!(replayed.messages.len() <= DEFAULT_KEEP_RECENT + 1);
}

#[tokio::test]
async fn harness_session_survives_compaction_entry() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let workspace = tmp.path().join("ws");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let store = SessionStore::open(&data_dir).unwrap();
    let _ = store
        .load_or_create("t2", &workspace, "gpt-5.5", Effort::Medium)
        .unwrap();
    store
        .append_compaction("t2", "earlier work summarized")
        .unwrap();
    store
        .append_message("t2", &Message::user_text("recent"))
        .unwrap();

    let harness = Harness::new(
        Config {
            discord_token: "t".into(),
            authorized_user_ids: vec!["1".into()],
            command_prefix: "saku".into(),
            workspace,
            data_dir,
            default_model: "gpt-5.5".into(),
            default_effort: Effort::Medium,
        },
        Arc::new(FakeProvider::new()),
    )
    .unwrap();
    let session = harness.session("t2").await.unwrap();
    let snap = session.snapshot().await;
    assert!(snap.messages.iter().any(|m| {
        matches!(
            m.content.first(),
            Some(saku_harness::ContentPart::Text { text }) if text.contains("earlier work summarized")
        )
    }));
    assert!(snap.messages.iter().any(|m| {
        matches!(
            m.content.first(),
            Some(saku_harness::ContentPart::Text { text }) if text == "recent"
        )
    }));
}
