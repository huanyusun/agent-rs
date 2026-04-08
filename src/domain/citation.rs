use serde::{Deserialize, Serialize};

/// Citation is the contract between synthesis and trust.
///
/// Why this design:
/// - In a research harness, traceability matters more than fluent prose, so every answer path
///   should carry enough metadata to show where a claim came from.
/// - We keep a short excerpt because users need a quick verification hook without reopening the
///   entire source file.
/// - An alternative would be to emit chunk ids only, but that forces manual lookups and makes the
///   CLI output harder to audit.
/// - Current limitation: citations reference document and section names, not page numbers, because
///   the MVP parser does not guarantee stable page mapping for PDFs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub document_title: String,
    pub section_heading: String,
    pub chunk_id: String,
    pub excerpt: String,
}
