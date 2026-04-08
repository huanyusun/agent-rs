use serde::{Deserialize, Serialize};

/// Section captures author-facing structure such as headings and hierarchy.
///
/// Why this design:
/// - Research output needs human-readable provenance, and sections are a better citation anchor
///   than raw chunks because users think in headings, not token windows.
/// - We keep both `level` and `ordinal_path` so the system can reconstruct nesting even when two
///   headings share the same title.
/// - An alternative would be to cite pages only, but that loses semantic structure for markdown
///   and plain text sources.
/// - Current limitation: PDF sections are inferred heuristically from extracted text, so heading
///   boundaries can be noisy for scanned or heavily formatted files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub document_id: String,
    pub heading: String,
    pub level: usize,
    pub ordinal_path: Vec<usize>,
    pub parent_id: Option<String>,
    pub content: String,
    pub chunk_ids: Vec<String>,
}
