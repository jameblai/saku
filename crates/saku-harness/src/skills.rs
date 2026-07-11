//! Agent Skills discovery, prompt formatting, and `$skill-name` expansion.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const MAX_NAME_LENGTH: usize = 64;
const MAX_DESCRIPTION_LENGTH: usize = 1024;
const SKILL_FILENAME: &str = "SKILL.md";
const AGENTS_SKILLS_SEGMENTS: &[&str] = &[".agents", "skills"];

/// One discovered Skill package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub file_path: PathBuf,
    pub base_dir: PathBuf,
    pub disable_model_invocation: bool,
}

/// Non-fatal discovery / validation note (pi-style diagnostics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDiagnostic {
    pub message: String,
    pub path: Option<PathBuf>,
}

/// Result of scanning skill directories.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadSkillsResult {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

/// Inputs for per-Run skill discovery.
#[derive(Debug, Clone, Copy)]
pub struct LoadSkillsOptions<'a> {
    /// Session Working Directory (project skills walk from here).
    pub cwd: &'a Path,
    /// Workspace root (ancestor walk stops here).
    pub workspace: &'a Path,
    /// Global skills directory (normally `~/.agents/skills`).
    pub global_skills_dir: &'a Path,
}

#[derive(Debug, Default, Deserialize)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(rename = "disable-model-invocation", default)]
    disable_model_invocation: bool,
}

/// Default global skills path: `~/.agents/skills`.
pub fn default_global_skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".agents")
        .join("skills")
}

/// Discover skills from global + project locations (cwd → Workspace ancestors).
///
/// Name collisions: first wins, with a diagnostic for the loser (pi behavior).
pub fn load_skills(opts: LoadSkillsOptions<'_>) -> LoadSkillsResult {
    let mut skill_map: HashMap<String, Skill> = HashMap::new();
    let mut real_paths: HashSet<PathBuf> = HashSet::new();
    let mut diagnostics = Vec::new();

    let mut add = |result: LoadSkillsResult| {
        diagnostics.extend(result.diagnostics);
        for skill in result.skills {
            let real =
                canonicalize_existing(&skill.file_path).unwrap_or_else(|| skill.file_path.clone());
            if real_paths.contains(&real) {
                continue;
            }
            if let Some(existing) = skill_map.get(&skill.name) {
                diagnostics.push(SkillDiagnostic {
                    message: format!(
                        "name \"{}\" collision; keeping {}",
                        skill.name,
                        existing.file_path.display()
                    ),
                    path: Some(skill.file_path.clone()),
                });
            } else {
                real_paths.insert(real);
                skill_map.insert(skill.name.clone(), skill);
            }
        }
    };

    add(load_skills_from_dir(opts.global_skills_dir));

    for dir in project_skills_dirs(opts.cwd, opts.workspace) {
        add(load_skills_from_dir(&dir));
    }

    let mut skills: Vec<Skill> = skill_map.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    LoadSkillsResult {
        skills,
        diagnostics,
    }
}

/// Load skills under a single skills root directory.
pub fn load_skills_from_dir(dir: &Path) -> LoadSkillsResult {
    load_skills_from_dir_internal(dir, true)
}

fn load_skills_from_dir_internal(dir: &Path, include_root_files: bool) -> LoadSkillsResult {
    let mut skills = Vec::new();
    let mut diagnostics = Vec::new();

    if !dir.is_dir() {
        return LoadSkillsResult {
            skills,
            diagnostics,
        };
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return LoadSkillsResult {
            skills,
            diagnostics,
        };
    };

    // Prefer SKILL.md in this directory (skill root — do not recurse further).
    let mut skill_md: Option<PathBuf> = None;
    let mut children: Vec<(PathBuf, bool, bool)> = Vec::new(); // path, is_dir, is_file

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "node_modules" {
            continue;
        }
        let path = entry.path();
        let meta = match entry.metadata().or_else(|_| fs::metadata(&path)) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if name_str == SKILL_FILENAME && meta.is_file() {
            skill_md = Some(path);
            continue;
        }
        children.push((path, meta.is_dir(), meta.is_file()));
    }

    if let Some(full_path) = skill_md {
        let result = load_skill_from_file(&full_path);
        if let Some(skill) = result.skill {
            skills.push(skill);
        }
        diagnostics.extend(result.diagnostics);
        return LoadSkillsResult {
            skills,
            diagnostics,
        };
    }

    for (path, is_dir, is_file) in children {
        if is_dir {
            let sub = load_skills_from_dir_internal(&path, false);
            skills.extend(sub.skills);
            diagnostics.extend(sub.diagnostics);
            continue;
        }
        if !is_file || !include_root_files {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".md") {
            continue;
        }
        let result = load_skill_from_file(&path);
        if let Some(skill) = result.skill {
            skills.push(skill);
        }
        diagnostics.extend(result.diagnostics);
    }

    LoadSkillsResult {
        skills,
        diagnostics,
    }
}

struct LoadFileResult {
    skill: Option<Skill>,
    diagnostics: Vec<SkillDiagnostic>,
}

fn load_skill_from_file(file_path: &Path) -> LoadFileResult {
    let mut diagnostics = Vec::new();
    let raw = match fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(err) => {
            diagnostics.push(SkillDiagnostic {
                message: err.to_string(),
                path: Some(file_path.to_path_buf()),
            });
            return LoadFileResult {
                skill: None,
                diagnostics,
            };
        }
    };

    let (frontmatter, _body) = parse_frontmatter(&raw);
    let skill_dir = file_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let parent_name = skill_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("skill")
        .to_string();

    let description = frontmatter.description.unwrap_or_default();
    if description.trim().is_empty() {
        diagnostics.push(SkillDiagnostic {
            message: "description is required".into(),
            path: Some(file_path.to_path_buf()),
        });
        return LoadFileResult {
            skill: None,
            diagnostics,
        };
    }
    if description.len() > MAX_DESCRIPTION_LENGTH {
        diagnostics.push(SkillDiagnostic {
            message: format!(
                "description exceeds {MAX_DESCRIPTION_LENGTH} characters ({})",
                description.len()
            ),
            path: Some(file_path.to_path_buf()),
        });
    }

    let name = frontmatter.name.unwrap_or(parent_name);
    for err in validate_name(&name) {
        diagnostics.push(SkillDiagnostic {
            message: err,
            path: Some(file_path.to_path_buf()),
        });
    }

    LoadFileResult {
        skill: Some(Skill {
            name,
            description,
            file_path: file_path.to_path_buf(),
            base_dir: skill_dir,
            disable_model_invocation: frontmatter.disable_model_invocation,
        }),
        diagnostics,
    }
}

fn validate_name(name: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if name.len() > MAX_NAME_LENGTH {
        errors.push(format!(
            "name exceeds {MAX_NAME_LENGTH} characters ({})",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        errors.push(
            "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)".into(),
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        errors.push("name must not start or end with a hyphen".into());
    }
    if name.contains("--") {
        errors.push("name must not contain consecutive hyphens".into());
    }
    errors
}

fn parse_frontmatter(content: &str) -> (SkillFrontmatter, String) {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    if !normalized.starts_with("---") {
        return (SkillFrontmatter::default(), normalized);
    }
    let after_open = normalized[3..]
        .strip_prefix('\n')
        .unwrap_or(&normalized[3..]);
    let Some(end) = after_open.find("\n---") else {
        return (SkillFrontmatter::default(), normalized);
    };
    let yaml = &after_open[..end];
    let body = after_open[end + 4..].trim_start_matches('\n').to_string();
    let frontmatter = serde_yaml::from_str::<SkillFrontmatter>(yaml).unwrap_or_default();
    (frontmatter, body)
}

fn strip_frontmatter(content: &str) -> String {
    parse_frontmatter(content).1
}

/// Project skill directories from `cwd` walking ancestors up to `workspace` (inclusive).
fn project_skills_dirs(cwd: &Path, workspace: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(cwd_canon) = cwd.canonicalize() else {
        return dirs;
    };
    let Ok(workspace_canon) = workspace.canonicalize() else {
        return dirs;
    };
    if !cwd_canon.starts_with(&workspace_canon) && cwd_canon != workspace_canon {
        return dirs;
    }

    let mut current = cwd_canon;
    loop {
        let mut skills = current.clone();
        for seg in AGENTS_SKILLS_SEGMENTS {
            skills.push(seg);
        }
        dirs.push(skills);

        if current == workspace_canon {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if !parent.starts_with(&workspace_canon) && parent != workspace_canon {
            break;
        }
        current = parent.to_path_buf();
    }
    dirs
}

/// Format skills for the System Prompt (`<available_skills>` XML).
/// Skills with `disable_model_invocation` are omitted.
pub fn format_skills_for_prompt(skills: &[Skill]) -> String {
    let visible: Vec<&Skill> = skills
        .iter()
        .filter(|s| !s.disable_model_invocation)
        .collect();
    if visible.is_empty() {
        return String::new();
    }

    let mut lines = vec![
        "The following skills provide specialized instructions for specific tasks.".to_string(),
        "Use the read tool to load a skill's file when the task matches its description.".to_string(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".to_string(),
        String::new(),
        "<available_skills>".to_string(),
    ];
    for skill in visible {
        lines.push("  <skill>".into());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.file_path.to_string_lossy())
        ));
        lines.push("  </skill>".into());
    }
    lines.push("</available_skills>".into());
    lines.join("\n")
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Expand a leading `$skill-name` token by injecting the skill body (Codex-style).
/// Unknown names pass through unchanged. Explicit-only skills are allowed.
pub fn expand_skill_invocations(text: &str, skills: &[Skill]) -> String {
    let trimmed = text.trim_start();
    let Some(rest) = trimmed.strip_prefix('$') else {
        return text.to_string();
    };
    let name_end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    let name = &rest[..name_end];
    if name.is_empty() {
        return text.to_string();
    }
    let Some(skill) = skills.iter().find(|s| s.name == name) else {
        return text.to_string();
    };
    let args = rest[name_end..].trim();
    let Ok(content) = fs::read_to_string(&skill.file_path) else {
        return text.to_string();
    };
    let body = strip_frontmatter(&content);
    let block = format!(
        "<skill name=\"{}\" location=\"{}\">\nReferences are relative to {}.\n\n{}\n</skill>",
        skill.name,
        skill.file_path.display(),
        skill.base_dir.display(),
        body.trim()
    );
    if args.is_empty() {
        block
    } else {
        format!("{block}\n\n{args}")
    }
}

/// Base directories of discovered skills (for path allowlisting).
pub fn skill_base_dirs(skills: &[Skill]) -> Vec<PathBuf> {
    skills.iter().map(|s| s.base_dir.clone()).collect()
}

/// Whether `path` (canonical) lies under any skill base directory.
pub fn path_under_skill_dirs(path: &Path, skill_roots: &[PathBuf]) -> bool {
    for root in skill_roots {
        let Ok(root_canon) = root.canonicalize() else {
            continue;
        };
        if path == root_canon || path.starts_with(&root_canon) {
            return true;
        }
    }
    false
}

fn canonicalize_existing(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill(dir: &Path, name: &str, body: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join(SKILL_FILENAME), body).unwrap();
    }

    fn basic_skill(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n\nDo the {name} thing.\n")
    }

    #[test]
    fn load_skills_from_dir_discovers_skill_md() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "triage",
            &basic_skill("triage", "Triage GitHub issues"),
        );
        let result = load_skills_from_dir(tmp.path());
        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].name, "triage");
        assert_eq!(result.skills[0].description, "Triage GitHub issues");
        assert!(!result.skills[0].disable_model_invocation);
        assert_eq!(result.skills[0].base_dir, tmp.path().join("triage"));
    }

    #[test]
    fn load_skills_skips_missing_description() {
        let tmp = TempDir::new().unwrap();
        write_skill(tmp.path(), "bad", "---\nname: bad\n---\n\nBody\n");
        let result = load_skills_from_dir(tmp.path());
        assert!(result.skills.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.message.contains("description"))
        );
    }

    #[test]
    fn disable_model_invocation_parsed() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "secret",
            "---\nname: secret\ndescription: Explicit only\ndisable-model-invocation: true\n---\n\nSecret steps.\n",
        );
        let result = load_skills_from_dir(tmp.path());
        assert_eq!(result.skills.len(), 1);
        assert!(result.skills[0].disable_model_invocation);
    }

    #[test]
    fn format_skills_omits_explicit_only_and_escapes_xml() {
        let skills = vec![
            Skill {
                name: "ok".into(),
                description: "Use when A & B < C".into(),
                file_path: PathBuf::from("/skills/ok/SKILL.md"),
                base_dir: PathBuf::from("/skills/ok"),
                disable_model_invocation: false,
            },
            Skill {
                name: "hidden".into(),
                description: "Nope".into(),
                file_path: PathBuf::from("/skills/hidden/SKILL.md"),
                base_dir: PathBuf::from("/skills/hidden"),
                disable_model_invocation: true,
            },
        ];
        let text = format_skills_for_prompt(&skills);
        assert!(text.contains("<available_skills>"));
        assert!(text.contains("<name>ok</name>"));
        assert!(text.contains("A &amp; B &lt; C"));
        assert!(text.contains("<location>/skills/ok/SKILL.md</location>"));
        assert!(!text.contains("hidden"));
        assert!(text.contains("Use the read tool"));
    }

    #[test]
    fn format_skills_empty_when_none_visible() {
        assert_eq!(format_skills_for_prompt(&[]), "");
        let only_hidden = vec![Skill {
            name: "x".into(),
            description: "y".into(),
            file_path: PathBuf::from("/x"),
            base_dir: PathBuf::from("/x"),
            disable_model_invocation: true,
        }];
        assert_eq!(format_skills_for_prompt(&only_hidden), "");
    }

    #[test]
    fn load_skills_global_and_project_ancestors_first_wins() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("ws");
        let nested = workspace.join("proj").join("sub");
        fs::create_dir_all(&nested).unwrap();

        let global = tmp.path().join("global-skills");
        write_skill(&global, "shared", &basic_skill("shared", "Global shared"));
        write_skill(
            &global,
            "only-global",
            &basic_skill("only-global", "Global only"),
        );

        write_skill(
            &workspace.join(".agents").join("skills"),
            "shared",
            &basic_skill("shared", "Workspace shared"),
        );
        write_skill(
            &nested.join(".agents").join("skills"),
            "local",
            &basic_skill("local", "Nested local"),
        );

        let result = load_skills(LoadSkillsOptions {
            cwd: &nested,
            workspace: &workspace,
            global_skills_dir: &global,
        });

        let by_name: HashMap<_, _> = result.skills.iter().map(|s| (s.name.as_str(), s)).collect();
        assert_eq!(by_name["shared"].description, "Global shared");
        assert!(by_name.contains_key("only-global"));
        assert!(by_name.contains_key("local"));
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.message.contains("collision") && d.message.contains("shared"))
        );
    }

    #[test]
    fn expand_skill_invocations_injects_body_and_args() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "triage",
            &basic_skill("triage", "Triage issues"),
        );
        let skills = load_skills_from_dir(tmp.path()).skills;
        let expanded = expand_skill_invocations("$triage handle #42", &skills);
        assert!(expanded.contains("<skill name=\"triage\""));
        assert!(expanded.contains("Do the triage thing."));
        assert!(expanded.contains("References are relative to"));
        assert!(expanded.ends_with("handle #42") || expanded.contains("\n\nhandle #42"));
        assert!(!expanded.contains("---"));
    }

    #[test]
    fn expand_allows_explicit_only_skills() {
        let tmp = TempDir::new().unwrap();
        write_skill(
            tmp.path(),
            "secret",
            "---\nname: secret\ndescription: Explicit\ndisable-model-invocation: true\n---\n\nSecret body.\n",
        );
        let skills = load_skills_from_dir(tmp.path()).skills;
        let expanded = expand_skill_invocations("$secret do it", &skills);
        assert!(expanded.contains("Secret body."));
        assert!(expanded.contains("do it"));
    }

    #[test]
    fn expand_unknown_skill_passes_through() {
        let text = "$missing please help";
        assert_eq!(expand_skill_invocations(text, &[]), text);
    }

    #[test]
    fn path_under_skill_dirs_allows_skill_assets() {
        let tmp = TempDir::new().unwrap();
        let skill_dir = tmp.path().join("triage");
        let scripts = skill_dir.join("scripts");
        fs::create_dir_all(&scripts).unwrap();
        let script = scripts.join("run.sh");
        fs::write(&script, "#!/bin/sh\n").unwrap();
        let roots = vec![skill_dir];
        let canon = script.canonicalize().unwrap();
        assert!(path_under_skill_dirs(&canon, &roots));
        assert!(!path_under_skill_dirs(
            &tmp.path()
                .join("other")
                .canonicalize()
                .unwrap_or(tmp.path().join("other")),
            &roots
        ));
    }
}
