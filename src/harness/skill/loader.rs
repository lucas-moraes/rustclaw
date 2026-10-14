//! Skill discovery: scans known directories for `SKILL.md` files and parses
//! their frontmatter (`name`, `description`) + markdown body.

use super::{SkillCatalog, SkillSpec};
use std::path::{Path, PathBuf};

/// Roots scanned for skills, in priority order (dedup by id, first wins).
/// Project-local beats user-global.
pub fn skill_roots(cwd: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    roots.push(cwd.join(".agents").join("skills"));
    roots.push(cwd.join(".opencode").join("skills"));
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".agents").join("skills"));
        roots.push(home.join(".config").join("opencode").join("skills"));
    }
    if let Ok(dir) = std::env::var("RUSTCLAW_SKILLS_DIR") {
        roots.push(PathBuf::from(dir));
    }
    roots
}

/// Discovers and parses all skills available for the given working directory.
/// Builtin skills are merged in; a user skill with the same id wins.
pub fn load_catalog(cwd: &Path) -> SkillCatalog {
    let mut catalog = load_from_roots(&skill_roots(cwd));
    merge_builtins(&mut catalog);
    catalog
}

/// Merges builtin skills into `catalog` without overriding user skills.
pub fn merge_builtins(catalog: &mut SkillCatalog) {
    for spec in super::builtin::builtin_skills() {
        if catalog.get(&spec.id).is_none() {
            catalog.skills.push(spec);
        }
    }
    catalog.skills.sort_by(|a, b| a.id.cmp(&b.id));
}

/// Loads skills from an explicit set of root directories (dedup by id, first wins).
pub fn load_from_roots(roots: &[PathBuf]) -> SkillCatalog {
    let mut seen = std::collections::HashMap::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }
            let Some(spec) = parse_skill(&skill_md, root) else {
                continue;
            };
            seen.entry(spec.id.clone()).or_insert(spec);
        }
    }
    let mut skills: Vec<SkillSpec> = seen.into_values().collect();
    skills.sort_by(|a, b| a.id.cmp(&b.id));
    SkillCatalog { skills }
}

/// Parses a single SKILL.md: frontmatter block + body.
fn parse_skill(path: &Path, root: &Path) -> Option<SkillSpec> {
    let raw = std::fs::read_to_string(path).ok()?;
    let (meta, body) = split_frontmatter(&raw).unwrap_or_else(|| (String::new(), raw.clone()));

    let name = get_field(&meta, "name").unwrap_or_else(|| default_id(path, root));
    let description = get_field(&meta, "description").unwrap_or_default();
    let id = sanitize_id(&name);

    // Directory name is a fallback/stable id when frontmatter has no `name`.
    let dir_name = path.parent()?.file_name()?.to_string_lossy().to_string();
    let id = if name == dir_name {
        sanitize_id(&dir_name)
    } else {
        id
    };

    Some(SkillSpec {
        id,
        name: name.clone(),
        description,
        body: body.trim().to_string(),
        source: path.to_string_lossy().to_string(),
    })
}

/// Splits YAML-ish frontmatter (`---\n...\n---`) from body. Returns (meta, body).
fn split_frontmatter(raw: &str) -> Option<(String, String)> {
    let rest = raw.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let meta = rest[..end].to_string();
    let body = rest[end + 4..].to_string();
    Some((meta, body))
}

/// Extracts a scalar field from frontmatter text like `name: frontend`.
fn get_field(meta: &str, key: &str) -> Option<String> {
    for line in meta.lines() {
        if let Some(v) = line
            .strip_prefix(&format!("{}:", key))
            .or_else(|| line.strip_prefix(&format!("{} :", key)))
        {
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn default_id(path: &Path, _root: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "skill".to_string())
}

pub fn sanitize_id(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str, desc: &str, body: &str) {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("SKILL.md"),
            format!(
                "---\nname: {}\ndescription: \"{}\"\n---\n\n{}",
                name, desc, body
            ),
        )
        .unwrap();
    }

    #[test]
    fn test_parse_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_skill(
            tmp.path(),
            "frontend",
            "Build UI components",
            "# Frontend\nDo things.",
        );
        let cat = load_from_roots(&[tmp.path().to_path_buf()]);
        assert_eq!(cat.skills.len(), 1);
        let s = &cat.skills[0];
        assert_eq!(s.id, "frontend");
        assert_eq!(s.description, "Build UI components");
        assert!(s.body.contains("Do things"));
    }

    #[test]
    fn test_no_frontmatter_uses_dir_name() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path().join("mytool");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), "# Just body\nno frontmatter").unwrap();
        let cat = load_from_roots(&[tmp.path().to_path_buf()]);
        assert_eq!(cat.skills.len(), 1);
        assert_eq!(cat.skills[0].id, "mytool");
    }

    #[test]
    fn test_dedup_project_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join(".agents").join("skills");
        write_skill(&project, "same", "project version", "body A");
        let home = tmp.path().join("home").join(".agents").join("skills");
        write_skill(&home, "same", "global version", "body B");

        // Project root takes priority over home root (first wins).
        let cat = load_from_roots(&[project, home]);
        assert_eq!(cat.skills.len(), 1);
        assert!(cat.skills[0].body.contains("A"));
    }

    #[test]
    fn test_builtin_merged_into_catalog() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cat = load_from_roots(&[tmp.path().to_path_buf()]);
        assert!(cat.get("terse").is_none(), "no builtin before merge");
        merge_builtins(&mut cat);
        let terse = cat.get("terse").expect("terse builtin merged");
        assert_eq!(terse.source, "builtin");
        // Catalog stays sorted by id.
        let ids: Vec<&str> = cat.skills.iter().map(|s| s.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn test_user_skill_overrides_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join(".agents").join("skills");
        write_skill(&project, "terse", "user override", "custom body");
        let cat = load_catalog(tmp.path());
        let terse = cat.get("terse").expect("terse present");
        assert_eq!(terse.description, "user override");
        assert!(terse.body.contains("custom body"));
        assert_ne!(terse.source, "builtin");
        assert_eq!(cat.skills.iter().filter(|s| s.id == "terse").count(), 1);
    }
}
