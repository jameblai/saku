//! Serenity bot wiring.

use std::sync::Arc;

use saku_harness::{
    ALLOWED_MODELS, Config, Effort, Harness, RunEvent, UserTurn, file_tools, is_allowed_model,
    is_supported_effort, search_tools, shell_tools, supported_efforts,
};
use serenity::Client;
use serenity::all::{
    ChannelId, Context, CreateMessage, EventHandler, GatewayIntents, Message, ReactionType,
};
use serenity::async_trait;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::chunk::chunk_message;
use crate::commands::{BotCommand, help_text, parse_command};
use crate::progress::{args_preview, format_progress};

const HOURGLASS: &str = "⏳";
const CHECKMARK: &str = "✅";

struct Handler {
    harness: Harness,
    config: Config,
    bot_user_id: Mutex<Option<serenity::all::UserId>>,
}

pub async fn run_bot(config: Config, harness: Harness) -> Result<(), String> {
    let token = config.discord_token.clone();
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MESSAGE_REACTIONS;

    let handler = Handler {
        harness,
        config,
        bot_user_id: Mutex::new(None),
    };

    let mut client = Client::builder(token, intents)
        .event_handler(handler)
        .await
        .map_err(|e| e.to_string())?;

    client.start().await.map_err(|e| e.to_string())
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: serenity::all::Ready) {
        *self.bot_user_id.lock().await = Some(ready.user.id);
        info!("saku connected as {}", ready.user.name);
        let _ = ctx;
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }
        let author_id = msg.author.id.to_string();
        if !self
            .config
            .authorized_user_ids
            .iter()
            .any(|id| id == &author_id)
        {
            return;
        }

        let bot_id = match *self.bot_user_id.lock().await {
            Some(id) => id,
            None => return,
        };

        let content_for_cmd = strip_mention(&msg.content, bot_id);
        if let Some(cmd) = parse_command(&self.config.command_prefix, &content_for_cmd) {
            if let Err(err) = self.handle_command(&ctx, &msg, cmd).await {
                let _ = reply_chunks(&ctx, &msg, &format!("Error: {err}")).await;
            }
            return;
        }

        let mentioned = msg.mentions.iter().any(|u| u.id == bot_id)
            || msg.content.contains(&format!("<@{bot_id}>"))
            || msg.content.contains(&format!("<@!{bot_id}>"));

        let in_thread = msg.thread.is_some()
            || ctx
                .http
                .get_channel(msg.channel_id)
                .await
                .ok()
                .and_then(|c| c.guild())
                .is_some_and(|g| g.thread_metadata.is_some());

        if !mentioned && !in_thread {
            return;
        }

        let (thread_id, _channel_id) = if mentioned && !in_thread {
            match ensure_session_thread(&ctx, &msg).await {
                Ok(id) => (id.to_string(), id),
                Err(err) => {
                    error!("failed to create thread: {err}");
                    return;
                }
            }
        } else {
            (msg.channel_id.to_string(), msg.channel_id)
        };

        let text = strip_mention(&msg.content, bot_id).trim().to_string();
        let images = download_attachments(&ctx, &msg).await;
        if text.is_empty() && images.is_empty() {
            return;
        }

        let turn = UserTurn { text, images };
        if let Err(err) = self.run_turn(&ctx, &msg, &thread_id, turn).await {
            error!("run error: {err}");
        }
    }
}

impl Handler {
    async fn handle_command(
        &self,
        ctx: &Context,
        msg: &Message,
        cmd: BotCommand,
    ) -> Result<(), String> {
        let thread_id = msg.channel_id.to_string();
        let session = self
            .harness
            .session(&thread_id)
            .await
            .map_err(|e| e.to_string())?;

        let reply = match cmd {
            BotCommand::Help => help_text(&self.config.command_prefix),
            BotCommand::Stop => {
                session.stop().await;
                "Stopped active Run and drained the Session queue.".into()
            }
            BotCommand::Steer(message) => {
                session.steer(message).await;
                "Steer noted; will apply after the current tool batch.".into()
            }
            BotCommand::Model { id: None, .. } => {
                format!("Models: {}", ALLOWED_MODELS.join(", "))
            }
            BotCommand::Model {
                id: Some(id),
                effort,
            } => {
                if !is_allowed_model(&id) {
                    format!(
                        "Unsupported model `{id}`. Allowed: {}",
                        ALLOWED_MODELS.join(", ")
                    )
                } else if let Some(effort_str) = effort {
                    let effort = parse_effort(&effort_str)
                        .ok_or_else(|| format!("unsupported effort `{effort_str}`"))?;
                    if !is_supported_effort(&id, effort) {
                        format!("unsupported effort `{effort_str}` for `{id}`")
                    } else {
                        session.set_model(&id).await?;
                        session.set_effort(effort).await?;
                        format!("Model set to `{id}` with effort `{effort}`")
                    }
                } else {
                    session.set_model(&id).await?;
                    format!("Model set to `{id}` (effort reset to default)")
                }
            }
            BotCommand::Effort { level: None } => {
                let model = session.snapshot().await.model;
                let levels: Vec<_> = supported_efforts(&model)
                    .iter()
                    .map(|e| e.as_str())
                    .collect();
                format!("Effort levels for `{model}`: {}", levels.join(", "))
            }
            BotCommand::Effort { level: Some(level) } => {
                let effort =
                    parse_effort(&level).ok_or_else(|| format!("unsupported effort `{level}`"))?;
                session.set_effort(effort).await?;
                format!("Effort set to `{effort}`")
            }
        };

        reply_chunks(ctx, msg, &reply).await?;
        react_ok(ctx, msg).await;
        Ok(())
    }

    async fn run_turn(
        &self,
        ctx: &Context,
        msg: &Message,
        thread_id: &str,
        turn: UserTurn,
    ) -> Result<(), String> {
        let session = self
            .harness
            .session(thread_id)
            .await
            .map_err(|e| e.to_string())?;

        let mut handle = session.run(turn).await;
        let mut answer = String::new();
        let mut progress_lines: Vec<(String, String)> = Vec::new();
        let mut progress_msg: Option<Message> = None;
        let mut failed = false;
        let mut aborted = false;
        let mut fail_note = String::new();
        let mut hourglass = false;

        while let Some(ev) = handle.next_event().await {
            match ev {
                RunEvent::Queued => {
                    let _ = msg
                        .react(ctx, ReactionType::Unicode(HOURGLASS.into()))
                        .await;
                    hourglass = true;
                }
                RunEvent::Dequeued => {
                    if hourglass {
                        let _ = ctx
                            .http
                            .delete_reaction_me(
                                msg.channel_id,
                                msg.id,
                                &ReactionType::Unicode(HOURGLASS.into()),
                            )
                            .await;
                        hourglass = false;
                    }
                }
                RunEvent::TextDelta { text } => answer.push_str(&text),
                RunEvent::ToolStarted { name, args } => {
                    progress_lines.push((name, args_preview(&args)));
                    let body = format_progress(&progress_lines);
                    if let Some(ref mut existing) = progress_msg {
                        let _ = existing
                            .edit(ctx, serenity::all::EditMessage::new().content(body))
                            .await;
                    } else {
                        match msg
                            .channel_id
                            .send_message(
                                ctx,
                                CreateMessage::new().content(body).reference_message(msg),
                            )
                            .await
                        {
                            Ok(m) => progress_msg = Some(m),
                            Err(err) => warn!("progress message failed: {err}"),
                        }
                    }
                }
                RunEvent::RunError { message } => {
                    failed = true;
                    fail_note = message;
                }
                RunEvent::RunAborted => {
                    aborted = true;
                    fail_note = "Run aborted.".into();
                }
                RunEvent::RunFinished => break,
                _ => {}
            }
        }

        if hourglass {
            let _ = ctx
                .http
                .delete_reaction_me(
                    msg.channel_id,
                    msg.id,
                    &ReactionType::Unicode(HOURGLASS.into()),
                )
                .await;
        }

        if failed || aborted {
            let note = if fail_note.is_empty() {
                "Run failed.".into()
            } else {
                fail_note
            };
            reply_chunks(ctx, msg, &note).await?;
            return Ok(());
        }

        if answer.trim().is_empty() {
            answer = "(no assistant text)".into();
        }
        reply_chunks(ctx, msg, &answer).await?;
        react_ok(ctx, msg).await;
        Ok(())
    }
}

async fn ensure_session_thread(ctx: &Context, msg: &Message) -> Result<ChannelId, String> {
    if let Some(thread) = &msg.thread {
        return Ok(thread.id);
    }
    let name = {
        let base = strip_mention_raw(&msg.content);
        let trimmed = base.chars().take(80).collect::<String>();
        if trimmed.trim().is_empty() {
            "saku".into()
        } else {
            trimmed
        }
    };
    let thread = msg
        .channel_id
        .create_thread_from_message(
            ctx,
            msg.id,
            serenity::all::CreateThread::new(name)
                .auto_archive_duration(serenity::all::AutoArchiveDuration::OneDay),
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(thread.id)
}

fn strip_mention(content: &str, bot_id: serenity::all::UserId) -> String {
    content
        .replace(&format!("<@{bot_id}>"), "")
        .replace(&format!("<@!{bot_id}>"), "")
}

fn strip_mention_raw(content: &str) -> String {
    let mut out = String::new();
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' && chars.peek() == Some(&'@') {
            for x in chars.by_ref() {
                if x == '>' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

async fn reply_chunks(ctx: &Context, msg: &Message, text: &str) -> Result<(), String> {
    for chunk in chunk_message(text) {
        msg.channel_id
            .send_message(
                ctx,
                CreateMessage::new().content(chunk).reference_message(msg),
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn react_ok(ctx: &Context, msg: &Message) {
    let _ = msg
        .react(ctx, ReactionType::Unicode(CHECKMARK.into()))
        .await;
}

async fn download_attachments(ctx: &Context, msg: &Message) -> Vec<saku_harness::ContentPart> {
    let mut images = Vec::new();
    for attachment in &msg.attachments {
        let mime = attachment
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".into());
        if !saku_harness::is_image_mime(&mime)
            && !saku_harness::is_image_path(std::path::Path::new(&attachment.filename))
        {
            continue;
        }
        match attachment.download().await {
            Ok(bytes) => match saku_harness::resize_for_provider(&bytes, Some(&mime)) {
                Ok((resized, out_mime)) => {
                    images.push(saku_harness::ContentPart::image(out_mime, resized));
                }
                Err(err) => warn!("image resize failed for {}: {err}", attachment.filename),
            },
            Err(err) => warn!("attachment download failed: {err}"),
        }
    }
    let _ = ctx;
    images
}

fn parse_effort(s: &str) -> Option<Effort> {
    match s.to_ascii_lowercase().as_str() {
        "minimal" => Some(Effort::Minimal),
        "low" => Some(Effort::Low),
        "medium" => Some(Effort::Medium),
        "high" => Some(Effort::High),
        "xhigh" => Some(Effort::Xhigh),
        "max" => Some(Effort::Max),
        _ => None,
    }
}

/// Register default v1 tools on a Harness.
pub async fn register_default_tools(harness: &Harness) {
    for tool in file_tools() {
        harness.register_tool(tool).await;
    }
    for tool in shell_tools() {
        harness.register_tool(tool).await;
    }
    for tool in search_tools(Arc::clone(harness.index())) {
        harness.register_tool(tool).await;
    }
}
