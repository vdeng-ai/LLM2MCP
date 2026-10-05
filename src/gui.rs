use eframe::egui;
use std::{
    fs,
    path::Path,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::{
    clients::{self, ClientKind, ClientStatus},
    config::{
        self, AppConfig, ExecutionMode, Language, ReasoningEffort, ReasoningTransport, ToolConfig,
    },
    i18n, install, jobs, llm, updater,
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

#[derive(Debug, Clone)]
enum UpdateUiState {
    Disabled,
    Checking,
    UpToDate,
    Available {
        version: String,
        notes: Option<String>,
    },
    Installing,
    Installed {
        version: String,
    },
    Error(String),
}

enum UiReply {
    Models(Option<String>, anyhow::Result<Vec<String>>),
    Clients(Vec<ClientStatus>),
    Jobs(Vec<jobs::JobRecord>),
    Message(String),
    Clipboard(String),
}
pub struct Llm2McpApp {
    pending: Option<mpsc::Receiver<UiReply>>,
    operation: crate::control::Control,
    last_jobs_refresh: Instant,
    job_filter: String,
    config: AppConfig,
    status: String,
    api_key_visible: bool,
    clients: Vec<ClientStatus>,
    jobs: Vec<jobs::JobRecord>,
    available_models: Vec<String>,
    models_profile: Option<String>,
    ui_zoom_factor: f32,
    active_tab: AppTab,
    update_state: UpdateUiState,
    update_rx: Option<mpsc::Receiver<updater::UpdateEvent>>,
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
        let update_rx = updater::spawn_check();
        let update_state = if update_rx.is_some() {
            UpdateUiState::Checking
        } else {
            UpdateUiState::Disabled
        };
        let mut app = Self {
            pending: None,
            operation: crate::control::Control::default(),
            last_jobs_refresh: Instant::now(),
            job_filter: String::new(),
            config,
            status: ready,
            api_key_visible: false,
            clients: Vec::new(),
            jobs: Vec::new(),
            available_models: Vec::new(),
            models_profile: None,
            ui_zoom_factor,
            active_tab,
            update_state,
            update_rx,
        };
        app.refresh_clients();
        app
    }

    fn start_operation(&mut self, work: impl FnOnce() -> UiReply + Send + 'static) {
        if self.pending.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.operation = crate::control::Control::default();
        let control = self.operation.clone();
        self.pending = Some(receiver);
        self.status = "Working / 正在执行…".to_owned();
        thread::spawn(move || {
            control.set_current();
            let _ = sender.send(work());
        });
    }
    fn poll_operation(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(reply) => {
                    self.pending = None;
                    match reply {
                        UiReply::Models(profile, Ok(models)) => {
                            self.models_profile = profile;
                            self.status = format!("{} models", models.len());
                            self.available_models = models;
                        }
                        UiReply::Models(profile, Err(error)) => {
                            self.models_profile = profile;
                            self.available_models.clear();
                            self.status = format!(
                                "{}: {}",
                                i18n::texts(self.config.language).connection_failed,
                                crate::privacy::redact(&format!("{error:#}"))
                            )
                        }
                        UiReply::Clients(clients) => {
                            self.clients = clients;
                            self.status = i18n::texts(self.config.language).ready.to_owned();
                        }
                        UiReply::Jobs(jobs) => self.jobs = jobs,
                        UiReply::Message(message) => self.status = crate::privacy::redact(&message),
                        UiReply::Clipboard(value) => {
                            ctx.copy_text(value);
                            self.status = i18n::texts(self.config.language)
                                .generic_config_copied
                                .to_owned();
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.status = "Background operation ended unexpectedly".to_owned();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100))
                }
            }
        }
    }
    fn refresh_clients(&mut self) {
        self.start_operation(|| UiReply::Clients(clients::statuses()));
    }
    fn refresh_jobs(&mut self) {
        self.start_operation(|| {
            let _ = jobs::recover_stale();
            UiReply::Jobs(jobs::list_recent(200).unwrap_or_default())
        });
        self.last_jobs_refresh = Instant::now();
    }
    fn refresh_models(&mut self) {
        let mut config = match self
            .config
            .with_profile(self.config.active_profile.as_deref())
        {
            Ok(config) => config,
            Err(error) => {
                self.status = format!("{error:#}");
                return;
            }
        };
        config.timeout_secs = 20;
        let profile = self.config.active_profile.clone();
        self.start_operation(move || UiReply::Models(profile, llm::list_models(&config)));
    }

    fn save(&mut self) {
        let text = i18n::texts(self.config.language);
        self.status = match config::save(&self.config) {
            Ok(()) => text.config_saved.to_owned(),
            Err(error) => format!("{}: {error:#}", text.save_failed),
        };
    }

    fn test(&mut self) {
        if let Err(error) = config::save(&self.config) {
            self.status = format!("{error:#}");
            return;
        }
        let config = self.config.clone();
        self.start_operation(move || UiReply::Message(crate::doctor::run(&config).text()));
    }
    fn install_client(&mut self, kind: ClientKind) {
        if let Err(error) = config::save(&self.config) {
            self.status = format!("{error:#}");
            return;
        }
        self.start_operation(move || {
            UiReply::Message(clients::install(kind).unwrap_or_else(|error| format!("{error:#}")))
        });
    }
    fn remove_client(&mut self, kind: ClientKind) {
        self.start_operation(move || {
            UiReply::Message(clients::remove(kind).unwrap_or_else(|error| format!("{error:#}")))
        });
    }

    fn start_update_check(&mut self) {
        if let Some(receiver) = updater::spawn_check() {
            self.update_state = UpdateUiState::Checking;
            self.update_rx = Some(receiver);
        }
    }

    fn start_update_install(&mut self) {
        if let Some(receiver) = updater::spawn_install() {
            self.update_state = UpdateUiState::Installing;
            self.update_rx = Some(receiver);
        }
    }

    fn poll_update(&mut self, ctx: &egui::Context) {
        let event = self
            .update_rx
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok());
        let Some(event) = event else {
            return;
        };
        self.update_rx = None;
        self.update_state = match event {
            updater::UpdateEvent::UpToDate => UpdateUiState::UpToDate,
            updater::UpdateEvent::Available { version, notes } => {
                UpdateUiState::Available { version, notes }
            }
            updater::UpdateEvent::Installed { version } => UpdateUiState::Installed { version },
            updater::UpdateEvent::Error(error) => UpdateUiState::Error(error),
        };
        ctx.request_repaint();
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
            if self.pending.is_some() {
                ui.spinner();
                if ui.button("Cancel / 取消").clicked() {
                    self.operation.cancel();
                }
            }
            brand_mark(ui, 38.0);
            ui.vertical(|ui| {
                ui.heading("LLM2MCP");
                ui.small(text.tagline);
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let update_state = self.update_state.clone();
                match update_state {
                    UpdateUiState::Disabled => {}
                    UpdateUiState::Checking => {
                        ui.add_enabled(false, egui::Button::new(text.checking_update));
                    }
                    UpdateUiState::UpToDate => {
                        if ui.small_button(text.up_to_date).clicked() {
                            self.start_update_check();
                        }
                    }
                    UpdateUiState::Available { version, notes } => {
                        if updater::automatic_install_supported() {
                            let response =
                                ui.button(format!("{} v{version}", text.install_update_app));
                            let clicked = response.clicked();
                            if let Some(notes) = notes.filter(|value| !value.trim().is_empty()) {
                                response.on_hover_text(notes);
                            }
                            if clicked {
                                self.start_update_install();
                            }
                        } else {
                            let response = ui.hyperlink_to(
                                format!("{} v{version}", text.download_update_app),
                                "https://github.com/vdeng-ai/LLM2MCP/releases/latest",
                            );
                            if let Some(notes) = notes.filter(|value| !value.trim().is_empty()) {
                                response.on_hover_text(notes);
                            }
                        }
                    }
                    UpdateUiState::Installing => {
                        ui.add_enabled(false, egui::Button::new(text.installing_update));
                    }
                    UpdateUiState::Installed { version } => {
                        ui.label(format!("{} ({version})", text.update_installed));
                    }
                    UpdateUiState::Error(error) => {
                        let response = ui.small_button(text.update_failed);
                        let clicked = response.clicked();
                        response.on_hover_text(error);
                        if clicked {
                            self.start_update_check();
                        }
                    }
                }

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
                if ui
                    .add_enabled(
                        self.pending.is_none(),
                        egui::Button::new(text.test_connection),
                    )
                    .clicked()
                {
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
        let names = self
            .config
            .profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>();
        profile_picker(
            ui,
            "Editing / 编辑配置",
            &mut self.config.active_profile,
            &names,
        );
        let selected = self.config.active_profile.clone();
        let mut api = match self.config.with_profile(selected.as_deref()) {
            Ok(api) => api,
            Err(error) => {
                ui.label(format!("{error:#}"));
                return;
            }
        };
        let mut refresh = false;
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
                        egui::TextEdit::singleline(&mut api.base_url).desired_width(field_width),
                    );
                    ui.end_row();

                    ui.label(text.model);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut api.model)
                                .desired_width((field_width - 130.0).max(120.0)),
                        );
                        if self.models_profile == selected && !self.available_models.is_empty() {
                            egui::ComboBox::from_id_salt("model_picker")
                                .selected_text("▾")
                                .show_ui(ui, |ui| {
                                    for model in &self.available_models {
                                        ui.selectable_value(&mut api.model, model.clone(), model);
                                    }
                                });
                        }
                        if ui
                            .add_enabled(
                                self.pending.is_none(),
                                egui::Button::new(text.refresh_models).small(),
                            )
                            .clicked()
                        {
                            refresh = true;
                        }
                    });
                    ui.end_row();

                    ui.label(text.api_key);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut api.api_key)
                                .password(!self.api_key_visible)
                                .desired_width((field_width - 58.0).max(120.0)),
                        );
                        ui.checkbox(&mut self.api_key_visible, text.show);
                    });
                    ui.end_row();

                    ui.label(text.reasoning_protocol);
                    egui::ComboBox::from_id_salt("reasoning_transport")
                        .selected_text(api.reasoning_transport.label())
                        .show_ui(ui, |ui| {
                            for transport in ReasoningTransport::ALL {
                                ui.selectable_value(
                                    &mut api.reasoning_transport,
                                    transport,
                                    transport.label(),
                                );
                            }
                        });
                    ui.end_row();
                });
        });
        if let Err(error) = self.config.update_api_fields(selected.as_deref(), &api) {
            self.status = format!("{error:#}");
        }
        if refresh {
            self.refresh_models();
        }
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
        let mut action = None;
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
            ui.text_edit_singleline(&mut self.job_filter).on_hover_text("Filter by workspace, tool or state / 按工作区、工具或状态筛选");
            ui.small("Source/return tokens are estimates; compression is not verified primary-model cost savings. / 源码与回传为估算，不代表已验证的主模型费用节省。");
            ui.add_space(6.0);

            if self.jobs.is_empty() {
                ui.small(text.no_jobs);
            } else {
                let job_rows = &self.jobs;
                egui::ScrollArea::vertical()
                    .id_salt("job_history_scroll")
                    .max_height(560.0)
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        for job in job_rows {
                            if !self.job_filter.trim().is_empty() && !format!("{} {} {}", job.workspace.display(), job.tool, jobs::state_name(job.state)).to_lowercase().contains(&self.job_filter.to_lowercase()) { continue; }
                            let duration = format_duration(jobs::display_duration_ms(job));
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
                                                "prompt {} · completion {} (usage reported {}/{} calls)",
                                                job.prompt_tokens, job.completion_tokens, job.usage_reported_calls, job.llm_calls
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
                                    ui.small(format!("source≈{} → return≈{} tokens · files={} · LLM wait={} · models={}", job.source_tokens, job.return_tokens, job.selected_files, format_duration(job.llm_wait_ms), job.used_models.join(", ")));
                                    if job.source_tokens > 0 { ui.small(format!("Return/source {:.1}% · truncated={}", 100.0 * job.return_tokens as f64 / job.source_tokens as f64, job.source_truncated)); }
                                    if let Some(cost) = job.estimated_llm_cost_usd { ui.small(format!("Estimated secondary cost ${cost:.6} · priced calls {}/{}", job.priced_llm_calls, job.llm_calls)); }
                                    let result = job.result.as_ref().and_then(|value| value.pointer("/content/0/text")).and_then(serde_json::Value::as_str).unwrap_or_default();
                                    ui.horizontal(|ui| {
                                        if ui.button("Copy result / 复制结果").clicked() { ui.ctx().copy_text(result.to_owned()); }
                                        if ui.button("Export JSON / 导出 JSON").clicked() { ui.ctx().copy_text(serde_json::to_string_pretty(job).unwrap_or_default()); }
                                        if job.state == jobs::JobState::Working { if ui.add_enabled(self.pending.is_none(), egui::Button::new("Cancel job / 取消任务")).clicked() { action = Some((job.id.clone(), false)); } }
                                        else if ui.add_enabled(self.pending.is_none(), egui::Button::new("Retry / 重试")).clicked() { action = Some((job.id.clone(), true)); }
                                    });
                                    if !result.is_empty() { egui::CollapsingHeader::new("View result / 查看结果").id_salt(format!("{}_result", job.id)).show(ui, |ui| { egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| { let mut preview = result; ui.add(egui::TextEdit::multiline(&mut preview).desired_width(ui.available_width()).desired_rows(12)); }); }); }
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
        if let Some((id, retry)) = action {
            self.start_operation(move || {
                UiReply::Message(if retry {
                    jobs::retry(&id)
                        .map(|record| format!("Started {}", record.id))
                        .unwrap_or_else(|error| format!("{error:#}"))
                } else {
                    jobs::cancel(&id)
                        .map(|_| "Cancellation requested".to_owned())
                        .unwrap_or_else(|error| format!("{error:#}"))
                })
            });
        }
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
            ui.small("Registration status is not a host runtime/Tasks test. / 注册状态不等于宿主运行和 Tasks 验证。");
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
                                .add_enabled(self.pending.is_none() && client.installed, egui::Button::new(text.remove))
                                .clicked()
                            {
                                self.remove_client(client.kind);
                            }
                            if ui
                                .add_enabled(
                                    self.pending.is_none() && client.available,
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
                if ui.add_enabled(self.pending.is_none(), egui::Button::new(text.copy_generic_config).small()).clicked() {
                    self.start_operation(|| match install::ensure_stable_install().and_then(|path| serde_json::to_string_pretty(&serde_json::json!({"mcpServers":{"llm2mcp":{"type":"stdio","command":path,"args":["mcp"]}}})).map_err(Into::into)) { Ok(config) => UiReply::Clipboard(config), Err(error) => UiReply::Message(format!("{error:#}")) });
                }
            });
        });
    }

    fn render_footer(&self, ui: &mut egui::Ui, text: &i18n::Texts) {
        ui.separator();
        ui.small(text.readonly_note);
    }
}

fn profile_picker(ui: &mut egui::Ui, label: &str, selected: &mut Option<String>, names: &[String]) {
    ui.label(label);
    egui::ComboBox::from_id_salt(label)
        .selected_text(selected.as_deref().unwrap_or("Default / 默认"))
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, None, "Default / 默认");
            for name in names {
                ui.selectable_value(selected, Some(name.clone()), name);
            }
        });
}
impl Llm2McpApp {
    fn render_routes(&mut self, ui: &mut egui::Ui) {
        let names = self
            .config
            .profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>();
        ui.group(|ui| {
            ui.heading("Model routing / 模型路由");
            for (label, route) in [
                ("Analyze", &mut self.config.tools.analyze.model_profile),
                (
                    "Debug issue",
                    &mut self.config.tools.debug_issue.model_profile,
                ),
                ("Plan", &mut self.config.tools.plan.model_profile),
                (
                    "Review diff",
                    &mut self.config.tools.review_diff.model_profile,
                ),
                (
                    "Document repo",
                    &mut self.config.tools.document_repo.model_profile,
                ),
                (
                    "Update docs",
                    &mut self.config.tools.update_docs.model_profile,
                ),
                ("Discovery", &mut self.config.discovery_profile),
                ("Document Map", &mut self.config.map_profile),
            ] {
                ui.horizontal(|ui| profile_picker(ui, label, route, &names));
            }
        });
    }
    fn render_profiles(&mut self, ui: &mut egui::Ui) {
        let names = self
            .config
            .profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>();
        ui.horizontal(|ui| {
            profile_picker(
                ui,
                "Active profile / 默认模型配置",
                &mut self.config.active_profile,
                &names,
            )
        });
        ui.small("The API fields above edit the selected profile; refresh, selection and inference tests use that same profile. / 上方 API 编辑当前选中配置；刷新、模型选择和推理测试均使用该配置。");
        let mut removed = None;
        let mut renamed = Vec::new();
        for (index, profile) in self.config.profiles.iter_mut().enumerate() {
            let old_name = profile.name.clone();
            egui::CollapsingHeader::new(format!("Profile / 模型配置: {}", profile.name))
                .id_salt(("profile", index))
                .show(ui, |ui| {
                    egui::Grid::new(("profile_fields", index))
                        .num_columns(2)
                        .show(ui, |ui| {
                            ui.label("Name / 名称");
                            ui.text_edit_singleline(&mut profile.name);
                            ui.end_row();
                            ui.label("API URL");
                            ui.text_edit_singleline(&mut profile.base_url);
                            ui.end_row();
                            ui.label("API key");
                            ui.add(
                                egui::TextEdit::singleline(&mut profile.api_key)
                                    .password(!self.api_key_visible),
                            );
                            ui.end_row();
                            ui.label("Model / 模型");
                            ui.text_edit_singleline(&mut profile.model);
                            ui.end_row();
                            ui.label("Reasoning transport");
                            egui::ComboBox::from_id_salt(("profile_transport", index))
                                .selected_text(profile.reasoning_transport.label())
                                .show_ui(ui, |ui| {
                                    for transport in ReasoningTransport::ALL {
                                        ui.selectable_value(
                                            &mut profile.reasoning_transport,
                                            transport,
                                            transport.label(),
                                        );
                                    }
                                });
                            ui.end_row();
                            ui.label("Total context tokens / 总上下文");
                            ui.add(
                                egui::DragValue::new(&mut profile.max_input_tokens)
                                    .range(1024..=2_000_000),
                            );
                            ui.end_row();
                            ui.label("Max output tokens / 输出上限");
                            ui.add(
                                egui::DragValue::new(&mut profile.max_output_tokens)
                                    .range(128..=200_000),
                            );
                            ui.end_row();
                            ui.label("Timeout seconds / 超时秒数");
                            ui.add(egui::DragValue::new(&mut profile.timeout_secs).range(5..=3600));
                            ui.end_row();
                            ui.label("Temperature");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut profile.send_temperature, "Send / 发送");
                                ui.add(
                                    egui::DragValue::new(&mut profile.temperature)
                                        .range(0.0..=2.0)
                                        .speed(0.05),
                                );
                            });
                            ui.end_row();
                            ui.label("Output budget parameter");
                            egui::ComboBox::from_id_salt(("profile_output", index))
                                .selected_text(&profile.completion_token_parameter)
                                .show_ui(ui, |ui| {
                                    for parameter in ["max_tokens", "max_completion_tokens"] {
                                        ui.selectable_value(
                                            &mut profile.completion_token_parameter,
                                            parameter.to_owned(),
                                            parameter,
                                        );
                                    }
                                });
                            ui.end_row();
                            ui.label("Cost USD / million tokens / 百万 token 费用");
                            ui.vertical(|ui| {
                                for (label, rate) in [
                                    ("Input", &mut profile.input_usd_per_million),
                                    ("Output", &mut profile.output_usd_per_million),
                                ] {
                                    let mut enabled = rate.is_some();
                                    ui.horizontal(|ui| {
                                        if ui.checkbox(&mut enabled, label).changed() {
                                            *rate = enabled.then_some(0.0);
                                        }
                                        if let Some(rate) = rate {
                                            ui.add(
                                                egui::DragValue::new(rate)
                                                    .range(0.0..=10000.0)
                                                    .speed(0.01),
                                            );
                                        }
                                    });
                                }
                            });
                            ui.end_row();
                        });
                    if ui.button("Delete profile / 删除配置").clicked() {
                        removed = Some(index);
                    }
                });
            if old_name != profile.name {
                renamed.push((old_name, Some(profile.name.clone())));
            }
        }
        if let Some(index) = removed {
            let profile = self.config.profiles.remove(index);
            renamed.push((profile.name, None));
        }
        for (old, new) in renamed {
            for route in [
                &mut self.config.active_profile,
                &mut self.config.discovery_profile,
                &mut self.config.map_profile,
                &mut self.config.tools.analyze.model_profile,
                &mut self.config.tools.debug_issue.model_profile,
                &mut self.config.tools.plan.model_profile,
                &mut self.config.tools.review_diff.model_profile,
                &mut self.config.tools.document_repo.model_profile,
                &mut self.config.tools.update_docs.model_profile,
            ] {
                if route.as_deref() == Some(old.as_str()) {
                    *route = new.clone();
                }
            }
        }
        if ui.button("Add profile / 添加模型配置").clicked() {
            let mut name = format!("Profile {}", self.config.profiles.len() + 1);
            while self
                .config
                .profiles
                .iter()
                .any(|profile| profile.name == name)
            {
                name.push('_');
            }
            self.config.profiles.push(config::ModelProfile {
                name,
                base_url: self.config.base_url.clone(),
                api_key: self.config.api_key.clone(),
                model: self.config.model.clone(),
                reasoning_transport: self.config.reasoning_transport,
                temperature: self.config.temperature,
                timeout_secs: self.config.timeout_secs,
                max_input_tokens: self.config.model_input_limit,
                max_output_tokens: self.config.model_output_limit,
                send_temperature: self.config.send_temperature,
                completion_token_parameter: self.config.completion_token_parameter.clone(),
                input_usd_per_million: self.config.input_usd_per_million,
                output_usd_per_million: self.config.output_usd_per_million,
            });
        }
    }
    fn render_runtime(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("Runtime and cache / 运行与缓存");
            for (label, value) in [
                (
                    "Concurrent jobs / 并发任务",
                    &mut self.config.max_concurrent_jobs,
                ),
                (
                    "Concurrent HTTP requests / 并发请求",
                    &mut self.config.max_concurrent_requests,
                ),
            ] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(egui::DragValue::new(value).range(1..=16));
                });
            }
            ui.horizontal(|ui| {
                ui.label("Cache MiB / 缓存大小");
                ui.add(egui::DragValue::new(&mut self.config.cache_max_mib).range(16..=16384));
            });
            ui.horizontal(|ui| {
                ui.label("Cache TTL days / 缓存保留天数");
                ui.add(egui::DragValue::new(&mut self.config.cache_ttl_days).range(1..=365));
            });
            ui.horizontal(|ui| {
                for (label, clear) in [
                    ("Inspect / prune / 检查清理", false),
                    ("Clear cache / 清空缓存", true),
                ] {
                    if ui
                        .add_enabled(self.pending.is_none(), egui::Button::new(label))
                        .clicked()
                    {
                        let max = self.config.cache_max_mib;
                        let ttl = self.config.cache_ttl_days;
                        self.start_operation(move || {
                            UiReply::Message(
                                crate::cache::maintain(max, ttl, clear)
                                    .and_then(|stats| {
                                        serde_json::to_string(&stats).map_err(Into::into)
                                    })
                                    .unwrap_or_else(|error| format!("{error:#}")),
                            )
                        });
                    }
                }
            });
        });
    }
}

impl eframe::App for Llm2McpApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_update(ctx);
        self.poll_operation(ctx);
        if self.active_tab == AppTab::JobHistory {
            ctx.request_repaint_after(Duration::from_secs(1));
            if self.last_jobs_refresh.elapsed() >= Duration::from_secs(2) && self.pending.is_none()
            {
                self.refresh_jobs();
            }
        }
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
                        AppTab::LlmApi => {
                            self.render_api_section(ui, &text);
                            self.render_profiles(ui);
                        }
                        AppTab::ContextLimits => {
                            self.render_context_section(ui, &text);
                            self.render_runtime(ui);
                        }
                        AppTab::Tools => {
                            self.render_tools_section(ui, &text);
                            self.render_routes(ui);
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_work_keeps_ui_polling_responsive_and_observes_cancel() {
        let mut app = Llm2McpApp {
            pending: None,
            operation: crate::control::Control::default(),
            last_jobs_refresh: Instant::now(),
            job_filter: String::new(),
            config: AppConfig::default(),
            status: String::new(),
            api_key_visible: false,
            clients: Vec::new(),
            jobs: Vec::new(),
            available_models: Vec::new(),
            models_profile: None,
            ui_zoom_factor: 1.0,
            active_tab: AppTab::LlmApi,
            update_state: UpdateUiState::Disabled,
            update_rx: None,
        };
        let (sender, ready) = mpsc::channel();
        app.start_operation(move || {
            sender.send(()).unwrap();
            while crate::control::Control::current().check().is_ok() {
                thread::sleep(Duration::from_millis(10));
            }
            UiReply::Message("cancelled".to_owned())
        });
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        let ctx = egui::Context::default();
        let started = Instant::now();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.poll_operation(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                app.render_profiles(ui);
                app.render_routes(ui);
                app.render_runtime(ui);
            });
        });
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(app.pending.is_some());
        app.operation.cancel();
        for _ in 0..100 {
            app.poll_operation(&ctx);
            if app.pending.is_none() {
                assert_eq!(app.status, "cancelled");
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("GUI cancellation did not complete");
    }
}
