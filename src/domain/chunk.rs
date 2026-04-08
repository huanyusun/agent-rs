use serde::{Deserialize, Serialize};

/// Chunk is the retrieval unit used for indexing and ranking.
///
/// Why this design:
/// - We keep a two-layer model: sections preserve document meaning for citations, while chunks keep
///   retrieval windows small enough for ranking and later LLM prompts.
/// - Each chunk points back to its parent section so retrieval can rank fine-grained text and still
///   report results at a human-readable section boundary.
/// - An alternative would be section-only retrieval, but long sections dilute relevance and create
///   unstable answer context.
/// - Current limitation: chunking is size-based with overlap, so it does not yet understand tables,
///   figure captions, or page geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub document_id: String,
    pub section_id: String,
    pub ordinal: usize,
    pub text: String,
    pub token_count: usize,
    pub keywords: Vec<String>,
}
