use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Document models one imported source file inside a workspace.
///
/// Why this design:
/// - The system must reason over a stable artifact after import, so we store both the original
///   source path and the copied workspace path instead of relying on the caller's filesystem.
/// - Sections and chunks are referenced by id rather than embedded to keep persistence simple and
///   allow rebuilding indexes without rewriting the full document record.
/// - An alternative would be to keep one giant nested JSON blob per document, but that makes
///   incremental updates and debugging harder in an MVP.
/// - Current limitation: metadata is intentionally small and does not yet capture author, page
///   numbers, or OCR confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub source_path: PathBuf,
    pub stored_path: PathBuf,
    pub media_type: String,
    pub imported_at: DateTime<Utc>,
    pub section_ids: Vec<String>,
    pub chunk_ids: Vec<String>,
}
