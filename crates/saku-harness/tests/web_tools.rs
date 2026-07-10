//! Web Tools: credential gate + Exa search/extract with a fake HTTP backend.

use std::sync::Arc;

use saku_harness::ExaClient;
use saku_harness::Harness;
use saku_harness::config::{Config, Effort};
use saku_harness::login_exa_api_key;
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::tools::{register_web_tools, web_tools};
use saku_harness::types::{ContentPart, RunEvent, UserTurn};
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    }
}

#[tokio::test]
async fn web_tools_absent_without_credential() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push_text("ok");

    let harness = Harness::new(config, fake.clone()).unwrap();
    register_web_tools(&harness).await;

    let session = harness.session("web-no-cred").await.unwrap();
    let events = session.run(UserTurn::text("hi")).await.collect().await;
    assert!(events.contains(&RunEvent::RunFinished));

    let tools = fake.last_request().expect("request").tools;
    assert!(
        !tools.iter().any(|t| t.name == "web_search"),
        "web_search must not be offered without Credential"
    );
    assert!(
        !tools.iter().any(|t| t.name == "web_extract"),
        "web_extract must not be offered without Credential"
    );
}

#[tokio::test]
async fn web_tools_present_with_credential() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let store = saku_harness::CredentialStore::open(&config.data_dir).unwrap();
    login_exa_api_key(&store, "test-exa-key").unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push_text("ok");

    let harness = Harness::new(config, fake.clone()).unwrap();
    register_web_tools(&harness).await;

    let session = harness.session("web-with-cred").await.unwrap();
    let events = session.run(UserTurn::text("hi")).await.collect().await;
    assert!(events.contains(&RunEvent::RunFinished));

    let tools = fake.last_request().expect("request").tools;
    assert!(
        tools.iter().any(|t| t.name == "web_search"),
        "web_search must be offered when Credential exists"
    );
    assert!(
        tools.iter().any(|t| t.name == "web_extract"),
        "web_extract must be offered when Credential exists"
    );
}

#[tokio::test]
async fn web_search_returns_hits_via_fake_http() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .and(header("x-api-key", "test-exa-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {
                    "title": "Rust Book",
                    "url": "https://doc.rust-lang.org/book/",
                    "highlights": ["The Rust Programming Language"]
                },
                {
                    "title": "Tokio",
                    "url": "https://tokio.rs/",
                    "highlights": ["async runtime"]
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "s1",
        "web_search",
        json!({"query": "rust async", "max_results": 2, "depth": "fast"}),
    )]));
    fake.push_text("done");

    let client = Arc::new(ExaClient::new("test-exa-key").with_base_url(server.uri()));
    let harness = Harness::new(config, fake).unwrap();
    for tool in web_tools(client) {
        harness.register_tool(tool).await;
    }

    let session = harness.session("web-search-ok").await.unwrap();
    let events = session
        .run(UserTurn::text("search rust"))
        .await
        .collect()
        .await;
    assert!(
        events.iter().any(
            |e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "web_search")
        )
    );

    let out = {
        let state = session.snapshot().await;
        let msg = state
            .messages
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("s1"))
            .expect("search result");
        match &msg.content[0] {
            ContentPart::Text { text } => text.clone(),
            _ => panic!("expected text"),
        }
    };
    assert!(out.contains("Rust Book"), "output: {out}");
    assert!(
        out.contains("https://doc.rust-lang.org/book/"),
        "output: {out}"
    );
    assert!(
        out.contains("The Rust Programming Language"),
        "output: {out}"
    );
    assert!(out.contains("Tokio"), "output: {out}");

    // Confirm Exa request shape: query, numResults, type, highlights (not full text bodies).
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["query"], "rust async");
    assert_eq!(body["numResults"], 2);
    assert_eq!(body["type"], "fast");
    assert_eq!(body["contents"]["highlights"], true);
    assert!(body["contents"].get("text").is_none());
}

#[tokio::test]
async fn web_search_defaults_max_results_and_depth() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": []
        })))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "s2",
        "web_search",
        json!({"query": "only query"}),
    )]));
    fake.push_text("done");

    let client = Arc::new(ExaClient::new("k").with_base_url(server.uri()));
    let harness = Harness::new(config, fake).unwrap();
    for tool in web_tools(client) {
        harness.register_tool(tool).await;
    }

    let session = harness.session("web-search-defaults").await.unwrap();
    let _ = session.run(UserTurn::text("search")).await.collect().await;

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["numResults"], 8);
    assert_eq!(body["type"], "auto");
}

#[tokio::test]
async fn web_extract_returns_page_text_via_fake_http() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contents"))
        .and(header("x-api-key", "test-exa-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {
                    "url": "https://example.com/docs",
                    "text": "Example page body for agents."
                }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "e1",
        "web_extract",
        json!({"url": "https://example.com/docs"}),
    )]));
    fake.push_text("done");

    let client = Arc::new(ExaClient::new("test-exa-key").with_base_url(server.uri()));
    let harness = Harness::new(config, fake).unwrap();
    for tool in web_tools(client) {
        harness.register_tool(tool).await;
    }

    let session = harness.session("web-extract-ok").await.unwrap();
    let events = session.run(UserTurn::text("extract")).await.collect().await;
    assert!(
        events.iter().any(
            |e| matches!(e, RunEvent::ToolFinished { name, ok: true } if name == "web_extract")
        )
    );

    let state = session.snapshot().await;
    let msg = state
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("e1"))
        .expect("extract result");
    let text = match &msg.content[0] {
        ContentPart::Text { text } => text,
        _ => panic!("expected text"),
    };
    assert_eq!(text, "Example page body for agents.");

    let received = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(body["urls"], json!(["https://example.com/docs"]));
    assert_eq!(body["text"], true);
    assert!(body.get("highlights").is_none());
    assert!(body.get("summary").is_none());
}

#[tokio::test]
async fn web_extract_http_error_is_tool_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contents"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "e2",
        "web_extract",
        json!({"url": "https://example.com/x"}),
    )]));
    fake.push_text("done");

    let client = Arc::new(ExaClient::new("bad-key").with_base_url(server.uri()));
    let harness = Harness::new(config, fake).unwrap();
    for tool in web_tools(client) {
        harness.register_tool(tool).await;
    }

    let session = harness.session("web-extract-err").await.unwrap();
    let events = session.run(UserTurn::text("extract")).await.collect().await;
    assert!(
        events.iter().any(
            |e| matches!(e, RunEvent::ToolFinished { name, ok: false } if name == "web_extract")
        )
    );

    let state = session.snapshot().await;
    let msg = state
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("e2"))
        .expect("extract error result");
    let text = match &msg.content[0] {
        ContentPart::Text { text } => text,
        _ => panic!("expected text"),
    };
    assert!(
        text.contains("401") || text.contains("unauthorized"),
        "got: {text}"
    );
}

#[tokio::test]
async fn web_extract_truncates_at_30000_chars() {
    let long = "x".repeat(30_050);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/contents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{ "text": long }]
        })))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "e3",
        "web_extract",
        json!({"url": "https://example.com/long"}),
    )]));
    fake.push_text("done");

    let client = Arc::new(ExaClient::new("k").with_base_url(server.uri()));
    let harness = Harness::new(config, fake).unwrap();
    for tool in web_tools(client) {
        harness.register_tool(tool).await;
    }

    let session = harness.session("web-extract-trunc").await.unwrap();
    let _ = session.run(UserTurn::text("extract")).await.collect().await;

    let state = session.snapshot().await;
    let msg = state
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("e3"))
        .expect("extract result");
    let text = match &msg.content[0] {
        ContentPart::Text { text } => text,
        _ => panic!("expected text"),
    };
    assert!(
        text.ends_with("\n...[truncated]"),
        "missing truncation marker"
    );
    assert_eq!(
        text.chars().count(),
        30_000 + "\n...[truncated]".chars().count()
    );
}

#[tokio::test]
async fn team_info_returns_team_name_via_fake_http() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/websets/v0/teams/me"))
        .and(header("x-api-key", "probe-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "team",
            "id": "team_1",
            "name": "Acme Labs"
        })))
        .mount(&server)
        .await;

    let client = ExaClient::new("probe-key").with_base_url(server.uri());
    let info = client.team_info().await.expect("team_info");
    assert_eq!(info.team_name, "Acme Labs");
}

#[tokio::test]
async fn team_info_maps_api_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/websets/v0/teams/me"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let client = ExaClient::new("bad").with_base_url(server.uri());
    let err = client.team_info().await.expect_err("should fail");
    let msg = err.to_string();
    assert!(msg.contains("401"), "{msg}");
}

#[tokio::test]
async fn fetch_web_backend_status_no_credential() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let store = saku_harness::CredentialStore::open(&config.data_dir).unwrap();
    let status = saku_harness::fetch_web_backend_status(&store, "exa").await;
    assert_eq!(status, saku_harness::WebBackendStatus::NoCredential);
}
