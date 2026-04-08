use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "research-harness",
    version,
    about = "Research Harness MVP for local document ingestion and question-driven analysis"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Workspace(WorkspaceArgs),
    Add(AddArgs),
    Ask(AskArgs),
    Report(ReportArgs),
}

#[derive(Debug, Args)]
pub struct WorkspaceArgs {
    #[command(subcommand)]
    pub command: WorkspaceCommand,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    Create { name: String },
    Use { name: String },
    Show,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    pub path: PathBuf,
}

#[derive(Debug, Args)]
pub struct AskArgs {
    pub query: String,
}

#[derive(Debug, Args)]
pub struct ReportArgs {
    #[command(subcommand)]
    pub command: ReportCommand,
}

#[derive(Debug, Subcommand)]
pub enum ReportCommand {
    Summary,
    Compare,
    Outline,
}
