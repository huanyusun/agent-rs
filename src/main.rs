use clap::Parser;
use research_harness::{app::App, cli::Cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(err) = async {
        let app = App::bootstrap().await?;
        app.run(cli).await
    }
    .await
    {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
