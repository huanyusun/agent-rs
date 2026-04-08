use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Workspace groups documents, derived artifacts, and outputs under one research context.
///
/// Why this design:
/// - NotebookLM-style work is about sets of documents, so the workspace is the top-level boundary
///   for ingestion, retrieval, and reporting.
/// - We store only stable ids here and let the store load documents and chunks from dedicated files;
///   this keeps workspace metadata small and resilient to future schema growth.
/// - An alternative would be one SQLite database from day one, but plain files are easier to inspect
///   during MVP development and match the requirement for observable flows.
/// - Current limitation: there is only one active workspace pointer, so concurrent workspace usage
///   is intentionally simple rather than multi-user aware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub root_dir: PathBuf,
    pub created_at: DateTime<Utc>,
    pub document_ids: Vec<String>,
}
