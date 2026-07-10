//! Minimal system prompt builder (ADR 0016).

use std::path::Path;

pub fn build_system_prompt(workspace: &Path, cwd: &Path, memory: &str) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are Saku, a Discord-hosted coding agent for an Authorised User.\n\
         Use tools to inspect and change the Workspace. Prefer find/grep before blind bash search. \
         Read a file before editing it. Keep Memory limited to durable important facts (2200 character cap).\n\n",
    );
    prompt.push_str(&format!("Workspace: {}\n", workspace.display()));
    prompt.push_str(&format!("Working Directory: {}\n", cwd.display()));
    prompt.push_str(
        "For ongoing work in a project directory, call `cd` to set the Session Working Directory before further tools. \
         Only use `cd … &&` inside `bash` for a one-shot command in a different directory without changing the Session.\n",
    );
    prompt.push_str("\n# Memory\n");
    if memory.trim().is_empty() {
        prompt.push_str("(empty)\n");
    } else {
        prompt.push_str(memory);
        if !memory.ends_with('\n') {
            prompt.push('\n');
        }
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn includes_workspace_cwd_and_memory() {
        let text = build_system_prompt(Path::new("/ws"), Path::new("/ws/proj"), "likes rust");
        assert!(text.contains("Workspace: /ws"));
        assert!(text.contains("Working Directory: /ws/proj"));
        assert!(text.contains("likes rust"));
        assert!(text.contains("Saku"));
    }

    #[test]
    fn instructs_session_cd_for_ongoing_project_work() {
        let text = build_system_prompt(Path::new("/ws"), Path::new("/ws"), "");
        assert!(
            text.contains(
                "For ongoing work in a project directory, call `cd` to set the Session Working Directory before further tools."
            )
        );
        assert!(
            text.contains(
                "Only use `cd … &&` inside `bash` for a one-shot command in a different directory without changing the Session."
            )
        );
    }

    #[test]
    fn empty_memory_noted() {
        let text = build_system_prompt(&PathBuf::from("/w"), &PathBuf::from("/w"), "");
        assert!(text.contains("(empty)"));
    }
}
