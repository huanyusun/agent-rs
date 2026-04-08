use crate::error::Result;
use async_trait::async_trait;

/// PromptContext is the normalized contract between retrieval/report assembly and LLM providers.
///
/// Why this design:
/// - Providers should not need to know whether evidence came from chunk retrieval or a report flow;
///   they only consume bounded, preformatted evidence packets.
/// - For `ask`, each evidence entry is a retrieved chunk labeled with document and section names.
/// - For `report`, each evidence entry is a section-level packet because reports summarize the
///   whole workspace rather than a retrieval result set.
#[derive(Debug, Clone)]
pub struct PromptContext {
    pub workspace_name: String,
    pub objective: String,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub summary: String,
    pub inference: String,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn generate(&self, prompt: &PromptContext) -> Result<LlmResponse>;
}
