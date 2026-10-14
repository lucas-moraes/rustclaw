//! Builtin skills shipped with the harness.
//!
//! Merged into the catalog by `loader::load_catalog` (a user skill with the
//! same id wins). Builtin skills are **off by default**: a skill only enters
//! a session when the user enables it via `/skills`.

use super::SkillSpec;

/// All builtin skills, in catalog order.
pub fn builtin_skills() -> Vec<SkillSpec> {
    vec![terse()]
}

/// `terse`: telegraphic response style. Reduces output tokens by favoring
/// short, dense answers (Caveman-inspired).
fn terse() -> SkillSpec {
    SkillSpec {
        id: "terse".to_string(),
        name: "terse".to_string(),
        description: "Telegraphic response style: short, dense answers, no filler.".to_string(),
        body: r#"# Terse mode — telegraphic responses

Optimize for token density. Every sentence must earn its tokens.

## Style
- Start with the answer. No preamble, no restating the question, no "Sure!".
- Lead with the conclusion; supporting detail only if it changes the decision.
- Prefer fragments over full sentences: "build passes", "3 files changed".
- Lists over prose. One fact per line.
- Drop filler: "basically", "actually", "I think", "note that".
- No sign-offs ("Let me know if...", "Hope this helps").

## Code
- Show only changed lines, with a `path:` comment for location.
- No narration of steps taken; state the outcome.
- Full files only when explicitly requested.

## Never sacrifice
- Correctness caveats (breaking changes, security, data loss).
- Answers to the actual question asked.
- Verification results: state them plainly ("tests: 12/12 pass").
"#
        .to_string(),
        source: "builtin".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terse_skill_shape() {
        let skills = builtin_skills();
        let terse = skills
            .iter()
            .find(|s| s.id == "terse")
            .expect("terse builtin");
        assert_eq!(terse.source, "builtin");
        assert!(terse.description.len() > 10);
        // Body must carry actionable instructions, not just a title.
        assert!(terse.body.contains("Start with the answer"));
        assert!(terse.body.contains("Never sacrifice"));
    }
}
