use crate::error::Result;

/// Markdown ingest keeps the raw text because heading markers are useful to the structure parser.
///
/// Why this design:
/// - Preserving `#` headings gives the parser a reliable structural signal without needing a full
///   markdown AST in the MVP.
/// - A richer alternative would parse links, code blocks, and tables separately, but that would add
///   substantial complexity before the retrieval loop is proven.
/// - Current limitation: inline markdown formatting is left in place and may appear in excerpts.
pub fn extract_text(bytes: &[u8]) -> Result<String> {
    Ok(String::from_utf8_lossy(bytes).into_owned())
}
