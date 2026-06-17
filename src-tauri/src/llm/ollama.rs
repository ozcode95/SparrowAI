use crate::llm::openai_compat;
use crate::llm::{ChatChunk, ChatRequest, LlmProvider};
use async_trait::async_trait;
use tokio::sync::mpsc;

pub struct OllamaProvider {
    pub base_url: String,
    pub http: reqwest::Client,
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn id(&self) -> &str {
        "ollama"
    }
    async fn chat_stream(
        &self,
        req: ChatRequest,
        sink: mpsc::Sender<anyhow::Result<ChatChunk>>,
    ) -> anyhow::Result<()> {
        // Ollama exposes `/v1/chat/completions` (OpenAI shim).
        openai_compat::stream_chat(&self.http, &self.base_url, None, req, sink).await
    }
}
