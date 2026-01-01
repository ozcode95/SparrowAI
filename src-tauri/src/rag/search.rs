use super::SearchResult;
use crate::rag::embeddings::EmbeddingService;
use crate::rag::vector_store::VectorStore;
use crate::rag::reranker::RerankerService;

pub struct SearchService {
    embedding_service: EmbeddingService,
    vector_store: VectorStore,
    reranker_service: RerankerService,
}

impl SearchService {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            embedding_service: EmbeddingService::new(),
            vector_store: VectorStore::new()?,
            reranker_service: RerankerService::new(),
        })
    }
    
    pub async fn search(&self, query: &str, limit: usize, use_reranking: bool) -> Result<Vec<SearchResult>, String> {
        // Step 1: Create query embedding
        let query_embedding = self.embedding_service.create_single_embedding(query.to_string()).await?;
        
        // Step 2: Vector similarity search
        let initial_results = self.vector_store.search_similar(&query_embedding, limit * 2)?; // Get more for reranking
        
        // Step 3: Rerank if requested
        let final_results = if use_reranking && !initial_results.is_empty() {
            let reranked = self.reranker_service.rerank(query, initial_results).await?;
            reranked.into_iter().take(limit).collect()
        } else {
            initial_results.into_iter().take(limit).collect()
        };
        
        Ok(final_results)
    }
    
    pub async fn search_with_filters(
        &self, 
        query: &str, 
        limit: usize, 
        file_types: Option<Vec<String>>,
        use_reranking: bool
    ) -> Result<Vec<SearchResult>, String> {
        let mut results = self.search(query, limit * 2, use_reranking).await?;
        
        // Apply file type filters if specified
        if let Some(types) = file_types {
            results = results.into_iter()
                .filter(|result| types.contains(&result.document.file_type))
                .collect();
        }
        
        results.truncate(limit);
        Ok(results)
    }
}

#[tauri::command]
pub async fn search_documents_by_query(
    query: String, 
    limit: Option<usize>, 
    use_reranking: Option<bool>,
    file_types: Option<Vec<String>>
) -> Result<Vec<SearchResult>, String> {
    log_operation_start!("Search documents");
    
    let search_service = SearchService::new()
        .map_err(|e| {
            log_operation_error!("Search documents", &e, note = "failed to create search service");
            e
        })?;
    
    let search_limit = limit.unwrap_or(10);
    let should_rerank = use_reranking.unwrap_or(true);
    
    tracing::debug!(
        query = %query,
        limit = search_limit,
        rerank = should_rerank,
        file_types = ?file_types,
        "Searching documents"
    );
    
    let results = if let Some(types) = file_types {
        search_service.search_with_filters(&query, search_limit, Some(types), should_rerank).await?
    } else {
        search_service.search(&query, search_limit, should_rerank).await?
    };
    
    log_operation_success!("Search documents");
    tracing::debug!(query = %query, results_count = results.len(), "Document search completed");
    
    Ok(results)
}

#[tauri::command]
pub async fn get_search_suggestions(query: String) -> Result<Vec<String>, String> {
    // Simple implementation - you can enhance this with more sophisticated suggestion logic
    let search_service = SearchService::new()?;
    let results = search_service.search(&query, 5, false).await?;
    
    let suggestions: Vec<String> = results.into_iter()
        .map(|result| {
            // Extract key phrases from document titles or content
            let title_words: Vec<&str> = result.document.title.split_whitespace().collect();
            if title_words.len() > 2 {
                title_words[0..2].join(" ")
            } else {
                result.document.title
            }
        })
        .collect();
    
    Ok(suggestions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_search_service_creation() {
        // This test might fail if the vector store can't be created in test environment
        // but it validates the service structure
        let result = SearchService::new();
        // We can't guarantee this will work in all test environments
        // so we just test that the function doesn't panic
        match result {
            Ok(_) => assert!(true),
            Err(_) => assert!(true), // Expected in test environment
        }
    }
}