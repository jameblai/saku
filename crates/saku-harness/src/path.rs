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
/// the result is inside `workspace`.
pub fn resolve_in_workspace(
    workspace: &Path,
    cwd: &Path,
    candidate: impl AsRef<Path>,
) -> Result<PathBuf, PathError> {
    resolve_in_workspace_or_allowlist(workspace, cwd, candidate, &[])
}

/// Like [`resolve_in_workspace`], but also allows paths under `allowlist_roots`
/// (discovered Skill base directories for `read` / skill assets).
pub fn resolve_in_workspace_or_allowlist(
    workspace: &Path,
    cwd: &Path,
    candidate: impl AsRef<Path>,
    allowlist_roots: &[PathBuf],
) -> Result<PathBuf, PathError> {
    let candidate = candidate.as_ref();
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };

    let canonical = canonicalize_existing_or_parent(&joined)?;
    let workspace_canon = workspace
        .canonicalize()
        .map_err(|source| PathError::Canonicalize {
            path: workspace.to_path_buf(),
            source,
        })?;

    if is_within(&canonical, &workspace_canon) {
        return Ok(canonical);
    }

    for root in allowlist_roots {
        let Ok(root_canon) = root.canonicalize() else {
            continue;
        };
        if is_within(&canonical, &root_canon) {
            return Ok(canonical);
        }
    }

    Err(PathError::OutsideWorkspace(canonical))
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
        let resolved = resolve_in_workspace(&workspace, &workspace, "file.txt").expect("inside");
        assert_eq!(resolved, workspace.join("file.txt").canonicalize().unwrap());
    }

    #[test]
    fn allowlist_permits_skill_path_outside_workspace() {
        let (_tmp, workspace) = setup();
        let skill_dir = _tmp.path().join("skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        fs::write(&skill_file, "x").unwrap();
        let resolved = resolve_in_workspace_or_allowlist(
            &workspace,
            &workspace,
            &skill_file,
            std::slice::from_ref(&skill_dir),
        )
        .expect("skill allowlisted");
        assert_eq!(resolved, skill_file.canonicalize().unwrap());
    }

    #[test]
    fn rejects_parent_traversal_outside_workspace() {
        let (_tmp, workspace) = setup();
        let err = resolve_in_workspace(&workspace, &workspace, "../secret").unwrap_err();
        assert!(matches!(err, PathError::OutsideWorkspace(_)));
    }

    #[test]
    fn rejects_absolute_path_outside_workspace() {
        let (_tmp, workspace) = setup();
        let outside = _tmp.path().join("outside.txt");
        fs::write(&outside, "x").unwrap();
        let err = resolve_in_workspace(&workspace, &workspace, &outside).unwrap_err();
        assert!(matches!(err, PathError::OutsideWorkspace(_)));
    }

    #[test]
    fn rejects_symlink_escape_outside_workspace() {
        let (_tmp, workspace) = setup();
        let outside = _tmp.path().join("outside.txt");
        fs::write(&outside, "x").unwrap();
        let link = workspace.join("escape");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let err = resolve_in_workspace(&workspace, &workspace, "escape").unwrap_err();
        assert!(matches!(err, PathError::OutsideWorkspace(_)));
    }

    #[test]
    fn rejects_exact_memory_path_outside_workspace() {
        let (_tmp, workspace) = setup();
        let data_dir = _tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let memory = data_dir.join("MEMORY.md");
        fs::write(&memory, "pref").unwrap();
        let err = resolve_in_workspace(&workspace, &workspace, &memory).unwrap_err();
        assert!(matches!(err, PathError::OutsideWorkspace(_)));
    }

    #[test]
    fn allows_relative_cd_within_workspace() {
        let (_tmp, workspace) = setup();
        let sub = workspace.join("sub");
        let resolved = resolve_in_workspace(&workspace, &workspace, "sub").expect("sub");
        assert_eq!(resolved, sub.canonicalize().unwrap());
    }
}
