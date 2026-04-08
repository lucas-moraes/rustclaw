use super::MemoryEntry;

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

pub fn search_similar_memories(
    query_embedding: &[f32],
    memories: &[MemoryEntry],
    top_k: usize,
    min_similarity: f32,
) -> Vec<(MemoryEntry, f32)> {
    let now = chrono::Utc::now();

    let mut results: Vec<(MemoryEntry, f32)> = memories
        .iter()
        .map(|memory| {
            let similarity = cosine_similarity(query_embedding, &memory.embedding);

            let age_days = (now - memory.timestamp).num_days().max(1) as f32;
            let recency_factor = 1.0 / age_days.sqrt();

            let final_score = similarity * 0.7 + recency_factor.min(0.5) * 0.3;

            (memory.clone(), final_score)
        })
        .filter(|(_, score)| *score >= min_similarity)
        .collect();

    results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    results.truncate(top_k);

    results
}

#[allow(dead_code)]
pub fn calculate_relevance_score(memory: &MemoryEntry, now: chrono::DateTime<chrono::Utc>) -> f32 {
    let base_importance = memory.importance;

    let age_days = (now - memory.timestamp).num_days() as f32;
    let recency_factor = (-age_days / 30.0).exp();

    let search_factor = (memory.search_count as f32 / 10.0).min(1.0);

    base_importance * 0.5 + recency_factor * 0.3 + search_factor * 0.2
}

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

        let scope_indicator = match memory.scope {
            super::MemoryScope::Session => "",
            super::MemoryScope::Project => " [projeto]",
            super::MemoryScope::Global => " [global]",
        };

        output.push_str(&format!(
            "\n{}. {}{} {} (relevância: {}%)\n   {}",
            i + 1,
            mem_type_emoji,
            scope_indicator,
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
