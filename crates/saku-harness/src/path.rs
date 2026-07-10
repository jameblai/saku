//! Workspace path jail: canonicalize and reject escapes.

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("path does not exist: {0}")]
    NotFound(PathBuf),
    #[error("failed to canonicalize {path}: {source}")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("path escapes Workspace: {0}")]
    OutsideWorkspace(PathBuf),
}

/// Resolve `candidate` relative to `cwd` (if relative), canonicalize, and ensure
/// the result is inside `workspace` — unless it is exactly `memory_allow`.
///
/// `memory_allow` is the exact Memory path that may lie outside the Workspace.
pub fn resolve_in_workspace(
    workspace: &Path,
    cwd: &Path,
    candidate: impl AsRef<Path>,
    memory_allow: Option<&Path>,
) -> Result<PathBuf, PathError> {
    let candidate = candidate.as_ref();
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };

    // Fast path: exact Memory allowlist before existence checks that would fail
    // for a not-yet-created Memory file on write.
    if let Some(allow) = memory_allow
        && (paths_equal_lexically(&joined, allow) || paths_equal_after_canonicalize(&joined, allow))
    {
        return Ok(allow.to_path_buf());
    }

    let canonical = canonicalize_existing_or_parent(&joined)?;
    let workspace_canon = workspace
        .canonicalize()
        .map_err(|source| PathError::Canonicalize {
            path: workspace.to_path_buf(),
            source,
        })?;

    if !is_within(&canonical, &workspace_canon) {
        // Memory allowlist after canonicalize (symlink / relative forms).
        if let Some(allow) = memory_allow {
            if let Ok(allow_canon) = allow.canonicalize()
                && canonical == allow_canon
            {
                return Ok(canonical);
            }
            if paths_equal_lexically(&canonical, allow) {
                return Ok(allow.to_path_buf());
            }
        }
        return Err(PathError::OutsideWorkspace(canonical));
    }

    Ok(canonical)
}

fn paths_equal_lexically(a: &Path, b: &Path) -> bool {
    a == b
}

fn paths_equal_after_canonicalize(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ac), Ok(bc)) => ac == bc,
        _ => false,
    }
}

/// Canonicalize an existing path, or canonicalize its parent and append the final
/// component (for create-new targets that do not exist yet).
fn canonicalize_existing_or_parent(path: &Path) -> Result<PathBuf, PathError> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|source| PathError::Canonicalize {
                path: path.to_path_buf(),
                source,
            });
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        return Err(PathError::NotFound(path.to_path_buf()));
    }
    let parent_canon = parent
        .canonicalize()
        .map_err(|source| PathError::Canonicalize {
            path: parent.to_path_buf(),
            source,
        })?;
    let name = path
        .file_name()
        .ok_or_else(|| PathError::NotFound(path.to_path_buf()))?;
    Ok(parent_canon.join(name))
}

fn is_within(path: &Path, root: &Path) -> bool {
    if path == root {
        return true;
    }
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("sub")).unwrap();
        fs::write(workspace.join("file.txt"), "hi").unwrap();
        (tmp, workspace)
    }

    #[test]
    fn allows_path_inside_workspace() {
        let (_tmp, workspace) = setup();
        let resolved =
            resolve_in_workspace(&workspace, &workspace, "file.txt", None).expect("inside");
        assert_eq!(resolved, workspace.join("file.txt").canonicalize().unwrap());
    }

    #[test]
    fn rejects_parent_traversal_outside_workspace() {
        let (_tmp, workspace) = setup();
        let err = resolve_in_workspace(&workspace, &workspace, "../secret", None).unwrap_err();
        assert!(matches!(err, PathError::OutsideWorkspace(_)));
    }

    #[test]
    fn rejects_absolute_path_outside_workspace() {
        let (_tmp, workspace) = setup();
        let outside = _tmp.path().join("outside.txt");
        fs::write(&outside, "x").unwrap();
        let err = resolve_in_workspace(&workspace, &workspace, &outside, None).unwrap_err();
        assert!(matches!(err, PathError::OutsideWorkspace(_)));
    }

    #[test]
    fn rejects_symlink_escape_outside_workspace() {
        let (_tmp, workspace) = setup();
        let outside = _tmp.path().join("outside.txt");
        fs::write(&outside, "x").unwrap();
        let link = workspace.join("escape");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let err = resolve_in_workspace(&workspace, &workspace, "escape", None).unwrap_err();
        assert!(matches!(err, PathError::OutsideWorkspace(_)));
    }

    #[test]
    fn allows_exact_memory_path_outside_workspace() {
        let (_tmp, workspace) = setup();
        let data_dir = _tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let memory = data_dir.join("MEMORY.md");
        fs::write(&memory, "pref").unwrap();
        let resolved =
            resolve_in_workspace(&workspace, &workspace, &memory, Some(&memory)).expect("memory");
        assert_eq!(resolved, memory.canonicalize().unwrap());
    }

    #[test]
    fn allows_relative_cd_within_workspace() {
        let (_tmp, workspace) = setup();
        let sub = workspace.join("sub");
        let resolved = resolve_in_workspace(&workspace, &workspace, "sub", None).expect("sub");
        assert_eq!(resolved, sub.canonicalize().unwrap());
    }
}
