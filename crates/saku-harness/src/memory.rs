//! Memory file helpers (`MEMORY.md` under the Data Dir).

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Hard character cap for Memory contents.
pub const MEMORY_CHAR_LIMIT: usize = 2200;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("failed to read Memory: {0}")]
    Io(#[from] std::io::Error),
    #[error("Memory exceeds {MEMORY_CHAR_LIMIT} characters ({actual} characters)")]
    TooLong { actual: usize },
}

/// Path to Memory given a Data Dir.
pub fn memory_path(data_dir: impl AsRef<Path>) -> PathBuf {
    data_dir.as_ref().join("MEMORY.md")
}

/// Read Memory contents; missing file yields an empty string.
pub fn read_memory(data_dir: impl AsRef<Path>) -> Result<String, MemoryError> {
    let path = memory_path(data_dir);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err.into()),
    }
}

/// Reject Memory writes over the character cap. Does not write to disk.
pub fn validate_memory_write(content: &str) -> Result<(), MemoryError> {
    let actual = content.chars().count();
    if actual > MEMORY_CHAR_LIMIT {
        return Err(MemoryError::TooLong { actual });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn read_missing_memory_is_empty() {
        let tmp = TempDir::new().unwrap();
        let text = read_memory(tmp.path()).unwrap();
        assert_eq!(text, "");
    }

    #[test]
    fn read_existing_memory() {
        let tmp = TempDir::new().unwrap();
        fs::write(memory_path(tmp.path()), "likes rust").unwrap();
        assert_eq!(read_memory(tmp.path()).unwrap(), "likes rust");
    }

    #[test]
    fn accepts_write_at_limit() {
        let content: String = "a".repeat(MEMORY_CHAR_LIMIT);
        validate_memory_write(&content).unwrap();
    }

    #[test]
    fn rejects_write_over_limit() {
        let content: String = "a".repeat(MEMORY_CHAR_LIMIT + 1);
        let err = validate_memory_write(&content).unwrap_err();
        assert!(matches!(
            err,
            MemoryError::TooLong {
                actual: a
            } if a == MEMORY_CHAR_LIMIT + 1
        ));
    }

    #[test]
    fn char_limit_counts_unicode_scalars() {
        // "😀" is one char but multiple bytes.
        let content: String = "😀".repeat(MEMORY_CHAR_LIMIT);
        validate_memory_write(&content).unwrap();
        let over: String = "😀".repeat(MEMORY_CHAR_LIMIT + 1);
        assert!(validate_memory_write(&over).is_err());
    }
}
