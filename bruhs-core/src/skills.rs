//! Skill folders for the `$` menu.
//!
//! Claude reads `~/.claude/skills` and then the project's `.claude/skills`.
//! Codex reads `~/.codex/skills`, then `.codex/skills` and `.agents/skills`
//! in the project. The home copy wins when the name clashes. A skill is a
//! child directory that contains a real `SKILL.md`. Symlinks are skipped.
//! Disabled skills and plugin skills (`plugin:review`) are not read: bruhs
//! has no Claude config-directory UI for them.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use bruhs_models::agent::AgentProvider;
use serde::Serialize;

const PREFIX_BYTES: usize = 4096;
const SCAN_LIMIT: usize = 128;
const RESULT_LIMIT: usize = 8;
const DESCRIPTION_CHARS: usize = 160;

/// One skill the `$` menu can insert.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SkillRef {
    pub name: String,
    pub description: String,
}

/// Skills enabled by the folders above, filtered by `query`.
///
/// `home` and `project` may be missing. An empty list is a normal result.
pub fn list_skills(
    provider: AgentProvider,
    home: Option<&Path>,
    project: Option<&Path>,
    query: &str,
) -> Vec<SkillRef> {
    let needle = query.trim().to_lowercase();
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in skill_roots(provider, home, project) {
        for skill in scan_root(&root) {
            if !seen.insert(skill.name.clone()) {
                continue;
            }
            if needle.is_empty() || skill.name.to_lowercase().contains(&needle) {
                found.push(skill);
            }
        }
    }
    found.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    found.truncate(RESULT_LIMIT);
    found
}

fn skill_roots(
    provider: AgentProvider,
    home: Option<&Path>,
    project: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = home {
        let folder = match provider {
            AgentProvider::Claude => home.join(".claude").join("skills"),
            AgentProvider::Codex => home.join(".codex").join("skills"),
        };
        roots.push(folder);
    }
    if let Some(project) = project {
        match provider {
            AgentProvider::Claude => roots.push(project.join(".claude").join("skills")),
            AgentProvider::Codex => {
                roots.push(project.join(".codex").join("skills"));
                roots.push(project.join(".agents").join("skills"));
            }
        }
    }
    roots
}

fn scan_root(root: &Path) -> Vec<SkillRef> {
    if !is_real_dir(root) {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut children: Vec<_> = entries.filter_map(Result::ok).collect();
    children.sort_by_key(|entry| entry.file_name());
    children.truncate(SCAN_LIMIT);
    children
        .into_iter()
        .filter_map(|entry| read_skill(&entry.path()))
        .collect()
}

fn read_skill(dir: &Path) -> Option<SkillRef> {
    if !is_real_dir(dir) {
        return None;
    }
    let file = dir.join("SKILL.md");
    if !is_real_file(&file) {
        return None;
    }
    let dir_name = dir.file_name()?.to_str()?.to_string();
    if dir_name.is_empty() || dir_name.contains('/') {
        return None;
    }
    let text = read_prefix(&file)?;
    let (front_name, front_description) = frontmatter(&text);
    let name = clean_name(front_name, &dir_name);
    if name.is_empty() {
        return None;
    }
    Some(SkillRef {
        name,
        description: clip_description(front_description.as_deref().unwrap_or("")),
    })
}

fn frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let Some(yaml) = split_frontmatter(text) else {
        return (None, None);
    };
    let mut name = None;
    let mut description = None;
    for line in yaml.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(unquote(value));
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(unquote(value));
        }
    }
    (name, description)
}

fn split_frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let end = rest.find("\r\n---").or_else(|| rest.find("\n---"))?;
    Some(&rest[..end])
}

fn clean_name(raw: Option<String>, dir_name: &str) -> String {
    match raw {
        Some(value) if !value.is_empty() && !value.chars().any(char::is_whitespace) => value,
        _ => dir_name.to_string(),
    }
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let mut chars = value.chars();
    let first = chars.next();
    let last = value.chars().last();
    if value.len() >= 2 && first == last && matches!(first, Some('"' | '\'')) {
        return value[1..value.len() - 1].trim().to_string();
    }
    value.to_string()
}

fn clip_description(text: &str) -> String {
    let mut one_line = String::new();
    for word in text.split_whitespace() {
        if !one_line.is_empty() {
            one_line.push(' ');
        }
        one_line.push_str(word);
    }
    if one_line.chars().count() <= DESCRIPTION_CHARS {
        return one_line;
    }
    let mut clipped: String = one_line.chars().take(DESCRIPTION_CHARS - 1).collect();
    clipped.push('…');
    clipped
}

fn read_prefix(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut buffer = [0_u8; PREFIX_BYTES];
    let read = file.read(&mut buffer).ok()?;
    Some(String::from_utf8_lossy(&buffer[..read]).into_owned())
}

fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir())
}

fn is_real_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(root: &Path, dir: &str, body: &str) {
        let folder = root.join(dir);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("SKILL.md"), body).unwrap();
    }

    #[test]
    fn home_wins_a_name_clash_and_the_query_is_a_contains_match() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        write_skill(
            &home.join(".claude").join("skills"),
            "review",
            "---\nname: review\ndescription: \"from home\"\n---\n",
        );
        write_skill(
            &project.join(".claude").join("skills"),
            "review",
            "---\nname: review\ndescription: from project\n---\n",
        );
        write_skill(
            &project.join(".claude").join("skills"),
            "ship",
            "---\nname: ship-it\ndescription: send it\n---\n",
        );
        let listed = list_skills(AgentProvider::Claude, Some(&home), Some(&project), "rev");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "review");
        assert_eq!(listed[0].description, "from home");
        let all = list_skills(AgentProvider::Claude, Some(&home), Some(&project), "");
        assert_eq!(
            all.iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            vec!["review", "ship-it"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_skipped_and_codex_reads_agents_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = tmp.path().join("project");
        let outside = tmp.path().join("outside");
        write_skill(&outside, "secret", "---\nname: secret\n---\n");
        let codex = project.join(".codex").join("skills");
        fs::create_dir_all(&codex).unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), codex.join("secret")).unwrap();
        write_skill(
            &project.join(".agents").join("skills"),
            "local",
            "---\nname: local\ndescription: in the repo\n---\n",
        );
        let listed = list_skills(AgentProvider::Codex, Some(&home), Some(&project), "");
        assert_eq!(
            listed
                .iter()
                .map(|skill| skill.name.as_str())
                .collect::<Vec<_>>(),
            vec!["local"]
        );
    }

    #[test]
    fn a_missing_name_uses_the_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        write_skill(
            &home.join(".claude").join("skills"),
            "plain",
            "no frontmatter\n",
        );
        let listed = list_skills(AgentProvider::Claude, Some(&home), None, "");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "plain");
        assert_eq!(listed[0].description, "");
    }
}
