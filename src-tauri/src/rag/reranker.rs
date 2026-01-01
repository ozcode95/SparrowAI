use super::SearchResult;

pub struct RerankerService {}

impl RerankerService {
    pub fn new() -> Self {
        Self {}
    }


    pub async fn rerank(
        &self,
        query: &str,
        results: Vec<SearchResult>
    ) -> Result<Vec<SearchResult>, String> {
        if results.is_empty() {
            return Ok(results);
        }

        // For now, implement a hybrid scoring approach
        // You can replace this with actual reranker model calls when available
        let mut reranked_results = results;

        for result in &mut reranked_results {
            let semantic_score = result.score; // Original embedding similarity
            let lexical_score = calculate_lexical_similarity(query, &result.document.content);
            let length_penalty = calculate_length_penalty(&result.document.content);

            // Combine scores with adjusted weights
            // Prioritize semantic similarity (70%), then lexical (20%), then length (10%)
            let combined_score = semantic_score * 0.7 + lexical_score * 0.2 + length_penalty * 0.1;

            result.rerank_score = Some(combined_score);
        }

        // Sort by reranked scores
        reranked_results.sort_by(|a, b| {
            b.rerank_score
                .unwrap_or(0.0)
                .partial_cmp(&a.rerank_score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(reranked_results)
    }

    pub async fn rerank_simple(
        &self,
        query: &str,
        results: Vec<SearchResult>
    ) -> Result<Vec<SearchResult>, String> {
        if results.is_empty() {
            return Ok(results);
        }

        let mut reranked_results = results;

        for result in &mut reranked_results {
            let lexical_score = calculate_lexical_similarity(query, &result.document.content);
            // Simple reranking: combine original score with lexical similarity
            let combined_score = result.score * 0.7 + lexical_score * 0.3;
            result.rerank_score = Some(combined_score);
        }

        reranked_results.sort_by(|a, b| {
            b.rerank_score
                .unwrap_or(0.0)
                .partial_cmp(&a.rerank_score.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(reranked_results)
    }
}

fn calculate_lexical_similarity(query: &str, content: &str) -> f32 {
    let query_words: std::collections::HashSet<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let content_words: std::collections::HashSet<String> = content
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if query_words.is_empty() || content_words.is_empty() {
        return 0.0;
    }

    let intersection_count = query_words.intersection(&content_words).count();
    let union_count = query_words.union(&content_words).count();

    if union_count == 0 {
        0.0
    } else {
        (intersection_count as f32) / (union_count as f32)
    }
}

fn calculate_length_penalty(content: &str) -> f32 {
    let length = content.len();

    // Prefer documents that are not too short or too long
    if length < 100 {
        0.3 // Short documents get lower score
    } else if length > 2000 {
        0.5 // Very long documents get moderate score
    } else {
        1.0 // Ideal length gets full score
    }
}

#[tauri::command]
pub async fn rerank_search_results(
    query: String,
    results: Vec<SearchResult>
) -> Result<Vec<SearchResult>, String> {
    let reranker = RerankerService::new();
    reranker.rerank(&query, results).await
}

#[tauri::command]
pub async fn rerank_search_results_simple(
    query: String,
    results: Vec<SearchResult>
) -> Result<Vec<SearchResult>, String> {
    let reranker = RerankerService::new();
    reranker.rerank_simple(&query, results).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexical_similarity() {
        let query = "machine learning algorithms";
        let content = "This document discusses various machine learning techniques and algorithms.";
        let similarity = calculate_lexical_similarity(query, content);
        assert!(similarity > 0.0);
    }

    #[test]
    fn test_length_penalty() {
        let short_content = "Short";
        let ideal_content = "This is a document with ideal length for processing and retrieval.";
        let long_content = "A".repeat(3000);

        assert!(
            calculate_length_penalty(&ideal_content) > calculate_length_penalty(&short_content)
        );
        assert!(calculate_length_penalty(&ideal_content) > calculate_length_penalty(&long_content));
    }
}
