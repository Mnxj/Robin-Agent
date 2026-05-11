use std::path::Path;

use parking_lot::RwLock;
use serde::Deserialize;
use tracing::{info, warn};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawRequires {
    pub bins: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct OpenClawMeta {
    pub requires: OpenClawRequires,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SkillMetadata {
    pub openclaw: OpenClawMeta,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub metadata: SkillMetadata,

    #[serde(skip)]
    pub body: String,
    #[serde(skip)]
    pub file_path: String,
}

pub struct Loader {
    pub skills: RwLock<Vec<Skill>>,
}

impl Loader {
    pub fn new() -> Self {
        Self { skills: RwLock::new(Vec::new()) }
    }

    pub fn load_from(&self, dirs: &[&str]) -> anyhow::Result<()> {
        let mut loaded = Vec::new();
        for dir in dirs {
            let meta = match std::fs::metadata(dir) {
                Ok(m) if m.is_dir() => m,
                _ => continue,
            };
            let _ = meta;
            let walk = walkdir::WalkDir::new(dir);
            for entry in walk.into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_dir() { continue; }
                let path = entry.path();
                let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                let is_skill_md = file_name.to_uppercase() == "SKILL.MD";
                let is_direct_md = file_name.to_lowercase().ends_with(".md")
                    && path.parent().and_then(|p| p.to_str()) == Some(dir);
                if !is_skill_md && !is_direct_md { continue; }

                match parse_skill_file(path.to_str().unwrap_or("")) {
                    Err(e) => warn!("failed to parse skill file path={:?} error={}", path, e),
                    Ok(skill) => {
                        let missing = missing_bins(&skill);
                        if !missing.is_empty() {
                            info!(name = %skill.name, binary = %missing[0], "skill skipped (missing binary)");
                            continue;
                        }
                        info!(name = %skill.name, path = ?path, "loaded skill");
                        loaded.push(skill);
                    }
                }
            }
        }
        *self.skills.write() = loaded;
        Ok(())
    }

    pub fn skills(&self) -> Vec<Skill> {
        self.skills.read().clone()
    }

    pub fn match_skills(&self, user_msg: &str, max_skills: usize) -> Vec<Skill> {
        let skills = self.skills.read();
        if skills.is_empty() { return Vec::new(); }
        let max = if max_skills == 0 { 3 } else { max_skills };
        let msg_lower = user_msg.to_lowercase();
        let msg_words: Vec<&str> = msg_lower.split_whitespace().collect();

        let mut results: Vec<(Skill, i32)> = Vec::new();
        for s in skills.iter() {
            let mut score = 0i32;
            let name_lower = s.name.to_lowercase();
            let desc_lower = s.description.to_lowercase();
            for &word in &msg_words {
                if word.len() < 3 { continue; }
                if name_lower.contains(word) { score += 3; }
                if desc_lower.contains(word) { score += 2; }
                for tag in &s.tags {
                    if tag.to_lowercase().contains(word) { score += 2; }
                }
            }
            if msg_lower.contains(&name_lower) { score += 5; }
            for tag in &s.tags {
                if msg_lower.contains(&tag.to_lowercase()) { score += 3; }
            }
            if score > 0 { results.push((s.clone(), score)); }
        }
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.into_iter().take(max).map(|(s, _)| s).collect()
    }

    pub fn format_index(&self) -> String {
        let skills = self.skills.read();
        if skills.is_empty() { return String::new(); }
        let mut b = String::from(
            "\n\n## Skills Index\n\nThe following skills are loaded and available. Their full instructions are injected only when the user's request matches one of them; if a request relates to any of these but the full instructions are not present, ask the user to be more specific so the right skill can be loaded.\n\n",
        );
        for s in skills.iter() {
            b.push_str("- **");
            b.push_str(&s.name);
            b.push_str("**");
            if !s.description.is_empty() {
                b.push_str(" — ");
                b.push_str(&s.description);
            }
            b.push('\n');
        }
        b
    }
}

impl Default for Loader {
    fn default() -> Self { Self::new() }
}

pub fn parse_skill_file(path: &str) -> anyhow::Result<Skill> {
    let data = std::fs::read_to_string(path)?;
    let (fm, body) = split_frontmatter(&data);
    let mut skill: Skill = if fm.is_empty() {
        Skill::default()
    } else {
        serde_yaml::from_str(fm)?
    };
    skill.body = body.trim().to_string();
    skill.file_path = path.to_string();
    if skill.name.is_empty() {
        let p = Path::new(path);
        let base = p.file_name().and_then(|f| f.to_str()).unwrap_or("");
        if base.to_uppercase() == "SKILL.MD" {
            skill.name = p.parent()
                .and_then(|d| d.file_name())
                .and_then(|f| f.to_str())
                .unwrap_or("")
                .to_string();
        } else {
            skill.name = p.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
        }
    }
    Ok(skill)
}

pub fn split_frontmatter(content: &str) -> (&str, &str) {
    let content = content.trim_start();
    if !content.starts_with("---") { return ("", content); }
    let rest = &content[3..];
    let rest = rest.trim_start_matches([' ', '\t']);
    let rest = rest.strip_prefix('\n').unwrap_or_else(|| rest.strip_prefix("\r\n").unwrap_or(rest));
    let end_idx = match rest.find("\n---") {
        Some(i) => i,
        None => return ("", content),
    };
    let fm = &rest[..end_idx];
    let body = &rest[end_idx + 4..];
    let body = body.strip_prefix('\n').unwrap_or_else(|| body.strip_prefix("\r\n").unwrap_or(body));
    (fm, body)
}

pub fn format_for_prompt(skills: &[Skill]) -> String {
    if skills.is_empty() { return String::new(); }
    let mut b = String::from(
        "\n\n## Available Skills\n\nIMPORTANT: The following skills are matched to the user's request. You MUST use these skills and their CLI tools via bash instead of alternative approaches (like opening a browser or using APIs directly). The skill provides the correct command-line tool and usage instructions — follow them.\n\n",
    );
    for s in skills {
        b.push_str("### ");
        b.push_str(&s.name);
        b.push_str("\n\n");
        if !s.body.is_empty() {
            b.push_str(&s.body);
            b.push_str("\n\n");
        }
    }
    b
}

pub fn missing_bins(s: &Skill) -> Vec<String> {
    s.metadata.openclaw.requires.bins.iter()
        .filter(|bin| which::which(bin).is_err())
        .cloned()
        .collect()
}