use crate::{
    domain::workspace::Workspace,
    error::Result,
    llm::provider::{LlmProvider, PromptContext},
    retrieval::retriever::RetrievalHit,
    storage::workspace_store::WorkspaceCorpus,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub enum ReportKind {
    Summary,
    Compare,
    Outline,
}

/// Synthesizer turns retrieved evidence into answer and report text.
///
/// Why this design:
/// - Synthesis is separated from retrieval so evidence ranking stays deterministic while generation
///   remains swappable through the LLM provider boundary.
/// - The output explicitly separates source-backed facts, system summary, and inference because a
///   research tool must privilege citations over polished but unverifiable answers.
/// - For `ask`, the LLM sees only retrieved chunk text plus document and section labels. This keeps
///   the prompt bounded even when one parsed section contains an entire PDF chapter or book.
/// - An alternative would be direct free-form generation from the whole corpus, but that weakens
///   provenance and makes debugging retrieval quality much harder.
/// - Current limitation: the mock provider uses template-based prose and does not perform deep
///   semantic reasoning across long documents.
pub struct Synthesizer {
    llm: Arc<dyn LlmProvider>,
}

impl Synthesizer {
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self { llm }
    }

    pub async fn answer_question(
        &self,
        workspace: &Workspace,
        query: &str,
        hits: &[RetrievalHit],
    ) -> Result<String> {
        // `ask` uses retrieved chunk bodies as evidence. Section metadata is included only as a
        // citation anchor, not as a signal to re-expand the full section text into the prompt.
        let context = PromptContext {
            workspace_name: workspace.name.clone(),
            objective: format!("Answer the research question: {query}"),
            evidence: hits
                .iter()
                .map(|hit| {
                    format!(
                        "[{} :: {}]\n{}",
                        hit.document.title, hit.section.heading, hit.chunk.text
                    )
                })
                .collect(),
        };
        let response = self.llm.generate(&context).await?;

        let citations = hits
            .iter()
            .map(|hit| {
                format!(
                    "- {} / {}: {}",
                    hit.citation.document_title, hit.citation.section_heading, hit.citation.excerpt
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(format!(
            "Question: {query}\n\nOriginal Content:\n{original}\n\nSystem Summary:\n{summary}\n\nInference:\n{inference}\n\nCitations:\n{citations}",
            original = list_original_content(hits),
            summary = response.summary,
            inference = response.inference,
        ))
    }

    pub async fn generate_report(
        &self,
        workspace: &Workspace,
        corpus: &WorkspaceCorpus,
        kind: ReportKind,
    ) -> Result<String> {
        let objective = match kind {
            ReportKind::Summary => "Generate a workspace summary with major themes.",
            ReportKind::Compare => {
                "Compare the imported documents and highlight agreements and differences."
            }
            ReportKind::Outline => "Generate a research outline based on the imported documents.",
        };

        // Reports are intentionally corpus-wide outputs, so they still operate over section bodies
        // rather than retrieval hits. This is different from `ask`, which is chunk-retrieval-first.
        let evidence = corpus
            .sections
            .iter()
            .map(|section| {
                let document = corpus
                    .documents
                    .iter()
                    .find(|document| document.id == section.document_id)
                    .expect("section document must exist");
                format!(
                    "[{} :: {}]\n{}",
                    document.title, section.heading, section.content
                )
            })
            .collect::<Vec<_>>();
        let response = self
            .llm
            .generate(&PromptContext {
                workspace_name: workspace.name.clone(),
                objective: objective.into(),
                evidence,
            })
            .await?;

        let citation_lines = corpus
            .sections
            .iter()
            .map(|section| {
                let document = corpus
                    .documents
                    .iter()
                    .find(|document| document.id == section.document_id)
                    .expect("section document must exist");
                format!("- {} / {}", document.title, section.heading)
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(format!(
            "Workspace: {}\nReport: {:?}\n\nSystem Summary:\n{}\n\nInference:\n{}\n\nCitation Map:\n{}",
            workspace.name, kind, response.summary, response.inference, citation_lines
        ))
    }
}

fn list_original_content(hits: &[RetrievalHit]) -> String {
    hits.iter()
        .map(|hit| {
            format!(
                "- [{} / {}] {}",
                hit.document.title, hit.section.heading, hit.citation.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
