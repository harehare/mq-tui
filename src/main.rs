use clap::Parser;
use miette::{IntoDiagnostic, miette};
use mq_tui::App;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "mq_tui")]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "TUI for mq, a jq-like Markdown processing tool", long_about = None)]
#[command(after_help = "Examples:\n\n
    Open a Markdown file:\n
    $ mq_tui README.md\n\n
    Use with mq CLI:\n
    $ mq tui file.md")]
struct Cli {
    /// Path to the Markdown file to open
    #[arg(value_name = "FILE")]
    file_path: Option<PathBuf>,
}

fn main() -> miette::Result<()> {
    let cli = Cli::parse();

    let file_path = cli.file_path.ok_or_else(|| {
        miette!("No file path provided.\nUsage: mq_tui <FILE>\nFor more information, try '--help'")
    })?;

    // Read from file
    let content = fs::read_to_string(&file_path).into_diagnostic()?;
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file.md")
        .to_string();

    // Create and run the app
    let mut app = App::with_file(content, filename);
    app.run()?;

    Ok(())
}
