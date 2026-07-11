//! Minimal system prompt builder (ADR 0016).

use std::path::Path;

use crate::project_context::{
    ProjectContextFile, format_project_context_block, load_project_context,
};
use crate::skills::Skill;

pub fn build_system_prompt(
    workspace: &Path,
    cwd: &Path,
    memory: &str,
    goal: Option<&str>,
) -> String {
    build_system_prompt_with_skills(workspace, cwd, memory, goal, &[])
}

/// Build the System Prompt, appending Project Context and `<available_skills>` when present.
pub fn build_system_prompt_with_skills(
    workspace: &Path,
    cwd: &Path,
    memory: &str,
    goal: Option<&str>,
    skills: &[Skill],
) -> String {
    let context = load_project_context(workspace, cwd);
    build_system_prompt_with_context(workspace, cwd, memory, goal, skills, &context)
}

pub fn build_system_prompt_with_context(
    workspace: &Path,
    cwd: &Path,
    memory: &str,
    goal: Option<&str>,
    skills: &[Skill],
    project_context: &[ProjectContextFile],
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
    if let Some(block) = format_project_context_block(project_context) {
        prompt.push_str(&block);
    }
    let skills_block = crate::skills::format_skills_for_prompt(skills);
    if !skills_block.is_empty() {
        prompt.push('\n');
        prompt.push_str(&skills_block);
        if !skills_block.ends_with('\n') {
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

    #[test]
    fn appends_available_skills_block() {
        let skills = vec![crate::skills::Skill {
            name: "triage".into(),
            description: "Triage issues".into(),
            file_path: PathBuf::from("/home/u/.agents/skills/triage/SKILL.md"),
            base_dir: PathBuf::from("/home/u/.agents/skills/triage"),
            disable_model_invocation: false,
        }];
        let text =
            build_system_prompt_with_skills(Path::new("/ws"), Path::new("/ws"), "", None, &skills);
        assert!(text.contains("<available_skills>"));
        assert!(text.contains("<name>triage</name>"));
        assert!(text.contains("Use the read tool"));
    }

    #[test]
    fn omits_project_context_when_none_loaded() {
        let text = build_system_prompt_with_context(
            Path::new("/ws"),
            Path::new("/ws"),
            "",
            None,
            &[],
            &[],
        );
        assert!(!text.contains("<project_context>"));
    }

    #[test]
    fn appends_project_context_after_memory() {
        let files = vec![ProjectContextFile {
            path: PathBuf::from("/ws/AGENTS.md"),
            content: "prefer rust".into(),
        }];
        let text = build_system_prompt_with_context(
            Path::new("/ws"),
            Path::new("/ws"),
            "likes tea",
            None,
            &[],
            &files,
        );
        let memory_pos = text.find("# Memory").expect("memory");
        let ctx_pos = text.find("<project_context>").expect("project context");
        assert!(memory_pos < ctx_pos);
        assert!(text.contains("likes tea"));
        assert!(text.contains(
            "<project_instructions path=\"/ws/AGENTS.md\">\nprefer rust\n</project_instructions>"
        ));
    }
}
