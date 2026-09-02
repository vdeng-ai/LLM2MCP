use eframe::egui;
use std::{fs, path::Path};

use crate::{
    clients::{self, ClientKind, ClientStatus},
    config::{
        self, AppConfig, ExecutionMode, Language, ReasoningEffort, ReasoningTransport, ToolConfig,
    },
    i18n, jobs, llm,
};

pub struct Llm2McpApp {
    config: AppConfig,
    status: String,
    api_key_visible: bool,
    clients: Vec<ClientStatus>,
    jobs: Vec<jobs::JobRecord>,
}

impl Llm2McpApp {
    pub fn new() -> Self {
        let config = config::load().unwrap_or_default();
        let ready = i18n::texts(config.language).ready.to_owned();
        Self {
            config,
            status: ready,
            api_key_visible: false,
            clients: clients::statuses(),
            jobs: jobs::list_recent(40).unwrap_or_default(),
        }
    }

    fn refresh_clients(&mut self) {
        self.clients = clients::statuses();
    }

    fn refresh_jobs(&mut self) {
        self.jobs = jobs::list_recent(40).unwrap_or_default();
    }

    fn save(&mut self) {
        let text = i18n::texts(self.config.language);
        self.status = match config::save(&self.config) {
            Ok(()) => text.config_saved.to_owned(),
            Err(error) => format!("{}: {error:#}", text.save_failed),
        };
    }

    fn test(&mut self) {
        self.save();
        let text = i18n::texts(self.config.language);
        self.status = match llm::test_connection(&self.config) {
            Ok(message) => message,
            Err(error) => format!("{}: {error:#}", text.connection_failed),
        };
    }

    fn install_client(&mut self, kind: ClientKind) {
        self.save();
        self.status = match clients::install(kind) {
            Ok(detail) => format!("{}: {detail}", kind.label()),
            Err(error) => format!("{}: {error:#}", kind.label()),
        };
        self.refresh_clients();
    }

    fn remove_client(&mut self, kind: ClientKind) {
        self.status = match clients::remove(kind) {
            Ok(detail) => format!("{}: {detail}", kind.label()),
            Err(error) => format!("{}: {error:#}", kind.label()),
        };
        self.refresh_clients();
    }
}

fn execution_label(mode: ExecutionMode, language: Language, text: &i18n::Texts) -> &'static str {
    match (mode, language) {
        (ExecutionMode::Sync, _) => text.execution_sync,
        (ExecutionMode::Auto, _) => text.execution_auto,
        (ExecutionMode::Async, _) => text.execution_async,
    }
}

fn format_duration(ms: u64) -> String {
    let seconds = ms / 1_000;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", seconds / 3_600, (seconds % 3_600) / 60)
    }
}

fn tool_row(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    tool: &mut ToolConfig,
    default_execution: ExecutionMode,
    language: Language,
    text: &i18n::Texts,
) {
    ui.label(label);
    egui::ComboBox::from_id_salt(format!("{id}_reasoning"))
        .selected_text(tool.reasoning.label())
        .show_ui(ui, |ui| {
            for effort in ReasoningEffort::ALL {
                ui.selectable_value(&mut tool.reasoning, effort, effort.label());
            }
        });
    let mut execution = tool.execution_or(default_execution);
    egui::ComboBox::from_id_salt(format!("{id}_execution"))
        .selected_text(execution_label(execution, language, text))
        .show_ui(ui, |ui| {
            for mode in ExecutionMode::ALL {
                if ui
                    .selectable_value(&mut execution, mode, execution_label(mode, language, text))
                    .changed()
                {
                    tool.execution = Some(execution);
                }
            }
        });
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut tool.max_output_tokens)
                .range(256..=16_000)
                .speed(128),
        );
        ui.label(text.max_output_tokens);
    });
    if let Some(return_tokens) = &mut tool.primary_return_tokens {
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(return_tokens)
                    .range(128..=8_000)
                    .speed(64),
            );
            ui.label("tokens");
        });
    } else {
        ui.label(text.full_artifact);
    }
    ui.end_row();
}

impl eframe::App for Llm2McpApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let text = *i18n::texts(self.config.language);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("LLM2MCP");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(self.config.language.label())
                        .show_ui(ui, |ui| {
                            for language in Language::ALL {
                                if ui
                                    .selectable_value(
                                        &mut self.config.language,
                                        language,
                                        language.label(),
                                    )
                                    .changed()
                                {
                                    let _ = config::save(&self.config);
                                }
                            }
                        });
                    ui.label(text.language);
                });
            });
            ui.label(text.tagline);
            ui.add_space(10.0);

            ui.group(|ui| {
                ui.heading(text.llm_api);
                egui::Grid::new("llm_api_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(text.base_url);
                        ui.text_edit_singleline(&mut self.config.base_url);
                        ui.end_row();

                        ui.label(text.model);
                        ui.text_edit_singleline(&mut self.config.model);
                        ui.end_row();

                        ui.label(text.api_key);
                        ui.horizontal(|ui| {
                            let edit = egui::TextEdit::singleline(&mut self.config.api_key)
                                .password(!self.api_key_visible)
                                .desired_width(260.0);
                            ui.add(edit);
                            ui.checkbox(&mut self.api_key_visible, text.show);
                        });
                        ui.end_row();

                        ui.label(text.reasoning_protocol);
                        egui::ComboBox::from_id_salt("reasoning_transport")
                            .selected_text(self.config.reasoning_transport.label())
                            .show_ui(ui, |ui| {
                                for transport in ReasoningTransport::ALL {
                                    ui.selectable_value(
                                        &mut self.config.reasoning_transport,
                                        transport,
                                        transport.label(),
                                    );
                                }
                            });
                        ui.end_row();
                    });

                ui.horizontal(|ui| {
                    if ui.button(text.save).clicked() {
                        self.save();
                    }
                    if ui.button(text.test_connection).clicked() {
                        self.test();
                    }
                });
            });

            ui.add_space(10.0);
            ui.group(|ui| {
                ui.heading(text.per_tool_reasoning);
                ui.label(text.per_tool_hint);
                egui::Grid::new("tool_grid")
                    .num_columns(5)
                    .spacing([14.0, 8.0])
                    .show(ui, |ui| {
                        ui.strong(text.tool);
                        ui.strong(text.reasoning);
                        ui.strong(text.execution);
                        ui.strong(text.output_budget);
                        ui.strong(text.return_budget);
                        ui.end_row();
                        tool_row(
                            ui,
                            text.analyze,
                            "analyze",
                            &mut self.config.tools.analyze,
                            ExecutionMode::Auto,
                            self.config.language,
                            &text,
                        );
                        tool_row(
                            ui,
                            text.plan,
                            "plan",
                            &mut self.config.tools.plan,
                            ExecutionMode::Auto,
                            self.config.language,
                            &text,
                        );
                        tool_row(
                            ui,
                            text.review_diff,
                            "review",
                            &mut self.config.tools.review_diff,
                            ExecutionMode::Sync,
                            self.config.language,
                            &text,
                        );
                        tool_row(
                            ui,
                            text.document_repo,
                            "document_repo",
                            &mut self.config.tools.document_repo,
                            ExecutionMode::Async,
                            self.config.language,
                            &text,
                        );
                        tool_row(
                            ui,
                            text.update_docs,
                            "update_docs",
                            &mut self.config.tools.update_docs,
                            ExecutionMode::Async,
                            self.config.language,
                            &text,
                        );
                    });
            });

            ui.add_space(10.0);
            ui.group(|ui| {
                ui.heading(text.context_limits);
                ui.horizontal(|ui| {
                    ui.label(text.max_source_tokens);
                    ui.add(
                        egui::DragValue::new(&mut self.config.max_source_tokens)
                            .range(2_500..=500_000),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(text.max_tokens_per_file);
                    ui.add(
                        egui::DragValue::new(&mut self.config.max_file_tokens).range(500..=200_000),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(text.discovery_index_tokens);
                    ui.add(
                        egui::DragValue::new(&mut self.config.discovery_index_tokens)
                            .range(1_000..=50_000),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(text.timeout_seconds);
                    ui.add(egui::DragValue::new(&mut self.config.timeout_secs).range(5..=3600));
                });
                ui.horizontal(|ui| {
                    ui.label(text.document_map_concurrency);
                    ui.add(
                        egui::DragValue::new(&mut self.config.document_map_concurrency)
                            .range(1..=8),
                    );
                });
            });

            ui.add_space(10.0);
            egui::CollapsingHeader::new(text.job_history)
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(text.job_history_hint);
                        if ui.small_button(text.refresh).clicked() {
                            self.refresh_jobs();
                        }
                    });
                    if self.jobs.is_empty() {
                        ui.small(text.no_jobs);
                    } else {
                        let job_rows = self.jobs.clone();
                        egui::ScrollArea::vertical()
                            .id_salt("job_history_scroll")
                            .max_height(240.0)
                            .show(ui, |ui| {
                                for job in job_rows {
                                    let duration = format_duration(jobs::display_duration_ms(&job));
                                    let heading = format!(
                                        "{} · {} · {} · {duration}",
                                        jobs::state_name(job.state),
                                        job.tool,
                                        job.stage
                                    );
                                    ui.collapsing(heading, |ui| {
                                        ui.small(format!("job_id: {}", job.id));
                                        ui.small(format!("workspace: {}", job.workspace.display()));
                                        ui.small(format!(
                                            "{}: {} · {}: prompt={} / completion={}",
                                            text.llm_calls,
                                            job.llm_calls,
                                            text.token_usage,
                                            job.prompt_tokens,
                                            job.completion_tokens
                                        ));
                                        ui.small(format!(
                                            "cache: symbol hit/miss={}/{} · evidence hit/miss={}/{}",
                                            job.symbol_index_hits,
                                            job.symbol_index_misses,
                                            job.evidence_cache_hits,
                                            job.evidence_cache_misses
                                        ));
                                        ui.small(format!("{}: {duration}", text.duration));
                                        ui.small(format!("created: {}", job.created_at));
                                        if let Some(diagnostics) = &job.last_llm_diagnostics {
                                            ui.small(format!("LLM: {diagnostics}"));
                                        }
                                        if let Some(error) = &job.error {
                                            ui.label(error);
                                        }
                                        if let Some(hint) = &job.error_hint {
                                            ui.small(format!("{}: {hint}", text.diagnostic_hint));
                                        }
                                    });
                                }
                            });
                    }
                });

            ui.add_space(10.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.heading(text.coding_agents);
                    if ui.small_button(text.refresh).clicked() {
                        self.refresh_clients();
                    }
                });
                ui.label(text.coding_agents_hint);

                let client_rows = self.clients.clone();
                for client in client_rows {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(client.kind.label());
                        let availability = if client.available {
                            text.available
                        } else {
                            text.unavailable
                        };
                        let installation = if client.installed {
                            text.installed
                        } else {
                            text.not_installed
                        };
                        ui.label(format!("{availability} · {installation}"));
                        if !client.detail.is_empty() {
                            ui.small(&client.detail);
                        }

                        if ui
                            .add_enabled(client.available, egui::Button::new(text.install_update))
                            .clicked()
                        {
                            self.install_client(client.kind);
                        }
                        if ui
                            .add_enabled(client.installed, egui::Button::new(text.remove))
                            .clicked()
                        {
                            self.remove_client(client.kind);
                        }
                    });
                    if client.kind == ClientKind::Pi {
                        ui.small(text.pi_note);
                    }
                }

                ui.separator();
                ui.small(text.generic_note);
            });

            ui.add_space(12.0);
            ui.separator();
            ui.label(&self.status);
            if let Ok(path) = config::config_path() {
                ui.small(format!("{}: {}", text.config_path, path.display()));
            }
            ui.small(text.readonly_note);
        });
    }
}

pub fn install_cjk_font(ctx: &egui::Context) {
    let candidates: &[&str] = if cfg!(target_os = "windows") {
        &[
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\msyhbd.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        ]
    };

    let Some((path, bytes)) = candidates
        .iter()
        .map(Path::new)
        .find_map(|path| fs::read(path).ok().map(|bytes| (path, bytes)))
    else {
        eprintln!("LLM2MCP: no system CJK font found; Simplified Chinese glyphs may be missing");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "llm2mcp-cjk".to_owned(),
        egui::FontData::from_owned(bytes).into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("llm2mcp-cjk".to_owned());
    }
    ctx.set_fonts(fonts);
    eprintln!("LLM2MCP: loaded CJK font from {}", path.display());
}
