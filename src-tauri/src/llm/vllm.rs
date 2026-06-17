use crate::llm::openai_compat;
use crate::llm::{ChatChunk, ChatRequest, LlmProvider};
use async_trait::async_trait;
use tokio::sync::mpsc;

pub struct VllmProvider {
    pub base_url: String,
    pub api_key: Option<String>,
    pub http: reqwest::Client,
}

#[async_trait]
impl LlmProvider for VllmProvider {
    fn id(&self) -> &str {
        "vllm"
    }
    async fn chat_stream(
        &self,
        req: ChatRequest,
        sink: mpsc::Sender<anyhow::Result<ChatChunk>>,
    ) -> anyhow::Result<()> {
        openai_compat::stream_chat(
            &self.http,
            &self.base_url,
            self.api_key.as_deref(),
            req,
            sink,
        )
        .await
    }
}
