//! `web_search` tool: public DuckDuckGo HTML search (no API key required).

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use serde_json::{json, Value};

const DDG_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/120.0 Safari/537.36";
const MAX_RESULTS: usize = 8;
const TIMEOUT_SECS: u64 = 10;

pub struct WebSearchTool;

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Pesquisa na web por documentação atualizada, crates de Rust, artigos \
técnicos ou resoluções de erros de compilação."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "A consulta de busca (ex: \"rust tokio select macro docs.rs\")"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }
        let query = args["query"]
            .as_str()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| "missing required argument: query".to_string())?;

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {}", e))?;

        let resp = client
            .get(DDG_ENDPOINT)
            .query(&[("q", query)])
            .send()
            .await
            .map_err(|e| format!("web search request failed: {}", e))?;

        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }

        if !resp.status().is_success() {
            return Err(format!("DuckDuckGo returned HTTP {}", resp.status()));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| format!("failed to read search response: {}", e))?;

        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }

        let results = parse_results(&html);
        if results.is_empty() {
            return Ok(ToolResult::simple(
                format!("web_search {}", preview(query, 40)),
                "(no results found)".to_string(),
            ));
        }

        let mut body = String::new();
        for (i, r) in results.iter().enumerate() {
            body.push_str(&format!(
                "{}. **{}**\n   URL: {}\n   {}\n\n",
                i + 1,
                r.title,
                r.url,
                r.snippet
            ));
        }

        Ok(ToolResult::simple(
            format!("web_search {}", preview(query, 40)),
            body,
        ))
    }
}

struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

/// Extracts up to MAX_RESULTS results from the DuckDuckGo HTML page.
fn parse_results(html: &str) -> Vec<SearchResult> {
    let mut out = Vec::new();
    // Each result block: a `result__a` link followed by a `result__snippet`.
    let re = regex::Regex::new(
        r#"(?s)<a[^>]*class="result__a"[^>]*href="([^"]*)"[^>]*>(.*?)</a>.*?<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#,
    )
    .unwrap();
    for cap in re.captures_iter(html) {
        if out.len() >= MAX_RESULTS {
            break;
        }
        let url = clean_url(&cap[1]);
        let title = strip_tags(&cap[2]);
        let snippet = strip_tags(&cap[3]);
        if title.is_empty() || url.is_empty() {
            continue;
        }
        out.push(SearchResult {
            title,
            url,
            snippet,
        });
    }
    out
}

fn strip_tags(s: &str) -> String {
    // Strip HTML tags and collapse whitespace.
    let re = regex::Regex::new(r"<[^>]+>").unwrap();
    let cleaned = re.replace_all(s, " ");
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// DuckDuckGo wraps result URLs in a redirect; unwrap the real target.
fn clean_url(href: &str) -> String {
    if let Some((_, rest)) = href.split_once("uddg=") {
        urlencoding::decode(rest)
            .map(|d| d.into_owned())
            .unwrap_or_else(|_| href.to_string())
    } else {
        href.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::permission::PermissionEngine;
    use crate::harness::tool::context::{AbortSignal, PathBufGuard};
    use std::sync::Arc;

    fn test_ctx() -> ToolContext {
        ToolContext {
            session_id: "s".into(),
            agent: "build".into(),
            agent_tools: vec![],
            cwd: PathBufGuard(std::path::PathBuf::from("/tmp")),
            abort: AbortSignal::new(),
            permission: Arc::new(PermissionEngine::default()),
            asker: Arc::new(AllowAsker),
            user_asker: Arc::new(NoUserAsker),
            todos: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            task_runner: None,
            project_memory: None,
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
    fn test_parse_results_extracts_links_and_snippets() {
        let html = r#"
        <div class="result">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Ftokio">Tokio docs</a>
          <a class="result__snippet" href="...">Async runtime <b>for Rust</b></a>
        </div>
        <div class="result">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fcrates.io">crates.io</a>
          <a class="result__snippet" href="...">Rust package registry</a>
        </div>
        "#;
        let results = parse_results(html);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Tokio docs");
        assert_eq!(results[0].url, "https://docs.rs/tokio");
        assert_eq!(results[0].snippet, "Async runtime for Rust");
        assert_eq!(results[1].url, "https://crates.io");
    }

    #[test]
    fn test_clean_url_unwraps_uddg() {
        assert_eq!(
            clean_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Ftokio"),
            "https://docs.rs/tokio"
        );
        assert_eq!(clean_url("https://plain.example"), "https://plain.example");
    }

    #[test]
    fn test_parse_results_empty_on_no_matches() {
        assert!(parse_results("<html><body>nothing here</body></html>").is_empty());
    }

    #[tokio::test]
    async fn test_missing_query_errors() {
        let tool = WebSearchTool;
        let err = tool.execute(json!({}), &test_ctx()).await.unwrap_err();
        assert!(err.contains("query"));
    }

    #[tokio::test]
    async fn test_empty_query_errors() {
        let tool = WebSearchTool;
        let err = tool
            .execute(json!({"query": "   "}), &test_ctx())
            .await
            .unwrap_err();
        assert!(err.contains("query"));
    }
}
