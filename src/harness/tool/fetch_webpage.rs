//! `fetch_webpage` tool: download a URL and convert HTML -> Markdown.

use super::{Tool, ToolResult};
use crate::harness::session::preview;
use crate::harness::tool::context::ToolContext;
use crate::harness::tool::truncate::truncate_output;
use serde_json::{json, Value};

const MAX_CONTENT_CHARS: usize = 15_000;
const TIMEOUT_SECS: u64 = 10;
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/120.0 Safari/537.36";

pub struct FetchWebpageTool;

#[async_trait::async_trait]
impl Tool for FetchWebpageTool {
    fn name(&self) -> &str {
        "fetch_webpage"
    }

    fn description(&self) -> &str {
        "Faz o download do conteúdo de uma página web ou documentação técnica \
(ex: docs.rs) e o converte para Markdown limpo para o contexto."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "A URL de destino (ex: https://docs.rs/tokio/latest/tokio/macro.select.html)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, String> {
        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }
        let url = args["url"]
            .as_str()
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .ok_or_else(|| "missing required argument: url".to_string())?;

        let parsed =
            reqwest::Url::parse(url).map_err(|e| format!("invalid URL `{}`: {}", url, e))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(format!("unsupported URL scheme `{}`", parsed.scheme()));
        }
        // SSRF guard: reject hosts that resolve to private/loopback/link-local
        // addresses (cloud metadata, internal services, localhost admin panels).
        validate_public_host(&parsed).await?;

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
            // No automatic redirects: a redirect to an internal host would
            // bypass the initial host check above.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("failed to build HTTP client: {}", e))?;

        let resp = client
            .get(parsed.clone())
            .send()
            .await
            .map_err(|e| format!("request failed for {}: {}", url, e))?;

        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }

        if !resp.status().is_success() {
            return Err(format!("HTTP {} for {}", resp.status(), url));
        }
        let html = resp
            .text()
            .await
            .map_err(|e| format!("failed to read response body: {}", e))?;

        if ctx.abort.is_aborted() {
            return Err("aborted".to_string());
        }

        let markdown = convert_html(&html).map_err(|e| format!("HTML->Markdown failed: {}", e))?;
        let output = truncate_output(&markdown, MAX_CONTENT_CHARS);

        Ok(ToolResult::simple(
            format!("fetch_webpage {}", preview(url, 40)),
            output,
        ))
    }
}

fn convert_html(html: &str) -> Result<String, String> {
    htmd::HtmlToMarkdown::new()
        .convert(html)
        .map_err(|e| format!("conversion error: {}", e))
}

/// Rejects hosts that resolve to private, loopback, link-local or unspecified
/// addresses (SSRF guard). Also rejects obvious local hostnames.
async fn validate_public_host(url: &reqwest::Url) -> Result<(), String> {
    let host = url
        .host_str()
        .ok_or_else(|| format!("URL has no host: {}", url))?;

    // Fast-path hostname checks (no DNS needed).
    let lower = host.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
    {
        return Err(format!("blocked: host `{host}` is not a public address"));
    }

    // If the host is already an IP literal, check it directly.
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return if is_private_ip(ip) {
            Err(format!("blocked: `{host}` is a private/loopback address"))
        } else {
            Ok(())
        };
    }

    // Resolve the hostname and reject if ANY resolved address is private.
    let port = url
        .port()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("failed to resolve `{host}`: {}", e))?;
    for addr in addrs {
        if is_private_ip(addr.ip()) {
            return Err(format!(
                "blocked: `{host}` resolves to private/loopback address {}",
                addr.ip()
            ));
        }
    }
    Ok(())
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
        }
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
    fn test_convert_html_to_markdown() {
        let md = convert_html("<h1>Hello</h1><p>World</p>").unwrap();
        assert!(md.contains("Hello"));
        assert!(md.contains("World"));
    }

    #[tokio::test]
    async fn test_invalid_url_errors() {
        let tool = FetchWebpageTool;
        let err = tool
            .execute(json!({"url": "not a url"}), &test_ctx())
            .await
            .unwrap_err();
        assert!(err.contains("invalid URL"));
    }

    #[tokio::test]
    async fn test_unsupported_scheme_errors() {
        let tool = FetchWebpageTool;
        let err = tool
            .execute(json!({"url": "file:///etc/passwd"}), &test_ctx())
            .await
            .unwrap_err();
        assert!(err.contains("unsupported URL scheme"));
    }

    #[tokio::test]
    async fn test_missing_url_errors() {
        let tool = FetchWebpageTool;
        let err = tool.execute(json!({}), &test_ctx()).await.unwrap_err();
        assert!(err.contains("url"));
    }

    #[test]
    fn test_truncation_at_limit() {
        let big = "x".repeat(20_000);
        let out = truncate_output(&big, MAX_CONTENT_CHARS);
        assert!(out.contains("[output truncated"));
        assert!(out.len() < MAX_CONTENT_CHARS + 100);
    }

    #[test]
    fn test_is_private_ip() {
        use std::net::IpAddr;
        // Private / loopback / link-local / unspecified → blocked.
        for ip in [
            "127.0.0.1",
            "127.8.8.8",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(is_private_ip(ip.parse::<IpAddr>().unwrap()), "{}", ip);
        }
        // Public addresses → allowed.
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700::1111"] {
            assert!(!is_private_ip(ip.parse::<IpAddr>().unwrap()), "{}", ip);
        }
    }

    #[tokio::test]
    async fn test_validate_public_host_blocks_localhost() {
        let url = reqwest::Url::parse("http://localhost:8080/admin").unwrap();
        let err = validate_public_host(&url).await.unwrap_err();
        assert!(err.contains("blocked"), "{}", err);
    }

    #[tokio::test]
    async fn test_validate_public_host_blocks_private_ip_literal() {
        let url = reqwest::Url::parse("http://169.254.169.254/latest/meta-data/").unwrap();
        let err = validate_public_host(&url).await.unwrap_err();
        assert!(err.contains("blocked"), "{}", err);
    }

    #[tokio::test]
    async fn test_validate_public_host_blocks_loopback_ip_literal() {
        let url = reqwest::Url::parse("http://127.0.0.1:3000/").unwrap();
        let err = validate_public_host(&url).await.unwrap_err();
        assert!(err.contains("blocked"), "{}", err);
    }

    #[tokio::test]
    async fn test_validate_public_host_allows_public_domain() {
        // docs.rs resolves to a public CDN IP; this should pass the guard.
        let url = reqwest::Url::parse("https://docs.rs/tokio/latest/tokio/").unwrap();
        assert!(validate_public_host(&url).await.is_ok());
    }
}
