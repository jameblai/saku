//! Session-scoped Background Processes (ADR 0019).

mod ring;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep, timeout};

use self::ring::RingBuffer;

pub const MAX_RUNNING: usize = 5;
pub const DEFAULT_SETTLE_SECS: f64 = 2.0;
pub const DEFAULT_LOG_LINES: usize = 100;
pub const MAX_LOG_LINES: usize = 2_000;
pub const RING_CAPACITY: usize = 1024 * 1024;
const STOP_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgStatus {
    Running,
    Exited { code: i32 },
}

#[derive(Debug, Clone)]
pub struct BgEntryView {
    pub pid: u32,
    pub command: String,
    pub cwd: PathBuf,
    pub status: BgStatus,
}

struct BgEntry {
    pid: u32,
    command: String,
    cwd: PathBuf,
    status: BgStatus,
    ring: Arc<Mutex<RingBuffer>>,
    /// Held so the Child stays alive until wait completes.
    _child_slot: Arc<Mutex<Option<Child>>>,
}

/// In-memory table of Background Processes for one Session.
pub struct BackgroundProcesses {
    entries: Mutex<Vec<BgEntry>>,
}

impl Default for BackgroundProcesses {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundProcesses {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }

    pub async fn running_count(&self) -> usize {
        self.entries
            .lock()
            .await
            .iter()
            .filter(|e| matches!(e.status, BgStatus::Running))
            .count()
    }

    pub async fn summary(&self) -> (usize, usize) {
        let entries = self.entries.lock().await;
        let running = entries
            .iter()
            .filter(|e| matches!(e.status, BgStatus::Running))
            .count();
        let exited = entries.len().saturating_sub(running);
        (running, exited)
    }

    pub async fn list(&self) -> Vec<BgEntryView> {
        self.entries
            .lock()
            .await
            .iter()
            .map(|e| BgEntryView {
                pid: e.pid,
                command: e.command.clone(),
                cwd: e.cwd.clone(),
                status: e.status,
            })
            .collect()
    }

    pub async fn list_text(&self) -> String {
        let entries = self.list().await;
        if entries.is_empty() {
            return "No Background Processes.".into();
        }
        let mut out = String::from("Background Processes:\n");
        for e in entries {
            let status = match e.status {
                BgStatus::Running => "running".into(),
                BgStatus::Exited { code } => format!("exited ({code})"),
            };
            out.push_str(&format!(
                "- pid {} [{status}] cwd={} cmd={}\n",
                e.pid,
                e.cwd.display(),
                truncate_cmd(&e.command, 80)
            ));
        }
        out
    }

    pub async fn logs_text(&self, pid: u32, lines: Option<usize>) -> Result<String, String> {
        let n = lines.unwrap_or(DEFAULT_LOG_LINES).clamp(1, MAX_LOG_LINES);
        let entries = self.entries.lock().await;
        let entry = entries
            .iter()
            .find(|e| e.pid == pid)
            .ok_or_else(|| format!("no Background Process with pid {pid}"))?;
        let ring = entry.ring.lock().await;
        let text = ring.tail_lines(n);
        if text.is_empty() {
            Ok("(no output yet)".into())
        } else {
            Ok(text)
        }
    }

    /// Start a Background Process. `self` must be the `Arc` stored on the Session.
    pub async fn start(
        self: &Arc<Self>,
        command: String,
        cwd: &Path,
        settle_secs: f64,
    ) -> Result<String, String> {
        if self.running_count().await >= MAX_RUNNING {
            return Err(format!(
                "Background Process cap reached ({MAX_RUNNING} running); stop one first"
            ));
        }

        let mut child = spawn_background(&command, cwd)?;
        let pid = child
            .id()
            .ok_or_else(|| "failed to read Background Process pid".to_string())?;

        let ring = Arc::new(Mutex::new(RingBuffer::new(RING_CAPACITY)));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "missing stdout pipe".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "missing stderr pipe".to_string())?;

        spawn_reader(stdout, Arc::clone(&ring));
        spawn_reader(stderr, Arc::clone(&ring));

        let child_slot = Arc::new(Mutex::new(Some(child)));
        {
            let mut entries = self.entries.lock().await;
            entries.push(BgEntry {
                pid,
                command: command.clone(),
                cwd: cwd.to_path_buf(),
                status: BgStatus::Running,
                ring: Arc::clone(&ring),
                _child_slot: Arc::clone(&child_slot),
            });
        }

        {
            let table = Arc::clone(self);
            let slot = Arc::clone(&child_slot);
            tokio::spawn(async move {
                let code = {
                    let mut guard = slot.lock().await;
                    if let Some(child) = guard.as_mut() {
                        match child.wait().await {
                            Ok(status) => status.code().unwrap_or(-1),
                            Err(_) => -1,
                        }
                    } else {
                        -1
                    }
                };
                let mut entries = table.entries.lock().await;
                if let Some(entry) = entries.iter_mut().find(|e| e.pid == pid)
                    && matches!(entry.status, BgStatus::Running)
                {
                    entry.status = BgStatus::Exited { code };
                }
            });
        }

        let settle = Duration::from_secs_f64(settle_secs.max(0.0));
        if !settle.is_zero() {
            sleep(settle).await;
        }

        // If it already exited during settle, surface that.
        {
            let entries = self.entries.lock().await;
            if let Some(entry) = entries.iter().find(|e| e.pid == pid)
                && let BgStatus::Exited { code } = entry.status
            {
                let snippet = entry.ring.lock().await.as_lossy_string();
                let mut out = format!("pid: {pid}\nexited during settle (exit {code})\n");
                if !snippet.trim().is_empty() {
                    out.push_str(&trim_snippet(&snippet));
                }
                return Ok(out);
            }
        }

        let snippet = {
            let ring = ring.lock().await;
            trim_snippet(&ring.as_lossy_string())
        };

        let mut out = format!("pid: {pid}\n");
        if !snippet.is_empty() {
            out.push_str(&snippet);
            if !snippet.ends_with('\n') {
                out.push('\n');
            }
        }
        Ok(out)
    }

    /// Stop one Background Process by pid, or all running when `pid` is `None`.
    pub async fn stop(&self, pid: Option<u32>) -> Result<String, String> {
        let targets: Vec<u32> = {
            let entries = self.entries.lock().await;
            match pid {
                Some(p) => {
                    if !entries.iter().any(|e| e.pid == p) {
                        return Err(format!("no Background Process with pid {p}"));
                    }
                    // Stopping an already-exited entry is a no-op success.
                    if entries
                        .iter()
                        .any(|e| e.pid == p && matches!(e.status, BgStatus::Exited { .. }))
                    {
                        return Ok(format!("pid {p} already exited"));
                    }
                    vec![p]
                }
                None => entries
                    .iter()
                    .filter(|e| matches!(e.status, BgStatus::Running))
                    .map(|e| e.pid)
                    .collect(),
            }
        };

        if targets.is_empty() {
            return Ok("No running Background Processes to stop.".into());
        }

        let mut lines = Vec::new();
        for target in targets {
            stop_process_group(target);
            let exited = wait_until_exited(self, target, STOP_GRACE).await;
            if !exited {
                kill_process_group(target);
                let _ = wait_until_exited(self, target, Duration::from_secs(2)).await;
            }
            {
                let mut entries = self.entries.lock().await;
                if let Some(entry) = entries.iter_mut().find(|e| e.pid == target) {
                    if matches!(entry.status, BgStatus::Running) {
                        entry.status = BgStatus::Exited { code: -1 };
                    }
                    let code = match entry.status {
                        BgStatus::Exited { code } => code,
                        BgStatus::Running => -1,
                    };
                    lines.push(format!("stopped pid {target} (exit {code})"));
                }
            }
        }
        Ok(lines.join("\n"))
    }
}

fn trim_snippet(text: &str) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let bytes = text.as_bytes();
    let start = bytes.len().saturating_sub(2048);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

fn truncate_cmd(cmd: &str, max: usize) -> String {
    if cmd.chars().count() <= max {
        cmd.to_string()
    } else {
        let mut t: String = cmd.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn spawn_background(command: &str, cwd: &Path) -> Result<Child, String> {
    let mut cmd = Command::new("bash");
    cmd.arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(false)
        .process_group(0);
    cmd.spawn()
        .map_err(|e| format!("failed to start Background Process: {e}"))
}

fn spawn_reader<R>(reader: R, ring: Arc<Mutex<RingBuffer>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut buf = vec![0u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    ring.lock().await.write(&buf[..n]);
                }
                Err(_) => break,
            }
        }
    });
}

fn signal_process_group(pid: u32, sig: i32) {
    let pgid = pid as i32;
    unsafe {
        let _ = libc::kill(-pgid, sig);
    }
}

fn stop_process_group(pid: u32) {
    signal_process_group(pid, libc::SIGTERM);
}

fn kill_process_group(pid: u32) {
    signal_process_group(pid, libc::SIGKILL);
}

async fn wait_until_exited(table: &BackgroundProcesses, pid: u32, grace: Duration) -> bool {
    let result = timeout(grace, async {
        loop {
            {
                let entries = table.entries.lock().await;
                if let Some(entry) = entries.iter().find(|e| e.pid == pid) {
                    if !matches!(entry.status, BgStatus::Running) {
                        return true;
                    }
                } else {
                    return true;
                }
            }
            if !os_alive(pid) {
                let mut entries = table.entries.lock().await;
                if let Some(entry) = entries.iter_mut().find(|e| e.pid == pid)
                    && matches!(entry.status, BgStatus::Running)
                {
                    entry.status = BgStatus::Exited { code: -1 };
                }
                return true;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    result.unwrap_or(false)
}

fn os_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
