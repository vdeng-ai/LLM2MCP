mod cache;
mod clients;
mod config;
mod control;
mod cursor;
mod doc_cache;
mod doc_edits;
mod doctor;
mod evidence;
mod gui;
mod i18n;
mod icon;
mod install;
mod jobs;
mod linux_desktop;
mod llm;
mod mcp;
mod privacy;
mod primary_result;
mod process;
mod repo_cache;
mod safe_fs;
mod scheduler;
mod search;
mod syntax;
mod updater;
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
    /// Probe real inference, stdio MCP and client registration.
    Doctor {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        profile: Option<String>,
        /// Execute a real analysis and verify durable Tasks, reconnection and cancellation.
        #[arg(long)]
        deep: bool,
        #[arg(long, requires = "deep")]
        workspace: Option<PathBuf>,
        /// A workspace-relative source file used for the opt-in deep analysis.
        #[arg(long, requires = "deep", default_value = "README.md")]
        path: String,
    },
    /// List/export recent jobs, request cancellation or rerun a terminal job.
    Jobs {
        #[arg(long)]
        json: bool,
        #[arg(long, conflicts_with = "cancel")]
        retry: Option<String>,
        #[arg(long)]
        cancel: Option<String>,
    },
    /// Inspect/prune source and map caches, or clear cached data.
    Cache {
        #[arg(long)]
        clear: bool,
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
        Some(Command::Doctor {
            json,
            profile,
            deep,
            workspace,
            path,
        }) => {
            let mut config = config::load()?;
            if profile.is_some() {
                config.active_profile = profile;
            }
            let mut report = doctor::run(&config);
            if deep {
                let workspace = workspace.unwrap_or(std::env::current_dir()?);
                report
                    .checks
                    .push(doctor::deep_check(&config, &workspace, &path));
            }
            println!(
                "{}",
                if json {
                    serde_json::to_string_pretty(&report)?
                } else {
                    report.text()
                }
            );
            if !report.ok() {
                anyhow::bail!("one or more required diagnostics failed");
            }
            Ok(())
        }
        Some(Command::Jobs {
            json,
            retry,
            cancel,
        }) => {
            jobs::recover_stale()?;
            if let Some(id) = retry {
                println!("{}", serde_json::to_string_pretty(&jobs::retry(&id)?)?);
            } else if let Some(id) = cancel {
                println!("{}", serde_json::to_string_pretty(&jobs::cancel(&id)?)?);
            } else {
                let records = jobs::list_recent(200)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&records)?);
                } else {
                    for record in records {
                        println!(
                            "{} {} {} {}",
                            record.id,
                            jobs::state_name(record.state),
                            record.tool,
                            record.stage
                        );
                    }
                }
            }
            Ok(())
        }
        Some(Command::Cache { clear }) => {
            let config = config::load()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&cache::maintain(
                    config.cache_max_mib,
                    config.cache_ttl_days,
                    clear
                )?)?
            );
            Ok(())
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
