//! Structured compression of large tool outputs ("read less").
//!
//! Large outputs enter the context compressed; the original is saved to
//! disk by [`crate::harness::tool::output_store`] and can be recovered
//! byte-exact via the `read` tool. Detection is intentionally
//! conservative: when in doubt, [`OutputKind::Plain`] falls back to the
//! existing truncate behavior.
//!
//! Diffs are **never** compressed — byte-exact output is a hard
//! requirement of the `edit` tool.

/// Default threshold: outputs smaller than this pass through untouched.
pub const DEFAULT_COMPRESS_THRESHOLD: usize = 4 * 1024;

/// Classified shape of a tool output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    /// `cargo test` / `cargo build` style output: keep failures + summary.
    TestOutput,
    /// JSON payload: re-serialize compactly, collapse large arrays.
    Json,
    /// Log lines: keep ERROR/WARN + head/tail, collapse repetitions.
    Logs,
    /// Unified diff: never compressed (byte-exact requirement).
    Diff,
    /// Anything else: falls back to plain truncation.
    Plain,
}

/// Result of compressing a tool output.
#[derive(Clone, Debug)]
pub struct Compressed {
    /// Compressed text that replaces the original in the context.
    pub summary: String,
}

/// Heuristically classifies `text` into an [`OutputKind`].
///
/// Conservative by design: ambiguous text returns [`OutputKind::Plain`].
pub fn detect_kind(text: &str) -> OutputKind {
    let trimmed = text.trim_start();
    if trimmed.starts_with("diff --git") || trimmed.starts_with("--- ") && trimmed.contains("\n@@")
    {
        return OutputKind::Diff;
    }
    // Test output: cargo test / cargo build markers.
    let has_test_summary = text.contains("test result:")
        || text.contains("running ")
            && text.contains(" tests")
            && (text.contains("FAILED") || text.contains("ok.") || text.contains("passed"));
    if has_test_summary {
        return OutputKind::TestOutput;
    }
    // JSON: starts with { or [ and parses.
    if (trimmed.starts_with('{') || trimmed.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok()
    {
        return OutputKind::Json;
    }
    // Logs: density of level markers above threshold.
    let lines: Vec<&str> = text.lines().collect();
    if !lines.is_empty() {
        let marker_lines = lines
            .iter()
            .filter(|l| {
                l.contains("ERROR")
                    || l.contains("WARN")
                    || l.contains("level=")
                    || l.contains("error:")
                    || l.contains("warning:")
            })
            .count();
        // >25% of lines look like log levels → Logs.
        if marker_lines * 4 > lines.len() {
            return OutputKind::Logs;
        }
    }
    OutputKind::Plain
}

/// Compresses `text` according to its detected [`OutputKind`].
///
/// Returns `None` when the text should pass through untouched (small
/// outputs, or diffs which are never compressed).
pub fn compress(text: &str, threshold: usize) -> Option<Compressed> {
    if text.len() <= threshold {
        return None;
    }
    let kind = detect_kind(text);
    let summary = match kind {
        OutputKind::Diff => return None, // byte-exact requirement
        OutputKind::TestOutput => compress_test_output(text),
        OutputKind::Json => compress_json(text),
        OutputKind::Logs => compress_logs(text),
        OutputKind::Plain => return None, // truncate.rs already handles this
    };
    if summary.len() >= text.len() {
        return None; // compression made it worse — keep original
    }
    Some(Compressed { summary })
}

/// Collapses `cargo test`-style output: keeps failures and the final
/// summary, collapses passing test lines into a count.
fn compress_test_output(text: &str) -> String {
    let mut out = String::new();
    let mut passed = 0usize;
    let mut kept_failures: Vec<&str> = Vec::new();
    let mut summary_lines: Vec<&str> = Vec::new();

    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("test ") && t.ends_with("... ok") {
            passed += 1;
            continue;
        }
        if t.starts_with("test ") && (t.contains("FAILED") || t.contains("panicked")) {
            kept_failures.push(line);
            continue;
        }
        if t.starts_with("failures:") || t.starts_with("---- ") || t.contains("test result:") {
            kept_failures.push(line);
            continue;
        }
        if t.starts_with("running ") || t.starts_with("Compiling") || t.starts_with("Finished") {
            summary_lines.push(line);
        }
    }
    for l in &summary_lines {
        out.push_str(l);
        out.push('\n');
    }
    if passed > 0 {
        out.push_str(&format!("[{} tests passed]\n", passed));
    }
    for l in &kept_failures {
        out.push_str(l);
        out.push('\n');
    }
    out
}

/// Re-serializes JSON compactly; large arrays collapse to first item + count.
fn compress_json(text: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return text.to_string();
    };
    match value {
        serde_json::Value::Array(items) => {
            if items.len() <= 1 {
                return serde_json::to_string(&items).unwrap_or_default();
            }
            let first = serde_json::to_string_pretty(&items[0]).unwrap_or_default();
            format!(
                "[\n  {},\n  … [{} items total — first shown]\n]",
                first,
                items.len()
            )
        }
        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
    }
}

/// Keeps ERROR/WARN lines plus head/tail; collapses consecutive repeats.
fn compress_logs(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let head = 3;
    let tail = 3;
    let mut out: Vec<String> = Vec::new();
    let mut repeated = 0usize;
    let mut last: Option<&str> = None;

    for (i, line) in lines.iter().enumerate() {
        let keep = i < head
            || i >= lines.len().saturating_sub(tail)
            || line.contains("ERROR")
            || line.contains("WARN")
            || line.contains("error:")
            || line.contains("warning:");
        if !keep {
            continue;
        }
        if Some(*line) == last {
            repeated += 1;
            continue;
        }
        if repeated > 0 {
            out.push(format!("[repeated {}×]", repeated));
            repeated = 0;
        }
        out.push((*line).to_string());
        last = Some(line);
    }
    if repeated > 0 {
        out.push(format!("[repeated {}×]", repeated));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_test_output() {
        let text = "running 3 tests\ntest a ... ok\ntest b ... ok\ntest result: ok. 3 passed";
        assert_eq!(detect_kind(text), OutputKind::TestOutput);
    }

    #[test]
    fn test_detect_json() {
        let text = r#"{"a": 1, "b": [1, 2, 3]}"#;
        assert_eq!(detect_kind(text), OutputKind::Json);
    }

    #[test]
    #[allow(clippy::disallowed_names)]
    fn test_detect_logs() {
        let text = "ERROR x1\nWARN y\nERROR x2\nINFO z\nERROR x3\nDEBUG w\nERROR x4";
        assert_eq!(detect_kind(text), OutputKind::Logs);
    }

    #[test]
    fn test_detect_diff() {
        let text = "diff --git a/x.rs b/x.rs\nindex 123..456 100644\n--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,2 @@";
        assert_eq!(detect_kind(text), OutputKind::Diff);
    }

    #[test]
    fn test_detect_plain() {
        assert_eq!(detect_kind("just some prose text"), OutputKind::Plain);
    }

    #[test]
    fn test_compress_small_passes_through() {
        let text = "test result: ok. 1 passed";
        assert!(compress(text, DEFAULT_COMPRESS_THRESHOLD).is_none());
    }

    #[test]
    fn test_compress_diff_never() {
        let big_diff = format!("diff --git a/x b/x\n{}", "+line\n".repeat(2000));
        assert!(compress(&big_diff, DEFAULT_COMPRESS_THRESHOLD).is_none());
    }

    #[test]
    fn test_compress_test_output_keeps_failures() {
        let mut text = String::from("running 100 tests\n");
        for i in 0..95 {
            text.push_str(&format!("test test_{} ... ok\n", i));
        }
        text.push_str("test bad_one ... FAILED\n");
        text.push_str("failures:\n---- bad_one stdout ----\nassertion failed\n");
        text.push_str("test result: FAILED. 95 passed; 1 failed\n");
        let c = compress(&text, 100).expect("should compress");
        assert!(c.summary.contains("[95 tests passed]"));
        assert!(c.summary.contains("bad_one"));
        assert!(c.summary.contains("test result: FAILED"));
        assert!(c.summary.len() < text.len() / 2);
    }

    #[test]
    fn test_compress_json_array() {
        let items: Vec<u32> = (0..500).collect();
        let text = serde_json::to_string(&items).unwrap();
        let c = compress(&text, 100).expect("should compress");
        assert!(c.summary.contains("[500 items total"));
        assert!(c.summary.len() < text.len() / 2);
    }

    #[test]
    fn test_compress_logs_repetitions() {
        let mut text = String::new();
        for _ in 0..100 {
            text.push_str("ERROR connection refused\n");
        }
        text.push_str("WARN retrying\n");
        let c = compress(&text, 100).expect("should compress");
        assert!(c.summary.contains("[repeated"));
        assert!(c.summary.contains("WARN retrying"));
        assert!(c.summary.len() < text.len() / 2);
    }

    #[test]
    fn test_compress_plain_returns_none() {
        let text = "word ".repeat(2000);
        assert!(compress(&text, DEFAULT_COMPRESS_THRESHOLD).is_none());
    }
}
