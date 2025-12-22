use super::Document;
use async_openai::{ types::CreateEmbeddingRequestArgs, Client };
use async_openai::config::OpenAIConfig;
use crate::constants;

pub struct EmbeddingService {
    client: Client<OpenAIConfig>,
}

impl EmbeddingService {
    pub fn new() -> Self {
        let api_base = format!("{}{}", constants::OVMS_API_BASE, constants::OVMS_OPENAI_PATH);
        let config = OpenAIConfig::new()
            .with_api_key("unused")
            .with_api_base(api_base);

        Self {
            client: Client::with_config(config),
        }
    }

    pub async fn create_embeddings(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let request = CreateEmbeddingRequestArgs::default()
            .model(constants::DEFAULT_EMBEDDING_MODEL)
            .input(texts)
            .build()
            .map_err(|e| format!("Failed to build embedding request: {}", e))?;

        let response = self.client
            .embeddings()
            .create(request).await
            .map_err(|e| format!("Failed to create embeddings: {}", e))?;

        let embeddings = response.data
            .into_iter()
            .map(|item| item.embedding)
            .collect();

        Ok(embeddings)
    }

    pub async fn create_single_embedding(&self, text: String) -> Result<Vec<f32>, String> {
        let embeddings = self.create_embeddings(vec![text]).await?;
        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| "No embedding returned".to_string())
    }
}

#[tauri::command]
pub async fn create_document_embeddings(documents: Vec<Document>) -> Result<Vec<Document>, String> {
    if documents.is_empty() {
        return Ok(documents);
    }

    let embedding_service = EmbeddingService::new();

    let texts: Vec<String> = documents
        .iter()
        .map(|doc| doc.content.clone())
        .collect();

    let embeddings = embedding_service.create_embeddings(texts).await?;

    let mut updated_docs = documents;
    for (i, embedding) in embeddings.into_iter().enumerate() {
        if let Some(doc) = updated_docs.get_mut(i) {
            doc.embedding = Some(embedding);
        }
    }

    Ok(updated_docs)
}

#[tauri::command]
pub async fn create_query_embedding(query: String) -> Result<Vec<f32>, String> {
    let embedding_service = EmbeddingService::new();
    embedding_service.create_single_embedding(query).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_embedding_service_creation() {
        let _service = EmbeddingService::new();
        // Just test that the service can be created
        assert!(true);
    }
}
