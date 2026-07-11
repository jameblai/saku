//! Minimal system prompt builder (ADR 0016).

use std::path::Path;

pub fn build_system_prompt(
    workspace: &Path,
    cwd: &Path,
    memory: &str,
    goal: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You are Saku, a Discord-hosted coding agent for an Authorised User.\n\
         Use tools to inspect and change the Workspace. Prefer find/grep before blind bash search. \
         Read a file before editing it. Update Memory only via the `memory` tool when retaining \
         durable facts across Sessions; never claim you remembered unless that call succeeded. \
         Keep Memory limited to durable important facts (2200 character cap).\n\n",
    );
    if let Some(condition) = goal.map(str::trim).filter(|c| !c.is_empty()) {
        prompt.push_str(&format!(
            "# Active Goal\n\
             You are working autonomously toward this Goal: {condition}\n\
             Each Run is one step; keep making concrete progress. After this Run a Goal Evaluator \
             judges whether the Goal is met, and if not you will be continued automatically. \
             The user's message for this Run restates the Goal and the Evaluator's last reason.\n\n",
        ));
    }
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
        let text = build_system_prompt(Path::new("/ws"), Path::new("/ws/proj"), "likes rust", None);
        assert!(text.contains("Workspace: /ws"));
        assert!(text.contains("Working Directory: /ws/proj"));
        assert!(text.contains("likes rust"));
        assert!(text.contains("Saku"));
    }

    #[test]
    fn active_goal_noted_when_present() {
        let text = build_system_prompt(
            Path::new("/ws"),
            Path::new("/ws"),
            "",
            Some("cargo test green"),
        );
        assert!(text.contains("# Active Goal"));
        assert!(text.contains("cargo test green"));
    }

    #[test]
    fn no_goal_section_without_active_goal() {
        let text = build_system_prompt(Path::new("/ws"), Path::new("/ws"), "", None);
        assert!(!text.contains("# Active Goal"));
        // Blank/whitespace conditions are treated as no Goal.
        let blank = build_system_prompt(Path::new("/ws"), Path::new("/ws"), "", Some("  "));
        assert!(!blank.contains("# Active Goal"));
    }

    #[test]
    fn instructs_session_cd_for_ongoing_project_work() {
        let text = build_system_prompt(Path::new("/ws"), Path::new("/ws"), "", None);
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
    fn instructs_memory_tool_for_durable_facts() {
        let text = build_system_prompt(Path::new("/ws"), Path::new("/ws"), "", None);
        assert!(text.contains("Update Memory only via the `memory` tool"));
        assert!(text.contains("never claim you remembered unless that call succeeded"));
    }

    #[test]
    fn empty_memory_noted() {
        let text = build_system_prompt(&PathBuf::from("/w"), &PathBuf::from("/w"), "", None);
        assert!(text.contains("(empty)"));
    }
}
