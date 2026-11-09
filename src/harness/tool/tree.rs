//! `tree` tool: render a directory tree, respecting common ignore dirs.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

/// Directories that are never descended into. Mirrors the glob tool's list
/// (plus a few build/dependency dirs) so the tree stays small and relevant.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    "vendor",
];

const DEFAULT_MAX_DEPTH: usize = 4;
const MAX_DEPTH_LIMIT: usize = 8;
const DEFAULT_MAX_ENTRIES: usize = 400;
const MAX_ENTRIES_LIMIT: usize = 2000;

pub struct TreeTool;

#[async_trait::async_trait]
impl Tool for TreeTool {
    fn name(&self) -> &str {
        "tree"
    }

    fn description(&self) -> &str {
        "Renders a directory tree rooted at `path` (default: working directory), \
skipping common ignored directories (.git, node_modules, target, ...). \
Optional `max_depth` (1-8, default 4) and `max_entries` (default 400). \
Use it to map a project's structure before searching."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Root directory (default: cwd)"},
                "max_depth": {"type": "integer", "description": "Max depth (1-8, default 4)"},
                "max_entries": {"type": "integer", "description": "Max entries listed (default 400)"}
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let root = args["path"]
            .as_str()
            .map(|p| ctx.cwd.resolve(p))
            .unwrap_or_else(|| ctx.cwd.path().to_path_buf());

        if !root.is_dir() {
            return Err(format!("path is not a directory: {}", root.display()));
        }

        let max_depth = match args["max_depth"].as_u64() {
            Some(n) => (n as usize).clamp(1, MAX_DEPTH_LIMIT),
            None => DEFAULT_MAX_DEPTH,
        };
        let max_entries = match args["max_entries"].as_u64() {
            Some(n) => (n as usize).clamp(1, MAX_ENTRIES_LIMIT),
            None => DEFAULT_MAX_ENTRIES,
        };

        let mut lines: Vec<String> = Vec::new();
        let mut truncated = false;
        walk(
            &root,
            "",
            0,
            max_depth,
            max_entries,
            &mut lines,
            &mut truncated,
        );

        let mut body = format!("{}", root.display());
        if !lines.is_empty() {
            body.push('\n');
            body.push_str(&lines.join("\n"));
        }
        if truncated {
            body.push_str("\n\n[truncated: raise max_depth/max_entries or narrow `path`]");
        }

        Ok(ToolResult::simple(
            format!("tree {}", preview(&root.display().to_string(), 40)),
            body,
        ))
    }
}

/// Recursive depth-first walk appending ASCII-tree lines.
///
/// `prefix` carries the accumulated `├── `/`└── ` gutter; `remaining` is the
/// depth budget below `dir`. Directories sort before files, both
/// case-insensitively, so the output reads like a file manager.
fn walk(
    dir: &std::path::Path,
    prefix: &str,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    lines: &mut Vec<String>,
    truncated: &mut bool,
) {
    if depth >= max_depth || *truncated {
        return;
    }

    let mut entries: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| !is_ignored_dir(p))
            .collect(),
        Err(_) => return,
    };
    entries.sort_by(|a, b| {
        let key = |p: &std::path::Path| {
            (
                !p.is_dir(),
                p.file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default(),
            )
        };
        key(a).cmp(&key(b))
    });

    let total = entries.len();
    for (i, entry) in entries.iter().enumerate() {
        if lines.len() >= max_entries {
            *truncated = true;
            return;
        }
        let last = i + 1 == total;
        let connector = if last {
            "\u{2514}\u{2500}\u{2500} "
        } else {
            "\u{251C}\u{2500}\u{2500} "
        };
        let name = entry
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if entry.is_dir() {
            lines.push(format!("{prefix}{connector}{name}/"));
            let child_prefix = format!("{prefix}{}", if last { "    " } else { "\u{2502}   " });
            walk(
                entry,
                &child_prefix,
                depth + 1,
                max_depth,
                max_entries,
                lines,
                truncated,
            );
        } else {
            lines.push(format!("{prefix}{connector}{name}"));
        }
    }
}

fn is_ignored_dir(path: &std::path::Path) -> bool {
    path.file_name()
        .map(|n| {
            let n = n.to_string_lossy();
            IGNORED_DIRS.iter().any(|d| *d == n)
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_basic_structure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/harness/tool")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), b"").unwrap();
        std::fs::write(dir.path().join("src/harness/mod.rs"), b"").unwrap();
        std::fs::write(dir.path().join("src/harness/tool/tree.rs"), b"").unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(dir.path().join("target/debug/foo"), b"").unwrap();

        let mut lines = Vec::new();
        let mut truncated = false;
        walk(dir.path(), "", 0, 8, 400, &mut lines, &mut truncated);

        let out = lines.join("\n");
        assert!(out.contains("src/"), "missing src/: {out}");
        assert!(out.contains("main.rs"), "missing main.rs: {out}");
        assert!(out.contains("harness/"), "missing harness/: {out}");
        assert!(out.contains("tree.rs"), "missing tree.rs: {out}");
        assert!(!out.contains("target"), "target must be ignored: {out}");
        assert!(!truncated);
    }

    #[test]
    fn test_tree_depth_cap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b/c/d/e")).unwrap();
        std::fs::write(dir.path().join("a/b/c/d/e/deep.txt"), b"").unwrap();

        let mut lines = Vec::new();
        let mut truncated = false;
        walk(dir.path(), "", 0, 2, 400, &mut lines, &mut truncated);

        let out = lines.join("\n");
        assert!(out.contains("a/"));
        assert!(out.contains("b/"));
        assert!(!out.contains("c/"), "depth 2 must not reach c/: {out}");
    }

    #[test]
    fn test_tree_entry_cap_truncates() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..30 {
            std::fs::write(dir.path().join(format!("f{i:02}.txt")), b"").unwrap();
        }

        let mut lines = Vec::new();
        let mut truncated = false;
        walk(dir.path(), "", 0, 8, 10, &mut lines, &mut truncated);

        assert_eq!(lines.len(), 10);
        assert!(truncated);
    }

    #[test]
    fn test_tree_dirs_sort_before_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("zfile.txt"), b"").unwrap();
        std::fs::create_dir_all(dir.path().join("adir")).unwrap();

        let mut lines = Vec::new();
        let mut truncated = false;
        walk(dir.path(), "", 0, 8, 400, &mut lines, &mut truncated);

        assert!(
            lines[0].ends_with("adir/"),
            "dir must come first: {:?}",
            lines
        );
        assert!(
            lines[1].ends_with("zfile.txt"),
            "file must come second: {:?}",
            lines
        );
    }
}
