//! Workspace FFF index for find/grep tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use fff_search::file_picker::{FFFMode, FilePicker, FilePickerOptions, FuzzySearchOptions};
use fff_search::grep::{GrepMode, GrepSearchOptions};
use fff_search::{PaginationArgs, QueryParser, SharedFilePicker, SharedFrecency, SharedQueryTracker};
use thiserror::Error;
use tokio::sync::OnceCell;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("fff index: {0}")]
    Fff(String),
    #[error("fff scan timed out")]
    ScanTimeout,
}

/// Long-lived FFF index over the Workspace.
pub struct WorkspaceIndex {
    workspace: PathBuf,
    picker: SharedFilePicker,
    query_tracker: SharedQueryTracker,
    ready: OnceCell<()>,
}

impl WorkspaceIndex {
    pub fn new(workspace: impl Into<PathBuf>, data_dir: impl AsRef<Path>) -> Result<Self, IndexError> {
        let workspace = workspace.into();
        let data_dir = data_dir.as_ref();
        let fff_dir = data_dir.join("fff");
        std::fs::create_dir_all(&fff_dir).map_err(|e| IndexError::Fff(e.to_string()))?;

        let shared_picker = SharedFilePicker::default();
        let shared_frecency = SharedFrecency::default();
        let shared_query_tracker = SharedQueryTracker::default();

        // Best-effort DB init; search still works without frecency.
        if let Ok(frecency) = fff_search::frecency::FrecencyTracker::open(fff_dir.join("frecency")) {
            let _ = shared_frecency.init(frecency);
        }
        if let Ok(qt) = fff_search::query_tracker::QueryTracker::open(fff_dir.join("queries")) {
            let _ = shared_query_tracker.init(qt);
        }

        let is_home = dirs::home_dir().is_some_and(|h| h == workspace);
        FilePicker::new_with_shared_state(
            shared_picker.clone(),
            shared_frecency.clone(),
            FilePickerOptions {
                base_path: workspace.display().to_string(),
                mode: FFFMode::Ai,
                watch: false, // tests + bot: rescan on demand is enough for v1
                enable_home_dir_scanning: is_home,
                enable_fs_root_scanning: false,
                ..Default::default()
            },
        )
        .map_err(|e| IndexError::Fff(e.to_string()))?;

        Ok(Self {
            workspace,
            picker: shared_picker,
            query_tracker: shared_query_tracker,
            ready: OnceCell::new(),
        })
    }

    pub async fn ensure_ready(&self) -> Result<(), IndexError> {
        self.ready
            .get_or_try_init(|| async {
                let picker = self.picker.clone();
                tokio::task::spawn_blocking(move || {
                    if picker.wait_for_scan(Duration::from_secs(30)) {
                        Ok(())
                    } else {
                        Err(IndexError::ScanTimeout)
                    }
                })
                .await
                .map_err(|e| IndexError::Fff(e.to_string()))?
            })
            .await?;
        Ok(())
    }

    pub fn find(&self, pattern: &str, limit: usize) -> Result<Vec<String>, IndexError> {
        if is_wildcard_only(pattern) {
            return Err(IndexError::Fff(
                "pattern is wildcard-only; provide a more specific query".into(),
            ));
        }
        let parser = QueryParser::default();
        let query = parser.parse(pattern);
        let guard = self
            .picker
            .read()
            .map_err(|e| IndexError::Fff(e.to_string()))?;
        let picker = guard
            .as_ref()
            .ok_or_else(|| IndexError::Fff("picker not initialized".into()))?;
        let qt = self
            .query_tracker
            .read()
            .map_err(|e| IndexError::Fff(e.to_string()))?;
        let results = picker.fuzzy_search(
            &query,
            qt.as_ref(),
            FuzzySearchOptions {
                max_threads: 0,
                current_file: None,
                pagination: PaginationArgs {
                    offset: 0,
                    limit: limit.max(1),
                },
                ..Default::default()
            },
        );
        Ok(results
            .items
            .iter()
            .map(|item| item.relative_path(picker))
            .collect())
    }

    pub fn grep(&self, pattern: &str, limit: usize) -> Result<Vec<String>, IndexError> {
        if is_wildcard_only(pattern) {
            return Err(IndexError::Fff(
                "pattern is wildcard-only; provide a more specific query".into(),
            ));
        }
        let parser = QueryParser::default();
        let query = parser.parse(pattern);
        let guard = self
            .picker
            .read()
            .map_err(|e| IndexError::Fff(e.to_string()))?;
        let picker = guard
            .as_ref()
            .ok_or_else(|| IndexError::Fff("picker not initialized".into()))?;

        let mut options = GrepSearchOptions {
            page_limit: limit.max(1),
            smart_case: true,
            mode: GrepMode::PlainText,
            trim_whitespace: true,
            ..Default::default()
        };
        let mut result = picker.grep(&query, &options);
        // Fuzzy fallback when plain text finds nothing and pattern has no spaces.
        if result.matches.is_empty() && !pattern.contains(' ') {
            options.mode = GrepMode::Fuzzy;
            result = picker.grep(&query, &options);
        }

        let mut lines = Vec::new();
        for m in result.matches.iter().take(limit) {
            let file = result
                .files
                .get(m.file_index)
                .map(|f| f.relative_path(picker))
                .unwrap_or_else(|| "?".into());
            lines.push(format!(
                "{file}:{}:{}",
                m.line_number,
                m.line_content.trim_end()
            ));
        }
        Ok(lines)
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
}

fn is_wildcard_only(pattern: &str) -> bool {
    let trimmed = pattern.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| matches!(c, '*' | '?' | '/' | '\\' | '.'))
}

/// Shared handle used by search tools.
pub type SharedIndex = Arc<WorkspaceIndex>;
