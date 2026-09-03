mod clients;
mod config;
mod cursor;
mod doc_cache;
mod gui;
mod i18n;
mod icon;
mod install;
mod jobs;
mod linux_desktop;
mod llm;
mod mcp;
mod repo_cache;
mod safe_fs;
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
    /// Install/update a stable per-user LLM2MCP executable and print its path.
    Install,
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
        Some(Command::Install) => {
            println!("{}", install::ensure_stable_install()?.display());
            Ok(())
        }
        Some(Command::JobWorker { job_id }) => mcp::run_job_worker(&job_id),
        None => run_gui(),
    }
}

fn run_gui() -> Result<()> {
    #[cfg(target_os = "linux")]
    match install::ensure_stable_install() {
        Ok(executable) => {
            if let Err(error) = linux_desktop::register_user_desktop_entry(&executable) {
                eprintln!("LLM2MCP: failed to register Linux desktop identity: {error:#}");
            }
        }
        Err(error) => {
            eprintln!("LLM2MCP: failed to refresh stable executable for desktop entry: {error:#}");
        }
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_app_id("llm2mcp")
            .with_inner_size([1280.0, 900.0])
            .with_icon(icon::app_icon()),
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "LLM2MCP",
        options,
        Box::new(|cc| {
            gui::install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(gui::Llm2McpApp::new(cc)))
        }),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}
