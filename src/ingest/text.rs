use crate::error::Result;

/// Plain text ingest is intentionally minimal because txt files already represent the normalized
/// format that the parser expects.
///
/// Why this design:
/// - Keeping txt ingest trivial reduces the chance of accidental transformations that would break
///   citations or retrieval scores.
/// - An alternative would normalize whitespace aggressively, but that makes excerpts less faithful.
/// - Current limitation: text files without obvious headings become a single synthetic section.
pub fn extract_text(bytes: &[u8]) -> Result<String> {
    Ok(String::from_utf8_lossy(bytes).into_owned())
}
