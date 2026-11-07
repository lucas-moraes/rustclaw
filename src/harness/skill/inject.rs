//! Renders enabled skills into the system prompt.

use super::{SessionSkill, SkillCatalog};

/// Max chars to inject per skill body (protects context window).
pub const MAX_SKILL_CHARS: usize = 12_000;
/// Soft cap on number of enabled skills per turn.
pub const MAX_SKILLS_PER_TURN: usize = 5;
/// Aggregate cap on total skill chars injected per turn. Prevents the worst
/// case of MAX_SKILLS_PER_TURN x MAX_SKILL_CHARS (60 KB) from reaching the
/// system prompt. Skills are filled in session order; later ones are skipped.
pub const MAX_TOTAL_SKILL_CHARS: usize = 24_000;

/// Agents where context-reduction features (terse skill, tool-output
/// compression/aging) are active. Outside these modes the session runs
/// verbatim: no style rewriting, no output rewriting.
pub const CONTEXT_MODES: &[&str] = &["build", "reason"];

/// Whether context-reduction features apply to `agent`.
pub fn context_modes_active(agent: &str) -> bool {
    CONTEXT_MODES.contains(&agent)
}

/// Renders the `# Session skills` block for a set of enabled skill ids.
/// Returns empty string when nothing is enabled or found.
pub fn render_enabled(
    catalog: &SkillCatalog,
    session_skills: &[SessionSkill],
    enabled_ids: &[String],
) -> String {
    let enabled: Vec<String> = enabled_ids
        .iter()
        .take(MAX_SKILLS_PER_TURN)
        .cloned()
        .collect();

    // Preserve session order.
    let mut ordered: Vec<String> = Vec::new();
    let mut ordered_set = std::collections::HashSet::new();
    for ss in session_skills {
        if enabled.contains(&ss.skill_id) && ordered_set.insert(ss.skill_id.clone()) {
            ordered.push(ss.skill_id.clone());
        }
    }
    // Include any enabled ids not present in session (safety).
    for id in &enabled {
        if ordered_set.insert(id.clone()) {
            ordered.push(id.clone());
        }
    }
    if ordered.is_empty() {
        return String::new();
    }

    let mut out = String::from("\n# Session skills\n");
    let mut budget = MAX_TOTAL_SKILL_CHARS;
    for id in ordered {
        if let Some(spec) = catalog.get(&id) {
            if budget == 0 {
                out.push_str(&format!(
                    "\n## {}\n[skipped: aggregate skill budget \
({MAX_TOTAL_SKILL_CHARS} bytes) exhausted for this turn]\n",
                    spec.name
                ));
                continue;
            }
            let body = truncate_utf8(&spec.body, MAX_SKILL_CHARS.min(budget));
            budget = budget.saturating_sub(body.len());
            out.push_str(&format!("\n## {}\n", spec.name));
            if !spec.description.is_empty() {
                out.push_str(&format!("*{}*\n", spec.description));
            }
            out.push('\n');
            out.push_str(&body);
            out.push('\n');
        }
    }
    out
}

/// Resolves which skill ids are included for a turn given session memory +
/// per-turn toggles (None = fall back to `include_by_default`).
pub fn enabled_for_turn(
    session_skills: &[SessionSkill],
    toggles: Option<&[(String, bool)]>,
) -> Vec<String> {
    if let Some(toggles) = toggles {
        return toggles
            .iter()
            .filter(|(_, inc)| *inc)
            .map(|(id, _)| id.clone())
            .collect();
    }
    session_skills
        .iter()
        .filter(|s| s.include_by_default)
        .map(|s| s.skill_id.clone())
        .collect()
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut idx = max_bytes;
    while !s.is_char_boundary(idx) {
        idx -= 1;
    }
    format!(
        "{}…\n[skill truncated at the {}-byte limit; content beyond this point \
was dropped. If you need the rest, read the skill file directly.]",
        &s[..idx],
        max_bytes
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::skill::{SessionSkill, SkillSpec};

    fn cat() -> SkillCatalog {
        SkillCatalog {
            skills: vec![
                SkillSpec {
                    id: "frontend".into(),
                    name: "frontend".into(),
                    description: "ui".into(),
                    body: "body-frontend".into(),
                    source: String::new(),
                },
                SkillSpec {
                    id: "backend".into(),
                    name: "backend".into(),
                    description: "api".into(),
                    body: "body-backend".into(),
                    source: String::new(),
                },
            ],
        }
    }

    #[test]
    fn test_render_only_enabled() {
        let c = cat();
        let sess = vec![
            SessionSkill::new("frontend", true),
            SessionSkill::new("backend", false),
        ];
        let out = render_enabled(&c, &sess, &["frontend".into()]);
        assert!(out.contains("body-frontend"));
        assert!(!out.contains("body-backend"));
        assert!(out.contains("# Session skills"));
    }

    #[test]
    fn test_render_empty_when_none() {
        let c = cat();
        let sess = vec![SessionSkill::new("backend", true)];
        assert_eq!(render_enabled(&c, &sess, &[]), "");
    }

    #[test]
    fn test_enabled_for_turn_defaults() {
        let sess = vec![SessionSkill::new("a", true), SessionSkill::new("b", false)];
        assert_eq!(enabled_for_turn(&sess, None), vec!["a".to_string()]);
        let toggles = vec![("a".to_string(), false), ("b".to_string(), true)];
        assert_eq!(
            enabled_for_turn(&sess, Some(&toggles)),
            vec!["b".to_string()]
        );
    }

    #[test]
    fn test_context_modes_active() {
        assert!(context_modes_active(crate::harness::agent::builtin::BUILD));
        assert!(context_modes_active(crate::harness::agent::builtin::REASON));
        assert!(!context_modes_active(
            crate::harness::agent::builtin::CHAT_FREE
        ));
        assert!(!context_modes_active("cursor"));
        assert!(!context_modes_active(""));
    }
}
