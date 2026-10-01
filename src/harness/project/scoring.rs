//! Fact scoring, ranking and rendering for project memory.
//!
//! Extracted from `memory.rs`: the `MemoryFact` struct, relevance scoring
//! (recency + usage + lexical overlap + optional BM25), FTS5 query helpers,
//! and the memory-block rendering that is injected into the system prompt.

/// A single curated fact with its metadata.
#[derive(Clone, Debug)]
pub struct MemoryFact {
    pub id: i64,
    pub text: String,
    pub kind: String,
    pub confidence: String,
    pub hit_count: i64,
    pub last_used: Option<String>,
    pub archived: bool,
}

/// Default budget (bytes) for curated memory injected into the system prompt.
pub const MAX_MEMORY_CHARS: usize = 2048;

/// Marker wrapping the per-turn memory block injected as a prefix of the user
/// message (kept OUT of the system prompt so the prompt prefix stays
/// byte-stable across turns and provider prompt caches keep hitting).
pub const MEMORY_BLOCK_START: &str = "<project-memory>";
/// Closing marker for [`MEMORY_BLOCK_START`].
pub const MEMORY_BLOCK_END: &str = "</project-memory>";

/// True when `text` is a memory block injected by the runtime (used by the
/// UI to hide these parts from the transcript and session previews).
pub fn is_memory_block(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with(MEMORY_BLOCK_START) && t.contains(MEMORY_BLOCK_END)
}

/// Strips a leading `<project-memory>...</project-memory>` block (runtime
/// injection) from message text. Returns the remaining user-visible content
/// (empty when the whole part was just the memory block).
pub fn strip_memory_blocks(text: &str) -> String {
    let t = text.trim_start();
    if !is_memory_block(t) {
        return text.to_string();
    }
    match t.find(MEMORY_BLOCK_END) {
        Some(end) => t[end + MEMORY_BLOCK_END.len()..].trim().to_string(),
        None => String::new(),
    }
}

/// Max bytes for the active structural `summary` before compaction rolls
/// lower-priority facts into the `archive` column.
pub const MAX_SUMMARY_CHARS: usize = 4096;

/// Facts with `hit_count >= AUTO_PROMOTE_HITS` are auto-promoted to skills
/// by `/memory gc`.
pub const AUTO_PROMOTE_HITS: i64 = 5;

/// Weight of the BM25 (FTS5) rank in the hybrid relevance score.
const W_BM25: f64 = 0.35;
/// Weights of the non-BM25 components when BM25 is available (they are
/// rescaled to sum to 1 - W_BM25).
const W_RECENCY: f64 = 0.4;
const W_HITS: f64 = 0.3;
const W_LEXICAL: f64 = 0.3;
/// Half-life (days) for recency decay.
const RECENCY_HALF_LIFE_DAYS: f64 = 30.0;

/// Computes a relevance score for a fact given the current turn text.
/// Higher is more relevant. Combines recency (decay since `last_used`),
/// usage (`hit_count`) and lexical overlap with the query.
pub fn score_fact(fact: &MemoryFact, query: &str, now: &chrono::DateTime<chrono::Utc>) -> f64 {
    // Recency: 1.0 when used now, decaying toward 0 with half-life.
    let recency = match &fact.last_used {
        Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
            Ok(t) => {
                let age_days = (now.signed_duration_since(t.with_timezone(&chrono::Utc)))
                    .num_seconds() as f64
                    / 86_400.0;
                if age_days <= 0.0 {
                    1.0
                } else {
                    0.5_f64.powf(age_days / RECENCY_HALF_LIFE_DAYS)
                }
            }
            Err(_) => 0.0,
        },
        None => 0.0,
    };

    // Usage: normalized hit count (capped at 10).
    let hits = (fact.hit_count as f64).min(10.0) / 10.0;

    // Lexical overlap: fraction of query tokens present in the fact text.
    let lexical = lexical_overlap(query, &fact.text);

    W_RECENCY * recency + W_HITS * hits + W_LEXICAL * lexical
}

/// Hybrid score combining `score_fact` with an optional FTS5 BM25 rank
/// (lower rank value = more relevant). When `bm25` is `Some`, the base
/// weights are rescaled to make room for the BM25 component.
pub fn score_fact_bm25(
    fact: &MemoryFact,
    query: &str,
    now: &chrono::DateTime<chrono::Utc>,
    bm25: Option<f64>,
) -> f64 {
    let Some(bm25) = bm25 else {
        return score_fact(fact, query, now);
    };
    let base = score_fact(fact, query, now); // in [0, 1], weights sum to 1
    let base_w = 1.0 - W_BM25;
    // Normalize BM25 (negative, unbounded) into [0, 1]: rank 0 → 1.0,
    // rank <= -10 → 0.0.
    let bm25_norm = (1.0 + bm25 / 10.0).clamp(0.0, 1.0);
    base * base_w + W_BM25 * bm25_norm
}

/// Escapes a user query for FTS5 MATCH: quotes each alphanumeric token so
/// special characters cannot break the query syntax, and joins them with
/// implicit AND. Returns an empty string when there are no usable terms.
pub(crate) fn fts_escape(query: &str) -> String {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    tokens.join(" ")
}

/// Fraction of query tokens (lowercased, alphanumeric) present in `text`.
fn lexical_overlap(query: &str, text: &str) -> f64 {
    let text_lower = text.to_lowercase();
    let tokens: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && t.len() > 2)
        .map(|t| t.to_string())
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let matched = tokens
        .iter()
        .filter(|t| text_lower.contains(t.as_str()))
        .count();
    matched as f64 / tokens.len() as f64
}

/// Renders the top active facts for the system prompt, ordered by relevance
/// and bounded by `max_chars`. Only non-archived facts are considered.
#[allow(dead_code)] // thin wrapper kept for tests / external callers
pub fn render_memory(facts: &[MemoryFact], query: &str, max_chars: usize) -> String {
    render_memory_ranked(facts, query, max_chars, &std::collections::HashMap::new())
}

/// Like `render_memory`, but boosts facts that matched an FTS5 full-text
/// query. `ranks` maps fact id → BM25 rank (lower = more relevant); facts
/// absent from the map are scored without the BM25 component.
pub fn render_memory_ranked(
    facts: &[MemoryFact],
    query: &str,
    max_chars: usize,
    ranks: &std::collections::HashMap<i64, f64>,
) -> String {
    let now = chrono::Utc::now();
    let mut active: Vec<&MemoryFact> = facts.iter().filter(|f| !f.archived).collect();
    active.sort_by(|a, b| {
        score_fact_bm25(b, query, &now, ranks.get(&b.id).copied())
            .partial_cmp(&score_fact_bm25(a, query, &now, ranks.get(&a.id).copied()))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut out = String::new();
    let mut used = 0usize;
    for fact in active {
        let line = format!("- {}\n", fact.text);
        if used + line.len() > max_chars && !out.is_empty() {
            break;
        }
        out.push_str(&line);
        used += line.len();
    }
    out
}

/// Normalizes a fact's text for dedup: strips the `- [timestamp] ` prefix,
/// lowercases and collapses whitespace.
pub(crate) fn normalize_fact(text: &str) -> String {
    let stripped = strip_timestamp_prefix(text);
    stripped
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Strips a leading `- [timestamp] ` prefix from a fact line, if present.
pub(crate) fn strip_timestamp_prefix(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("- [") {
        if let Some(end) = rest.find("] ") {
            return rest[end + 2..].to_string();
        }
    }
    trimmed.to_string()
}
