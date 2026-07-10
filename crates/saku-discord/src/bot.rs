//! Serenity bot wiring.

use std::sync::Arc;

use saku_harness::{
    ALLOWED_MODELS, Config, Effort, Harness, RunEvent, UserTurn, background_tools, file_tools,
    is_allowed_model, is_supported_effort, register_web_tools, search_tools, shell_tools,
    supported_efforts,
};
use serenity::Client;
use serenity::all::{
    ChannelId, Context, CreateMessage, EventHandler, GatewayIntents, Message, MessageReference,
    ReactionType,
};
use serenity::async_trait;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::chunk::chunk_message;
use crate::commands::{BotCommand, help_text, parse_command};
use crate::progress::{args_preview, format_progress};
use crate::typing::{TYPING_REFRESH, TypingIndicator};

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
                let _ = reply_chunks(&ctx, msg.channel_id, &msg, &format!("Error: {err}")).await;
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

        let session_thread = if mentioned && !in_thread {
            match ensure_session_thread(&ctx, &msg).await {
                Ok(id) => id,
                Err(err) => {
                    error!("failed to create thread: {err}");
                    return;
                }
            }
        } else {
            msg.channel_id
        };
        let thread_id = session_thread.to_string();
        let output_channel = run_output_channel(msg.channel_id, session_thread);

        let text = strip_mention(&msg.content, bot_id).trim().to_string();
        let images = download_attachments(&ctx, &msg).await;
        if text.is_empty() && images.is_empty() {
            return;
        }

        let turn = UserTurn { text, images };
        if let Err(err) = self
            .run_turn(&ctx, &msg, &thread_id, output_channel, turn)
            .await
        {
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
            BotCommand::Status => session.status_text().await,
            BotCommand::BgList => session.bg_list_text().await,
            BotCommand::BgLogs { pid } => {
                session.bg_logs_text(pid, None).await.unwrap_or_else(|e| e)
            }
            BotCommand::BgStop { pid } => session.bg_stop(pid).await.unwrap_or_else(|e| e),
        };

        reply_chunks(ctx, msg.channel_id, msg, &reply).await?;
        react_ok(ctx, msg).await;
        Ok(())
    }

    async fn run_turn(
        &self,
        ctx: &Context,
        msg: &Message,
        thread_id: &str,
        output_channel: ChannelId,
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
        let mut typing: Option<TypingIndicator> = None;

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
                RunEvent::RunStarted => {
                    let http = ctx.http.clone();
                    let channel = output_channel;
                    typing = Some(TypingIndicator::start(TYPING_REFRESH, move || {
                        let http = http.clone();
                        async move {
                            channel
                                .broadcast_typing(&*http)
                                .await
                                .map(|_| ())
                                .map_err(|_| ())
                        }
                    }));
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
                        match output_channel
                            .send_message(ctx, build_reply(body, output_channel, msg))
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

        if let Some(t) = typing.take() {
            t.stop();
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
            reply_chunks(ctx, output_channel, msg, &note).await?;
            return Ok(());
        }

        if answer.trim().is_empty() {
            answer = "(no assistant text)".into();
        }
        reply_chunks(ctx, output_channel, msg, &answer).await?;
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

/// Channel that receives Progress/Answer messages for a Session Run.
///
/// A channel @mention creates a Session thread while the triggering message
/// stays in the parent channel. Bot output must go to the Session thread.
fn run_output_channel(_trigger_channel: ChannelId, session_thread: ChannelId) -> ChannelId {
    session_thread
}

/// Reply reference that still posts if Discord rejects a cross-channel link
/// (parent-channel starter message → Session thread).
fn reply_reference(msg: &Message) -> MessageReference {
    MessageReference::from(msg).fail_if_not_exists(false)
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

/// Whether Discord allows a reply-reference from `output_channel` to the
/// triggering message in `trigger_channel`.
///
/// A channel @mention creates a Session thread while the trigger stays in the
/// parent channel. Cross-channel `message_reference` fails the send (the Run
/// answer is stored but never appears in Discord). Same-channel references
/// (follow-ups already in the thread) are fine.
fn can_reference_trigger(output_channel: ChannelId, trigger_channel: ChannelId) -> bool {
    output_channel == trigger_channel
}

fn build_reply(
    content: impl Into<String>,
    output_channel: ChannelId,
    trigger: &Message,
) -> CreateMessage {
    let mut message = CreateMessage::new().content(content);
    if can_reference_trigger(output_channel, trigger.channel_id) {
        message = message.reference_message(reply_reference(trigger));
    }
    message
}

async fn reply_chunks(
    ctx: &Context,
    channel_id: ChannelId,
    msg: &Message,
    text: &str,
) -> Result<(), String> {
    for chunk in chunk_message(text) {
        channel_id
            .send_message(ctx, build_reply(chunk, channel_id, msg))
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
    for tool in background_tools() {
        harness.register_tool(tool).await;
    }
    for tool in search_tools(Arc::clone(harness.index())) {
        harness.register_tool(tool).await;
    }
    register_web_tools(harness).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_mention_output_goes_to_session_thread_not_parent() {
        let parent = ChannelId::new(111);
        let thread = ChannelId::new(222);
        assert_ne!(parent, thread);
        assert_eq!(run_output_channel(parent, thread), thread);
    }

    #[test]
    fn in_thread_output_stays_on_same_channel() {
        let thread = ChannelId::new(333);
        assert_eq!(run_output_channel(thread, thread), thread);
    }

    #[test]
    fn new_thread_reply_does_not_reference_parent_channel_message() {
        let parent = ChannelId::new(111);
        let thread = ChannelId::new(222);
        // Repro: first @mention Run posts to the new thread but referenced the
        // parent-channel starter message → Discord rejected the send, so the
        // answer never appeared (follow-ups in-thread worked).
        assert!(
            !can_reference_trigger(thread, parent),
            "must not cross-reference parent message when posting into new thread"
        );
        assert!(can_reference_trigger(thread, thread));
    }
}
