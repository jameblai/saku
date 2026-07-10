//! Serenity bot wiring.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use saku_harness::{
    Config, Effort, Harness, RunEvent, UserTurn, file_tools, search_tools, shell_tools,
    ALLOWED_MODELS, is_allowed_model, is_supported_effort, supported_efforts,
};
use serenity::all::{
    ChannelId, Context, CreateMessage, EventHandler, GatewayIntents, Message, MessageId, ReactionType,
};
use serenity::async_trait;
use serenity::Client;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::chunk::chunk_message;
use crate::commands::{help_text, parse_command, BotCommand};
use crate::progress::{args_preview, format_progress};

const HOURGLASS: &str = "⏳";
const CHECKMARK: &str = "✅";

struct QueuedTurn {
    message_id: MessageId,
    channel_id: ChannelId,
    turn: UserTurn,
}

struct SessionQueue {
    active: bool,
    queue: VecDeque<QueuedTurn>,
}

struct Handler {
    harness: Harness,
    config: Config,
    bot_user_id: Mutex<Option<serenity::all::UserId>>,
    queues: Mutex<HashMap<String, SessionQueue>>,
    /// Pending steer text applied after the current tool batch (v1: next provider round).
    steers: Mutex<HashMap<String, String>>,
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
        queues: Mutex::new(HashMap::new()),
        steers: Mutex::new(HashMap::new()),
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

        // Bot commands in threads (and channels) with prefix.
        let content_for_cmd = strip_mention(&msg.content, bot_id);
        if let Some(cmd) = parse_command(&self.config.command_prefix, &content_for_cmd) {
            if let Err(err) = self.handle_command(&ctx, &msg, cmd).await {
                warn!("command error: {err}");
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

        // Channel @mention: create or use a thread.
        let (thread_id, channel_id) = if mentioned && !in_thread {
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
        if text.is_empty() {
            return;
        }

        let turn = UserTurn::text(text);
        if let Err(err) = self
            .enqueue_or_run(&ctx, &msg, &thread_id, channel_id, turn)
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
                self.drain_queue(ctx, &thread_id).await?;
                "Stopped active Run and drained the Session queue.".into()
            }
            BotCommand::Steer(message) => {
                self.steers
                    .lock()
                    .await
                    .insert(thread_id.clone(), message);
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
                    let effort = parse_effort(&effort_str).ok_or_else(|| {
                        format!("unsupported effort `{effort_str}`")
                    })?;
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
            BotCommand::Effort {
                level: Some(level),
            } => {
                let effort = parse_effort(&level)
                    .ok_or_else(|| format!("unsupported effort `{level}`"))?;
                session.set_effort(effort).await?;
                format!("Effort set to `{effort}`")
            }
        };

        reply_chunks(ctx, msg, &reply).await?;
        react_ok(ctx, msg).await;
        Ok(())
    }

    async fn drain_queue(&self, ctx: &Context, thread_id: &str) -> Result<(), String> {
        let mut queues = self.queues.lock().await;
        if let Some(q) = queues.get_mut(thread_id) {
            while let Some(item) = q.queue.pop_front() {
                let _ = ctx
                    .http
                    .delete_reaction_me(
                        item.channel_id,
                        item.message_id,
                        &ReactionType::Unicode(HOURGLASS.into()),
                    )
                    .await;
            }
            q.active = false;
        }
        Ok(())
    }

    async fn enqueue_or_run(
        &self,
        ctx: &Context,
        msg: &Message,
        thread_id: &str,
        channel_id: ChannelId,
        turn: UserTurn,
    ) -> Result<(), String> {
        let should_start;
        {
            let mut queues = self.queues.lock().await;
            let entry = queues.entry(thread_id.to_string()).or_insert(SessionQueue {
                active: false,
                queue: VecDeque::new(),
            });
            if entry.active {
                entry.queue.push_back(QueuedTurn {
                    message_id: msg.id,
                    channel_id,
                    turn: turn.clone(),
                });
                should_start = false;
            } else {
                entry.active = true;
                should_start = true;
            }
        }

        if !should_start {
            let _ = msg
                .react(ctx, ReactionType::Unicode(HOURGLASS.into()))
                .await;
            return Ok(());
        }

        self.run_turn(ctx, msg, thread_id, turn).await?;
        self.pump_queue(ctx, thread_id).await?;
        Ok(())
    }

    async fn pump_queue(&self, ctx: &Context, thread_id: &str) -> Result<(), String> {
        loop {
            let next = {
                let mut queues = self.queues.lock().await;
                let Some(entry) = queues.get_mut(thread_id) else {
                    return Ok(());
                };
                if let Some(item) = entry.queue.pop_front() {
                    Some(item)
                } else {
                    entry.active = false;
                    None
                }
            };
            let Some(item) = next else {
                return Ok(());
            };

            let _ = ctx
                .http
                .delete_reaction_me(
                    item.channel_id,
                    item.message_id,
                    &ReactionType::Unicode(HOURGLASS.into()),
                )
                .await;

            let msg = ctx
                .http
                .get_message(item.channel_id, item.message_id)
                .await
                .map_err(|e| e.to_string())?;
            self.run_turn(ctx, &msg, thread_id, item.turn).await?;
        }
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

        // Apply pending steer as an extra user note on this turn if present.
        let mut turn = turn;
        if let Some(steer) = self.steers.lock().await.remove(thread_id) {
            turn.text = format!("{}\n\n[steer] {steer}", turn.text);
        }

        let mut handle = session.run(turn).await;
        let mut answer = String::new();
        let mut progress_lines: Vec<(String, String)> = Vec::new();
        let mut progress_msg: Option<Message> = None;
        let mut failed = false;
        let mut fail_note = String::new();

        while let Some(ev) = handle.next_event().await {
            match ev {
                RunEvent::TextDelta { text } => answer.push_str(&text),
                RunEvent::ToolStarted { name, args } => {
                    progress_lines.push((name, args_preview(&args)));
                    let body = format_progress(&progress_lines);
                    if let Some(ref mut existing) = progress_msg {
                        let _ = existing.edit(ctx, serenity::all::EditMessage::new().content(body)).await;
                    } else {
                        match msg
                            .channel_id
                            .send_message(
                                ctx,
                                CreateMessage::new()
                                    .content(body)
                                    .reference_message(msg),
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
                    failed = true;
                    fail_note = "Run aborted.".into();
                }
                RunEvent::RunFinished => break,
                _ => {}
            }
        }

        if failed {
            let note = if fail_note.is_empty() {
                "Run failed.".into()
            } else {
                format!("Run failed: {fail_note}")
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
            while let Some(x) = chars.next() {
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
                CreateMessage::new()
                    .content(chunk)
                    .reference_message(msg),
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
