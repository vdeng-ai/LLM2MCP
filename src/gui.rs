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
    ui.strong(label);
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
    ui.add(
        egui::DragValue::new(&mut tool.max_output_tokens)
            .range(256..=16_000)
            .speed(128),
    );
    if let Some(return_tokens) = &mut tool.primary_return_tokens {
        ui.add(
            egui::DragValue::new(return_tokens)
                .range(128..=8_000)
                .speed(64),
        );
    } else {
        ui.small(text.full_artifact);
    }
    ui.end_row();
}

fn tool_card(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    tool: &mut ToolConfig,
    default_execution: ExecutionMode,
    language: Language,
    text: &i18n::Texts,
) {
    ui.group(|ui| {
        ui.strong(label);
        ui.add_space(4.0);
        egui::Grid::new(format!("{id}_compact_grid"))
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label(text.reasoning);
                egui::ComboBox::from_id_salt(format!("{id}_compact_reasoning"))
                    .selected_text(tool.reasoning.label())
                    .show_ui(ui, |ui| {
                        for effort in ReasoningEffort::ALL {
                            ui.selectable_value(&mut tool.reasoning, effort, effort.label());
                        }
                    });
                ui.end_row();

                ui.label(text.execution);
                let mut execution = tool.execution_or(default_execution);
                egui::ComboBox::from_id_salt(format!("{id}_compact_execution"))
                    .selected_text(execution_label(execution, language, text))
                    .show_ui(ui, |ui| {
                        for mode in ExecutionMode::ALL {
                            if ui
                                .selectable_value(
                                    &mut execution,
                                    mode,
                                    execution_label(mode, language, text),
                                )
                                .changed()
                            {
                                tool.execution = Some(execution);
                            }
                        }
                    });
                ui.end_row();

                ui.label(text.output_budget);
                ui.add(
                    egui::DragValue::new(&mut tool.max_output_tokens)
                        .range(256..=16_000)
                        .speed(128),
                );
                ui.end_row();

                ui.label(text.return_budget);
                if let Some(return_tokens) = &mut tool.primary_return_tokens {
                    ui.add(
                        egui::DragValue::new(return_tokens)
                            .range(128..=8_000)
                            .speed(64),
                    );
                } else {
                    ui.small(text.full_artifact);
                }
                ui.end_row();
            });
    });
}

fn cache_ratio(hits: u64, misses: u64) -> String {
    let total = hits.saturating_add(misses);
    if total == 0 {
        return "—".to_owned();
    }
    format!("{hits}/{total}")
}

fn section_header(ui: &mut egui::Ui, title: &str, hint: Option<&str>) {
    ui.heading(title);
    if let Some(hint) = hint {
        ui.small(hint);
    }
    ui.add_space(6.0);
}

impl Llm2McpApp {
    fn render_header(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading("LLM2MCP");
                ui.small(text.tagline);
            });
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
                if ui.button(text.test_connection).clicked() {
                    self.test();
                }
                if ui.button(text.save).clicked() {
                    self.save();
                }
            });
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.strong(&self.status);
            if let Ok(path) = config::config_path() {
                ui.separator();
                ui.small(format!("{}: {}", text.config_path, path.display()));
            }
        });
    }

    fn render_api_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            section_header(ui, text.llm_api, None);
            let field_width = (ui.available_width() - 120.0).clamp(180.0, 420.0);
            egui::Grid::new("llm_api_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(text.base_url);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.config.base_url)
                            .desired_width(field_width),
                    );
                    ui.end_row();

                    ui.label(text.model);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.config.model)
                            .desired_width(field_width),
                    );
                    ui.end_row();

                    ui.label(text.api_key);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.api_key)
                                .password(!self.api_key_visible)
                                .desired_width((field_width - 58.0).max(120.0)),
                        );
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
        });
    }

    fn render_context_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            section_header(ui, text.context_limits, None);
            egui::Grid::new("context_limit_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(text.max_source_tokens);
                    ui.add(
                        egui::DragValue::new(&mut self.config.max_source_tokens)
                            .range(2_500..=500_000),
                    );
                    ui.end_row();

                    ui.label(text.max_tokens_per_file);
                    ui.add(
                        egui::DragValue::new(&mut self.config.max_file_tokens).range(500..=200_000),
                    );
                    ui.end_row();

                    ui.label(text.discovery_index_tokens);
                    ui.add(
                        egui::DragValue::new(&mut self.config.discovery_index_tokens)
                            .range(1_000..=50_000),
                    );
                    ui.end_row();

                    ui.label(text.timeout_seconds);
                    ui.add(egui::DragValue::new(&mut self.config.timeout_secs).range(5..=3600));
                    ui.end_row();

                    ui.label(text.document_map_concurrency);
                    ui.add(
                        egui::DragValue::new(&mut self.config.document_map_concurrency)
                            .range(1..=8),
                    );
                    ui.end_row();
                });
        });
    }

    fn render_tools_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            section_header(ui, text.per_tool_reasoning, Some(text.per_tool_hint));
            if ui.available_width() >= 820.0 {
                egui::Grid::new("tool_grid")
                    .num_columns(5)
                    .striped(true)
                    .spacing([18.0, 9.0])
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
                            text,
                        );
                        tool_row(
                            ui,
                            text.plan,
                            "plan",
                            &mut self.config.tools.plan,
                            ExecutionMode::Auto,
                            self.config.language,
                            text,
                        );
                        tool_row(
                            ui,
                            text.review_diff,
                            "review",
                            &mut self.config.tools.review_diff,
                            ExecutionMode::Sync,
                            self.config.language,
                            text,
                        );
                        tool_row(
                            ui,
                            text.document_repo,
                            "document_repo",
                            &mut self.config.tools.document_repo,
                            ExecutionMode::Async,
                            self.config.language,
                            text,
                        );
                        tool_row(
                            ui,
                            text.update_docs,
                            "update_docs",
                            &mut self.config.tools.update_docs,
                            ExecutionMode::Async,
                            self.config.language,
                            text,
                        );
                    });
            } else {
                tool_card(
                    ui,
                    text.analyze,
                    "analyze",
                    &mut self.config.tools.analyze,
                    ExecutionMode::Auto,
                    self.config.language,
                    text,
                );
                ui.add_space(6.0);
                tool_card(
                    ui,
                    text.plan,
                    "plan",
                    &mut self.config.tools.plan,
                    ExecutionMode::Auto,
                    self.config.language,
                    text,
                );
                ui.add_space(6.0);
                tool_card(
                    ui,
                    text.review_diff,
                    "review",
                    &mut self.config.tools.review_diff,
                    ExecutionMode::Sync,
                    self.config.language,
                    text,
                );
                ui.add_space(6.0);
                tool_card(
                    ui,
                    text.document_repo,
                    "document_repo",
                    &mut self.config.tools.document_repo,
                    ExecutionMode::Async,
                    self.config.language,
                    text,
                );
                ui.add_space(6.0);
                tool_card(
                    ui,
                    text.update_docs,
                    "update_docs",
                    &mut self.config.tools.update_docs,
                    ExecutionMode::Async,
                    self.config.language,
                    text,
                );
            }
        });
    }

    fn render_jobs_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.heading(text.job_history);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(text.refresh).clicked() {
                        self.refresh_jobs();
                    }
                });
            });
            ui.small(text.job_history_hint);
            ui.add_space(6.0);

            if self.jobs.is_empty() {
                ui.small(text.no_jobs);
                return;
            }

            let job_rows = self.jobs.clone();
            egui::ScrollArea::vertical()
                .id_salt("job_history_scroll")
                .max_height(320.0)
                .show(ui, |ui| {
                    for job in job_rows {
                        let duration = format_duration(jobs::display_duration_ms(&job));
                        let heading = format!(
                            "{} · {} · {} · {duration}",
                            jobs::state_name(job.state),
                            job.tool,
                            job.stage
                        );
                        egui::CollapsingHeader::new(heading)
                            .id_salt(&job.id)
                            .show(ui, |ui| {
                                egui::Grid::new(format!("job_{}_detail", job.id))
                                    .num_columns(2)
                                    .spacing([10.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.small("Workspace");
                                        ui.small(job.workspace.display().to_string());
                                        ui.end_row();
                                        ui.small(text.llm_calls);
                                        ui.small(job.llm_calls.to_string());
                                        ui.end_row();
                                        ui.small(text.token_usage);
                                        ui.small(format!(
                                            "prompt {} · completion {}",
                                            job.prompt_tokens, job.completion_tokens
                                        ));
                                        ui.end_row();
                                        ui.small("Symbol Index");
                                        ui.small(cache_ratio(
                                            job.symbol_index_hits,
                                            job.symbol_index_misses,
                                        ));
                                        ui.end_row();
                                        ui.small("Evidence Cache");
                                        ui.small(cache_ratio(
                                            job.evidence_cache_hits,
                                            job.evidence_cache_misses,
                                        ));
                                        ui.end_row();
                                        ui.small(text.duration);
                                        ui.small(&duration);
                                        ui.end_row();
                                    });
                                ui.small(format!("job_id: {}", job.id));
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
        });
    }

    fn render_clients_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.heading(text.coding_agents);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(text.refresh).clicked() {
                        self.refresh_clients();
                    }
                });
            });
            ui.small(text.coding_agents_hint);
            ui.add_space(6.0);

            let client_rows = self.clients.clone();
            for client in client_rows {
                ui.group(|ui| {
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
                        ui.small(format!("{availability} · {installation}"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_enabled(client.installed, egui::Button::new(text.remove))
                                .clicked()
                            {
                                self.remove_client(client.kind);
                            }
                            if ui
                                .add_enabled(
                                    client.available,
                                    egui::Button::new(text.install_update),
                                )
                                .clicked()
                            {
                                self.install_client(client.kind);
                            }
                        });
                    });
                    if !client.detail.is_empty() {
                        ui.small(&client.detail);
                    }
                    if client.kind == ClientKind::Pi {
                        ui.small(text.pi_note);
                    }
                });
                ui.add_space(5.0);
            }
            ui.small(text.generic_note);
        });
    }

    fn render_footer(&self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.separator();
        ui.small(text.readonly_note);
    }
}

impl eframe::App for Llm2McpApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let text = *i18n::texts(self.config.language);

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_header(ui, &text);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            egui::ScrollArea::vertical()
                .id_salt("main_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let wide = ui.available_width() >= 900.0;
                    if wide {
                        ui.columns(2, |columns| {
                            self.render_api_section(&mut columns[0], &text);
                            self.render_context_section(&mut columns[1], &text);
                        });
                    } else {
                        self.render_api_section(ui, &text);
                        ui.add_space(8.0);
                        self.render_context_section(ui, &text);
                    }

                    ui.add_space(10.0);
                    self.render_tools_section(ui, &text);
                    ui.add_space(10.0);

                    if wide {
                        ui.columns(2, |columns| {
                            self.render_jobs_section(&mut columns[0], &text);
                            self.render_clients_section(&mut columns[1], &text);
                        });
                    } else {
                        self.render_jobs_section(ui, &text);
                        ui.add_space(10.0);
                        self.render_clients_section(ui, &text);
                    }

                    ui.add_space(10.0);
                    self.render_footer(ui, &text);
                    ui.add_space(8.0);
                });
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
