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
            let facts = runtime.project_memory.list_facts(cwd)?;
            if facts.is_empty() {
                return Ok("no project memory yet (use the `remember` tool)".to_string());
            }
            let mut out = format!("project memory ({}):", facts.len());
            for (id, fact) in facts {
                out.push_str(&format!("\n  [{}] {}", id, fact));
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
        _ => Ok("usage: /memory [list] [rm|delete <id>] [clear]".to_string()),
    }
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
        let runtime = SessionRuntime::new(
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
}
