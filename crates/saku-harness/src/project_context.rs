//! Project Context: discover `AGENTS.md` and format it for System Prompt injection.

use std::fs;
use std::path::{Path, PathBuf};

/// One discovered `AGENTS.md` file (path + full contents).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectContextFile {
    pub path: PathBuf,
    pub content: String,
}

/// Discover `AGENTS.md` files for a Run.
///
/// Walks Workspace ancestors from root → cwd. Stops at the Workspace root
/// (never climbs outside it). No home/global `AGENTS.md`.
pub fn load_project_context(workspace: &Path, cwd: &Path) -> Vec<ProjectContextFile> {
    let mut files = Vec::new();
    for dir in ancestor_dirs_within_workspace(workspace, cwd) {
        let candidate = dir.join("AGENTS.md");
        if let Some(file) = read_agents_md(&candidate) {
            files.push(file);
        }
    }
    files
}

/// pi-compatible `<project_context>` block, or `None` when nothing was loaded.
pub fn format_project_context_block(files: &[ProjectContextFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let mut out = String::from("\n\n<project_context>\n\n");
    out.push_str("Project-specific instructions and guidelines:\n\n");
    for file in files {
        out.push_str(&format!(
            "<project_instructions path=\"{}\">\n{}\n</project_instructions>\n\n",
            file.path.display(),
            file.content
        ));
    }
    out.push_str("</project_context>\n");
    Some(out)
}

fn read_agents_md(path: &Path) -> Option<ProjectContextFile> {
    match fs::read_to_string(path) {
        Ok(content) => Some(ProjectContextFile {
            path: path.to_path_buf(),
            content,
        }),
        Err(_) => None,
    }
}

/// Directories from Workspace root → cwd (inclusive), or empty if cwd is outside.
fn ancestor_dirs_within_workspace(workspace: &Path, cwd: &Path) -> Vec<PathBuf> {
    let Ok(ws) = workspace.canonicalize() else {
        return Vec::new();
    };
    let Ok(mut cur) = cwd.canonicalize() else {
        return Vec::new();
    };
    if !is_within(&cur, &ws) {
        return Vec::new();
    }

    let mut dirs = Vec::new();
    loop {
        dirs.push(cur.clone());
        if cur == ws {
            break;
        }
        match cur.parent() {
            Some(parent) => cur = parent.to_path_buf(),
            None => break,
        }
    }
    dirs.reverse();
    dirs
}

fn is_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_workspace() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        fs::create_dir_all(workspace.join("crates/foo")).unwrap();
        (tmp, workspace)
    }

    #[test]
    fn missing_files_yield_empty() {
        let (_tmp, workspace) = setup_workspace();
        let cwd = workspace.join("crates/foo");
        let files = load_project_context(&workspace, &cwd);
        assert!(files.is_empty());
    }

    #[test]
    fn loads_ancestors_root_to_cwd() {
        let (_tmp, workspace) = setup_workspace();
        fs::write(workspace.join("AGENTS.md"), "workspace norms").unwrap();
        fs::write(workspace.join("crates/foo/AGENTS.md"), "crate norms").unwrap();

        let cwd = workspace.join("crates/foo");
        let files = load_project_context(&workspace, &cwd);

        assert_eq!(
            files
                .iter()
                .map(|f| (f.path.clone(), f.content.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (workspace.join("AGENTS.md"), "workspace norms"),
                (workspace.join("crates/foo/AGENTS.md"), "crate norms"),
            ]
        );
    }

    #[test]
    fn skips_missing_intermediate_agents_md() {
        let (_tmp, workspace) = setup_workspace();
        fs::write(workspace.join("AGENTS.md"), "workspace").unwrap();
        // crates/ and foo/ have no AGENTS.md — only workspace root.

        let cwd = workspace.join("crates/foo");
        let files = load_project_context(&workspace, &cwd);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "workspace");
    }

    #[test]
    fn workspace_boundary_excludes_outside_agents_md() {
        let (tmp, workspace) = setup_workspace();
        // Parent of Workspace (would be reached if we walked to filesystem root).
        fs::write(tmp.path().join("AGENTS.md"), "outside secrets").unwrap();
        fs::write(workspace.join("AGENTS.md"), "inside").unwrap();

        let cwd = workspace.join("crates/foo");
        let files = load_project_context(&workspace, &cwd);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].content, "inside");
        assert!(!files.iter().any(|f| f.content.contains("outside")));
    }

    #[test]
    fn cwd_outside_workspace_loads_nothing() {
        let (tmp, workspace) = setup_workspace();
        let outside = tmp.path().join("elsewhere");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("AGENTS.md"), "should not load").unwrap();
        fs::write(workspace.join("AGENTS.md"), "inside").unwrap();

        let files = load_project_context(&workspace, &outside);
        assert!(files.is_empty());
    }

    #[test]
    fn format_block_none_when_empty() {
        assert!(format_project_context_block(&[]).is_none());
    }

    #[test]
    fn format_block_wraps_pi_style_xml() {
        let files = vec![
            ProjectContextFile {
                path: PathBuf::from("/ws/AGENTS.md"),
                content: "rule a".into(),
            },
            ProjectContextFile {
                path: PathBuf::from("/ws/crates/foo/AGENTS.md"),
                content: "rule b".into(),
            },
        ];
        let block = format_project_context_block(&files).unwrap();
        assert!(block.starts_with("\n\n<project_context>\n\n"));
        assert!(block.contains("Project-specific instructions and guidelines:\n\n"));
        assert!(block.contains(
            "<project_instructions path=\"/ws/AGENTS.md\">\nrule a\n</project_instructions>"
        ));
        assert!(block.contains(
            "<project_instructions path=\"/ws/crates/foo/AGENTS.md\">\nrule b\n</project_instructions>"
        ));
        assert!(block.ends_with("</project_context>\n"));
    }
}
