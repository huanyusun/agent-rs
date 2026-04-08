use crate::{
    error::Result,
    llm::provider::{LlmProvider, LlmResponse, PromptContext},
};
use async_trait::async_trait;

/// MockLlmProvider keeps the MVP runnable without requiring external credentials.
///
/// Why this design:
/// - The core risk in a research harness is ingest and citation correctness, not model choice, so
///   a deterministic provider is the fastest way to validate the loop locally.
/// - An alternative would be to require OpenAI from day one, but that would make verification
///   depend on network and keys instead of repository code.
/// - Current limitation: summaries are pattern-based and therefore conservative rather than nuanced.
pub struct MockLlmProvider {
    model: String,
}

impl MockLlmProvider {
    pub fn new(model: String) -> Self {
        Self { model }
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn generate(&self, prompt: &PromptContext) -> Result<LlmResponse> {
        let snippets = prompt
            .evidence
            .iter()
            .take(4)
            .map(|entry| entry.lines().skip(1).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>();
        let summary = if snippets.is_empty() {
            format!(
                "Model {} found no evidence in workspace {}.",
                self.model, prompt.workspace_name
            )
        } else {
            format!(
                "Workspace {} is primarily about: {}",
                prompt.workspace_name,
                snippets.join(" | ")
            )
        };
        let inference = format!(
            "This conclusion is a mock synthesis derived from the retrieved evidence for objective: {}",
            prompt.objective
        );

        Ok(LlmResponse { summary, inference })
    }
}
