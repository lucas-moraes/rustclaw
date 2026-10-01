//! Headless-Chrome rendering for `fetch_webpage` (ROADMAP item 6).
//!
//! Plain HTTP fetching returns the server's initial HTML, which is empty for
//! Single Page Applications (React/Vue/Angular, many modern docs). This module
//! drives a real Chrome/Chromium via the DevTools protocol (`chromiumoxide`),
//! waits for the page to settle, and returns the *rendered* DOM as HTML.
//!
//! The browser binary is located through [`crate::harness::deps`]; when it is
//! missing, [`render`] returns a clear error and the caller falls back to the
//! plain-HTTP path.

use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use futures_util::StreamExt;

use crate::harness::deps;

/// How long to wait for the page to finish loading before reading the DOM.
const NAV_TIMEOUT: Duration = Duration::from_secs(20);
/// Extra settle time after navigation for client-side JS to paint content.
const SETTLE: Duration = Duration::from_millis(800);

/// Renders `url` in headless Chrome and returns the post-JS HTML.
///
/// Returns `Err` when no Chrome/Chromium binary is available or the page fails
/// to load; the caller is expected to fall back to plain HTTP.
pub async fn render(url: &str) -> Result<String, String> {
    let executable = deps::chrome_path().ok_or_else(|| {
        "no Chrome/Chromium binary found — install one or use render=\"http\" \
         (see `rustclaw doctor`)"
            .to_string()
    })?;

    let config = BrowserConfig::builder()
        .chrome_executable(executable)
        .no_sandbox()
        .build()
        .map_err(|e| format!("failed to build browser config: {e}"))?;

    let (mut browser, mut handler) = Browser::launch(config)
        .await
        .map_err(|e| format!("failed to launch headless Chrome: {e}"))?;

    // The handler drives the CDP connection; it must be polled for the browser
    // to make progress. Run it in the background and abort it on return.
    let handler_task = tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            if h.is_err() {
                break;
            }
        }
    });

    let result = render_inner(&browser, url).await;

    // Best-effort teardown: close the browser, then stop the handler task.
    let _ = browser.close().await;
    let _ = browser.wait().await;
    handler_task.abort();

    result
}

/// Navigates to `url` and reads the rendered DOM.
async fn render_inner(browser: &Browser, url: &str) -> Result<String, String> {
    let page = browser
        .new_page("about:blank")
        .await
        .map_err(|e| format!("failed to open a browser page: {e}"))?;

    let nav = page.goto(url);
    match tokio::time::timeout(NAV_TIMEOUT, nav).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(format!("navigation failed for {url}: {e}")),
        Err(_) => return Err(format!("navigation timed out for {url}")),
    }

    // Give client-side frameworks a moment to mount and paint.
    tokio::time::sleep(SETTLE).await;

    let html = page
        .content()
        .await
        .map_err(|e| format!("failed to read rendered DOM: {e}"))?;

    let _ = page.close().await;
    Ok(html)
}

/// Heuristic: does the converted Markdown look like an empty SPA shell?
///
/// Used to suggest a `render: "browser"` retry when plain HTTP returned almost
/// nothing. Kept deliberately loose — false positives only cost a suggestion.
pub fn looks_empty(markdown: &str) -> bool {
    let trimmed = markdown.trim();
    if trimmed.len() < 200 {
        return true;
    }
    // Count non-whitespace characters; a shell page is mostly markup noise.
    let visible = trimmed.chars().filter(|c| !c.is_whitespace()).count();
    visible < 120
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_empty_for_short_content() {
        assert!(looks_empty(""));
        assert!(looks_empty("   \n  "));
        assert!(looks_empty("<div id=\"root\"></div>"));
    }

    #[test]
    fn test_looks_empty_for_spa_shell() {
        // A typical SPA shell: lots of markup, almost no visible text.
        let shell = "<html><head><script src=\"/app.js\"></script></head>\
                     <body><div id=\"root\"></div></body></html>";
        assert!(looks_empty(shell));
    }

    #[test]
    fn test_not_empty_for_real_content() {
        let content = "This is a real documentation page with plenty of text. \
            It explains how the API works, shows examples, and describes the \
            parameters in detail so that a reader can follow along and build \
            something useful with the library.";
        assert!(!looks_empty(content));
    }

    /// Live smoke test: renders a real page in headless Chrome. Requires a
    /// Chrome/Chromium binary on the host (see `rustclaw doctor`).
    #[tokio::test]
    #[ignore = "requires a Chrome/Chromium binary and network access"]
    async fn test_render_real_page() {
        if deps::chrome_path().is_none() {
            eprintln!("skipping: no Chrome/Chromium found");
            return;
        }
        let html = render("https://example.com").await.expect("render failed");
        assert!(html.contains("Example Domain"), "got: {html}");
    }
}
