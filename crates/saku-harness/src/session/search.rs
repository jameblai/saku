//! Cross-Session full-text search backed by a SQLite FTS5 index.
//!
//! Indexes user + assistant `text` content and Compaction summaries across every
//! Session JSONL file under the Data Dir. Tool results and image bytes are never
//! indexed. Exposed to the model through the `session_search` Tool and surfaced in
//! `saku status` via [`SessionSearchIndex::session_count`].

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use thiserror::Error;

use crate::session::store::{SessionEntry, SessionHeader};
use crate::types::{ContentPart, Role};

/// Default number of hits returned by `session_search`.
pub const DEFAULT_SEARCH_LIMIT: usize = 10;

/// SQLite database filename under the Data Dir.
const DB_FILENAME: &str = "session-search.db";

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("session search db: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("session search io: {0}")]
    Io(#[from] std::io::Error),
}

/// One ranked hit returned from a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub thread_id: String,
    /// Session creation timestamp (unix seconds, as stored in the header).
    pub session_date: String,
    /// `user`, `assistant`, or `compaction`.
    pub role: String,
    /// ~200-char excerpt around the match.
    pub excerpt: String,
}

/// Persistent SQLite FTS index over past Session text.
pub struct SessionSearchIndex {
    conn: Mutex<Connection>,
}

impl SessionSearchIndex {
    /// Open (creating if needed) the index database under `data_dir`.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, SearchError> {
        let data_dir = data_dir.as_ref();
        fs::create_dir_all(data_dir)?;
        let conn = Connection::open(data_dir.join(DB_FILENAME))?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory index (tests).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, SearchError> {
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Clear the index and rebuild it from every `*.jsonl` file in `sessions_dir`.
    ///
    /// Returns the number of Session files indexed. Best-effort per file: a file
    /// that fails to parse is skipped rather than aborting the whole rebuild.
    pub fn reindex_all(&self, sessions_dir: impl AsRef<Path>) -> Result<usize, SearchError> {
        let sessions_dir = sessions_dir.as_ref();
        let mut conn = self.conn.lock().expect("search index poisoned");
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM messages", [])?;
        tx.execute("DELETE FROM sessions", [])?;

        let mut count = 0usize;
        if sessions_dir.is_dir() {
            let mut paths: Vec<PathBuf> = fs::read_dir(sessions_dir)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|ext| ext == "jsonl"))
                .collect();
            paths.sort();
            for path in paths {
                if index_file(&tx, &path)? {
                    count += 1;
                }
            }
        }
        tx.commit()?;
        Ok(count)
    }

    /// Index one user/assistant text message. No-op for empty text.
    ///
    /// `path` is the Session JSONL file, used to register the Session's date on
    /// first sighting.
    pub fn index_message(
        &self,
        path: &Path,
        thread_id: &str,
        role: Role,
        text: &str,
    ) -> Result<(), SearchError> {
        // Tool results are never indexed.
        let Some(role_label) = role_label(role) else {
            return Ok(());
        };
        if text.trim().is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().expect("search index poisoned");
        ensure_session(&conn, path, thread_id)?;
        insert_message(&conn, thread_id, role_label, text)?;
        Ok(())
    }

    /// Index a Compaction summary.
    pub fn index_compaction(
        &self,
        path: &Path,
        thread_id: &str,
        summary: &str,
    ) -> Result<(), SearchError> {
        if summary.trim().is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().expect("search index poisoned");
        ensure_session(&conn, path, thread_id)?;
        insert_message(&conn, thread_id, "compaction", summary)?;
        Ok(())
    }

    /// Number of Sessions currently indexed (matches the JSONL file count).
    pub fn session_count(&self) -> Result<usize, SearchError> {
        let conn = self.conn.lock().expect("search index poisoned");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// Ranked full-text search. Returns up to `limit` hits, best match first.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, SearchError> {
        let fts = fts_query(query);
        if fts.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().expect("search index poisoned");
        let mut stmt = conn.prepare(
            "SELECT m.thread_id, \
                    COALESCE(s.created_at, ''), \
                    m.role, \
                    snippet(messages_fts, 0, '', '', ' … ', 32) \
             FROM messages m \
             JOIN messages_fts ON messages_fts.rowid = m.rowid \
             LEFT JOIN sessions s ON s.thread_id = m.thread_id \
             WHERE messages_fts MATCH ?1 \
             ORDER BY bm25(messages_fts) \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts, limit as i64], |row| {
            Ok(SearchHit {
                thread_id: row.get(0)?,
                session_date: row.get(1)?,
                role: row.get(2)?,
                excerpt: excerpt(&row.get::<_, String>(3)?),
            })
        })?;
        let mut hits = Vec::new();
        for hit in rows {
            hits.push(hit?);
        }
        Ok(hits)
    }
}

fn init_schema(conn: &Connection) -> Result<(), SearchError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (\
            thread_id TEXT PRIMARY KEY, \
            created_at TEXT NOT NULL\
         );\
         CREATE TABLE IF NOT EXISTS messages (\
            id INTEGER PRIMARY KEY, \
            thread_id TEXT NOT NULL, \
            role TEXT NOT NULL, \
            body TEXT NOT NULL\
         );\
         CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(\
            body, \
            content='messages', \
            content_rowid='id'\
         );\
         CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN \
            INSERT INTO messages_fts(rowid, body) VALUES (new.id, new.body); \
         END;\
         CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN \
            INSERT INTO messages_fts(messages_fts, rowid, body) VALUES('delete', old.id, old.body); \
         END;",
    )?;
    Ok(())
}

/// Register a Session's date on first sighting, reading it from the JSONL header.
fn ensure_session(conn: &Connection, path: &Path, thread_id: &str) -> Result<(), SearchError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sessions WHERE thread_id = ?1",
            params![thread_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_some() {
        return Ok(());
    }
    let created_at = read_header(path).map(|h| h.created_at).unwrap_or_default();
    conn.execute(
        "INSERT OR IGNORE INTO sessions (thread_id, created_at) VALUES (?1, ?2)",
        params![thread_id, created_at],
    )?;
    Ok(())
}

/// Index one JSONL Session file within a rebuild transaction. Returns whether the
/// file was a valid Session (header parsed).
fn index_file(conn: &Connection, path: &Path) -> Result<bool, SearchError> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(false);
    };
    let mut lines = text.lines();
    let Some(header_line) = lines.next() else {
        return Ok(false);
    };
    let Ok(header) = serde_json::from_str::<SessionHeader>(header_line) else {
        return Ok(false);
    };
    if header.entry_type != "session" {
        return Ok(false);
    }
    conn.execute(
        "INSERT OR IGNORE INTO sessions (thread_id, created_at) VALUES (?1, ?2)",
        params![header.thread_id, header.created_at],
    )?;

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<SessionEntry>(line) else {
            continue;
        };
        match entry {
            SessionEntry::Message { role, content, .. } => {
                let Some(role_label) = role_label(role) else {
                    continue; // tool results are never indexed
                };
                let body = extract_text(&content);
                if body.trim().is_empty() {
                    continue;
                }
                insert_message(conn, &header.thread_id, role_label, &body)?;
            }
            SessionEntry::Compaction { summary } => {
                if summary.trim().is_empty() {
                    continue;
                }
                insert_message(conn, &header.thread_id, "compaction", &summary)?;
            }
            _ => {}
        }
    }
    Ok(true)
}

fn read_header(path: &Path) -> Option<SessionHeader> {
    let text = fs::read_to_string(path).ok()?;
    let first = text.lines().next()?;
    serde_json::from_str::<SessionHeader>(first).ok()
}

/// FTS role label for an indexable message role. `None` means "never index"
/// (tool results), so all three index paths share one exclusion rule.
fn role_label(role: Role) -> Option<&'static str> {
    match role {
        Role::User => Some("user"),
        Role::Assistant => Some("assistant"),
        Role::Tool => None,
    }
}

/// Insert one row into `messages` (the FTS triggers mirror it into `messages_fts`).
fn insert_message(
    conn: &Connection,
    thread_id: &str,
    role: &str,
    body: &str,
) -> Result<(), SearchError> {
    conn.execute(
        "INSERT INTO messages (thread_id, role, body) VALUES (?1, ?2, ?3)",
        params![thread_id, role, body],
    )?;
    Ok(())
}

/// Concatenate `Text` parts of a message, skipping images.
pub(crate) fn extract_text(content: &[ContentPart]) -> String {
    let mut parts = Vec::new();
    for part in content {
        if let ContentPart::Text { text } = part {
            parts.push(text.as_str());
        }
    }
    parts.join("\n")
}

/// Build a safe FTS5 MATCH expression from free-text user input.
///
/// Each whitespace-separated token is wrapped in double quotes (embedded quotes
/// doubled) and joined by spaces, i.e. an implicit AND of terms. This avoids FTS5
/// syntax errors from user-supplied punctuation.
fn fts_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect();
    tokens.join(" ")
}

/// Collapse whitespace and cap an excerpt at ~200 characters.
fn excerpt(raw: &str) -> String {
    let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 200 {
        let mut out: String = collapsed.chars().take(200).collect();
        out.push('…');
        out
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_quotes_tokens_and_escapes() {
        assert_eq!(
            fts_query("compaction summary"),
            "\"compaction\" \"summary\""
        );
        assert_eq!(fts_query("  spaced   out "), "\"spaced\" \"out\"");
        assert_eq!(fts_query(r#"say "hi""#), "\"say\" \"\"\"hi\"\"\"");
        assert_eq!(fts_query(""), "");
    }

    #[test]
    fn excerpt_collapses_and_caps() {
        assert_eq!(excerpt("a\n  b   c"), "a b c");
        let long = "x ".repeat(300);
        let e = excerpt(&long);
        assert_eq!(e.chars().count(), 201); // 200 + ellipsis
        assert!(e.ends_with('…'));
    }

    #[test]
    fn search_excludes_tool_results() {
        let index = SessionSearchIndex::open_in_memory().unwrap();
        let path = Path::new("/nonexistent/thread-a.jsonl");
        index
            .index_message(path, "thread-a", Role::User, "how does compaction work")
            .unwrap();
        index
            .index_message(
                path,
                "thread-a",
                Role::Assistant,
                "compaction summarizes earlier turns to save context",
            )
            .unwrap();
        // Tool result containing the same term must not be indexed.
        index
            .index_message(
                path,
                "thread-a",
                Role::Tool,
                "compaction tool blob marker-xyz",
            )
            .unwrap();

        let hits = index.search("compaction", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.role == "user"));
        assert!(hits.iter().any(|h| h.role == "assistant"));
        // The tool text is genuinely absent — its unique marker finds nothing.
        assert!(index.search("marker-xyz", 10).unwrap().is_empty());
    }

    #[test]
    fn extract_text_skips_image_bytes() {
        let content = vec![
            ContentPart::text("visible caption"),
            ContentPart::image("image/png", vec![0xDE, 0xAD, 0xBE, 0xEF]),
        ];
        assert_eq!(extract_text(&content), "visible caption");
    }

    #[test]
    fn reindex_indexes_text_not_tool_or_image_bytes() {
        use crate::config::Effort;
        use crate::session::SessionStore;
        use crate::types::Message;

        let tmp = tempfile::tempdir().unwrap();
        let store = SessionStore::open(tmp.path()).unwrap();
        store
            .load_or_create("t", tmp.path(), "gpt-5.5", Effort::Medium)
            .unwrap();
        // A user message mixing text and image bytes.
        store
            .append_message(
                "t",
                &Message {
                    role: Role::User,
                    content: vec![
                        ContentPart::text("please look at zebra-marker"),
                        ContentPart::image("image/png", vec![0xFF, 0x00, 0xFF, 0x00]),
                    ],
                    tool_call_id: None,
                    tool_calls: Vec::new(),
                },
            )
            .unwrap();
        // A tool result that must not be indexed.
        store
            .append_message(
                "t",
                &Message {
                    role: Role::Tool,
                    content: vec![ContentPart::text("tool-marker output")],
                    tool_call_id: Some("call-1".into()),
                    tool_calls: Vec::new(),
                },
            )
            .unwrap();

        let index = SessionSearchIndex::open_in_memory().unwrap();
        assert_eq!(index.reindex_all(tmp.path().join("sessions")).unwrap(), 1);
        // Text is searchable; tool text is not; raw image bytes never enter the index.
        assert_eq!(index.search("zebra-marker", 10).unwrap().len(), 1);
        assert!(index.search("tool-marker", 10).unwrap().is_empty());
    }

    #[test]
    fn search_ranks_more_relevant_first() {
        let index = SessionSearchIndex::open_in_memory().unwrap();
        let path = Path::new("/x/t.jsonl");
        // A passing mention vs. a document that is strongly about the term.
        index
            .index_message(
                path,
                "t",
                Role::User,
                "a long note mentioning quokka once here",
            )
            .unwrap();
        index
            .index_message(path, "t", Role::Assistant, "quokka quokka quokka")
            .unwrap();
        let hits = index.search("quokka", 10).unwrap();
        assert_eq!(hits.len(), 2);
        // BM25 ranks the denser match first.
        assert_eq!(hits[0].role, "assistant");
    }

    #[test]
    fn search_empty_corpus_and_no_match() {
        let index = SessionSearchIndex::open_in_memory().unwrap();
        assert!(index.search("anything", 10).unwrap().is_empty());
        index
            .index_message(Path::new("/x/t.jsonl"), "t", Role::User, "hello world")
            .unwrap();
        assert!(index.search("compaction", 10).unwrap().is_empty());
    }

    #[test]
    fn compaction_summaries_are_indexed() {
        let index = SessionSearchIndex::open_in_memory().unwrap();
        index
            .index_compaction(
                Path::new("/x/t.jsonl"),
                "t",
                "user asked about release channels",
            )
            .unwrap();
        let hits = index.search("release channels", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].role, "compaction");
    }

    #[test]
    fn limit_caps_results() {
        let index = SessionSearchIndex::open_in_memory().unwrap();
        for i in 0..5 {
            index
                .index_message(
                    Path::new("/x/t.jsonl"),
                    "t",
                    Role::User,
                    &format!("compaction note number {i}"),
                )
                .unwrap();
        }
        assert_eq!(index.search("compaction", 3).unwrap().len(), 3);
        assert_eq!(index.search("compaction", 10).unwrap().len(), 5);
    }
}
