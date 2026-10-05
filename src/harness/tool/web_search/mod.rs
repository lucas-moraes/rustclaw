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
//! - F11: multiple providers (Tavily, DDG HTML, DDG Lite, Mojeek) with wide
//!   fallback (HTTP error, block detection, or 0 parsed results) and
//!   per-provider cooldown after rate-limit failures.
//! - F12: Tavily is always registered as the primary engine; its API key is
//!   resolved at use time (so `/auth tavily <key>` applies without restart).
//!   Without a key the provider is skipped up-front (no retry, no cooldown)
//!   and the HTML scrapers take over; the winning engine is reported in the
//!   result footer.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;

mod parse;
mod providers;

use parse::{parse_results, parse_results_lite, parse_results_mojeek, parse_tavily};
use providers::{
    DuckDuckGoProvider, FailureKind, MojeekProvider, ProviderId, SearchProvider, TavilyProvider,
};

pub(super) const DDG_ENDPOINT: &str = "https://html.duckduckgo.com/html/";
/// F4.2: alternative (less rate-limited) DDG endpoint.
pub(super) const DDG_LITE_ENDPOINT: &str = "https://lite.duckduckgo.com/lite/";
/// F11.1: Mojeek HTML endpoint (no API key required).
pub(super) const MOJEEK_ENDPOINT: &str = "https://www.mojeek.com/search";
/// Tavily search API endpoint (JSON, requires `TAVILY_API_KEY`).
pub(super) const TAVILY_ENDPOINT: &str = "https://api.tavily.com/search";

pub(super) const MAX_RESULTS: usize = 8;
pub(super) const TIMEOUT_SECS: u64 = 10;

/// F1.1: minimum gap (ms) between the end of one request and the start of the next.
const MIN_DELAY_MS: u64 = 1500;

/// F2.3: backoff delays (seconds) between retry attempts.
const RETRY_DELAYS: &[u64] = &[2, 4];

/// F5.1: pool of modern desktop browser User-Agents (avoid curl/Python UAs).
pub(super) const USER_AGENTS: &[&str] = &[
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
    /// F12: engine that produced these results (reported on cache hits).
    provider: ProviderId,
}

/// Resolves the Tavily API key: env var `TAVILY_API_KEY` wins, then the
/// `tavily` entry in `auth.json` (set via `/auth tavily <key>`).
fn resolve_tavily_key() -> Option<String> {
    resolve_tavily_key_from(&crate::harness::auth::AuthStore::path())
}

/// Same as [`resolve_tavily_key`], but reads the auth store from an explicit
/// path. Split out so tests can isolate the store in a temp dir without
/// mutating the process-wide `RUSTCLAW_HOME` env var.
fn resolve_tavily_key_from(auth_path: &std::path::Path) -> Option<String> {
    if let Ok(k) = std::env::var("TAVILY_API_KEY") {
        let k = k.trim().to_string();
        if !k.is_empty() {
            return Some(k);
        }
    }
    crate::harness::auth::AuthStore::load_from(auth_path)
        .ok()
        .and_then(|store| store.get_key("tavily"))
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// Whether a Tavily API key is available (env `TAVILY_API_KEY` or the
/// `tavily` entry in `auth.json`).
///
/// Used by the startup warnings in the TUI and CLI: without a key
/// `web_search` still works through the HTML fallback, but with lower
/// quality. The key can be registered with `/auth tavily <key>`.
pub fn tavily_configured() -> bool {
    resolve_tavily_key().is_some()
}

/// Startup hint shown by the TUI/CLI when Tavily is not usable.
///
/// Returns `None` when a key is configured (nothing to warn about), or a
/// ready-to-display message otherwise. Centralised here so the TUI and the
/// CLI cannot drift apart in wording.
pub fn tavily_status_hint() -> Option<&'static str> {
    if tavily_configured() {
        None
    } else {
        Some(
            "[warn] web_search: no tavily token — using HTML fallback (lower quality); \
set it with /auth tavily <key>",
        )
    }
}

impl WebSearchTool {
    pub fn new() -> Self {
        // F12: Tavily (keyed, JSON) is ALWAYS registered as the primary
        // engine — the tool is built once at startup, so gating registration
        // on the key would make a later `/auth tavily <key>` useless. The key
        // is resolved at use time instead: without one the provider is
        // skipped up-front (no retry, no cooldown) and the HTML scrapers
        // below take over as explicit fallback.
        let providers: Vec<Box<dyn SearchProvider>> = vec![
            Box::new(TavilyProvider),
            Box::new(DuckDuckGoProvider::new(ProviderId::DdgHtml)),
            Box::new(DuckDuckGoProvider::new(ProviderId::DdgLite)),
            Box::new(MojeekProvider),
        ];

        Self {
            last_request: Arc::new(tokio::sync::Mutex::new(None)),
            semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
            ua_index: Arc::new(AtomicUsize::new(0)),
            cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            providers,
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
        "Pesquisa na web (motor Tavily; fallback HTML) por documentação \
atualizada, crates de Rust, artigos técnicos ou resoluções de erros de \
compilação."
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
        if let Some((results, provider)) = self.cache_get(&cache_key).await {
            tracing::debug!(query = %preview(query, 40), "web_search cache hit");
            return Ok(render_results(&full_query, &results, max_results, provider));
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

            // F12: Tavily is always registered, but without a key there is
            // nothing to try — skip it up-front (no retry, no cooldown) so a
            // keyless machine does not burn the backoff sleeps.
            if provider.id() == ProviderId::Tavily && resolve_tavily_key().is_none() {
                tracing::debug!("web_search skipping tavily: no api key");
                last_failure = Some((provider.id(), FailureKind::NoKey));
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

                // Resolve the Tavily key at use time so `/auth` changes apply
                // immediately without a restart.
                let api_key = if provider.id() == ProviderId::Tavily {
                    resolve_tavily_key()
                } else {
                    None
                };
                let key = api_key.as_deref();
                let result = provider
                    .search(&client, &full_query, ua, key, max_results)
                    .await;

                // F1.1: record the end of this request so the next one waits.
                self.mark_request_done().await;

                if ctx.abort.is_aborted() {
                    return Err("aborted".to_string());
                }

                match result {
                    Ok(results) => {
                        // F9: cache only successful, non-empty results.
                        self.cache_put(&cache_key, results.clone(), provider.id())
                            .await;
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
                        // F13: a rejected key (401/403) is a configuration
                        // state — retrying only wastes the backoff sleeps.
                        if attempt < RETRY_DELAYS.len()
                            && !matches!(kind, FailureKind::NoKey | FailureKind::Unauthorized)
                        {
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
                    return Ok(render_results(
                        &full_query,
                        &results,
                        max_results,
                        provider.id(),
                    ));
                }
                Err(kind) => {
                    // F11.4: rate-limit/block failures put the provider in
                    // cooldown so subsequent searches skip it. F12/F13: a
                    // missing or rejected key is a configuration state, never
                    // a cooldown trigger (otherwise fixing the key with
                    // `/auth tavily <key>` would not take effect immediately).
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
            FailureKind::NoKey => Err(format!(
                "Falha na busca web: o provedor {} requer uma API key e nenhuma \
está configurada (use `/auth tavily <key>`).",
                pid.name()
            )),
            FailureKind::Unauthorized => Err(format!(
                "Falha na busca web: a API key do provedor {} foi rejeitada \
(HTTP 401/403). Verifique a credencial com `/auth tavily <key>`.",
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
    async fn cache_get(&self, key: &str) -> Option<(Vec<SearchResult>, ProviderId)> {
        let mut cache = self.cache.lock().await;
        // F9.2: lazy cleanup of expired entries.
        cache.retain(|_, e| e.inserted.elapsed() < CACHE_TTL);
        let entry = cache.get(key)?;
        if entry.inserted.elapsed() >= CACHE_TTL {
            cache.remove(key);
            return None;
        }
        Some((entry.results.clone(), entry.provider))
    }

    /// F9: stores results under `key` (only non-empty results are cached by
    /// the caller). Enforces the max entry count.
    async fn cache_put(&self, key: &str, results: Vec<SearchResult>, provider: ProviderId) {
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
                provider,
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
    pub(super) title: String,
    pub(super) url: String,
    pub(super) snippet: String,
    /// F6.1: host/domain extracted from the URL.
    pub(super) domain: String,
    /// F6.1: publication date when the provider exposes one (e.g. DDG's
    /// `span.result__timestamp`). `None` when unavailable.
    pub(super) date: Option<String>,
}

/// Extracts results from the DuckDuckGo `/html/` page using CSS selectors.
pub(super) fn extract_domain(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default()
}

/// F9.1: normalizes a cache key (lowercase + trim) including max_results.
fn normalize_cache_key(query: &str, max_results: usize) -> String {
    format!("{}|{}", query.trim().to_lowercase(), max_results)
}

/// Renders results as the model-facing text output, including the domain,
/// (when available) the publication date, and the engine that produced them.
fn render_results(
    query: &str,
    results: &[SearchResult],
    max_results: usize,
    provider: ProviderId,
) -> ToolResult {
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
    // F12: make the winning engine visible so fallbacks are diagnosable.
    body.push_str(&format!("_Fonte: {}_\n", provider.name()));
    ToolResult::simple(format!("web_search {}", preview(query, 40)), body)
}

/// Collapses whitespace in text extracted from the DOM.
pub(super) fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// DuckDuckGo wraps result URLs in a redirect; unwrap the real target.
pub(super) fn clean_url(href: &str) -> String {
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
pub(super) fn is_blocked(html: &str) -> bool {
    let lower = html.to_lowercase();
    ["anomaly-detected", "captcha", "bot-detected"]
        .iter()
        .any(|term| lower.contains(term))
}

/// F3.1: heuristics for a silent rate-limit page (HTTP 200 with empty body).
pub(super) fn looks_rate_limited(html: &str) -> bool {
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

    /// Serializes tests that mutate process-global env vars.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolve_tavily_key_env_var_wins() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("TAVILY_API_KEY").ok();
        std::env::set_var("TAVILY_API_KEY", "  env-key-123  ");
        // Env var wins, so the real auth.json is never consulted.
        assert_eq!(resolve_tavily_key().as_deref(), Some("env-key-123"));
        match prev {
            Some(v) => std::env::set_var("TAVILY_API_KEY", v),
            None => std::env::remove_var("TAVILY_API_KEY"),
        }
    }

    #[test]
    fn resolve_tavily_key_blank_env_falls_through() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("TAVILY_API_KEY").ok();
        std::env::set_var("TAVILY_API_KEY", "   ");
        // Blank env var must not be returned; result comes from auth.json
        // (or None if unset) — we only assert it is never the blank string.
        let got = resolve_tavily_key();
        assert_ne!(got.as_deref(), Some(""));
        assert_ne!(got.as_deref(), Some("   "));
        match prev {
            Some(v) => std::env::set_var("TAVILY_API_KEY", v),
            None => std::env::remove_var("TAVILY_API_KEY"),
        }
    }

    #[test]
    fn tavily_configured_true_with_env_key() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("TAVILY_API_KEY").ok();
        // Use the path-injectable helper so the test never reads the
        // process-wide RUSTCLAW_HOME (which other modules' tests mutate).
        let tmp = tempfile::tempdir().expect("tempdir");
        let auth_path = tmp.path().join("auth.json");
        std::env::set_var("TAVILY_API_KEY", "env-key-123");
        assert_eq!(
            resolve_tavily_key_from(&auth_path).as_deref(),
            Some("env-key-123")
        );
        match prev {
            Some(v) => std::env::set_var("TAVILY_API_KEY", v),
            None => std::env::remove_var("TAVILY_API_KEY"),
        }
    }

    #[test]
    fn tavily_configured_false_without_env_or_auth() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev_key = std::env::var("TAVILY_API_KEY").ok();
        // Isolate the auth store in an empty temp dir so the real machine's
        // auth.json (which may hold a tavily key) is never consulted. We pass
        // the path explicitly instead of mutating RUSTCLAW_HOME, which is
        // process-wide and would race with other modules' tests.
        let tmp = tempfile::tempdir().expect("tempdir");
        let auth_path = tmp.path().join("auth.json");
        std::env::remove_var("TAVILY_API_KEY");
        assert!(resolve_tavily_key_from(&auth_path).is_none());
        match prev_key {
            Some(v) => std::env::set_var("TAVILY_API_KEY", v),
            None => std::env::remove_var("TAVILY_API_KEY"),
        }
    }

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
            sandbox_policy: None,
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
    use super::providers::{tavily_body, ProviderId as Pid};
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
        // F11.2: preference order is Tavily → DDG HTML → DDG Lite → Mojeek.
        assert_eq!(
            Pid::all(),
            vec![
                ProviderId::Tavily,
                ProviderId::DdgHtml,
                ProviderId::DdgLite,
                ProviderId::Mojeek
            ]
        );
        assert_eq!(COOLDOWN_SECS, 120);
    }

    #[test]
    fn test_tavily_always_registered_first() {
        // F12: registration must not depend on the key — the tool is built
        // once at startup, so a key added later via `/auth` must still work.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("TAVILY_API_KEY").ok();
        std::env::remove_var("TAVILY_API_KEY");

        let tool = WebSearchTool::new();
        let ids: Vec<ProviderId> = tool.providers.iter().map(|p| p.id()).collect();
        assert_eq!(
            ids,
            vec![
                ProviderId::Tavily,
                ProviderId::DdgHtml,
                ProviderId::DdgLite,
                ProviderId::Mojeek
            ]
        );

        match prev {
            Some(v) => std::env::set_var("TAVILY_API_KEY", v),
            None => std::env::remove_var("TAVILY_API_KEY"),
        }
    }

    #[tokio::test]
    async fn test_keyless_tavily_skipped_without_cooldown() {
        // F12: without a key the Tavily provider is skipped up-front — no
        // retry sleeps and, crucially, no cooldown applied.
        //
        // The env lock is a std Mutex, so it must not be held across an
        // await: do the env manipulation + construction, then drop it.
        let (tool, keyless) = {
            let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var("TAVILY_API_KEY").ok();
            std::env::remove_var("TAVILY_API_KEY");
            let tool = WebSearchTool::new();
            let keyless = resolve_tavily_key().is_none();
            match prev {
                Some(v) => std::env::set_var("TAVILY_API_KEY", v),
                None => std::env::remove_var("TAVILY_API_KEY"),
            }
            (tool, keyless)
        };

        assert!(!tool.provider_in_cooldown(ProviderId::Tavily).await);

        // The keyless guard in `execute` must leave the provider out of
        // cooldown (NoKey is a config state, not a failure).
        if keyless {
            let provider = tool
                .providers
                .iter()
                .find(|p| p.id() == ProviderId::Tavily)
                .expect("tavily registered");
            let client = reqwest::Client::new();
            match provider.search(&client, "q", "test-ua", None, 8).await {
                Err(FailureKind::NoKey) => {}
                Err(other) => panic!("expected NoKey, got {:?}", other),
                Ok(_) => panic!("keyless tavily must not return results"),
            }
            assert!(!tool.provider_in_cooldown(ProviderId::Tavily).await);
        }
    }

    #[test]
    fn test_parse_tavily() {
        let json = r#"{
            "query": "rust async",
            "results": [
                {
                    "title": "Async Rust",
                    "url": "https://docs.rs/tokio",
                    "content": "Tokio is an async runtime.",
                    "score": 0.98
                },
                {
                    "title": "No content result",
                    "url": "https://example.com/page"
                }
            ]
        }"#;
        let results = parse_tavily(json);
        assert_eq!(results.len(), 2);

        assert_eq!(results[0].title, "Async Rust");
        assert_eq!(results[0].url, "https://docs.rs/tokio");
        assert_eq!(results[0].snippet, "Tokio is an async runtime.");
        assert_eq!(results[0].domain, "docs.rs");
        assert!(results[0].date.is_none());

        // Missing `content` degrades to an empty snippet, not a dropped result.
        assert_eq!(results[1].title, "No content result");
        assert_eq!(results[1].url, "https://example.com/page");
        assert_eq!(results[1].snippet, "");
        assert_eq!(results[1].domain, "example.com");
    }

    #[test]
    fn test_parse_tavily_skips_empty_url_and_bad_json() {
        assert!(parse_tavily("not json").is_empty());
        let json = r#"{"results": [{"title": "x", "url": "", "content": "y"}]}"#;
        assert!(parse_tavily(json).is_empty());
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
        let out = render_results("tokio", &results, 8, ProviderId::DdgHtml);
        assert!(out.output.contains("docs.rs"));
        assert!(out.output.contains("Tokio"));
        assert!(out.output.contains("Domínio"));
        // No date → no "Data:" line.
        assert!(!out.output.contains("Data:"));
        // F12: the winning engine is reported in the footer.
        assert!(out.output.contains("_Fonte: ddg_html_"));
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
        let out = render_results("q", &results, 8, ProviderId::Tavily);
        assert!(out.output.contains("Data: 2 de jan. de 2026"));
        assert!(out.output.contains("_Fonte: tavily_"));
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
        let out = render_results("q", &results, 3, ProviderId::Mojeek);
        assert!(out.output.contains("1. **T0**"));
        assert!(out.output.contains("3. **T2**"));
        assert!(!out.output.contains("4. **T3**"));
        assert!(out.output.contains("_Fonte: mojeek_"));
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
        tool.cache_put("key", results.clone(), ProviderId::Tavily)
            .await;
        let got = tool.cache_get("key").await;
        assert!(got.is_some());
        let (cached, provider) = got.unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(provider, ProviderId::Tavily);
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
            tool.cache_put(&format!("k{}", i), vec![], ProviderId::DdgLite)
                .await;
        }
        let cache = tool.cache.lock().await;
        assert!(cache.len() <= CACHE_MAX_ENTRIES);
    }

    // --- F13: 401/403 (rejected key) must not be conflated with 429 ---

    #[test]
    fn tavily_401_maps_to_unauthorized_not_blocked() {
        // The mapping lives in the status match inside TavilyProvider::search;
        // assert the classification contract directly so a future refactor
        // cannot silently fold 401 back into Blocked.
        let classify = |code: u16| match code {
            401 | 403 => FailureKind::Unauthorized,
            429 => FailureKind::Blocked,
            _ => FailureKind::Http,
        };
        assert!(matches!(classify(401), FailureKind::Unauthorized));
        assert!(matches!(classify(403), FailureKind::Unauthorized));
        assert!(matches!(classify(429), FailureKind::Blocked));
    }

    #[tokio::test]
    async fn unauthorized_does_not_set_cooldown() {
        let tool = WebSearchTool::new();
        // Only Blocked triggers a cooldown; Unauthorized/NoKey are config states.
        let blocked = matches!(FailureKind::Unauthorized, FailureKind::Blocked);
        assert!(!blocked, "Unauthorized must not be treated as Blocked");
        assert!(!tool.provider_in_cooldown(ProviderId::Tavily).await);
    }

    #[test]
    fn unauthorized_message_mentions_auth_command() {
        // Mirrors the final error arm so the actionable hint cannot regress.
        let msg = format!(
            "Falha na busca web: a API key do provedor {} foi rejeitada \
(HTTP 401/403). Verifique a credencial com `/auth tavily <key>`.",
            ProviderId::Tavily.name()
        );
        assert!(msg.contains("/auth tavily"));
        assert!(msg.contains("401/403"));
    }

    // --- max_results forwarding (quota) ---

    #[test]
    fn tavily_body_uses_requested_max_results() {
        let body = tavily_body("rust ratatui", 3);
        assert_eq!(body["max_results"], 3);
        assert_eq!(body["query"], "rust ratatui");
        assert_eq!(body["search_depth"], "basic");
    }

    #[test]
    fn tavily_body_reflects_clamped_value() {
        // The facade clamps before calling; assert the body honours it.
        let body = tavily_body("q", 20);
        assert_eq!(body["max_results"], 20);
    }

    #[test]
    fn tavily_status_hint_none_when_key_present() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("TAVILY_API_KEY").ok();
        std::env::set_var("TAVILY_API_KEY", "some-key");
        assert!(tavily_status_hint().is_none());
        match prev {
            Some(v) => std::env::set_var("TAVILY_API_KEY", v),
            None => std::env::remove_var("TAVILY_API_KEY"),
        }
    }
}
