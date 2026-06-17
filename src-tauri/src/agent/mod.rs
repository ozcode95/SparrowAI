//! Agent loop (ReAct-style). Phase-4 placeholder.
//!
//! Outline:
//!   1. Build messages = [system_prompt, recalled_memory..., conversation...].
//!   2. Ask the LLM for either a final answer or a tool call (JSON schema).
//!   3. If tool call → run via `mcp::registry`, append result, loop.
//!   4. Cap at `settings.agent_max_iterations`.

use anyhow::Result;

pub struct AgentRequest {
    pub conversation_id: String,
    pub user_message_id: String,
    pub max_iterations: u32,
}

pub async fn run(_req: AgentRequest) -> Result<()> {
    // TODO: implement.
    Ok(())
}
