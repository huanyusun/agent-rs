use crate::{
    cli::{
        AddArgs, AskArgs, Cli, Commands, ReportArgs, ReportCommand, WorkspaceArgs, WorkspaceCommand,
    },
    config::AppConfig,
    error::{AppError, Result},
    index::simple_index::SimpleIndex,
    ingest::DocumentIngestor,
    llm::{mock::MockLlmProvider, openai::OpenAiProvider, provider::LlmProvider},
    logging,
    retrieval::retriever::{RetrievalRequest, Retriever},
    storage::workspace_store::WorkspaceStore,
    synthesis::synthesizer::{ReportKind, Synthesizer},
};
use std::sync::Arc;
use tracing_appender::non_blocking::WorkerGuard;

pub struct App {
    config: AppConfig,
    _log_guard: WorkerGuard,
    store: WorkspaceStore,
    ingestor: DocumentIngestor,
    retriever: Retriever,
    synthesizer: Synthesizer,
}

impl App {
    pub async fn bootstrap() -> Result<Self> {
        let config = AppConfig::load()?;
        let log_guard = logging::init_logging(&config)?;
        let store = WorkspaceStore::new(config.clone());
        let llm = build_llm_provider(&config);
        let index = SimpleIndex::new(config.index.clone());
        let retriever = Retriever::new(index, config.index.top_k);
        let synthesizer = Synthesizer::new(llm);
        let ingestor = DocumentIngestor::new(config.index.clone());

        Ok(Self {
            config,
            _log_guard: log_guard,
            store,
            ingestor,
            retriever,
            synthesizer,
        })
    }

    pub async fn run(&self, cli: Cli) -> Result<()> {
        match cli.command {
            Commands::Workspace(args) => self.run_workspace(args).await,
            Commands::Add(args) => self.run_add(args).await,
            Commands::Ask(args) => self.run_ask(args).await,
            Commands::Report(args) => self.run_report(args).await,
        }
    }

    async fn run_workspace(&self, args: WorkspaceArgs) -> Result<()> {
        match args.command {
            WorkspaceCommand::Create { name } => {
                let workspace = self.store.create_workspace(&name)?;
                println!(
                    "workspace={} root={}",
                    workspace.name,
                    workspace.root_dir.display()
                );
                Ok(())
            }
            WorkspaceCommand::Use { name } => {
                let workspace = self.store.set_active_workspace(&name)?;
                println!(
                    "workspace={} root={}",
                    workspace.name,
                    workspace.root_dir.display()
                );
                Ok(())
            }
            WorkspaceCommand::Show => {
                let workspace = self.store.load_active_workspace()?;
                println!(
                    "workspace={} documents={} root={}",
                    workspace.name,
                    workspace.document_ids.len(),
                    workspace.root_dir.display()
                );
                Ok(())
            }
        }
    }

    async fn run_add(&self, args: AddArgs) -> Result<()> {
        let mut workspace = self.store.load_active_workspace()?;
        let parsed = self.ingestor.ingest(&args.path, &workspace)?;
        let index = self.store.load_index(&workspace)?;
        let updated_index = self.retriever.rebuild_index(index, &parsed.chunks);

        self.store.persist_ingested_document(&workspace, &parsed)?;
        self.store.save_index(&workspace, &updated_index)?;
        workspace.document_ids.push(parsed.document.id.clone());
        self.store.save_workspace(&workspace)?;
        self.store.append_log(
            &workspace,
            "document_added",
            &format!(
                "document={} sections={} chunks={} source={}",
                parsed.document.title,
                parsed.sections.len(),
                parsed.chunks.len(),
                args.path.display()
            ),
        )?;

        println!(
            "workspace={} added={} sections={} chunks={}",
            workspace.name,
            parsed.document.title,
            parsed.sections.len(),
            parsed.chunks.len()
        );
        Ok(())
    }

    async fn run_ask(&self, args: AskArgs) -> Result<()> {
        let workspace = self.store.load_active_workspace()?;
        let index = self.store.load_index(&workspace)?;
        let corpus = self.store.load_corpus(&workspace)?;
        let request = RetrievalRequest {
            query: args.query.clone(),
            top_k: self.config.index.top_k,
        };
        let mut hits = self.retriever.retrieve(&request, &index, &corpus)?;
        if is_summary_query(&args.query) {
            hits = self.retriever.augment_with_summary_leads(hits, &corpus, 3);
        }
        let answer = self
            .synthesizer
            .answer_question(&workspace, &args.query, &hits)
            .await?;
        let output_path =
            self.store
                .save_output(&workspace, "ask", sanitize_file_name(&args.query), &answer)?;
        self.store.append_log(
            &workspace,
            "ask_completed",
            &format!(
                "query={} hits={} output={}",
                args.query,
                hits.len(),
                output_path.display()
            ),
        )?;

        println!("{answer}");
        println!("output_file={}", output_path.display());
        Ok(())
    }

    async fn run_report(&self, args: ReportArgs) -> Result<()> {
        let workspace = self.store.load_active_workspace()?;
        let corpus = self.store.load_corpus(&workspace)?;
        if corpus.documents.is_empty() {
            return Err(AppError::Workspace(
                "active workspace has no documents; run `cargo run -- add <path>` first".into(),
            ));
        }

        let (kind, slug) = match args.command {
            ReportCommand::Summary => (ReportKind::Summary, "summary"),
            ReportCommand::Compare => (ReportKind::Compare, "compare"),
            ReportCommand::Outline => (ReportKind::Outline, "outline"),
        };

        let content = self
            .synthesizer
            .generate_report(&workspace, &corpus, kind)
            .await?;
        let output_path =
            self.store
                .save_output(&workspace, "report", slug.to_string(), &content)?;
        self.store.append_log(
            &workspace,
            "report_completed",
            &format!("kind={slug} output={}", output_path.display()),
        )?;

        println!("{content}");
        println!("output_file={}", output_path.display());
        Ok(())
    }
}

fn build_llm_provider(config: &AppConfig) -> Arc<dyn LlmProvider> {
    match config.llm.provider.as_str() {
        "openai" => Arc::new(OpenAiProvider::new(config.llm.clone())),
        _ => Arc::new(MockLlmProvider::new(config.llm.model.clone())),
    }
}

fn sanitize_file_name(input: &str) -> String {
    let mut slug = input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let trimmed = slug.trim_matches('-').chars().take(48).collect::<String>();
    if trimmed.is_empty() {
        "query".to_string()
    } else {
        trimmed
    }
}

fn is_summary_query(query: &str) -> bool {
    let normalized = query.to_ascii_lowercase();
    let english_markers = [
        "summary",
        "summarize",
        "main purpose",
        "main topic",
        "what is this document about",
        "what is this book about",
    ];
    if english_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return true;
    }

    let chinese_markers = [
        "核心主题",
        "主要内容",
        "讲什么",
        "主要讲",
        "主旨",
        "概要",
        "概述",
        "总结",
    ];
    chinese_markers.iter().any(|marker| query.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{ReportCommand, WorkspaceCommand};
    use tempfile::TempDir;

    #[test]
    fn summary_query_detection_covers_chinese_and_english() {
        assert!(is_summary_query("这本书的核心主题是什么？"));
        assert!(is_summary_query("What is this document about?"));
        assert!(!is_summary_query("殷墟人祭坑有多少层？"));
    }

    #[tokio::test]
    async fn create_add_and_ask_flow_runs() {
        let temp = TempDir::new().unwrap();
        std::env::set_current_dir(temp.path()).unwrap();
        std::fs::create_dir_all("config").unwrap();
        std::fs::write(
            "config/default.toml",
            r#"
[app]
name = "research-harness"
workspace_root = "workspace"
active_workspace_file = ".research-harness/active-workspace"

[llm]
provider = "mock"
model = "mock-research"
timeout_secs = 30

[llm.openai]
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[index]
chunk_size = 200
chunk_overlap = 40
embedding_dimensions = 16
top_k = 4
"#,
        )
        .unwrap();
        std::fs::create_dir_all("examples").unwrap();
        std::fs::write(
            "examples/demo.md",
            "# Demo\n\n## Core Idea\n\nResearch Harness helps inspect documents with citations.\n",
        )
        .unwrap();

        let app = App::bootstrap().await.unwrap();
        app.run(Cli {
            command: Commands::Workspace(WorkspaceArgs {
                command: WorkspaceCommand::Create {
                    name: "demo".into(),
                },
            }),
        })
        .await
        .unwrap();
        app.run(Cli {
            command: Commands::Add(AddArgs {
                path: std::path::PathBuf::from("examples/demo.md"),
            }),
        })
        .await
        .unwrap();
        app.run(Cli {
            command: Commands::Ask(AskArgs {
                query: "What is the core idea?".into(),
            }),
        })
        .await
        .unwrap();
        app.run(Cli {
            command: Commands::Report(ReportArgs {
                command: ReportCommand::Summary,
            }),
        })
        .await
        .unwrap();

        let workspace = app.store.load_active_workspace().unwrap();
        assert_eq!(workspace.name, "demo");
    }
}
