use super::{
    is_blocked, looks_rate_limited, parse_results, parse_results_lite, parse_results_mojeek,
    parse_tavily, SearchResult, DDG_ENDPOINT, DDG_LITE_ENDPOINT, MOJEEK_ENDPOINT, TAVILY_ENDPOINT,
};

/// F11.1: identifier of a search provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderId {
    Tavily,
    DdgHtml,
    DdgLite,
    Mojeek,
}

impl ProviderId {
    pub(super) fn name(self) -> &'static str {
        match self {
            ProviderId::Tavily => "tavily",
            ProviderId::DdgHtml => "ddg_html",
            ProviderId::DdgLite => "ddg_lite",
            ProviderId::Mojeek => "mojeek",
        }
    }

    pub(super) fn endpoint(self) -> &'static str {
        match self {
            ProviderId::Tavily => TAVILY_ENDPOINT,
            ProviderId::DdgHtml => DDG_ENDPOINT,
            ProviderId::DdgLite => DDG_LITE_ENDPOINT,
            ProviderId::Mojeek => MOJEEK_ENDPOINT,
        }
    }

    /// F11.2: default order of preference (Tavily first when a key is set).
    #[cfg(test)]
    pub(super) fn all() -> Vec<ProviderId> {
        vec![
            ProviderId::Tavily,
            ProviderId::DdgHtml,
            ProviderId::DdgLite,
            ProviderId::Mojeek,
        ]
    }
}

/// F11.1: why a provider attempt failed (drives fallback + cooldown).
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FailureKind {
    /// HTTP error (non-2xx) or network/timeout failure.
    Http,
    /// Bot detection / CAPTCHA / silent rate-limit page.
    Blocked,
    /// Page fetched fine but the parser extracted 0 results.
    Empty,
    /// F12: the provider needs an API key and none is configured. Never
    /// triggers a cooldown (it is a configuration state, not a failure).
    NoKey,
    /// F13: the provider rejected the configured API key (HTTP 401/403).
    /// Like `NoKey`, this is a configuration state rather than a transient
    /// failure: it must not trigger a cooldown nor a retry, otherwise fixing
    /// the key with `/auth tavily <key>` would not take effect immediately.
    Unauthorized,
}

/// F11.1: a single search provider (HTML scraping, no API key).
#[async_trait::async_trait]
pub(super) trait SearchProvider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Fetches and parses results for `query`. Returns the parsed results or
    /// the failure kind (which drives fallback/cooldown in the facade).
    ///
    /// `api_key` is only meaningful for keyed providers (e.g. Tavily); the
    /// HTML-scraping providers ignore it.
    ///
    /// `max_results` is a hint forwarded to providers that support it (Tavily
    /// bills per result, so requesting only what the caller needs saves
    /// quota). Scraping providers ignore it and rely on the facade's render
    /// truncation instead.
    async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        ua: &str,
        api_key: Option<&str>,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, FailureKind>;
}

/// F11.1: DuckDuckGo provider — shared fetch logic, two parsers (HTML + Lite).
pub(super) struct DuckDuckGoProvider {
    id: ProviderId,
}

impl DuckDuckGoProvider {
    pub(super) fn new(id: ProviderId) -> Self {
        Self { id }
    }
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
        _api_key: Option<&str>,
        _max_results: usize,
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
pub(super) struct MojeekProvider;

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
        _api_key: Option<&str>,
        _max_results: usize,
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

/// Tavily API provider (JSON, requires `TAVILY_API_KEY`).
pub(super) struct TavilyProvider;

/// Builds the JSON body sent to the Tavily `/search` endpoint.
///
/// Extracted as a pure function so the request shape (notably `max_results`)
/// can be unit-tested without hitting the network. Tavily bills per returned
/// result, so the caller's `max_results` is forwarded instead of a fixed 10.
pub(super) fn tavily_body(query: &str, max_results: usize) -> serde_json::Value {
    serde_json::json!({
        "query": query,
        "max_results": max_results,
        "search_depth": "basic"
    })
}

#[async_trait::async_trait]
impl SearchProvider for TavilyProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Tavily
    }

    async fn search(
        &self,
        client: &reqwest::Client,
        query: &str,
        _ua: &str,
        api_key: Option<&str>,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, FailureKind> {
        // F12: the facade short-circuits keyless Tavily before calling here;
        // this guard is a safety net and must not be treated as a real failure.
        let key = api_key.ok_or(FailureKind::NoKey)?;
        let body = tavily_body(query, max_results);
        let resp = client
            .post(TAVILY_ENDPOINT)
            .header("Authorization", format!("Bearer {}", key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|_| FailureKind::Http)?;
        let status = resp.status();
        // F13: 401/403 mean the key is invalid/expired (a configuration
        // state), while 429 is a transient rate-limit. They must not be
        // conflated: only the latter justifies a cooldown.
        match status.as_u16() {
            401 | 403 => return Err(FailureKind::Unauthorized),
            429 => return Err(FailureKind::Blocked),
            _ => {}
        }
        if !status.is_success() {
            return Err(FailureKind::Http);
        }
        let text = resp.text().await.map_err(|_| FailureKind::Http)?;
        let results = parse_tavily(&text);
        if results.is_empty() {
            Err(FailureKind::Empty)
        } else {
            Ok(results)
        }
    }
}
