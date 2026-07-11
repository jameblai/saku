//! Serenity bot wiring.

use saku_harness::{
    ALLOWED_MODELS, Config, Effort, GoalDecision, Harness, MAX_GOAL_RUNS, RunEvent, RunHandle,
    Session, UserTurn, is_allowed_model, is_supported_effort, supported_efforts,
};
use serenity::Client;
use serenity::all::{
    ChannelId, Context, CreateMessage, EditMessage, EventHandler, GatewayIntents, Message,
    MessageReference, ReactionType,
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
        self.resume_goals(&ctx).await;
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
            BotCommand::Goal { condition: None } => match session.goal().await {
                Some(goal) => {
                    let mut text = format!(
                        "Goal · {}\nRun {}/{}",
                        goal.condition, goal.run_count, MAX_GOAL_RUNS
                    );
                    if let Some(reason) = goal.last_evaluator_reason {
                        text.push_str(&format!("\nLast reason: {reason}"));
                    }
                    text
                }
                None => "No active Goal. Set one with `saku goal <condition>`.".into(),
            },
            BotCommand::Goal {
                condition: Some(condition),
            } => {
                session.set_goal(&condition).await?;
                reply_chunks(ctx, msg.channel_id, msg, &format!("Goal set · {condition}")).await?;
                react_ok(ctx, msg).await;
                // Drive the outer Goal loop, seeding the first Run with the condition.
                // If a driver is already active (a prior Goal), it picks up the
                // replaced Goal on its next evaluation and this returns immediately.
                drive_goal(
                    ctx,
                    msg.channel_id,
                    Some(msg),
                    &session,
                    Some(UserTurn::text(condition)),
                )
                .await;
                return Ok(());
            }
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

    /// Resume outer Goal loops for Sessions that had an active Goal at shutdown.
    async fn resume_goals(&self, ctx: &Context) {
        let thread_ids = self.harness.sessions_with_active_goal().await;
        for thread_id in thread_ids {
            let Ok(channel_snowflake) = thread_id.parse::<u64>() else {
                continue;
            };
            let ctx = ctx.clone();
            let harness = self.harness.clone();
            tokio::spawn(async move {
                let Ok(session) = harness.session(&thread_id).await else {
                    return;
                };
                let channel = ChannelId::new(channel_snowflake);
                info!("resuming Goal for thread {thread_id}");
                drive_goal(&ctx, channel, None, &session, None).await;
            });
        }
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

        let handle = session.run(turn).await;
        render_run(ctx, Some(msg), output_channel, handle).await?;
        Ok(())
    }
}

/// Outcome of rendering one Run to Discord.
struct RunRender {
    failed: bool,
    aborted: bool,
}

/// Stream one Run's events to Discord: hourglass/typing, Progress Message, and
/// the Answer Message. `reference` is the triggering user Message (for reply
/// references and reactions) or `None` for auto-continuation Goal Runs.
async fn render_run(
    ctx: &Context,
    reference: Option<&Message>,
    output_channel: ChannelId,
    mut handle: RunHandle,
) -> Result<RunRender, String> {
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
                if let Some(m) = reference {
                    let _ = m.react(ctx, ReactionType::Unicode(HOURGLASS.into())).await;
                    hourglass = true;
                }
            }
            RunEvent::Dequeued => {
                if hourglass && let Some(m) = reference {
                    let _ = ctx
                        .http
                        .delete_reaction_me(
                            m.channel_id,
                            m.id,
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
                    let _ = existing.edit(ctx, EditMessage::new().content(body)).await;
                } else {
                    match output_channel
                        .send_message(ctx, build_reply_opt(body, output_channel, reference))
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

    if hourglass && let Some(m) = reference {
        let _ = ctx
            .http
            .delete_reaction_me(m.channel_id, m.id, &ReactionType::Unicode(HOURGLASS.into()))
            .await;
    }

    if failed || aborted {
        let note = if fail_note.is_empty() {
            "Run failed.".into()
        } else {
            fail_note
        };
        send_note(ctx, output_channel, reference, &note).await?;
        return Ok(RunRender { failed, aborted });
    }

    if answer.trim().is_empty() {
        answer = "(no assistant text)".into();
    }
    send_note(ctx, output_channel, reference, &answer).await?;
    if let Some(m) = reference {
        react_ok(ctx, m).await;
    }
    Ok(RunRender { failed, aborted })
}

/// Drive the outer Goal loop for `session`: optionally run an initial turn, then
/// after each working Run consult the Goal Evaluator and either continue with a
/// fresh Run, report achievement, or stop at the `MAX_GOAL_RUNS` cap.
///
/// Holds the single Goal-driver slot for its duration; if another loop already
/// owns it (e.g. a resumed Goal), this returns without doing anything.
async fn drive_goal(
    ctx: &Context,
    output_channel: ChannelId,
    reference: Option<&Message>,
    session: &Session,
    initial_turn: Option<UserTurn>,
) {
    let _guard = match session.try_become_goal_driver() {
        Some(guard) => guard,
        None => return,
    };

    if let Some(turn) = initial_turn {
        let handle = session.run(turn).await;
        match render_run(ctx, reference, output_channel, handle).await {
            Ok(render) if render.aborted || render.failed => return,
            Ok(_) => {}
            Err(err) => {
                warn!("goal run render failed: {err}");
                return;
            }
        }
    }

    loop {
        let decision = match session.evaluate_goal().await {
            Ok(decision) => decision,
            Err(err) => {
                warn!("goal evaluator failed: {err}");
                session.clear_goal().await;
                let _ = send_note(
                    ctx,
                    output_channel,
                    None,
                    &format!("Goal evaluator failed: {err} · Goal cleared."),
                )
                .await;
                return;
            }
        };

        match decision {
            GoalDecision::Inactive => return,
            GoalDecision::Achieved { condition, .. } => {
                let _ = send_note(
                    ctx,
                    output_channel,
                    None,
                    &format!("◎ goal achieved · {condition}"),
                )
                .await;
                return;
            }
            GoalDecision::Exhausted { condition, run } => {
                let _ = send_note(
                    ctx,
                    output_channel,
                    None,
                    &format!("Goal stopped after {run} Runs (max {MAX_GOAL_RUNS}) · {condition}"),
                )
                .await;
                return;
            }
            GoalDecision::Continue { message, .. } => {
                let handle = session.run(UserTurn::text(message)).await;
                match render_run(ctx, None, output_channel, handle).await {
                    // A `saku stop` aborts the Run and clears the Goal; end the loop.
                    Ok(render) if render.aborted => return,
                    Ok(_) => {}
                    Err(err) => {
                        warn!("goal run render failed: {err}");
                        return;
                    }
                }
            }
        }
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

/// Build a reply Message, tolerating the absence of a triggering Message
/// (auto-continuation Goal Runs post without a reply reference).
fn build_reply_opt(
    content: impl Into<String>,
    output_channel: ChannelId,
    trigger: Option<&Message>,
) -> CreateMessage {
    let mut message = CreateMessage::new().content(content);
    if let Some(trigger) = trigger
        && can_reference_trigger(output_channel, trigger.channel_id)
    {
        message = message.reference_message(reply_reference(trigger));
    }
    message
}

/// Send `text` (chunked) to `channel_id`, referencing `reference` when present.
async fn send_note(
    ctx: &Context,
    channel_id: ChannelId,
    reference: Option<&Message>,
    text: &str,
) -> Result<(), String> {
    for chunk in chunk_message(text) {
        channel_id
            .send_message(ctx, build_reply_opt(chunk, channel_id, reference))
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn reply_chunks(
    ctx: &Context,
    channel_id: ChannelId,
    msg: &Message,
    text: &str,
) -> Result<(), String> {
    send_note(ctx, channel_id, Some(msg), text).await
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
