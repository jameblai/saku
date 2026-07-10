//! Vision + Compaction harness tests.

use std::sync::Arc;

use saku_harness::compaction::{compact_messages, estimate_tokens, DEFAULT_KEEP_RECENT};
use saku_harness::config::{Config, Effort};
use saku_harness::provider::fake::tool_call;
use saku_harness::provider::{FakeProvider, ScriptedResponse};
use saku_harness::tools::file_tools;
use saku_harness::types::{ContentPart, Message, RunEvent, UserTurn};
use saku_harness::{resize_for_provider, Harness};
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
    }
}

#[tokio::test]
async fn read_image_returns_image_content_part() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    // Write a tiny PNG.
    let img = image::DynamicImage::new_rgb8(8, 8);
    let path = cfg.workspace.join("pic.png");
    img.save(&path).unwrap();

    let fake = Arc::new(FakeProvider::new());
    fake.push(ScriptedResponse::ToolCalls(vec![tool_call(
        "1",
        "read",
        json!({"path": "pic.png"}),
    )]));
    fake.push_text("saw image");
    let harness = Harness::new(cfg, fake).unwrap();
    for tool in file_tools() {
        harness.register_tool(tool).await;
    }
    let session = harness.session("vision-1").await.unwrap();
    let _ = session.run(UserTurn::text("look")).await.collect().await;
    let state = session.snapshot().await;
    let tool_msg = state
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("1"))
        .expect("tool result");
    assert!(matches!(
        tool_msg.content.first(),
        Some(ContentPart::Image { .. })
    ));
}

#[tokio::test]
async fn user_turn_images_reach_provider_request() {
    let tmp = TempDir::new().unwrap();
    let cfg = config(&tmp);
    let png = {
        let img = image::DynamicImage::new_rgb8(4, 4);
        let mut buf = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut buf),
            image::ImageFormat::Png,
        )
        .unwrap();
        buf
    };
    let (resized, mime) = resize_for_provider(&png, Some("image/png")).unwrap();
    let fake = Arc::new(FakeProvider::new());
    fake.push_text("ok");
    let harness = Harness::new(cfg, fake.clone()).unwrap();
    let session = harness.session("vision-2").await.unwrap();
    let turn = UserTurn {
        text: "what is this?".into(),
        images: vec![ContentPart::image(mime, resized)],
    };
    let events = session.run(turn).await.collect().await;
    assert!(events.contains(&RunEvent::RunFinished));
    let req = fake.last_request().unwrap();
    assert!(req.messages[0]
        .content
        .iter()
        .any(|c| matches!(c, ContentPart::Image { .. })));
}

#[test]
fn compaction_reduces_estimated_tokens() {
    let mut messages = Vec::new();
    for i in 0..100 {
        messages.push(Message::user_text("word ".repeat(200) + &i.to_string()));
    }
    let before = estimate_tokens(&messages, "sys");
    let compacted = compact_messages(&messages, DEFAULT_KEEP_RECENT, |older| {
        format!("summary of {}", older.len())
    })
    .unwrap();
    let after = estimate_tokens(&compacted.kept_messages, "sys");
    assert!(after < before);
}
