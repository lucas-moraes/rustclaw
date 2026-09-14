//! `web_search` tool: web search via pluggable HTML providers (no API key).
//!
//! Architecture (see TODO.md F11): the tool is a facade over a list of
//! `SearchProvider` implementations, tried in order with automatic fallback.
//!
//! Robustness features:
//! - F1: internal rate limiting (min delay between searches + concurrency semaphore)
//! - F2: retry with backoff on blocking / suspicious empty responses
//! - F3: silent rate-limit detection (HTTP 200 with empty page)
//! - F4: fallback to the DDG Lite endpoint when `/html/` fails
//! - F5: User-Agent rotation (modern desktop browsers only)
//! - F9: in-memory result cache (never caches empty/error responses)
//! - F10: diagnostic logging of failures/blocks
//! - F11: multiple providers (DDG HTML, DDG Lite, Mojeek) with wide fallback
//!   (HTTP error, block detection, or 0 parsed results) and per-provider
//!   cooldown after rate-limit failures.

use scraper::{Html, Selector};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;

const DDG_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
/// F4.2: alternative (less rate-limited) DDG endpoint.
const DDG_LITE_ENDPOINT: &str = "https://lite.duckduckgo.com/lite/";
/// F11.1: Mojeek HTML endpoint (no API key required).
const MOJEEK_ENDPOINT: &str = "https://www.mojeek.com/search";

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

/// F9.1: cache TTL (1 hour).
const CACHE_TTL: Duration = Duration::from_secs(3600);
/// F9.2: maximum number of cache entries.
const CACHE_MAX_ENTRIES: usize = 100;

/// F7.1: default and bounds for `max_results`.
const DEFAULT_MAX_RESULTS: usize = 8;
const MAX_RESULTS_MIN: usize = 1;
const MAX_RESULTS_MAX: usize = 20;

/// F11.4: cooldown applied to a provider after a rate-limit/block failure.
const COOLDOWN_SECS: u64 = 120;

/// F11.1: identifier of a search provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderId {
    DdgHtml,
    DdgLite,
    Mojeek,
}

impl ProviderId {
    fn name(self) -> &'static str {
        match self {
            ProviderId::DdgHtml => "ddg_html",
            ProviderId::DdgLite => "ddg_lite",
            ProviderId::Mojeek => "mojeek",
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            ProviderId::DdgHtml => DDG_ENDPOINT,
            ProviderId::DdgLite => DDG_LITE_ENDPOINT,
            ProviderId::Mojeek => MOJEEK_ENDPOINT,
        }
    }

    /// F11.2: default order of preference.
    #[cfg(test)]
    fn all() -> [ProviderId; 3] {
        [ProviderId::DdgHtml, ProviderId::DdgLite, ProviderId::Mojeek]
    }
}

/// F11.1: why a provider attempt failed (drives fallback + cooldown).
#[derive(Debug, Clone, PartialEq)]
enum FailureKind {
    /// HTTP error (non-2xx) or network/timeout failure.
    Http,
    /// Bot detection / CAPTCHA / silent rate-limit page.
    Blocked,
    /// Page fetched fine but the parser extracted 0 results.
    Empty,
}

/// F11.1: a single search provider (HTML scraping, no API key).
#[async_trait::async_trait]
trait SearchProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Fetches and parses results for `query`. Returns the parsed results or
    /// the failure kind (which drives fallback/cooldown in the facade).
    async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        ua: &str,
    ) -> Result<Vec<SearchResult>, FailureKind>;
}

/// F11.1: DuckDuckGo provider — shared fetch logic, two parsers (HTML + Lite).
struct DuckDuckGoProvider {
    id: ProviderId,
}

#[async_trait::async_trait]
impl SearchProvider for DuckDuckGoProvider {
    fn id(&self) -> ProviderId {
        self.id
    }

    async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        ua: &str,
    ) -> Result<Vec<SearchResult>, FailureKind> {
        let url = reqwest::Url::parse_with_params(self.id.endpoint(), &[("q", query)])
            .map_err(|_| FailureKind::Http)?;

        let resp = client
            .get(url)
            .header(reqwest::header::USER_AGENT, ua)
            .send()
            .await
            .map_err(|_| FailureKind::Http)?;

        let status = resp.status();
        if !status.is_success() {
            return Err(FailureKind::Http);
        }

        let html = resp.text().await.map_err(|_| FailureKind::Http)?;

        if is_blocked(&html) || looks_rate_limited(&html) {
            return Err(FailureKind::Blocked);
        }

        let results = match self.id {
            ProviderId::DdgHtml => parse_results(&html),
            ProviderId::DdgLite => parse_results_lite(&html),
            _ => return Err(FailureKind::Empty),
        };

        if results.is_empty() {
            Err(FailureKind::Empty)
        } else {
            Ok(results)
        }
    }
}

/// F11.1: Mojeek provider (plain HTML results, no API key).
struct MojeekProvider;

#[async_trait::async_trait]
impl SearchProvider for MojeekProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Mojeek
    }

    async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        ua: &str,
    ) -> Result<Vec<SearchResult>, FailureKind> {
        let url = reqwest::Url::parse_with_params(MOJEEK_ENDPOINT, &[("q", query)])
            .map_err(|_| FailureKind::Http)?;

        let resp = client
            .get(url)
            .header(reqwest::header::USER_AGENT, ua)
            .send()
            .await
            .map_err(|_| FailureKind::Http)?;

        let status = resp.status();
        if !status.is_success() {
            return Err(FailureKind::Http);
        }

        let html = resp.text().await.map_err(|_| FailureKind::Http)?;

        if is_blocked(&html) || looks_rate_limited(&html) {
            return Err(FailureKind::Blocked);
        }

        let results = parse_results_mojeek(&html);
        if results.is_empty() {
            Err(FailureKind::Empty)
        } else {
            Ok(results)
        }
    }
}

/// Shared state for the web search tool, kept across calls within a runtime.
pub struct WebSearchTool {
    /// F1.1: timestamp of the last completed request (for dynamic min-delay).
    last_request: Arc<tokio::sync::Mutex<Option<Instant>>>,
    /// F1.2: concurrency semaphore (permit=1) serializing searches.
    semaphore: Arc<tokio::sync::Semaphore>,
    /// F5.1: rotating index into `USER_AGENTS`.
    ua_index: Arc<AtomicUsize>,
    /// F9: in-memory result cache (query key → results + timestamp).
    cache: Arc<tokio::sync::Mutex<HashMap<String, CacheEntry>>>,
    /// F11.1: providers in preference order.
    providers: Vec<Box<dyn SearchProvider>>,
    /// F11.4: per-provider cooldown (provider → instant until which it is skipped).
    cooldowns: Arc<tokio::sync::Mutex<HashMap<ProviderId, Instant>>>,
}

/// A cached search result set with its insertion timestamp (for TTL).
struct CacheEntry {
    inserted: Instant,
    results: Vec<SearchResult>,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            last_request: Arc::new(tokio::sync::Mutex::new(None)),
            semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
            ua_index: Arc::new(AtomicUsize::new(0)),
            cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            providers: vec![
                Box::new(DuckDuckGoProvider {
                    id: ProviderId::DdgHtml,
                }),
                Box::new(DuckDuckGoProvider {
                    id: ProviderId::DdgLite,
                }),
                Box::new(MojeekProvider),
            ],
            cooldowns: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 8,
                    "description": "Número máximo de resultados (1–20, default 8)"
                },
                "site": {
                    "type": "string",
                    "description": "Restringe a busca a um domínio (ex: \"docs.rs\")"
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

        // F7: parse and clamp max_results (1–20, default 8).
        let max_results = args["max_results"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_RESULTS)
            .clamp(MAX_RESULTS_MIN, MAX_RESULTS_MAX);

        // F8: optional site restriction → "query site:domain".
        let site = args["site"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let full_query = match site {
            Some(s) => format!("{} site:{}", query, s),
            None => query.to_string(),
        };

        // F9: normalized cache key (lowercase + trim) including site/max_results.
        let cache_key = normalize_cache_key(&full_query, max_results);

        // F9: check cache before hitting the network.
        if let Some(results) = self.cache_get(&cache_key).await {
            tracing::debug!(query = %preview(query, 40), "web_search cache hit");
            return Ok(render_results(&full_query, &results, max_results));
        }

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

        // F11: iterate providers in order, with per-provider retry/backoff.
        // Fallback triggers on: HTTP error, block detection, or 0 results.
        let mut last_failure: Option<(ProviderId, FailureKind)> = None;

        for provider in &self.providers {
            if ctx.abort.is_aborted() {
                return Err("aborted".to_string());
            }

            // F11.4: skip providers in cooldown (lazy expiry).
            if self.provider_in_cooldown(provider.id()).await {
                tracing::debug!(
                    provider = provider.id().name(),
                    "web_search skipping provider in cooldown"
                );
                continue;
            }

            let mut attempt = 0usize;
            let outcome: Result<Vec<SearchResult>, FailureKind> = loop {
                if ctx.abort.is_aborted() {
                    return Err("aborted".to_string());
                }

                // F5.1: rotate UA on every attempt (retries use different UAs).
                let ua = self.next_user_agent();

                tracing::debug!(
                    endpoint = provider.id().endpoint(),
                    provider = provider.id().name(),
                    query = %preview(&full_query, 40),
                    attempt,
                    "web_search request"
                );

                let result = provider.search(&client, &full_query, ua).await;

                // F1.1: record the end of this request so the next one waits.
                self.mark_request_done().await;

                if ctx.abort.is_aborted() {
                    return Err("aborted".to_string());
                }

                match result {
                    Ok(results) => {
                        // F9: cache only successful, non-empty results.
                        self.cache_put(&cache_key, results.clone()).await;
                        break Ok(results);
                    }
                    Err(kind) => {
                        tracing::warn!(
                            provider = provider.id().name(),
                            ?kind,
                            attempt,
                            "web_search provider attempt failed"
                        );
                        // F2: retry with backoff (except for hard HTTP errors
                        // on the last attempt — retrying an HTTP 403/5xx
                        // immediately rarely helps, but backoff is cheap).
                        if attempt < RETRY_DELAYS.len() {
                            attempt += 1;
                            self.sleep_retry(ctx, attempt).await?;
                            continue;
                        }
                        break Err(kind);
                    }
                }
            };

            match outcome {
                Ok(results) => {
                    return Ok(render_results(&full_query, &results, max_results));
                }
                Err(kind) => {
                    // F11.4: rate-limit/block failures put the provider in
                    // cooldown so subsequent searches skip it.
                    let blocked = matches!(kind, FailureKind::Blocked);
                    if blocked {
                        self.set_cooldown(provider.id()).await;
                    }
                    tracing::warn!(
                        from = provider.id().name(),
                        ?kind,
                        "web_search falling back to next provider"
                    );
                    last_failure = Some((provider.id(), kind));
                }
            }
        }

        // All providers failed (or returned 0 results).
        let (pid, kind) = last_failure.unwrap_or((ProviderId::DdgHtml, FailureKind::Empty));
        match kind {
            FailureKind::Empty => {
                // F11.3: only report "no results" when every provider was
                // tried and all returned empty.
                Ok(ToolResult::simple(
                    format!("web_search {}", preview(&full_query, 40)),
                    "(no results found)".to_string(),
                ))
            }
            FailureKind::Blocked => Err(format!(
                "Todos os provedores de busca bloquearam a requisição (último: {}, \
possível detecção de bot/CAPTCHA ou rate limiting). Tente novamente em \
alguns instantes ou reformule a consulta.",
                pid.name()
            )),
            FailureKind::Http => Err(format!(
                "Falha na busca web: todos os provedores retornaram erro HTTP \
(último: {}). Verifique a conexão e tente novamente.",
                pid.name()
            )),
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

    /// F9: returns cached results for `key` if present and not expired.
    /// Lazily removes expired entries.
    async fn cache_get(&self, key: &str) -> Option<Vec<SearchResult>> {
        let mut cache = self.cache.lock().await;
        // F9.2: lazy cleanup of expired entries.
        cache.retain(|_, e| e.inserted.elapsed() < CACHE_TTL);
        let entry = cache.get(key)?;
        if entry.inserted.elapsed() >= CACHE_TTL {
            cache.remove(key);
            return None;
        }
        Some(entry.results.clone())
    }

    /// F9: stores results under `key` (only non-empty results are cached by
    /// the caller). Enforces the max entry count.
    async fn cache_put(&self, key: &str, results: Vec<SearchResult>) {
        let mut cache = self.cache.lock().await;
        // F9.2: cap the cache size (drop oldest by insertion order).
        if cache.len() >= CACHE_MAX_ENTRIES && !cache.contains_key(key) {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, e)| e.inserted)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            key.to_string(),
            CacheEntry {
                inserted: Instant::now(),
                results,
            },
        );
    }

    /// F11.4: whether the provider is currently in cooldown (lazy expiry).
    async fn provider_in_cooldown(&self, id: ProviderId) -> bool {
        let mut cooldowns = self.cooldowns.lock().await;
        // Lazy expiry: drop entries whose cooldown has passed.
        cooldowns.retain(|_, until| *until > Instant::now());
        cooldowns.contains_key(&id)
    }

    /// F11.4: puts a provider in cooldown for `COOLDOWN_SECS`.
    async fn set_cooldown(&self, id: ProviderId) {
        let mut cooldowns = self.cooldowns.lock().await;
        let until = Instant::now() + Duration::from_secs(COOLDOWN_SECS);
        tracing::warn!(
            provider = id.name(),
            cooldown_secs = COOLDOWN_SECS,
            "web_search provider placed in cooldown"
        );
        cooldowns.insert(id, until);
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

#[derive(Clone)]
pub(crate) struct SearchResult {
    title: String,
    url: String,
    snippet: String,
    /// F6.1: host/domain extracted from the URL.
    domain: String,
    /// F6.1: publication date when the provider exposes one (e.g. DDG's
    /// `span.result__timestamp`). `None` when unavailable.
    date: Option<String>,
}

/// Extracts results from the DuckDuckGo `/html/` page using CSS selectors.
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
    // F6.1: DDG exposes a publication date in `span.result__timestamp` when
    // the result is dated (news/blog posts); absent for most pages.
    let Ok(date_selector) = Selector::parse("span.result__timestamp") else {
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
            let domain = extract_domain(&url);
            let date = result
                .select(&date_selector)
                .next()
                .map(|d| clean_text(&d.text().collect::<Vec<_>>().join(" ")))
                .filter(|d| !d.is_empty());
            Some(SearchResult {
                title,
                url,
                snippet,
                domain,
                date,
            })
        })
        .take(MAX_RESULTS)
        .collect()
}

/// F4.1: extracts results from the DDG Lite page. The Lite layout is
/// table-based: each result is a `<tr>` containing `a.result-link` (title +
/// redirect href), optionally followed by a `td.result-snippet` row.
fn parse_results_lite(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let Ok(link_selector) = Selector::parse("a.result-link") else {
        return Vec::new();
    };
    let Ok(snippet_selector) = Selector::parse("td.result-snippet") else {
        return Vec::new();
    };

    // Collect snippets in document order; each snippet row follows its
    // result-link row, so we pair them positionally.
    let snippets: Vec<String> = document
        .select(&snippet_selector)
        .map(|s| clean_text(&s.text().collect::<Vec<_>>().join(" ")))
        .collect();

    document
        .select(&link_selector)
        .enumerate()
        .filter_map(|(i, link)| {
            let title = clean_text(&link.text().collect::<Vec<_>>().join(" "));
            let url = clean_url(link.value().attr("href").unwrap_or(""));
            if title.is_empty() || url.is_empty() {
                return None;
            }
            let snippet = snippets.get(i).cloned().unwrap_or_default();
            let domain = extract_domain(&url);
            Some(SearchResult {
                title,
                url,
                snippet,
                domain,
                date: None,
            })
        })
        .take(MAX_RESULTS)
        .collect()
}

/// F11.1: extracts results from the Mojeek HTML page. Results are
/// `ul.results-standard > li` with `a.title` and `p.s` snippets.
fn parse_results_mojeek(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);
    let Ok(result_selector) = Selector::parse("ul.results-standard li") else {
        return Vec::new();
    };
    let Ok(title_selector) = Selector::parse("a.title") else {
        return Vec::new();
    };
    let Ok(snippet_selector) = Selector::parse("p.s") else {
        return Vec::new();
    };

    document
        .select(&result_selector)
        .filter_map(|result| {
            let title_el = result.select(&title_selector).next()?;
            let title = clean_text(&title_el.text().collect::<Vec<_>>().join(" "));
            let url = title_el.value().attr("href").unwrap_or("").to_string();
            if title.is_empty() || url.is_empty() {
                return None;
            }
            let snippet = result
                .select(&snippet_selector)
                .next()
                .map(|s| clean_text(&s.text().collect::<Vec<_>>().join(" ")))
                .unwrap_or_default();
            let domain = extract_domain(&url);
            Some(SearchResult {
                title,
                url,
                snippet,
                domain,
                date: None,
            })
        })
        .take(MAX_RESULTS)
        .collect()
}

/// F6.1: extracts the host/domain from a URL (e.g. "https://docs.rs/tokio"
/// → "docs.rs"). Returns empty string on parse failure.
fn extract_domain(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

/// F9.1: normalizes a cache key (lowercase + trim) including max_results.
fn normalize_cache_key(query: &str, max_results: usize) -> String {
    format!("{}|{}", query.trim().to_lowercase(), max_results)
}

/// Renders results as the model-facing text output, including the domain and
/// (when available) the publication date.
fn render_results(query: &str, results: &[SearchResult], max_results: usize) -> ToolResult {
    let mut body = String::new();
    for (i, r) in results.iter().take(max_results).enumerate() {
        let date = r
            .date
            .as_deref()
            .map(|d| format!("   Data: {}\n", d))
            .unwrap_or_default();
        body.push_str(&format!(
            "{}. **{}**\n   URL: {}\n   Domínio: {}\n{}   {}\n\n",
            i + 1,
            r.title,
            r.url,
            r.domain,
            date,
            r.snippet
        ));
    }
    ToolResult::simple(format!("web_search {}", preview(query, 40)), body)
}

/// Collapses whitespace in text extracted from the DOM.
fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// DuckDuckGo wraps result URLs in a redirect; unwrap the real target.
fn clean_url(href: &str) -> String {
    if let Some((_, rest)) = href.split_once("uddg=") {
        let end = rest.find("&rut=").unwrap_or(rest.len());
        let target = &rest[..end];
        urlencoding::decode(target)
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
        "automated queries",
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
            depth: 0,
            events: crate::harness::event::event_channel().0,
            project_memory: None,
            hooks: Default::default(),
            checkpoints: std::sync::Arc::new(
                crate::harness::tool::checkpoint::FileCheckpoints::new(),
            ),
            jobs: std::sync::Arc::new(crate::harness::tool::jobs::JobRegistry::new()),
            semantic_index: None,
            embedder: None,
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

    /// Real DDG /html/ sample captured from production (10 results).
    const DDG_SAMPLE: &str = include_str!("ddg_sample.html");
    /// Real DDG Lite sample captured from production (10 results).
    const LITE_SAMPLE: &str = include_str!("lite_sample.html");

    #[test]
    fn test_parse_results_with_scraper() {
        let html = r#"
        <div class="result">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Ftokio">Tokio docs</a>
          <a class="result__snippet" href="...">Async runtime <b>for Rust</b></a>
        </div>
        <div class="result">
          <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fcrates.io">crates.io</a>
          <a class="result__snippet" href="...">crates registry</a>
        </div>
        "#;
        let results = parse_results(html);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Tokio docs");
        assert_eq!(results[0].url, "https://docs.rs/tokio");
        assert_eq!(results[0].snippet, "Async runtime for Rust");
        assert_eq!(results[1].url, "https://crates.io");
    }

    // F4.1: parser for the Lite endpoint against a real captured page.
    #[test]
    fn test_parse_results_lite_real_sample() {
        let results = parse_results_lite(LITE_SAMPLE);
        // MAX_RESULTS caps at 8 even though the page has 10 results.
        assert_eq!(
            results.len(),
            8,
            "expected 8 (capped) results from Lite sample"
        );
        assert_eq!(results[0].title, "Tokio - An asynchronous Rust runtime");
        assert_eq!(results[0].url, "https://tokio.rs/");
        assert_eq!(results[0].domain, "tokio.rs");
        assert!(
            results[0].snippet.contains("Tokio"),
            "snippet should be paired with its result"
        );
        assert_eq!(results[1].url, "https://docs.rs/tokio/latest/tokio/");
        assert!(results.iter().all(|r| !r.url.is_empty()));
    }

    #[test]
    fn test_parse_results_ddg_real_sample() {
        let results = parse_results(DDG_SAMPLE);
        // MAX_RESULTS caps at 8 even though the page has 10 results.
        assert_eq!(
            results.len(),
            8,
            "expected 8 (capped) results from /html/ sample"
        );
        assert_eq!(results[0].title, "Tokio - An asynchronous Rust runtime");
        assert_eq!(results[0].url, "https://tokio.rs/");
    }

    #[test]
    fn test_clean_url_unwraps_uddg() {
        assert_eq!(
            clean_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Ftokio"),
            "https://docs.rs/tokio"
        );
        // With the `rut` tracking parameter appended (real Lite format).
        assert_eq!(
            clean_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Ftokio.rs%2F&rut=abc123"),
            "https://tokio.rs/"
        );
        assert_eq!(clean_url("https://plain.example"), "https://plain.example");
    }

    #[test]
    fn test_parse_results_empty_on_no_matches() {
        assert!(parse_results("<html><body>nothing here</body></html>").is_empty());
        assert!(parse_results_lite("<html><body>nothing here</body></html>").is_empty());
        assert!(parse_results_mojeek("<html><body>nothing here</body></html>").is_empty());
    }

    #[test]
    fn test_is_blocked_detects_captcha() {
        assert!(is_blocked("<html>Anomaly-Detected! please verify</html>"));
        assert!(is_blocked(
            "<html>please solve the Captcha to continue</html>"
        ));
        assert!(is_blocked("<html>bot-detected</html>"));
    }

    #[test]
    fn test_is_blocked_false_for_normal() {
        assert!(!is_blocked(DDG_SAMPLE));
        assert!(!is_blocked(LITE_SAMPLE));
    }

    #[test]
    fn test_looks_rate_limited_detects_terms() {
        assert!(looks_rate_limited("<html>Too Many Requests</html>"));
        assert!(looks_rate_limited("<html>unusual traffic detected</html>"));
        assert!(looks_rate_limited("<html>network anomaly</html>"));
        assert!(looks_rate_limited("<html>rate limit exceeded</html>"));
        // Mojeek block page wording.
        assert!(looks_rate_limited(
            "<html>sending automated queries so we can't process</html>"
        ));
    }

    #[test]
    fn test_looks_rate_limited_false_for_normal() {
        assert!(!looks_rate_limited(DDG_SAMPLE));
        assert!(!looks_rate_limited(LITE_SAMPLE));
        assert!(!looks_rate_limited("<html>normal search results</html>"));
    }

    #[test]
    fn test_user_agent_rotation() {
        let tool = WebSearchTool::new();
        let a = tool.next_user_agent();
        let b = tool.next_user_agent();
        let c = tool.next_user_agent();
        assert_ne!(a, b);
        assert_ne!(b, c);
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

    #[test]
    fn test_provider_order_and_cooldown_constant() {
        // F11.2: preference order is DDG HTML → DDG Lite → Mojeek.
        assert_eq!(
            ProviderId::all(),
            [ProviderId::DdgHtml, ProviderId::DdgLite, ProviderId::Mojeek]
        );
        assert_eq!(COOLDOWN_SECS, 120);
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
        let tool = WebSearchTool::new();
        let permit = tool.semaphore.try_acquire().unwrap();
        assert!(tool.semaphore.try_acquire().is_err());
        drop(permit);
        assert!(tool.semaphore.try_acquire().is_ok());
    }

    #[tokio::test]
    async fn test_cooldown_skips_provider() {
        // F11.4: a provider in cooldown is skipped by provider_in_cooldown.
        let tool = WebSearchTool::new();
        assert!(!tool.provider_in_cooldown(ProviderId::DdgHtml).await);
        tool.set_cooldown(ProviderId::DdgHtml).await;
        assert!(tool.provider_in_cooldown(ProviderId::DdgHtml).await);
        // Other providers are unaffected.
        assert!(!tool.provider_in_cooldown(ProviderId::DdgLite).await);
    }

    #[tokio::test]
    async fn test_cooldown_expires() {
        // F11.4: cooldown expires (lazy) — simulate by inserting a past
        // expiry directly.
        let tool = WebSearchTool::new();
        {
            let mut cooldowns = tool.cooldowns.lock().await;
            cooldowns.insert(ProviderId::Mojeek, Instant::now() - Duration::from_secs(1));
        }
        // Lazy expiry removes the stale entry and reports not-in-cooldown.
        assert!(!tool.provider_in_cooldown(ProviderId::Mojeek).await);
        let cooldowns = tool.cooldowns.lock().await;
        assert!(!cooldowns.contains_key(&ProviderId::Mojeek));
    }

    #[test]
    fn test_extract_domain() {
        assert_eq!(extract_domain("https://docs.rs/tokio"), "docs.rs");
        assert_eq!(extract_domain("https://crates.io"), "crates.io");
        assert_eq!(
            extract_domain("https://www.example.com/path?q=1"),
            "www.example.com"
        );
        assert_eq!(extract_domain("not a url"), "");
    }

    #[test]
    fn test_render_results_includes_domain() {
        let results = vec![SearchResult {
            title: "Tokio".into(),
            url: "https://docs.rs/tokio".into(),
            snippet: "Async runtime".into(),
            domain: "docs.rs".into(),
            date: None,
        }];
        let out = render_results("tokio", &results, 8);
        assert!(out.output.contains("docs.rs"));
        assert!(out.output.contains("Tokio"));
        assert!(out.output.contains("Domínio"));
        // No date → no "Data:" line.
        assert!(!out.output.contains("Data:"));
    }

    #[test]
    fn test_render_results_includes_date_when_present() {
        let results = vec![SearchResult {
            title: "Post".into(),
            url: "https://example.com/post".into(),
            snippet: "s".into(),
            domain: "example.com".into(),
            date: Some("2 de jan. de 2026".into()),
        }];
        let out = render_results("q", &results, 8);
        assert!(out.output.contains("Data: 2 de jan. de 2026"));
    }

    #[test]
    fn test_render_results_respects_max_results() {
        let results: Vec<SearchResult> = (0..5)
            .map(|i| SearchResult {
                title: format!("T{}", i),
                url: format!("https://example.com/{}", i),
                snippet: "s".into(),
                domain: "example.com".into(),
                date: None,
            })
            .collect();
        let out = render_results("q", &results, 3);
        assert!(out.output.contains("1. **T0**"));
        assert!(out.output.contains("3. **T2**"));
        assert!(!out.output.contains("4. **T3**"));
    }

    #[test]
    fn test_normalize_cache_key() {
        assert_eq!(normalize_cache_key("  Rust Tokio  ", 8), "rust tokio|8");
        assert_eq!(normalize_cache_key("Rust Tokio", 8), "rust tokio|8");
        assert_ne!(
            normalize_cache_key("rust", 3),
            normalize_cache_key("rust", 8)
        );
    }

    #[tokio::test]
    async fn test_cache_put_get() {
        let tool = WebSearchTool::new();
        let results = vec![SearchResult {
            title: "T".into(),
            url: "https://example.com".into(),
            snippet: "s".into(),
            domain: "example.com".into(),
            date: None,
        }];
        tool.cache_put("key", results.clone()).await;
        let got = tool.cache_get("key").await;
        assert!(got.is_some());
        assert_eq!(got.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_cache_miss_returns_none() {
        let tool = WebSearchTool::new();
        assert!(tool.cache_get("missing").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_evicts_oldest_when_full() {
        let tool = WebSearchTool::new();
        for i in 0..(CACHE_MAX_ENTRIES + 5) {
            tool.cache_put(&format!("k{}", i), vec![]).await;
        }
        let cache = tool.cache.lock().await;
        assert!(cache.len() <= CACHE_MAX_ENTRIES);
    }
}
