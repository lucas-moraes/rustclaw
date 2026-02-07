use super::MemoryEntry;

/// Calculate cosine similarity between two vectors
/// Returns value between -1.0 and 1.0 (1.0 = identical)
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return 0.0;
    }

    dot_product / (magnitude_a * magnitude_b)
}

/// Search for most similar memories to a query embedding
pub fn search_similar_memories(
    query_embedding: &[f32],
    memories: &[MemoryEntry],
    top_k: usize,
    min_similarity: f32,
) -> Vec<(MemoryEntry, f32)> {
    let mut results: Vec<(MemoryEntry, f32)> = memories
        .iter()
        .map(|memory| {
            let similarity = cosine_similarity(query_embedding, &memory.embedding);
            (memory.clone(), similarity)
        })
        .filter(|(_, similarity)| *similarity >= min_similarity)
        .collect();

    // Sort by similarity (descending)
    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Take top k
    results.truncate(top_k);

    results
}

/// Calculate importance score based on:
/// - Base importance
/// - Recency (newer = higher)
/// - Search frequency (more searched = higher)
pub fn calculate_relevance_score(memory: &MemoryEntry, now: chrono::DateTime<chrono::Utc>) -> f32 {
    let base_importance = memory.importance;

    // Recency factor (decay over 30 days)
    let age_days = (now - memory.timestamp).num_days() as f32;
    let recency_factor = (-age_days / 30.0).exp();

    // Search frequency factor (boost for frequently accessed memories)
    let search_factor = (memory.search_count as f32 / 10.0).min(1.0);

    // Combined score (weighted)
    base_importance * 0.5 + recency_factor * 0.3 + search_factor * 0.2
}

/// Format memories for injection into prompt
pub fn format_memories_for_prompt(memories: &[(MemoryEntry, f32)]) -> String {
    if memories.is_empty() {
        return String::new();
    }

    let mut output = String::from("\n📚 Memórias relevantes de conversas anteriores:\n");

    for (i, (memory, similarity)) in memories.iter().enumerate() {
        let relevance = (similarity * 100.0) as i32;
        let mem_type_emoji = match memory.memory_type {
            super::MemoryType::Fact => "📌",
            super::MemoryType::Episode => "💭",
            super::MemoryType::ToolResult => "🔧",
        };

        output.push_str(&format!(
            "\n{}. {} {} (relevância: {}%)\n   {}",
            i + 1,
            mem_type_emoji,
            memory.timestamp.format("%d/%m/%Y"),
            relevance,
            memory.content.lines().next().unwrap_or(&memory.content)
        ));

        if memory.content.lines().count() > 1 {
            output.push_str("...");
        }
    }

    output.push_str("\n\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b) - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 0.001);
    }
}
