//! `grep` tool: search file contents with a regex, returning file:line:content.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;

const MAX_MATCHES: usize = 200;

pub struct GrepTool;

#[async_trait::async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Searches file contents using a regular expression. \
Returns `path:line:content`. Use `glob` to find files, then `grep` to find matches."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regex pattern to search for"},
                "path": {"type": "string", "description": "Directory or file to search (default: cwd)"},
                "include": {"type": "string", "description": "Only search files matching this glob (e.g. *.rs)"},
                "case_insensitive": {"type": "boolean", "description": "Case-insensitive match (default false)"}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| "missing required argument: pattern".to_string())?;
        let base = args["path"]
            .as_str()
            .map(|p| ctx.cwd.resolve(p))
            .unwrap_or_else(|| ctx.cwd.path().to_path_buf());
        let include = args["include"].as_str().map(|s| s.to_string());
        let case_insensitive = args["case_insensitive"].as_bool().unwrap_or(false);

        let regex_str = if case_insensitive {
            format!("(?i){}", pattern)
        } else {
            pattern.to_string()
        };
        let re =
            Regex::new(&regex_str).map_err(|e| format!("invalid regex `{}`: {}", pattern, e))?;

        let mut matches: Vec<String> = Vec::new();
        walk(&base, include.as_deref(), &re, &mut matches);

        let truncated = matches.len() > MAX_MATCHES;
        let mut body = matches
            .iter()
            .take(MAX_MATCHES)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if body.is_empty() {
            body = "(no matches)".to_string();
        } else if truncated {
            body.push_str(&format!(
                "\n\n[showing first {} of {} matches]",
                MAX_MATCHES,
                matches.len()
            ));
        }

        Ok(ToolResult::simple(
            format!("grep {}", preview(pattern, 40)),
            body,
        ))
    }
}

fn walk(base: &Path, include: Option<&str>, re: &Regex, out: &mut Vec<String>) {
    if base.is_file() {
        if include_matches(base, include) {
            search_file(base, re, out);
        }
        return;
    }
    if !base.is_dir() {
        return;
    }
    let entries = match std::fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if !super::glob::is_ignored(&path, base) {
                dirs.push(path);
            }
        } else if path.is_file() && include_matches(&path, include) {
            search_file(&path, re, out);
        }
    }
    for dir in dirs {
        walk(&dir, include, re, out);
    }
}

fn include_matches(path: &Path, include: Option<&str>) -> bool {
    match include {
        None => true,
        Some(inc) => {
            let name = path.to_string_lossy().to_string();
            let inc = inc.trim_start_matches("*");
            name.ends_with(inc)
        }
    }
}

fn search_file(path: &Path, re: &Regex, out: &mut Vec<String>) {
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    for (idx, line) in content.lines().enumerate() {
        if re.is_match(line) {
            out.push(format!("{}:{}:{}", path.display(), idx + 1, line));
            if out.len() > MAX_MATCHES {
                return;
            }
        }
    }
}
