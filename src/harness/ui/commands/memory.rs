//! `/memory` slash command: list, remove and clear project memory facts that
//! were persisted to SQLite by the `remember` tool.

use crate::harness::runtime::SessionRuntime;
use anyhow::Result;
use std::path::Path;

/// Handles `/memory [list] [rm|delete <id>] [clear]`, returning a single
/// feedback line rendered by the caller. Resolves the project from the current
/// working directory.
pub fn handle_memory_command(runtime: &SessionRuntime, args: &[&str]) -> Result<String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    render_memory_command(runtime, &cwd, args)
}

/// Core implementation, split out for testability with an explicit project root.
fn render_memory_command(runtime: &SessionRuntime, cwd: &Path, args: &[&str]) -> Result<String> {
    match args {
        [] | ["list"] => {
            let facts = runtime.project_memory.list_fact_rows(cwd)?;
            if facts.is_empty() {
                return Ok("no project memory yet (use the `remember` tool)".to_string());
            }
            let mut out = format!("project memory ({}):", facts.len());
            for (i, f) in facts.iter().enumerate() {
                let mark = if f.archived { " (archived)" } else { "" };
                out.push_str(&format!(
                    "\n  [{}] {} [{}|{}|hits {}]{mark}",
                    i + 1,
                    f.text,
                    f.kind,
                    f.confidence,
                    f.hit_count
                ));
            }
            Ok(out)
        }
        ["rm", id] | ["delete", id] => {
            let index: usize = match id.trim().parse() {
                Ok(i) => i,
                Err(_) => {
                    return Ok(format!(
                        "invalid memory id: {} (usage: /memory rm <id>)",
                        id
                    ))
                }
            };
            if runtime.project_memory.delete_fact_by_index(cwd, index)? {
                Ok(format!("deleted memory item [{}]", index))
            } else {
                Ok(format!("no memory item at index [{}]", index))
            }
        }
        ["clear"] => {
            runtime.project_memory.clear_memory(cwd)?;
            Ok("cleared project memory".to_string())
        }
        ["gc"] => {
            let merged = runtime.project_memory.dedup(cwd)?;
            let archived = runtime.project_memory.archive_stale(cwd, 60)?;
            let compacted = runtime.project_memory.compact(cwd)?;
            Ok(format!(
                "gc: merged {} duplicate(s), archived {} stale fact(s), compacted {} fact(s)",
                merged, archived, compacted
            ))
        }
        ["promote", id] => {
            let index: usize = match id.trim().parse() {
                Ok(i) => i,
                Err(_) => {
                    return Ok(format!(
                        "invalid memory id: {} (usage: /memory promote <id>)",
                        id
                    ))
                }
            };
            promote_fact(runtime, cwd, index)
        }
        _ => Ok("usage: /memory [list] [rm|delete <id>] [clear] [promote <id>]".to_string()),
    }
}

/// Promotes the fact at `index` (1-based) into a permanent `SKILL.md` under
/// `<cwd>/.agents/skills/<slug>/`, then archives the fact so it no longer
/// pollutes the active memory. Returns a feedback line.
fn promote_fact(runtime: &SessionRuntime, cwd: &Path, index: usize) -> Result<String> {
    let facts = runtime.project_memory.list_fact_rows(cwd)?;
    if index == 0 || index > facts.len() {
        return Ok(format!("no memory item at index [{}]", index));
    }
    let fact = &facts[index - 1];
    if fact.archived {
        return Ok(format!(
            "memory item [{}] is already archived (promoted or removed)",
            index
        ));
    }

    // Derive a slug from the fact text (strip timestamp prefix).
    let text = strip_timestamp(&fact.text);
    let slug = crate::harness::skill::loader::sanitize_id(&text);
    let dir = cwd.join(".agents").join("skills").join(&slug);
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("failed to create skill dir: {}", e))?;
    let skill_path = dir.join("SKILL.md");
    let body = format!(
        "---\nname: {}\ndescription: \"{}\"\n---\n\n{}",
        slug, fact.kind, text
    );
    std::fs::write(&skill_path, body)
        .map_err(|e| anyhow::anyhow!("failed to write SKILL.md: {}", e))?;

    runtime
        .project_memory
        .set_archived(cwd, fact.id, true)
        .map_err(|e| anyhow::anyhow!("failed to archive fact: {}", e))?;

    Ok(format!(
        "promoted memory item [{}] → {}",
        index,
        skill_path.display()
    ))
}

/// Strips a leading `- [timestamp] ` prefix from a fact line, if present.
fn strip_timestamp(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("- [") {
        if let Some(end) = rest.find("] ") {
            return rest[end + 2..].to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::project::ProjectMemoryStore;
    use std::sync::Arc;

    struct TestRuntime {
        _store: Arc<ProjectMemoryStore>,
        runtime: SessionRuntime,
        _dir: tempfile::TempDir,
    }

    fn test_runtime() -> TestRuntime {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = Arc::new(ProjectMemoryStore::open(&db).unwrap());
        let runtime = SessionRuntime::new_in(
            dir.path(),
            crate::harness::provider::opencode_go::build_provider(
                "opencode-go",
                crate::harness::provider::HttpConfig {
                    client: crate::harness::provider::build_http_client(),
                    base_url: "http://localhost:9".to_string(),
                    api_key: "x".to_string(),
                },
            )
            .unwrap(),
            crate::harness::tool::registry::ToolRegistry::builder().build(),
            crate::harness::runtime::HarnessConfig::default(),
            &db,
            Arc::new(crate::harness::permission::PermissionEngine::default()),
            Arc::new(AllowAsker),
            Arc::new(NoUserAsker),
        )
        .unwrap();
        TestRuntime {
            _store: store.clone(),
            runtime: SessionRuntime {
                project_memory: store,
                ..runtime
            },
            _dir: dir,
        }
    }

    struct AllowAsker;
    struct NoUserAsker;

    #[async_trait::async_trait]
    impl crate::harness::tool::context::PermissionAsker for AllowAsker {
        async fn ask(&self, _req: crate::harness::tool::context::PermissionAskInput) -> bool {
            true
        }
    }

    #[async_trait::async_trait]
    impl crate::harness::tool::context::UserAsker for NoUserAsker {
        async fn ask(&self, _q: String, _o: Vec<String>) -> Option<String> {
            None
        }
    }

    #[test]
    fn test_memory_list_empty() {
        let tr = test_runtime();
        let out = render_memory_command(&tr.runtime, tr._dir.path(), &[]).unwrap();
        assert!(out.contains("no project memory yet"));
    }

    #[test]
    fn test_memory_list_formats_numbered() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        tr.runtime.project_memory.append(cwd, "fact one").unwrap();
        tr.runtime.project_memory.append(cwd, "fact two").unwrap();
        let out = render_memory_command(&tr.runtime, cwd, &["list"]).unwrap();
        assert!(out.contains("project memory (2):"));
        assert!(out.contains("[1]"));
        assert!(out.contains("fact one"));
        assert!(out.contains("[2]"));
        assert!(out.contains("fact two"));
    }

    #[test]
    fn test_memory_list_shows_metadata() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        tr.runtime
            .project_memory
            .append_fact(cwd, "fact one", "command", "confirmed")
            .unwrap();
        let out = render_memory_command(&tr.runtime, cwd, &["list"]).unwrap();
        assert!(out.contains("[command|confirmed|hits 0]"));
    }

    #[test]
    fn test_memory_rm_deletes_by_index() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        tr.runtime.project_memory.append(cwd, "fact one").unwrap();
        tr.runtime.project_memory.append(cwd, "fact two").unwrap();
        let out = render_memory_command(&tr.runtime, cwd, &["rm", "1"]).unwrap();
        assert!(out.contains("deleted memory item [1]"));
        let facts = tr.runtime.project_memory.list_facts(cwd).unwrap();
        assert_eq!(facts.len(), 1);
        assert!(facts[0].1.contains("fact two"));
    }

    #[test]
    fn test_memory_rm_invalid_id() {
        let tr = test_runtime();
        let out = render_memory_command(&tr.runtime, tr._dir.path(), &["rm", "abc"]).unwrap();
        assert!(out.contains("invalid memory id"));
    }

    #[test]
    fn test_memory_rm_out_of_range() {
        let tr = test_runtime();
        let out = render_memory_command(&tr.runtime, tr._dir.path(), &["rm", "9"]).unwrap();
        assert!(out.contains("no memory item at index [9]"));
    }

    #[test]
    fn test_memory_clear() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        tr.runtime.project_memory.append(cwd, "fact one").unwrap();
        let out = render_memory_command(&tr.runtime, cwd, &["clear"]).unwrap();
        assert!(out.contains("cleared project memory"));
        assert!(tr
            .runtime
            .project_memory
            .list_facts(cwd)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_memory_unknown_usage() {
        let tr = test_runtime();
        let out = render_memory_command(&tr.runtime, tr._dir.path(), &["bogus"]).unwrap();
        assert!(out.contains("usage: /memory"));
    }

    #[test]
    fn test_memory_promote_creates_skill_and_archives() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        tr.runtime
            .project_memory
            .append_fact(
                cwd,
                "- [2024-01-01 10:00] use cargo test",
                "command",
                "confirmed",
            )
            .unwrap();
        let out = render_memory_command(&tr.runtime, cwd, &["promote", "1"]).unwrap();
        assert!(out.contains("promoted memory item [1]"));
        assert!(out.contains(".agents/skills/"));

        // SKILL.md created.
        let skill_dir = cwd.join(".agents").join("skills");
        let entries: Vec<_> = std::fs::read_dir(&skill_dir).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
        let skill_md = entries[0].path().join("SKILL.md");
        let content = std::fs::read_to_string(&skill_md).unwrap();
        assert!(content.contains("use cargo test"));
        assert!(content.contains("name: use-cargo-test"));

        // Fact archived.
        let facts = tr.runtime.project_memory.list_fact_rows(cwd).unwrap();
        assert!(facts[0].archived);
    }

    #[test]
    fn test_memory_promote_invalid_index() {
        let tr = test_runtime();
        let out = render_memory_command(&tr.runtime, tr._dir.path(), &["promote", "9"]).unwrap();
        assert!(out.contains("no memory item at index [9]"));
    }

    #[test]
    fn test_memory_promote_archived_fact_errors() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        tr.runtime
            .project_memory
            .append_fact(cwd, "fact one", "fact", "inferred")
            .unwrap();
        let id = tr.runtime.project_memory.list_fact_rows(cwd).unwrap()[0].id;
        tr.runtime
            .project_memory
            .set_archived(cwd, id, true)
            .unwrap();
        let out = render_memory_command(&tr.runtime, cwd, &["promote", "1"]).unwrap();
        assert!(out.contains("already archived"));
    }

    #[test]
    fn test_memory_gc_reports_counts() {
        let tr = test_runtime();
        let cwd = tr._dir.path();
        // Two duplicates → one merged.
        tr.runtime
            .project_memory
            .append_fact(
                cwd,
                "- [2024-01-01 10:00] use cargo test",
                "fact",
                "inferred",
            )
            .unwrap();
        tr.runtime
            .project_memory
            .append_fact(
                cwd,
                "- [2024-01-02 11:00] use cargo test",
                "fact",
                "inferred",
            )
            .unwrap();
        let out = render_memory_command(&tr.runtime, cwd, &["gc"]).unwrap();
        assert!(out.contains("merged 1 duplicate(s)"));
        assert!(out.contains("archived 0 stale fact(s)"));
        assert!(out.contains("compacted 0 fact(s)"));
    }
}
