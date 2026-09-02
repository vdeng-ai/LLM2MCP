mod clients;
mod config;
mod cursor;
mod doc_cache;
mod gui;
mod i18n;
mod jobs;
mod llm;
mod mcp;
mod workspace;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "llm2mcp",
    version,
    about = "Turn an OpenAI-compatible LLM into a local MCP sidecar for AI coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run as a stdio MCP server. Defaults to the current working directory.
    Mcp {
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Internal durable background worker for asynchronous MCP jobs.
    #[command(hide = true)]
    JobWorker {
        #[arg(long)]
        job_id: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Mcp { workspace }) => {
            let workspace = workspace.unwrap_or(std::env::current_dir()?);
            mcp::run(&workspace)
        }
        Some(Command::JobWorker { job_id }) => mcp::run_job_worker(&job_id),
        None => run_gui(),
    }
}

fn run_gui() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([720.0, 820.0])
            .with_min_inner_size([560.0, 660.0]),
        ..Default::default()
    };
    eframe::run_native(
        "LLM2MCP",
        options,
        Box::new(|cc| {
            gui::install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(gui::Llm2McpApp::new()))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}
