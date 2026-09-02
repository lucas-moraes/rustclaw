//! Lightweight, LLM-free project profiler.
//!
//! Heuristically inspects a project root to detect stack, build/test/run
//! commands and entry points. Used to seed the `# Project context` block.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Primary runtime stack of a project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StackKind {
    Rust,
    Node,
    Python,
    Go,
    #[default]
    Unknown,
}

/// In-memory representation of what the profiler knows about a project.
#[derive(Clone, Debug, Default)]
pub struct ProjectContext {
    pub cwd: PathBuf,
    pub stack: StackKind,
    pub manifests: Vec<PathBuf>,
    pub languages: Vec<String>,
    pub build_cmd: Option<String>,
    pub test_cmd: Option<String>,
    pub run_cmd: Option<String>,
    pub entry_points: Vec<PathBuf>,
    pub has_docker: bool,
    /// Seconds-since-epoch mtime per manifest, used for invalidation.
    pub source_mtimes: HashMap<PathBuf, u64>,
}

/// Max parent directories to walk when looking for a manifest.
const FIND_UP_MAX: usize = 5;

/// Profiler wrapper over a cached `ProjectContext`.
#[derive(Clone, Debug)]
pub struct ProjectProfiler {
    pub inner: ProjectContext,
}

impl ProjectProfiler {
    /// Detects stack/commands for the given working directory.
    pub fn analyze(cwd: &Path) -> ProjectContext {
        let mut ctx = ProjectContext {
            cwd: cwd.to_path_buf(),
            ..Default::default()
        };

        if let Some(p) = find_up(cwd, "Cargo.toml") {
            ctx.manifests.push(p.clone());
            ctx.languages.push("rust".to_string());
            ctx.build_cmd = Some("cargo build".to_string());
            ctx.test_cmd = Some("cargo test".to_string());
            ctx.run_cmd = Some("cargo run".to_string());
            ctx.entry_points.push(cwd.join("src/main.rs"));
            ctx.stack = StackKind::Rust;
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if let Some(p) = find_up(cwd, "package.json") {
            ctx.manifests.push(p.clone());
            ctx.languages.push("typescript/javascript".to_string());
            ctx.run_cmd = Some("npm start".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Node;
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        let py_signal = ["pyproject.toml", "requirements.txt", "setup.py"]
            .iter()
            .find_map(|f| find_up(cwd, f));
        if let Some(p) = py_signal {
            ctx.manifests.push(p.clone());
            ctx.languages.push("python".to_string());
            ctx.test_cmd = Some("pytest".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Python;
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if let Some(p) = find_up(cwd, "go.mod") {
            ctx.manifests.push(p.clone());
            ctx.languages.push("go".to_string());
            ctx.build_cmd = Some("go build ./...".to_string());
            ctx.test_cmd = Some("go test ./...".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Go;
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if cwd.join("Dockerfile").exists() || cwd.join("docker-compose.yml").exists() {
            ctx.has_docker = true;
        }

        ctx
    }

    pub fn new(cwd: &Path) -> Self {
        Self {
            inner: Self::analyze(cwd),
        }
    }

    /// Re-analyzes the project in place (used after `remember` updates state).
    #[allow(dead_code)]
    pub fn refresh(&mut self) {
        self.inner = Self::analyze(&self.inner.cwd);
    }

    /// Renders the `# Project context` block from structural data.
    pub fn render_summary(&self) -> String {
        let c = &self.inner;
        let mut s = String::from("# Project context\n");
        s.push_str(&format!("- Stack: {}\n", stack_label(c.stack)));
        if let Some(b) = &c.build_cmd {
            s.push_str(&format!("- Build: {}\n", b));
        }
        if let Some(t) = &c.test_cmd {
            s.push_str(&format!("- Test: {}\n", t));
        }
        if let Some(r) = &c.run_cmd {
            s.push_str(&format!("- Run: {}\n", r));
        }
        if c.has_docker {
            s.push_str("- Docker: present\n");
        }
        if !c.entry_points.is_empty() {
            let eps = c
                .entry_points
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            s.push_str(&format!("- Entry points: {}\n", eps));
        }
        s
    }
}

fn stack_label(s: StackKind) -> &'static str {
    match s {
        StackKind::Rust => "rust",
        StackKind::Node => "node",
        StackKind::Python => "python",
        StackKind::Go => "go",
        StackKind::Unknown => "unknown",
    }
}

fn snapshot_mtime(map: &mut HashMap<PathBuf, u64>, p: &Path) {
    if let Ok(md) = std::fs::metadata(p) {
        if let Ok(t) = md.modified() {
            if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                map.insert(p.to_path_buf(), d.as_secs());
            }
        }
    }
}

/// Looks for `name` walking up the directory tree (max FIND_UP_MAX levels).
fn find_up(start: &Path, name: &str) -> Option<PathBuf> {
    let mut cur = Some(start.to_path_buf());
    let mut depth = 0;
    while let Some(dir) = cur {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if depth >= FIND_UP_MAX {
            return None;
        }
        cur = dir.parent().map(|p| p.to_path_buf());
        depth += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_detect_rust() {
        let d = tempdir();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Rust);
        assert_eq!(ctx.build_cmd.as_deref(), Some("cargo build"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("cargo test"));
        assert!(ctx.manifests.iter().any(|p| p.ends_with("Cargo.toml")));
    }

    #[test]
    fn test_detect_node() {
        let d = tempdir();
        std::fs::write(
            d.path().join("package.json"),
            r#"{"scripts":{"build":"tsc","start":"node dist/index.js"}}"#,
        )
        .unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Node);
        assert_eq!(ctx.run_cmd.as_deref(), Some("npm start"));
    }

    #[test]
    fn test_detect_python() {
        let d = tempdir();
        std::fs::write(d.path().join("pyproject.toml"), "[project]\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Python);
        assert_eq!(ctx.test_cmd.as_deref(), Some("pytest"));
    }

    #[test]
    fn test_detect_go() {
        let d = tempdir();
        std::fs::write(d.path().join("go.mod"), "module x\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Go);
        assert_eq!(ctx.test_cmd.as_deref(), Some("go test ./..."));
    }

    #[test]
    fn test_unknown_stack() {
        let d = tempdir();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Unknown);
    }

    #[test]
    fn test_find_up_walks_parents() {
        let d = tempdir();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\n").unwrap();
        let sub = d.path().join("src").join("deep");
        std::fs::create_dir_all(&sub).unwrap();
        let found = find_up(&sub, "Cargo.toml");
        assert!(found.is_some());
        assert!(found.unwrap().ends_with("Cargo.toml"));
    }

    #[test]
    fn test_render_summary() {
        let d = tempdir();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(d.path().join("Dockerfile"), "FROM rust\n").unwrap();
        let p = ProjectProfiler::new(d.path());
        let s = p.render_summary();
        assert!(s.contains("# Project context"));
        assert!(s.contains("Stack: rust"));
        assert!(s.contains("Build: cargo build"));
        assert!(s.contains("Docker: present"));
    }
}
