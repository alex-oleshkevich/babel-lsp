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
use cli::pybabel_cmd::{
    CompileArgs, ExtractArgs, UpdateArgs, run_compile, run_extract, run_update,
};
use cli::stats::{StatsArgs, run_stats};

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
    /// Print per-locale translation coverage
    Stats(StatsArgs),
    /// Extract messages into a POT template via pybabel
    Extract(ExtractArgs),
    /// Merge POT template into locale PO files via pybabel
    Update(UpdateArgs),
    /// Compile PO files to binary MO files via pybabel
    Compile(CompileArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Lsp) {
        Command::Lsp => run_lsp().await,
        Command::Check(args) => std::process::exit(run_check(args)),
        Command::Stats(args) => std::process::exit(run_stats(args)),
        Command::Extract(args) => std::process::exit(run_extract(args)),
        Command::Update(args) => std::process::exit(run_update(args)),
        Command::Compile(args) => std::process::exit(run_compile(args)),
    }
}

async fn run_lsp() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
