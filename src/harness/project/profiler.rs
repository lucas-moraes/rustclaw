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
    Cpp,
    Ruby,
    Java,
    Php,
    Elixir,
    Swift,
    Zig,
    Deno,
    Bun,
    Dotnet,
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

        // C++: CMake has priority; Makefile is the fallback. Commands are only
        // set when no primary stack claimed them (a Rust project may still
        // ship a Makefile as a helper wrapper).
        let cpp_signal = find_up(cwd, "CMakeLists.txt").or_else(|| find_up(cwd, "Makefile"));
        if let Some(p) = cpp_signal {
            ctx.manifests.push(p.clone());
            ctx.languages.push("cpp".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Cpp;
                if p.file_name().is_some_and(|n| n == "CMakeLists.txt") {
                    ctx.build_cmd = Some("cmake -B build".to_string());
                    ctx.test_cmd = Some("ctest --test-dir build".to_string());
                } else {
                    ctx.build_cmd = Some("make".to_string());
                    ctx.test_cmd = Some("make test".to_string());
                }
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if let Some(p) = find_up(cwd, "Gemfile") {
            ctx.manifests.push(p.clone());
            ctx.languages.push("ruby".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Ruby;
                ctx.build_cmd = Some("bundle install".to_string());
                ctx.test_cmd = Some("bundle exec rspec".to_string());
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        let jvm_signal = ["pom.xml", "build.gradle", "build.gradle.kts"]
            .iter()
            .find_map(|f| find_up(cwd, f));
        if let Some(p) = jvm_signal {
            ctx.manifests.push(p.clone());
            ctx.languages.push("java/kotlin".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Java;
                if p.file_name().is_some_and(|n| n == "pom.xml") {
                    ctx.build_cmd = Some("mvn -q compile".to_string());
                    ctx.test_cmd = Some("mvn test".to_string());
                } else {
                    ctx.build_cmd = Some("gradle build".to_string());
                    ctx.test_cmd = Some("gradle test".to_string());
                }
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if let Some(p) = find_up(cwd, "composer.json") {
            ctx.manifests.push(p.clone());
            ctx.languages.push("php".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Php;
                ctx.build_cmd = Some("composer install".to_string());
                ctx.test_cmd = Some("composer test".to_string());
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if let Some(p) = find_up(cwd, "mix.exs") {
            ctx.manifests.push(p.clone());
            ctx.languages.push("elixir".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Elixir;
                ctx.build_cmd = Some("mix deps.get".to_string());
                ctx.test_cmd = Some("mix test".to_string());
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if let Some(p) = find_up(cwd, "Package.swift") {
            ctx.manifests.push(p.clone());
            ctx.languages.push("swift".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Swift;
                ctx.build_cmd = Some("swift build".to_string());
                ctx.test_cmd = Some("swift test".to_string());
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if let Some(p) = find_up(cwd, "build.zig") {
            ctx.manifests.push(p.clone());
            ctx.languages.push("zig".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Zig;
                ctx.build_cmd = Some("zig build".to_string());
                ctx.test_cmd = Some("zig build test".to_string());
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        // Deno before Bun: bun.lockb is a lockfile, deno.json is a manifest.
        if let Some(p) = find_up(cwd, "deno.json").or_else(|| find_up(cwd, "deno.jsonc")) {
            ctx.manifests.push(p.clone());
            ctx.languages.push("typescript/javascript".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Deno;
                ctx.build_cmd = Some("deno task build".to_string());
                ctx.test_cmd = Some("deno test".to_string());
                ctx.run_cmd = Some("deno task start".to_string());
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        if let Some(p) = find_up(cwd, "bun.lockb").or_else(|| find_up(cwd, "bun.lock")) {
            ctx.manifests.push(p.clone());
            ctx.languages.push("typescript/javascript".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Bun;
                ctx.build_cmd = Some("bun install".to_string());
                ctx.test_cmd = Some("bun test".to_string());
                ctx.run_cmd = Some("bun start".to_string());
            }
            snapshot_mtime(&mut ctx.source_mtimes, &p);
        }

        // .NET: solution file first, then any project file in the root.
        let dotnet_signal = find_up(cwd, "global.json")
            .or_else(|| find_up(cwd, "Directory.Build.props"))
            .or_else(|| {
                std::fs::read_dir(cwd).ok().and_then(|entries| {
                    entries.filter_map(|e| e.ok()).find_map(|e| {
                        let name = e.file_name();
                        let name = name.to_string_lossy();
                        if name.ends_with(".sln")
                            || name.ends_with(".csproj")
                            || name.ends_with(".fsproj")
                        {
                            Some(e.path())
                        } else {
                            None
                        }
                    })
                })
            });
        if let Some(p) = dotnet_signal {
            ctx.manifests.push(p.clone());
            ctx.languages.push("csharp/fsharp".to_string());
            if ctx.stack == StackKind::Unknown {
                ctx.stack = StackKind::Dotnet;
                ctx.build_cmd = Some("dotnet build".to_string());
                ctx.test_cmd = Some("dotnet test".to_string());
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
        StackKind::Cpp => "cpp",
        StackKind::Ruby => "ruby",
        StackKind::Java => "java",
        StackKind::Php => "php",
        StackKind::Elixir => "elixir",
        StackKind::Swift => "swift",
        StackKind::Zig => "zig",
        StackKind::Deno => "deno",
        StackKind::Bun => "bun",
        StackKind::Dotnet => "dotnet",
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
    fn test_detect_cpp_via_cmake() {
        let d = tempdir();
        std::fs::write(d.path().join("CMakeLists.txt"), "project(x)\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Cpp);
        assert_eq!(ctx.build_cmd.as_deref(), Some("cmake -B build"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("ctest --test-dir build"));
    }

    #[test]
    fn test_detect_cpp_via_makefile() {
        let d = tempdir();
        std::fs::write(d.path().join("Makefile"), "all:\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Cpp);
        assert_eq!(ctx.build_cmd.as_deref(), Some("make"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("make test"));
    }

    #[test]
    fn test_cmake_beats_makefile() {
        let d = tempdir();
        std::fs::write(d.path().join("CMakeLists.txt"), "project(x)\n").unwrap();
        std::fs::write(d.path().join("Makefile"), "all:\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Cpp);
        assert_eq!(ctx.build_cmd.as_deref(), Some("cmake -B build"));
    }

    #[test]
    fn test_makefile_does_not_override_rust_commands() {
        // A Rust project may ship a Makefile as a helper wrapper; the
        // primary stack (Cargo) must keep its build/test commands.
        let d = tempdir();
        std::fs::write(d.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::write(d.path().join("Makefile"), "all:\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Rust);
        assert_eq!(ctx.build_cmd.as_deref(), Some("cargo build"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("cargo test"));
    }

    #[test]
    fn test_detect_ruby() {
        let d = tempdir();
        std::fs::write(d.path().join("Gemfile"), "source 'https://rubygems.org'\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Ruby);
        assert_eq!(ctx.build_cmd.as_deref(), Some("bundle install"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("bundle exec rspec"));
    }

    #[test]
    fn test_detect_java_maven() {
        let d = tempdir();
        std::fs::write(d.path().join("pom.xml"), "<project/>\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Java);
        assert_eq!(ctx.build_cmd.as_deref(), Some("mvn -q compile"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("mvn test"));
    }

    #[test]
    fn test_detect_java_gradle() {
        let d = tempdir();
        std::fs::write(d.path().join("build.gradle"), "plugins {}\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Java);
        assert_eq!(ctx.build_cmd.as_deref(), Some("gradle build"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("gradle test"));
    }

    #[test]
    fn test_detect_php() {
        let d = tempdir();
        std::fs::write(d.path().join("composer.json"), "{}\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Php);
        assert_eq!(ctx.build_cmd.as_deref(), Some("composer install"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("composer test"));
    }

    #[test]
    fn test_detect_elixir() {
        let d = tempdir();
        std::fs::write(d.path().join("mix.exs"), "defmodule X.MixProject do\nend\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Elixir);
        assert_eq!(ctx.build_cmd.as_deref(), Some("mix deps.get"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("mix test"));
    }

    #[test]
    fn test_detect_swift() {
        let d = tempdir();
        std::fs::write(
            d.path().join("Package.swift"),
            "// swift-tools-version:5.9\n",
        )
        .unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Swift);
        assert_eq!(ctx.build_cmd.as_deref(), Some("swift build"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("swift test"));
    }

    #[test]
    fn test_detect_zig() {
        let d = tempdir();
        std::fs::write(
            d.path().join("build.zig"),
            "pub fn build(b: *std.Build) void {}\n",
        )
        .unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Zig);
        assert_eq!(ctx.build_cmd.as_deref(), Some("zig build"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("zig build test"));
    }

    #[test]
    fn test_detect_deno() {
        let d = tempdir();
        std::fs::write(d.path().join("deno.json"), "{}\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Deno);
        assert_eq!(ctx.build_cmd.as_deref(), Some("deno task build"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("deno test"));
        assert_eq!(ctx.run_cmd.as_deref(), Some("deno task start"));
    }

    #[test]
    fn test_detect_bun() {
        let d = tempdir();
        std::fs::write(d.path().join("bun.lockb"), b"\0bun\0").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Bun);
        assert_eq!(ctx.build_cmd.as_deref(), Some("bun install"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("bun test"));
        assert_eq!(ctx.run_cmd.as_deref(), Some("bun start"));
    }

    #[test]
    fn test_detect_dotnet() {
        let d = tempdir();
        std::fs::write(d.path().join("App.sln"), "\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Dotnet);
        assert_eq!(ctx.build_cmd.as_deref(), Some("dotnet build"));
        assert_eq!(ctx.test_cmd.as_deref(), Some("dotnet test"));
    }

    #[test]
    fn test_detect_dotnet_csproj() {
        let d = tempdir();
        std::fs::write(d.path().join("App.csproj"), "<Project/>\n").unwrap();
        let ctx = ProjectProfiler::analyze(d.path());
        assert_eq!(ctx.stack, StackKind::Dotnet);
        assert_eq!(ctx.build_cmd.as_deref(), Some("dotnet build"));
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
