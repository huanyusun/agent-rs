use crate::error::{AppError, Result};
use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub app: AppSettings,
    pub llm: LlmConfig,
    pub index: IndexConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppSettings {
    pub name: String,
    pub workspace_root: PathBuf,
    pub active_workspace_file: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub timeout_secs: u64,
    pub openai: OpenAiConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key_env: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexConfig {
    pub chunk_size: usize,
    pub chunk_overlap: usize,
    pub embedding_dimensions: usize,
    pub top_k: usize,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok();
        let config_path = Path::new("config/default.toml");
        let raw = fs::read_to_string(config_path)?;
        let mut config: Self = toml::from_str(&raw)?;
        config.normalize_paths(env::current_dir()?);
        config.validate()?;
        Ok(config)
    }

    fn normalize_paths(&mut self, root: PathBuf) {
        self.app.workspace_root = join_if_relative(&root, &self.app.workspace_root);
        self.app.active_workspace_file = join_if_relative(&root, &self.app.active_workspace_file);
    }

    fn validate(&self) -> Result<()> {
        if self.app.name.trim().is_empty() {
            return Err(AppError::Config("app.name must not be empty".into()));
        }
        if self.index.chunk_size == 0 {
            return Err(AppError::Config("index.chunk_size must be > 0".into()));
        }
        if self.index.chunk_overlap >= self.index.chunk_size {
            return Err(AppError::Config(
                "index.chunk_overlap must be smaller than index.chunk_size".into(),
            ));
        }
        if self.index.embedding_dimensions == 0 {
            return Err(AppError::Config(
                "index.embedding_dimensions must be > 0".into(),
            ));
        }
        if self.index.top_k == 0 {
            return Err(AppError::Config("index.top_k must be > 0".into()));
        }
        if self.llm.model.trim().is_empty() {
            return Err(AppError::Config("llm.model must not be empty".into()));
        }
        Ok(())
    }
}

fn join_if_relative(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
