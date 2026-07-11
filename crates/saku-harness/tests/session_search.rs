//! Session Search: startup reindex, incremental indexing, the `session_search`
//! Tool, and the `saku status` indexed-session count.

use std::sync::Arc;

use saku_harness::config::{Config, Effort};
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::types::{ContentPart, RunEvent, UserTurn};
use saku_harness::{Harness, SessionStore};
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

/// A message appended during a live Run is searchable immediately (no reindex).
#[tokio::test]
async fn incremental_index_makes_new_messages_searchable() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_text("compaction summarizes earlier turns to save context");

    let harness = Harness::new(config(&tmp), fake).unwrap();
    let session = harness.session("thread-1").await.unwrap();
    let events = session
        .run(UserTurn::text("how does compaction work here"))
        .await
        .collect()
        .await;
    assert!(events.contains(&RunEvent::RunFinished));

    // Both the user prompt and the assistant reply are indexed.
    let hits = harness.search_index().search("compaction", 10).unwrap();
    assert!(hits.iter().any(|h| h.role == "user"));
    assert!(hits.iter().any(|h| h.role == "assistant"));
    assert!(hits.iter().all(|h| h.thread_id == "thread-1"));

    // Status reports one indexed Session.
    let status = session.status_text().await;
    assert!(status.contains("Sessions indexed: 1"), "status: {status}");
}

/// Startup reindex migrates Sessions written before the index existed.
#[tokio::test]
async fn startup_reindex_migrates_existing_jsonl() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);

    // Write two Sessions directly through a store with no index attached.
    {
        let store = SessionStore::open(&cfg.data_dir).unwrap();
        for (thread, text) in [
            ("a", "release channels decision"),
            ("b", "vision bytes noise"),
        ] {
            store
                .load_or_create(thread, &cfg.workspace, "gpt-5.5", Effort::Medium)
                .unwrap();
            store
                .append_message(thread, &saku_harness::types::Message::user_text(text))
                .unwrap();
        }
    }

    let harness = Harness::new(cfg, Arc::new(FakeProvider::new())).unwrap();
    // Index is empty until reindex runs.
    assert_eq!(harness.search_index().session_count().unwrap(), 0);

    let indexed = harness.reindex_sessions().await.unwrap();
    assert_eq!(indexed, 2);
    assert_eq!(harness.search_index().session_count().unwrap(), 2);

    let hits = harness
        .search_index()
        .search("release channels", 10)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].thread_id, "a");
}

/// The `session_search` Tool returns snippets and excludes tool-result text.
#[tokio::test]
async fn session_search_tool_returns_snippets_excluding_tool_results() {
    let tmp = TempDir::new().unwrap();
    let fake = Arc::new(FakeProvider::new());

    // Seed a past Session with a memorable phrase.
    fake.push_text("we decided to use SQLite FTS for session search");
    let harness = Harness::new(config(&tmp), fake.clone()).unwrap();
    harness.register_default_tools().await;
    let past = harness.session("past-thread").await.unwrap();
    past.run(UserTurn::text("what backend for search"))
        .await
        .collect()
        .await;

    // A new Session where the model calls session_search, then replies.
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "session_search",
        json!({"query": "SQLite FTS"}),
    )]));
    fake.push_text("done");
    let session = harness.session("current-thread").await.unwrap();
    let events = session
        .run(UserTurn::text("did we pick a search backend"))
        .await
        .collect()
        .await;
    assert!(events.iter().any(
        |e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "session_search")
    ));

    let state = session.snapshot().await;
    let result = state
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("1"))
        .expect("session_search result");
    let text = match &result.content[0] {
        ContentPart::Text { text } => text,
        _ => panic!("expected text"),
    };
    assert!(text.contains("SQLite FTS"), "tool output: {text}");
    assert!(text.contains("past-thread"), "tool output: {text}");
    // The tool result (which echoes "SQLite FTS") must not itself be indexed:
    // every hit comes from the seeded past Session, never the current thread's
    // tool output.
    let hits = harness.search_index().search("SQLite FTS", 10).unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits.iter().all(|h| h.thread_id == "past-thread"),
        "unexpected hit from a non-seeded thread: {hits:?}"
    );
}

/// Empty corpus and no-match queries return no hits gracefully.
#[tokio::test]
async fn empty_and_unmatched_queries_return_no_hits() {
    let tmp = TempDir::new().unwrap();
    let harness = Harness::new(config(&tmp), Arc::new(FakeProvider::new())).unwrap();
    assert!(
        harness
            .search_index()
            .search("anything", 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(harness.reindex_sessions().await.unwrap(), 0);
    assert!(harness.search_index().search("", 10).unwrap().is_empty());
}
