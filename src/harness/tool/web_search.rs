//! `web_search` tool: public DuckDuckGo HTML search (no API key required).

use scraper::{Html, Selector};
use serde_json::{json, Value};

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;

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

        let url = reqwest::Url::parse_with_params(DDG_ENDPOINT, &[("q", query)])
            .map_err(|e| format!("failed to build search URL: {}", e))?;
        let resp = client
            .get(url)
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

        if is_blocked(&html) {
            return Err(
                "DuckDuckGo bloqueou a requisição (possível detecção de bot/CAPTCHA). \
Tente novamente em alguns instantes ou reformule a consulta."
                    .to_string(),
            );
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

/// Extracts up to MAX_RESULTS results from the DuckDuckGo HTML page using
/// CSS selectors.
fn parse_results(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let Ok(result_selector) = Selector::parse("div.result") else {
        return Vec::new();
    };
    let Ok(title_selector) = Selector::parse("a.result__a") else {
        return Vec::new();
    };
    let Ok(snippet_selector) = Selector::parse("a.result__snippet") else {
        return Vec::new();
    };

    document
        .select(&result_selector)
        .filter_map(|result| {
            let title_el = result.select(&title_selector).next()?;
            let title = clean_text(&title_el.text().collect::<Vec<_>>().join(" "));
            let url = title_el.value().attr("href").unwrap_or("").to_string();
            let url = clean_url(&url);
            if title.is_empty() || url.is_empty() {
                return None;
            }
            let snippet = result
                .select(&snippet_selector)
                .next()
                .map(|s| clean_text(&s.text().collect::<Vec<_>>().join(" ")))
                .unwrap_or_default();
            Some(SearchResult {
                title,
                url,
                snippet,
            })
        })
        .take(MAX_RESULTS)
        .collect()
}

/// Collapses whitespace in text extracted from the DOM.
fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
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

/// Detects whether DuckDuckGo served a bot-detection / CAPTCHA page instead
/// of search results.
fn is_blocked(html: &str) -> bool {
    let lower = html.to_lowercase();
    ["anomaly-detected", "captcha", "bot-detected"]
        .iter()
        .any(|term| lower.contains(term))
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
            events: crate::harness::event::event_channel().0,
            project_memory: None,
            hooks: Default::default(),
            checkpoints: std::sync::Arc::new(
                crate::harness::tool::checkpoint::FileCheckpoints::new(),
            ),
            jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
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
    fn test_parse_results_with_scraper() {
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

    #[test]
    fn test_is_blocked_detects_captcha() {
        assert!(is_blocked("<html>Anomaly-Detected! please verify</html>"));
        assert!(is_blocked(
            "<html>please solve the Captcha to continue</html>"
        ));
        assert!(is_blocked("<html>bot-detected request blocked</html>"));
    }

    #[test]
    fn test_is_blocked_false_for_normal() {
        assert!(!is_blocked("<html>normal search results here</html>"));
        assert!(!is_blocked(
            "<html>normal search results with relevant links</html>"
        ));
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
