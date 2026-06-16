mod catalog;
mod cli;
mod config;
mod extract;
mod features;
mod server;
mod state;
mod util;

use clap::{Parser, Subcommand};
use server::Backend;
use tower_lsp_server::{LspService, Server};

use cli::check::{CheckArgs, run_check};

#[derive(Parser)]
#[command(
    name = "babel-lsp",
    version,
    about = "Language server for Python Babel i18n"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the language server over stdio
    Lsp,
    /// Run headless diagnostics (CI linter)
    Check(CheckArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Lsp) {
        Command::Lsp => run_lsp().await,
        Command::Check(args) => std::process::exit(run_check(args)),
    }
}

async fn run_lsp() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
