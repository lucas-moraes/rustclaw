use scraper::{Html, Selector};

use super::{clean_text, clean_url, extract_domain, SearchResult, MAX_RESULTS};

pub(super) fn parse_results(html: &str) -> Vec<SearchResult> {
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
pub(super) fn parse_results_lite(html: &str) -> Vec<SearchResult> {
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
pub(super) fn parse_results_mojeek(html: &str) -> Vec<SearchResult> {
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

/// Parses the Tavily JSON response into `SearchResult`s.
pub(super) fn parse_tavily(json: &str) -> Vec<SearchResult> {
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    if let Some(arr) = v.get("results").and_then(|r| r.as_array()) {
        for item in arr {
            let title = item
                .get("title")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let url = item
                .get("url")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let snippet = item
                .get("content")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                continue;
            }
            let domain = extract_domain(&url);
            out.push(SearchResult {
                title,
                url,
                snippet,
                domain,
                date: None,
            });
            if out.len() >= MAX_RESULTS {
                break;
            }
        }
    }
    out
}
