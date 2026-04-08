use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml deserialize error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("workspace error: {0}")]
    Workspace(String),
    #[error("ingest error: {0}")]
    Ingest(String),
    #[error("index error: {0}")]
    Index(String),
    #[error("llm error: {0}")]
    Llm(String),
}
