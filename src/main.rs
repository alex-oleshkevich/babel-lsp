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

#[derive(Parser)]
#[command(
    name = "babel-lsp",
    version,
    about = "Language server for Python Babel i18n"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Lsp,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Lsp => run_lsp().await,
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
