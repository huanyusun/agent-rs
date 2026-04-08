use crate::{
    config::AppConfig,
    domain::{chunk::Chunk, document::Document, section::Section, workspace::Workspace},
    error::{AppError, Result},
    index::simple_index::SimpleIndexData,
    ingest::IngestedDocument,
};
use chrono::Utc;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct WorkspaceCorpus {
    pub documents: Vec<Document>,
    pub sections: Vec<Section>,
    pub chunks: Vec<Chunk>,
}

/// WorkspaceStore owns the on-disk contract for the MVP.
///
/// Why this design:
/// - The repository needs observable flows, so storing JSON and copied source files per workspace is
///   easier to inspect than hiding state behind a database immediately.
/// - This module centralizes filesystem layout so ingest, retrieval, and synthesis do not hardcode
///   directory rules.
/// - An alternative would be SQLite from day one, but that would make schema iteration slower while
///   the document pipeline is still changing.
/// - Current limitation: writes are coarse-grained and rewrite whole JSON files instead of using
///   incremental updates.
pub struct WorkspaceStore {
    config: AppConfig,
}

impl WorkspaceStore {
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    pub fn create_workspace(&self, name: &str) -> Result<Workspace> {
        let root = self.config.app.workspace_root.join(name);
        fs::create_dir_all(root.join("documents"))?;
        fs::create_dir_all(root.join("chunks"))?;
        fs::create_dir_all(root.join("index"))?;
        fs::create_dir_all(root.join("outputs"))?;
        fs::create_dir_all(root.join("logs"))?;

        let workspace = Workspace {
            name: name.to_string(),
            root_dir: root,
            created_at: Utc::now(),
            document_ids: Vec::new(),
        };
        self.save_workspace(&workspace)?;
        self.write_active_workspace_name(name)?;
        Ok(workspace)
    }

    pub fn set_active_workspace(&self, name: &str) -> Result<Workspace> {
        let root = self.config.app.workspace_root.join(name);
        let workspace = self.load_workspace_from_root(&root)?;
        self.write_active_workspace_name(name)?;
        Ok(workspace)
    }

    pub fn load_active_workspace(&self) -> Result<Workspace> {
        let name = fs::read_to_string(&self.config.app.active_workspace_file).map_err(|_| {
            AppError::Workspace(
                "no active workspace configured; run `cargo run -- workspace create <name>` first"
                    .into(),
            )
        })?;
        let root = self.config.app.workspace_root.join(name.trim());
        self.load_workspace_from_root(&root)
    }

    pub fn save_workspace(&self, workspace: &Workspace) -> Result<()> {
        fs::create_dir_all(&workspace.root_dir)?;
        let path = workspace.root_dir.join("workspace.json");
        fs::write(path, serde_json::to_vec_pretty(workspace)?)?;
        Ok(())
    }

    pub fn persist_ingested_document(
        &self,
        workspace: &Workspace,
        ingested: &IngestedDocument,
    ) -> Result<()> {
        let documents_dir = workspace.root_dir.join("documents");
        let chunks_dir = workspace.root_dir.join("chunks");
        fs::create_dir_all(&documents_dir)?;
        fs::create_dir_all(&chunks_dir)?;

        let asset_name = format!(
            "{}-{}.{}",
            ingested.document.id,
            sanitize_file_name(&ingested.original_file_name),
            ingested.original_extension
        );
        let stored_asset_path = documents_dir.join(asset_name);
        fs::write(&stored_asset_path, &ingested.original_bytes)?;

        let mut document = ingested.document.clone();
        document.stored_path = stored_asset_path;
        fs::write(
            documents_dir.join(format!("{}.json", document.id)),
            serde_json::to_vec_pretty(&document)?,
        )?;

        #[derive(Serialize)]
        struct ChunkBundle<'a> {
            sections: &'a [Section],
            chunks: &'a [Chunk],
        }

        fs::write(
            chunks_dir.join(format!("{}.json", document.id)),
            serde_json::to_vec_pretty(&ChunkBundle {
                sections: &ingested.sections,
                chunks: &ingested.chunks,
            })?,
        )?;

        Ok(())
    }

    pub fn load_index(&self, workspace: &Workspace) -> Result<SimpleIndexData> {
        let path = workspace.root_dir.join("index/index.json");
        if path.exists() {
            Ok(serde_json::from_slice(&fs::read(path)?)?)
        } else {
            Ok(SimpleIndexData::default())
        }
    }

    pub fn save_index(&self, workspace: &Workspace, index: &SimpleIndexData) -> Result<()> {
        fs::create_dir_all(workspace.root_dir.join("index"))?;
        fs::write(
            workspace.root_dir.join("index/index.json"),
            serde_json::to_vec_pretty(index)?,
        )?;
        Ok(())
    }

    pub fn load_corpus(&self, workspace: &Workspace) -> Result<WorkspaceCorpus> {
        let mut documents = Vec::new();
        let mut sections = Vec::new();
        let mut chunks = Vec::new();

        let documents_dir = workspace.root_dir.join("documents");
        if documents_dir.exists() {
            for entry in fs::read_dir(&documents_dir)? {
                let path = entry?.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                    documents.push(serde_json::from_slice(&fs::read(path)?)?);
                }
            }
        }

        let chunks_dir = workspace.root_dir.join("chunks");
        if chunks_dir.exists() {
            for entry in fs::read_dir(&chunks_dir)? {
                let path = entry?.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                    #[derive(serde::Deserialize)]
                    struct ChunkBundle {
                        sections: Vec<Section>,
                        chunks: Vec<Chunk>,
                    }
                    let bundle: ChunkBundle = serde_json::from_slice(&fs::read(path)?)?;
                    sections.extend(bundle.sections);
                    chunks.extend(bundle.chunks);
                }
            }
        }

        Ok(WorkspaceCorpus {
            documents,
            sections,
            chunks,
        })
    }

    pub fn save_output(
        &self,
        workspace: &Workspace,
        prefix: &str,
        slug: String,
        content: &str,
    ) -> Result<PathBuf> {
        let path = workspace
            .root_dir
            .join("outputs")
            .join(format!("{}-{}.md", prefix, slug));
        fs::create_dir_all(workspace.root_dir.join("outputs"))?;
        fs::write(&path, content)?;
        Ok(path)
    }

    pub fn append_log(&self, workspace: &Workspace, event: &str, detail: &str) -> Result<()> {
        fs::create_dir_all(workspace.root_dir.join("logs"))?;
        let line = serde_json::json!({
            "timestamp": Utc::now(),
            "event": event,
            "detail": detail,
        });
        let path = workspace.root_dir.join("logs/activity.jsonl");
        let mut existing = if path.exists() {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };
        existing.push_str(&format!("{line}\n"));
        fs::write(path, existing)?;
        Ok(())
    }

    fn write_active_workspace_name(&self, name: &str) -> Result<()> {
        if let Some(parent) = self.config.app.active_workspace_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.config.app.active_workspace_file, name)?;
        Ok(())
    }

    fn load_workspace_from_root(&self, root: &Path) -> Result<Workspace> {
        let path = root.join("workspace.json");
        if !path.exists() {
            return Err(AppError::Workspace(format!(
                "workspace metadata not found at {}",
                path.display()
            )));
        }
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }
}

fn sanitize_file_name(input: &str) -> String {
    input
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}
