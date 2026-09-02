//! Minimal unified-diff helper for surfacing edit/write changes in the UI.

use std::fmt::Write as _;

const CONTEXT: usize = 3;
const MAX_DIFF_LINES: usize = 200;

/// Operation with source indices tracked during backtrack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// Same line: `a` index and `b` index are equal.
    Same(usize),
    /// Line deleted from `before` at `a` index.
    Del(usize),
    /// Line added to `after` at `b` index.
    Add(usize),
}

/// Computes a unified-style diff between `before` and `after`.
/// Returns the diff text (hunk headers + lines with +/ -/ spaces), truncated to
/// `MAX_DIFF_LINES` with a trailing notice when truncated.
pub fn unified_diff(before: &str, after: &str) -> String {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();

    let ops = compute_ops(&a, &b);
    let mut out = String::new();
    let mut emitted = 0usize;
    let mut truncated = false;

    let mut i = 0usize; // index into ops
    while i < ops.len() {
        // Skip unchanged runs at the front of this window.
        let mut start = i;
        while start < ops.len() && matches!(ops[start], Op::Same(_)) {
            start += 1;
        }
        if start >= ops.len() {
            break;
        }
        // Find end of this change run (a change followed by CONTEXT same lines).
        let mut end = start;
        let mut same_since = 0usize;
        while end < ops.len() {
            match ops[end] {
                Op::Same(_) => {
                    same_since += 1;
                    if same_since > CONTEXT {
                        break;
                    }
                }
                Op::Del(_) | Op::Add(_) => same_since = 0,
            }
            end += 1;
        }

        // Emit pre-context lines immediately before the change.
        let pre_start = start.saturating_sub(CONTEXT);
        // Ensure pre-context starts on a Same op boundary (or clamp).
        for op in ops.iter().take(start).skip(pre_start) {
            if let Op::Same(idx) = op {
                emit_line(&mut out, ' ', a[*idx], &mut emitted, &mut truncated);
            }
        }

        // Emit the change run (including trailing context up to CONTEXT).
        for op in ops.iter().take(end).skip(start) {
            match op {
                Op::Same(idx) => emit_line(&mut out, ' ', a[*idx], &mut emitted, &mut truncated),
                Op::Del(idx) => emit_line(&mut out, '-', a[*idx], &mut emitted, &mut truncated),
                Op::Add(idx) => emit_line(&mut out, '+', b[*idx], &mut emitted, &mut truncated),
            }
        }

        // Advance i past this hunk. `end` already includes the trailing context
        // that may overlap the next hunk's pre-context; that's acceptable.
        i = end;
    }

    if truncated {
        out.push_str("\n[diff truncated]");
    }
    out
}

fn emit_line(
    out: &mut String,
    prefix: char,
    line: &str,
    emitted: &mut usize,
    truncated: &mut bool,
) {
    if *emitted >= MAX_DIFF_LINES {
        *truncated = true;
        return;
    }
    let _ = writeln!(out, "{} {}", prefix, line);
    *emitted += 1;
}

/// Backtracks the LCS DP table to produce a list of operations with source indices.
fn compute_ops(a: &[&str], b: &[&str]) -> Vec<Op> {
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(Op::Same(i));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push(Op::Del(i));
            i += 1;
        } else {
            ops.push(Op::Add(j));
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Del(i));
        i += 1;
    }
    while j < m {
        ops.push(Op::Add(j));
        j += 1;
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_identical() {
        assert_eq!(unified_diff("a\nb\nc\n", "a\nb\nc\n").trim(), "");
    }

    #[test]
    fn test_diff_single_change() {
        let d = unified_diff("a\nb\nc\n", "a\nB\nc\n");
        assert!(d.contains("- b"), "expected -b, got:\n{}", d);
        assert!(d.contains("+ B"), "expected +B, got:\n{}", d);
    }

    #[test]
    fn test_diff_new_file() {
        let d = unified_diff("", "hello\nworld\n");
        assert!(d.contains("+ hello"));
        assert!(d.contains("+ world"));
    }

    #[test]
    fn test_diff_append() {
        let d = unified_diff("line1\n", "line1\nline2\n");
        assert!(d.contains("+ line2"), "got:\n{}", d);
    }

    #[test]
    fn test_diff_truncates_long() {
        // Many scattered single-line changes across a large file -> many hunks
        // that exceed the line budget.
        let mut before: Vec<String> = (0..300).map(|i| format!("line {}", i)).collect();
        let mut after = before.clone();
        for i in (0..300).step_by(3) {
            after[i] = format!("CHANGED {}", i);
        }
        let d = unified_diff(&before.join("\n"), &after.join("\n"));
        assert!(
            d.contains("[diff truncated]"),
            "expected truncation notice, got {} lines",
            d.lines().count()
        );
        before.clear();
    }
}
