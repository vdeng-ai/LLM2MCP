use eframe::egui;
use std::{fs, path::Path};

use crate::{
    clients::{self, ClientKind, ClientStatus},
    config::{
        self, AppConfig, ExecutionMode, Language, ReasoningEffort, ReasoningTransport, ToolConfig,
    },
    i18n, install, jobs, llm,
};

const UI_ZOOM_STORAGE_KEY: &str = "llm2mcp_ui_zoom_factor";
const ACTIVE_TAB_STORAGE_KEY: &str = "llm2mcp_active_tab";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppTab {
    LlmApi,
    ContextLimits,
    Tools,
    SystemPrompt,
    JobHistory,
    CodingAgents,
}

impl AppTab {
    const ALL: [Self; 6] = [
        Self::LlmApi,
        Self::ContextLimits,
        Self::Tools,
        Self::SystemPrompt,
        Self::JobHistory,
        Self::CodingAgents,
    ];

    fn index(self) -> u8 {
        match self {
            Self::LlmApi => 0,
            Self::ContextLimits => 1,
            Self::Tools => 2,
            Self::SystemPrompt => 3,
            Self::JobHistory => 4,
            Self::CodingAgents => 5,
        }
    }

    fn from_index(index: u8) -> Self {
        match index {
            1 => Self::ContextLimits,
            2 => Self::Tools,
            3 => Self::SystemPrompt,
            4 => Self::JobHistory,
            5 => Self::CodingAgents,
            _ => Self::LlmApi,
        }
    }

    fn label(self, text: &i18n::Texts) -> &'static str {
        match self {
            Self::LlmApi => text.llm_api,
            Self::ContextLimits => text.context_limits,
            Self::Tools => text.per_tool_reasoning,
            Self::SystemPrompt => text.system_prompt_prefix,
            Self::JobHistory => text.job_history,
            Self::CodingAgents => text.coding_agents,
        }
    }
}

pub struct Llm2McpApp {
    config: AppConfig,
    status: String,
    api_key_visible: bool,
    clients: Vec<ClientStatus>,
    jobs: Vec<jobs::JobRecord>,
    available_models: Vec<String>,
    ui_zoom_factor: f32,
    active_tab: AppTab,
}

impl Llm2McpApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config = config::load().unwrap_or_default();
        let ready = i18n::texts(config.language).ready.to_owned();
        let ui_zoom_factor = cc
            .storage
            .and_then(|storage| eframe::get_value::<f32>(storage, UI_ZOOM_STORAGE_KEY))
            .filter(|value| value.is_finite())
            .unwrap_or(1.0)
            .clamp(0.5, 2.0);
        cc.egui_ctx.set_zoom_factor(ui_zoom_factor);
        let active_tab = cc
            .storage
            .and_then(|storage| eframe::get_value::<u8>(storage, ACTIVE_TAB_STORAGE_KEY))
            .map(AppTab::from_index)
            .unwrap_or(AppTab::LlmApi);
        Self {
            config,
            status: ready,
            api_key_visible: false,
            clients: clients::statuses(),
            jobs: jobs::list_recent(40).unwrap_or_default(),
            available_models: Vec::new(),
            ui_zoom_factor,
            active_tab,
        }
    }

    fn refresh_clients(&mut self) {
        self.clients = clients::statuses();
    }

    fn refresh_jobs(&mut self) {
        self.jobs = jobs::list_recent(40).unwrap_or_default();
    }

    fn refresh_models(&mut self) {
        let text = i18n::texts(self.config.language);
        match llm::list_models(&self.config) {
            Ok(models) => {
                self.status = format!("{}: {}", text.refresh_models, models.len());
                self.available_models = models;
            }
            Err(error) => {
                self.status = format!("{}: {error:#}", text.connection_failed);
            }
        }
    }

    fn generic_mcp_config(&self) -> anyhow::Result<String> {
        let command = install::ensure_stable_install()?.display().to_string();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": {
                "llm2mcp": {
                    "type": "stdio",
                    "command": command,
                    "args": ["mcp"]
                }
            }
        }))?)
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

fn brand_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same((size * 0.22) as u8),
        egui::Color32::from_rgb(10, 29, 72),
    );

    let cyan = egui::Color32::from_rgb(53, 198, 255);
    let white = egui::Color32::from_rgb(244, 248, 255);
    let cy = rect.center().y;
    let left = egui::pos2(rect.left() + size * 0.28, cy);
    let middle = egui::pos2(rect.left() + size * 0.52, cy);
    let plug = egui::Rect::from_center_size(
        egui::pos2(rect.left() + size * 0.72, cy),
        egui::vec2(size * 0.16, size * 0.30),
    );

    painter.circle_filled(left, size * 0.09, white);
    painter.circle_stroke(left, size * 0.16, egui::Stroke::new(2.0_f32, cyan));
    painter.line_segment([left, middle], egui::Stroke::new(2.5_f32, cyan));
    painter.line_segment(
        [middle, egui::pos2(middle.x - size * 0.08, cy - size * 0.07)],
        egui::Stroke::new(2.5_f32, cyan),
    );
    painter.line_segment(
        [middle, egui::pos2(middle.x - size * 0.08, cy + size * 0.07)],
        egui::Stroke::new(2.5_f32, cyan),
    );
    painter.rect_filled(plug, egui::CornerRadius::same(3), white);
    for offset in [-0.08_f32, 0.08_f32] {
        painter.line_segment(
            [
                egui::pos2(plug.right(), cy + size * offset),
                egui::pos2(plug.right() + size * 0.12, cy + size * offset),
            ],
            egui::Stroke::new(2.0_f32, white),
        );
    }
}

impl Llm2McpApp {
    fn render_header(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.horizontal(|ui| {
            brand_mark(ui, 38.0);
            ui.vertical(|ui| {
                ui.heading("LLM2MCP");
                ui.small(text.tagline);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let zoom_percent = (ui.ctx().zoom_factor() * 100.0).round() as i32;
                ui.menu_button(format!("A {zoom_percent}%"), |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("A−").clicked() {
                            egui::gui_zoom::zoom_out(ui.ctx());
                            self.ui_zoom_factor = ui.ctx().zoom_factor();
                        }
                        if ui.button("100%").clicked() {
                            ui.ctx().set_zoom_factor(1.0);
                            self.ui_zoom_factor = 1.0;
                        }
                        if ui.button("A+").clicked() {
                            egui::gui_zoom::zoom_in(ui.ctx());
                            self.ui_zoom_factor = ui.ctx().zoom_factor();
                        }
                    });
                });
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
                                self.status = i18n::texts(language).ready.to_owned();
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

    fn render_tabs(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(2, 4))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for tab in AppTab::ALL {
                        let selected = self.active_tab == tab;
                        let label = egui::RichText::new(tab.label(text)).strong();
                        if ui
                            .add(
                                egui::Button::new(label)
                                    .selected(selected)
                                    .min_size(egui::vec2(118.0, 30.0)),
                            )
                            .clicked()
                        {
                            self.active_tab = tab;
                        }
                    }
                });
            });
    }

    fn render_api_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            section_header(ui, text.llm_api, None);
            let field_width = (ui.available_width() - 140.0).clamp(240.0, 720.0);
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
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.model)
                                .desired_width((field_width - 130.0).max(120.0)),
                        );
                        if !self.available_models.is_empty() {
                            egui::ComboBox::from_id_salt("model_picker")
                                .selected_text("▾")
                                .show_ui(ui, |ui| {
                                    for model in &self.available_models {
                                        ui.selectable_value(
                                            &mut self.config.model,
                                            model.clone(),
                                            model,
                                        );
                                    }
                                });
                        }
                        if ui.small_button(text.refresh_models).clicked() {
                            self.refresh_models();
                        }
                    });
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
            ui.set_min_width(ui.available_width());
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

                    ui.label(text.job_ttl_hours);
                    ui.add(
                        egui::DragValue::new(&mut self.config.job_ttl_hours).range(1..=24 * 365),
                    );
                    ui.end_row();

                    ui.label(text.job_poll_interval_ms);
                    ui.add(
                        egui::DragValue::new(&mut self.config.job_poll_interval_ms)
                            .range(1_000..=60_000)
                            .speed(250),
                    );
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
            ui.set_min_width(ui.available_width());
            section_header(ui, text.per_tool_reasoning, Some(text.per_tool_hint));
            if ui.available_width() >= 560.0 {
                egui::Grid::new("tool_grid")
                    .num_columns(5)
                    .striped(true)
                    .spacing([20.0, 10.0])
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
                            text.debug_issue,
                            "debug_issue",
                            &mut self.config.tools.debug_issue,
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
                    text.debug_issue,
                    "debug_issue",
                    &mut self.config.tools.debug_issue,
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

    fn render_system_prompt_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            section_header(ui, text.system_prompt_prefix, None);
            ui.small(text.system_prompt_hint);
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.config.system_prompt_prefix)
                    .hint_text(text.system_prompt_placeholder)
                    .desired_rows(20)
                    .desired_width(ui.available_width()),
            );
        });
    }

    fn render_jobs_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
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
            } else {
                let job_rows = self.jobs.clone();
                egui::ScrollArea::vertical()
                    .id_salt("job_history_scroll")
                    .max_height(560.0)
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
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
            }
        });
    }

    fn render_clients_section(&mut self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
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
            ui.horizontal_wrapped(|ui| {
                ui.small(text.generic_note);
                if ui.small_button(text.copy_generic_config).clicked() {
                    match self.generic_mcp_config() {
                        Ok(config) => {
                            ui.ctx().copy_text(config);
                            self.status = text.generic_config_copied.to_owned();
                        }
                        Err(error) => {
                            self.status = format!("{}: {error:#}", text.save_failed);
                        }
                    }
                }
            });
        });
    }

    fn render_footer(&self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.separator();
        ui.small(text.readonly_note);
    }
}

impl eframe::App for Llm2McpApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui_zoom_factor = ctx.zoom_factor();
        let text = *i18n::texts(self.config.language);

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_header(ui, &text);
            ui.add_space(8.0);
            ui.separator();
            self.render_tabs(ui, &text);
            ui.separator();
            ui.add_space(8.0);

            let active_tab = self.active_tab;
            egui::ScrollArea::vertical()
                .id_salt(format!("tab_page_{}", active_tab.index()))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    match active_tab {
                        AppTab::LlmApi => self.render_api_section(ui, &text),
                        AppTab::ContextLimits => self.render_context_section(ui, &text),
                        AppTab::Tools => self.render_tools_section(ui, &text),
                        AppTab::SystemPrompt => self.render_system_prompt_section(ui, &text),
                        AppTab::JobHistory => self.render_jobs_section(ui, &text),
                        AppTab::CodingAgents => self.render_clients_section(ui, &text),
                    }

                    ui.add_space(12.0);
                    self.render_footer(ui, &text);
                    ui.add_space(8.0);
                });
        });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, UI_ZOOM_STORAGE_KEY, &self.ui_zoom_factor);
        eframe::set_value(storage, ACTIVE_TAB_STORAGE_KEY, &self.active_tab.index());
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
