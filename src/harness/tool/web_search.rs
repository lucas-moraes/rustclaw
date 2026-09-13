//! `web_search` tool: public DuckDuckGo HTML search (no API key required).
//!
//! Robustness features (see TODO.md):
//! - F1: internal rate limiting (min delay between searches + concurrency semaphore)
//! - F2: retry with backoff on blocking / suspicious empty responses
//! - F3: silent rate-limit detection (HTTP 200 with empty page)
//! - F5: User-Agent rotation (modern desktop browsers only)
//! - F10: diagnostic logging of failures/blocks

use scraper::{Html, Selector};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;

const DDG_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
const MAX_RESULTS: usize = 8;
const TIMEOUT_SECS: u64 = 10;

/// F1.1: minimum gap (ms) between the end of one request and the start of the next.
const MIN_DELAY_MS: u64 = 1500;

/// F2.3: backoff delays (seconds) between retry attempts.
const RETRY_DELAYS: &[u64] = &[2, 4];

/// F5.1: pool of modern desktop browser User-Agents (avoid curl/Python UAs).
const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/17.1 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36 Edg/119.0.0.0",
];

/// Shared state for the web search tool, kept across calls within a runtime.
pub struct WebSearchTool {
    /// F1.1: timestamp of the last completed request (for dynamic min-delay).
    last_request: Arc<tokio::sync::Mutex<Option<Instant>>>,
    /// F1.2: concurrency semaphore (permit=1) serializing searches.
    semaphore: Arc<tokio::sync::Semaphore>,
    /// F5.1: rotating index into `USER_AGENTS`.
    ua_index: Arc<AtomicUsize>,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            last_request: Arc::new(tokio::sync::Mutex::new(None)),
            semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
            ua_index: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

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

        // F1.2: acquire the concurrency permit (serializes concurrent searches).
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| "web_search semaphore closed".to_string())?;

        // F1.1: dynamic min-delay between the end of the last request and now.
        self.wait_min_delay(ctx).await?;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {}", e))?;

        // F5.1: pick a UA (rotating). On retry we rotate again so identical
        // requests use different UAs.
        let mut attempt = 0usize;
        loop {
            if ctx.abort.is_aborted() {
                return Err("aborted".to_string());
            }
            let ua = self.next_user_agent();
            let url = reqwest::Url::parse_with_params(DDG_ENDPOINT, &[("q", query)])
                .map_err(|e| format!("failed to build search URL: {}", e))?;

            tracing::debug!(
                endpoint = DDG_ENDPOINT,
                query = %preview(query, 40),
                attempt,
                "web_search request"
            );

            let resp = client
                .get(url)
                .header(reqwest::header::USER_AGENT, ua)
                .send()
                .await
                .map_err(|e| format!("web search request failed: {}", e))?;

            if ctx.abort.is_aborted() {
                return Err("aborted".to_string());
            }

            let status = resp.status();
            if !status.is_success() {
                tracing::warn!(%status, attempt, "web_search non-success status");
                if attempt < RETRY_DELAYS.len() {
                    attempt += 1;
                    self.sleep_retry(ctx, attempt).await?;
                    continue;
                }
                return Err(format!("DuckDuckGo returned HTTP {}", status));
            }

            let html = resp
                .text()
                .await
                .map_err(|e| format!("failed to read search response: {}", e))?;

            // F1.1: record the end of this request so the next one waits.
            self.mark_request_done().await;

            if ctx.abort.is_aborted() {
                return Err("aborted".to_string());
            }

            // F3: silent rate-limit detection (HTTP 200 with empty/suspicious page).
            let blocked = is_blocked(&html);
            let rate_limited = looks_rate_limited(&html);

            if blocked || rate_limited {
                tracing::warn!(
                    blocked,
                    rate_limited,
                    html_len = html.len(),
                    attempt,
                    "web_search detected blocking/rate-limit"
                );
                if attempt < RETRY_DELAYS.len() {
                    attempt += 1;
                    self.sleep_retry(ctx, attempt).await?;
                    continue;
                }
                return Err(
                    "DuckDuckGo bloqueou a requisição (possível detecção de bot/CAPTCHA ou \
rate limiting). Tente novamente em alguns instantes ou reformule a consulta."
                        .to_string(),
                );
            }

            let results = parse_results(&html);

            // F2.2: empty + suspicious → treat as blocking and retry.
            if results.is_empty() && looks_rate_limited(&html) {
                tracing::warn!(
                    attempt,
                    "web_search empty results with rate-limit indicators; retrying"
                );
                if attempt < RETRY_DELAYS.len() {
                    attempt += 1;
                    self.sleep_retry(ctx, attempt).await?;
                    continue;
                }
                return Err(
                    "DuckDuckGo retornou uma página vazia com indícios de rate limiting. \
Tente novamente em alguns instantes."
                        .to_string(),
                );
            }

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

            return Ok(ToolResult::simple(
                format!("web_search {}", preview(query, 40)),
                body,
            ));
        }
    }
}

impl WebSearchTool {
    /// F1.1: sleeps until `MIN_DELAY_MS` has elapsed since the last request
    /// finished. Abort-aware.
    async fn wait_min_delay(&self, ctx: &ToolContext) -> Result<(), String> {
        let last = self.last_request.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            let min = Duration::from_millis(MIN_DELAY_MS);
            if elapsed < min {
                let wait = min - elapsed;
                drop(last);
                if !sleep_abortable(ctx, wait).await {
                    return Err("aborted".to_string());
                }
                return Ok(());
            }
        }
        Ok(())
    }

    /// F1.1: records the end of a request (called after the request completes).
    async fn mark_request_done(&self) {
        let mut last = self.last_request.lock().await;
        *last = Some(Instant::now());
    }

    /// F5.1: returns the next User-Agent in the rotation.
    fn next_user_agent(&self) -> &'static str {
        let idx = self.ua_index.fetch_add(1, Ordering::Relaxed);
        USER_AGENTS[idx % USER_AGENTS.len()]
    }

    /// F2.3: sleeps for the backoff delay of the given attempt (1-based).
    async fn sleep_retry(&self, ctx: &ToolContext, attempt: usize) -> Result<(), String> {
        let delay = RETRY_DELAYS
            .get(attempt - 1)
            .copied()
            .unwrap_or(RETRY_DELAYS[RETRY_DELAYS.len() - 1]);
        tracing::warn!(attempt, delay_secs = delay, "web_search retrying");
        if !sleep_abortable(ctx, Duration::from_secs(delay)).await {
            return Err("aborted".to_string());
        }
        Ok(())
    }
}

/// Sleeps for `dur`, returning false if aborted during the wait.
async fn sleep_abortable(ctx: &ToolContext, dur: Duration) -> bool {
    let sleep = tokio::time::sleep(dur);
    tokio::pin!(sleep);
    tokio::select! {
        _ = &mut sleep => true,
        _ = ctx.abort.wait() => false,
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

/// F3.1: heuristics for a silent rate-limit page (HTTP 200 with empty body).
fn looks_rate_limited(html: &str) -> bool {
    let lower = html.to_lowercase();
    [
        "too many requests",
        "unusual traffic",
        "network anomaly",
        "anomaly",
        "rate limit",
        "rate-limited",
    ]
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

    #[test]
    fn test_looks_rate_limited_detects_terms() {
        assert!(looks_rate_limited("<html>Too Many Requests</html>"));
        assert!(looks_rate_limited("<html>unusual traffic detected</html>"));
        assert!(looks_rate_limited("<html>network anomaly</html>"));
        assert!(looks_rate_limited("<html>rate limit exceeded</html>"));
    }

    #[test]
    fn test_looks_rate_limited_false_for_normal() {
        assert!(!looks_rate_limited("<html>normal search results</html>"));
        assert!(!looks_rate_limited("<html>nothing here</html>"));
    }

    #[test]
    fn test_user_agent_rotation() {
        let tool = WebSearchTool::new();
        let a = tool.next_user_agent();
        let b = tool.next_user_agent();
        let c = tool.next_user_agent();
        assert_ne!(a, b);
        assert_ne!(b, c);
        // All must be modern desktop browser UAs (no curl/Python).
        for ua in [a, b, c] {
            assert!(ua.contains("Mozilla/5.0"));
            assert!(!ua.contains("curl"));
            assert!(!ua.contains("Python"));
        }
    }

    #[test]
    fn test_retry_delays_configured() {
        assert_eq!(RETRY_DELAYS, &[2, 4]);
    }

    #[tokio::test]
    async fn test_missing_query_errors() {
        let tool = WebSearchTool::new();
        let err = tool.execute(json!({}), &test_ctx()).await.unwrap_err();
        assert!(err.contains("query"));
    }

    #[tokio::test]
    async fn test_empty_query_errors() {
        let tool = WebSearchTool::new();
        let err = tool
            .execute(json!({"query": "   "}), &test_ctx())
            .await
            .unwrap_err();
        assert!(err.contains("query"));
    }

    #[tokio::test]
    async fn test_min_delay_between_requests() {
        // F1.1: after marking a request done, the next wait_min_delay must
        // sleep for at least the remaining gap.
        let tool = WebSearchTool::new();
        let ctx = test_ctx();
        tool.mark_request_done().await;
        let start = Instant::now();
        tool.wait_min_delay(&ctx).await.unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(MIN_DELAY_MS),
            "expected at least {}ms delay, got {:?}",
            MIN_DELAY_MS,
            elapsed
        );
    }

    #[tokio::test]
    async fn test_no_delay_when_no_prior_request() {
        // F1.1: first call (no prior request) should not sleep.
        let tool = WebSearchTool::new();
        let ctx = test_ctx();
        let start = Instant::now();
        tool.wait_min_delay(&ctx).await.unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(MIN_DELAY_MS),
            "first call should not sleep, got {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn test_semaphore_serializes_concurrent() {
        // F1.2: the semaphore has permit=1, so concurrent acquisitions are
        // serialized. We verify the permit count is 1.
        let tool = WebSearchTool::new();
        let permit = tool.semaphore.try_acquire().unwrap();
        // Second acquisition must fail (only 1 permit).
        assert!(tool.semaphore.try_acquire().is_err());
        drop(permit);
        // After release, acquisition succeeds again.
        assert!(tool.semaphore.try_acquire().is_ok());
    }
}
