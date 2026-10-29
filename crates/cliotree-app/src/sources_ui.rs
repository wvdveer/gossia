//! *Tools > Data sources...* (DESIGN.md, "Data sources"): the definitions of
//! this user on this machine, with their credentials, which every workbook
//! refers to by name. The panel lists them, and a form creates or edits one,
//! with a *Test* that runs in the background. Nothing here runs Python, so it
//! works in an untrusted workbook too.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use cliotree_calc::Setting;
use cliotree_calc::sources::{HttpAuth, SourceDef, SourceKind, TestOutcome, TlsMode, test};
use egui::RichText;

use crate::app::ClioApp;
use crate::view::error_color;

/// The panel, while it is open.
#[derive(Debug, Default)]
pub struct SourcesPanel {
    /// The name of the definition chosen in the list.
    pub selected: Option<String>,
    /// The form, while a definition is created or edited.
    pub form: Option<SourceForm>,
    /// The definition the user asked to delete, until they confirm.
    pub confirm_delete: Option<String>,
    /// Why the last change could not be made.
    pub error: Option<String>,
    /// A test of a definition in the list, and its name.
    pub test: Option<(String, RunningTest)>,
    /// What the last test of a definition in the list found.
    pub result: Option<(String, TestOutcome)>,
}

/// A test running in the background. Dropping it abandons the test: its
/// thread ends on its own, within the call timeout, and nobody reads it.
#[derive(Debug)]
pub struct RunningTest {
    receiver: mpsc::Receiver<TestOutcome>,
    started: Instant,
}

impl RunningTest {
    fn start(def: SourceDef, timeout: Duration) -> RunningTest {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(test(&def, timeout));
        });
        RunningTest {
            receiver,
            started: Instant::now(),
        }
    }

    /// The outcome, once there is one.
    fn poll(&self) -> Option<TestOutcome> {
        match self.receiver.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(TestOutcome {
                ok: false,
                message: "the test stopped without a result".to_owned(),
            }),
        }
    }
}

/// A password or token in the form. A stored one is never shown: it is kept,
/// replaced or cleared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Secret {
    Keep,
    Change(String),
    Clear,
}

impl Secret {
    fn resolve(&self, stored: &Option<String>) -> Option<String> {
        match self {
            Secret::Keep => stored.clone(),
            Secret::Change(text) if text.is_empty() => None,
            Secret::Change(text) => Some(text.clone()),
            Secret::Clear => None,
        }
    }
}

/// The form of one definition.
#[derive(Debug)]
pub struct SourceForm {
    /// The name the definition had, when an existing one is edited.
    pub old: Option<String>,
    /// The settings, without the secrets.
    pub draft: SourceDef,
    stored_password: Option<String>,
    stored_token: Option<String>,
    pub password: Secret,
    pub token: Secret,
    /// What is typed for the numbers and the headers.
    pub port: String,
    pub pooling: bool,
    pub pool: String,
    /// One `Name: value` a line.
    pub headers: String,
    pub test: Option<RunningTest>,
    pub result: Option<TestOutcome>,
    pub error: Option<String>,
}

impl SourceForm {
    pub fn new() -> SourceForm {
        SourceForm::of(SourceDef::new("", SourceKind::Http), None)
    }

    /// The form of `def`; `old` is its name when it is edited in place.
    fn of(def: SourceDef, old: Option<String>) -> SourceForm {
        let secret = |stored: &Option<String>| match stored {
            Some(s) if !s.is_empty() => Secret::Keep,
            _ => Secret::Change(String::new()),
        };
        SourceForm {
            old,
            password: secret(&def.password),
            token: secret(&def.token),
            stored_password: def.password.clone(),
            stored_token: def.token.clone(),
            port: def.port.map(|p| p.to_string()).unwrap_or_default(),
            pooling: def.pool.is_some(),
            pool: def.pool.unwrap_or(4).to_string(),
            headers: def
                .headers
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("\n"),
            draft: def.without_secrets(),
            test: None,
            result: None,
            error: None,
        }
    }

    /// The form of an existing definition.
    pub fn edit(def: SourceDef) -> SourceForm {
        let old = Some(def.name.clone());
        SourceForm::of(def, old)
    }

    /// A new definition with the settings of `def` but not its credentials,
    /// which the copy needs of its own.
    pub fn copy(def: &SourceDef, name: String) -> SourceForm {
        let mut copy = def.without_secrets();
        copy.name = name;
        SourceForm::of(copy, None)
    }

    fn has_stored(&self, which: &Secret) -> bool {
        let stored = if std::ptr::eq(which, &self.password) {
            &self.stored_password
        } else {
            &self.stored_token
        };
        stored.as_deref().is_some_and(|s| !s.is_empty())
    }

    /// The definition as the form has it, or the first thing wrong with it.
    pub fn definition(&self) -> Result<SourceDef, String> {
        let mut def = self.draft.clone();
        def.name = def.name.trim().to_owned();
        def.port = match self.port.trim() {
            "" => None,
            text => Some(
                text.parse::<u16>()
                    .ok()
                    .filter(|p| *p > 0)
                    .ok_or("the port must be a whole number from 1 to 65535")?,
            ),
        };
        def.pool = match self.pooling && def.kind != SourceKind::Sqlite {
            false => None,
            true => Some(
                self.pool
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or("pooling needs a maximum of at least 1 connection")?,
            ),
        };
        def.headers = Vec::new();
        for line in self
            .headers
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("the header line {line:?} needs a colon: Name: value"))?;
            def.headers
                .push((name.trim().to_owned(), value.trim().to_owned()));
        }
        // Only what the kind uses is kept, so that a definition does not
        // carry a password typed for another kind.
        let kind = def.kind;
        let http = kind == SourceKind::Http;
        let server = kind.is_database_server();
        let odbc = kind == SourceKind::Odbc;
        let basic = http && def.auth == HttpAuth::Basic;
        let token = http && def.auth.uses_token();
        let keyed = http && matches!(def.auth, HttpAuth::KeyHeader | HttpAuth::KeyQuery);
        def.password = if server || odbc || basic {
            self.password.resolve(&self.stored_password)
        } else {
            None
        };
        def.token = if token {
            self.token.resolve(&self.stored_token)
        } else {
            None
        };
        let keep = |on: bool, v: Option<String>| v.filter(|_| on);
        def.user = keep(server || odbc || basic, def.user.take());
        def.connection = keep(odbc, def.connection.take());
        def.host = keep(server, def.host.take());
        def.database = keep(server, def.database.take());
        def.port = def.port.filter(|_| server);
        if !server {
            def.tls = TlsMode::Prefer;
        }
        def.url = keep(http, def.url.take());
        def.test_path = keep(http, def.test_path.take());
        def.key_name = keep(keyed, def.key_name.take());
        def.path = keep(kind == SourceKind::Sqlite, def.path.take());
        if !http {
            def.headers.clear();
            def.auth = HttpAuth::None;
        }
        for field in [
            &mut def.user,
            &mut def.host,
            &mut def.database,
            &mut def.url,
            &mut def.test_path,
            &mut def.key_name,
            &mut def.path,
        ] {
            if field.as_deref().is_some_and(|s| s.trim().is_empty()) {
                *field = None;
            }
        }
        match def.problems().into_iter().next() {
            Some(why) => Err(why),
            None => Ok(def),
        }
    }
}

impl Default for SourceForm {
    fn default() -> Self {
        SourceForm::new()
    }
}

/// What the form asks for when it is drawn.
enum FormAction {
    None,
    Save,
    Cancel,
}

/// What a field of the form is called for the screen reader and the tests:
/// "File of the data source", not "File", which is also a menu.
fn field_name(label: &str) -> String {
    format!("{label} of the data source")
}

fn labeled(response: &egui::Response, role: egui::Role, name: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(role, true, name));
}

fn text_row(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, hint: &str) {
    ui.label(label);
    let mut text = value.clone().unwrap_or_default();
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(340.0)
            .hint_text(hint),
    );
    labeled(&response, egui::Role::TextInput, &field_name(label));
    if response.changed() {
        *value = Some(text);
    }
    ui.end_row();
}

/// A password or token field: a stored secret is never shown.
fn secret_row(ui: &mut egui::Ui, label: &str, secret: &mut Secret, stored: bool) {
    ui.label(label);
    ui.horizontal(|ui| match secret {
        Secret::Keep => {
            ui.weak("Stored");
            if ui.button("Change").clicked() {
                *secret = Secret::Change(String::new());
            }
            if ui.button("Clear").clicked() {
                *secret = Secret::Clear;
            }
        }
        Secret::Clear => {
            ui.weak("None");
            if stored && ui.button("Undo").clicked() {
                *secret = Secret::Keep;
            }
        }
        Secret::Change(text) => {
            let response = ui.add(
                egui::TextEdit::singleline(text)
                    .password(true)
                    .desired_width(240.0),
            );
            labeled(&response, egui::Role::PasswordInput, &field_name(label));
            if stored && ui.button("Keep stored").clicked() {
                *secret = Secret::Keep;
            }
        }
    });
    ui.end_row();
}

/// The form's fields; what is shown depends on the kind.
fn form_fields(ui: &mut egui::Ui, form: &mut SourceForm) {
    let stored_password = form.has_stored(&form.password);
    let stored_token = form.has_stored(&form.token);
    egui::Grid::new("source_form")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Name");
            let response = ui.add(
                egui::TextEdit::singleline(&mut form.draft.name)
                    .desired_width(340.0)
                    .hint_text("Sales"),
            );
            labeled(&response, egui::Role::TextInput, "Name of the data source");
            ui.end_row();

            ui.label("Kind");
            let mut kind = form.draft.kind;
            egui::ComboBox::from_id_salt("source_kind")
                .selected_text(kind.label())
                .show_ui(ui, |ui| {
                    for k in SourceKind::available() {
                        ui.selectable_value(&mut kind, k, k.label());
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Kind of the data source")
                });
            form.draft.kind = kind;
            ui.end_row();

            match kind {
                SourceKind::Http => {
                    text_row(
                        ui,
                        "Base URL",
                        &mut form.draft.url,
                        "https://api.example.com/v1",
                    );
                    ui.label("Authentication");
                    let mut auth = form.draft.auth;
                    egui::ComboBox::from_id_salt("source_auth")
                        .selected_text(auth.label())
                        .show_ui(ui, |ui| {
                            for a in HttpAuth::ALL {
                                ui.selectable_value(&mut auth, a, a.label());
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Authentication")
                        });
                    form.draft.auth = auth;
                    ui.end_row();
                    match auth {
                        HttpAuth::None => {}
                        HttpAuth::Basic => {
                            text_row(ui, "User name", &mut form.draft.user, "");
                            secret_row(ui, "Password", &mut form.password, stored_password);
                        }
                        HttpAuth::Bearer => {
                            secret_row(ui, "Token", &mut form.token, stored_token);
                        }
                        HttpAuth::KeyHeader | HttpAuth::KeyQuery => {
                            let what = match auth {
                                HttpAuth::KeyHeader => "Header name",
                                _ => "Parameter name",
                            };
                            text_row(ui, what, &mut form.draft.key_name, "X-Api-Key");
                            secret_row(ui, "API key", &mut form.token, stored_token);
                        }
                    }
                    ui.label("Default headers");
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut form.headers)
                            .desired_rows(2)
                            .desired_width(340.0)
                            .hint_text("Accept: application/json"),
                    );
                    labeled(&response, egui::Role::MultilineTextInput, "Default headers");
                    ui.end_row();
                    text_row(ui, "Test path", &mut form.draft.test_path, "/health");
                }
                SourceKind::Sqlite => {
                    text_row(ui, "File", &mut form.draft.path, "C:\\data\\sales.db");
                }
                SourceKind::Odbc => {
                    text_row(
                        ui,
                        "DSN or connection string",
                        &mut form.draft.connection,
                        "Sales, or Driver={...};Server=...",
                    );
                    text_row(ui, "User name", &mut form.draft.user, "");
                    secret_row(ui, "Password", &mut form.password, stored_password);
                }
                SourceKind::Postgres | SourceKind::Mariadb => {
                    text_row(ui, "Host", &mut form.draft.host, "db.example.com");
                    ui.label("Port");
                    let port = kind.default_port().unwrap_or(0).to_string();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut form.port)
                            .desired_width(80.0)
                            .hint_text(port),
                    );
                    labeled(&response, egui::Role::TextInput, &field_name("Port"));
                    ui.end_row();
                    text_row(ui, "Database", &mut form.draft.database, "");
                    text_row(ui, "User name", &mut form.draft.user, "");
                    secret_row(ui, "Password", &mut form.password, stored_password);
                    ui.label("TLS");
                    let mut tls = form.draft.tls;
                    egui::ComboBox::from_id_salt("source_tls")
                        .selected_text(tls.label())
                        .show_ui(ui, |ui| {
                            for t in TlsMode::ALL {
                                ui.selectable_value(&mut tls, t, t.label());
                            }
                        })
                        .response
                        .on_hover_text(
                            "When the connection is encrypted, the server's certificate is \
                             checked against this computer's certificate store. A self-signed \
                             certificate cannot be checked: use Unchecked for such a server, \
                             which still encrypts but would not notice a server pretending to \
                             be this one.",
                        )
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "TLS")
                        });
                    form.draft.tls = tls;
                    ui.end_row();
                }
            }

            if kind != SourceKind::Sqlite {
                ui.label("Pooling");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut form.pooling, "Keep connections, at most");
                    let response = ui.add_enabled(
                        form.pooling,
                        egui::TextEdit::singleline(&mut form.pool).desired_width(40.0),
                    );
                    labeled(&response, egui::Role::TextInput, "Most connections kept");
                });
                ui.end_row();
            }
            ui.label("Writes");
            ui.checkbox(&mut form.draft.writes, "Allow writes")
                .on_hover_text(
                    "Off: SQLite is opened read-only, database sessions are read-only, \
                 and HTTP may only GET. ODBC asks the driver for read-only access, which \
                 not every driver honours: use a read-only account where it matters.",
                );
            ui.end_row();
        });
}

impl ClioApp {
    /// *Tools > Data sources...*
    pub fn open_sources(&mut self) {
        if self.sources_panel.is_none() {
            self.sources_panel = Some(SourcesPanel::default());
        }
    }

    fn source_timeout(&self) -> Duration {
        Duration::from_millis(self.effective_settings().get(Setting::SourceTimeout))
    }

    /// Saves the form's definition. `Err` says why not; the form stays.
    pub fn save_source_form(&mut self, form: &SourceForm) -> Result<String, String> {
        let def = form.definition()?;
        let name = def.name.clone();
        self.user.sources.save(def, form.old.as_deref(), false)?;
        Ok(name)
    }

    /// A name for a copy of `name` that no definition has.
    fn copy_name(&self, name: &str) -> String {
        let taken: Vec<String> = self
            .user
            .sources
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.name)
            .collect();
        let mut candidate = format!("{name}_copy");
        let mut n = 2;
        while taken.contains(&candidate) {
            candidate = format!("{name}_copy{n}");
            n += 1;
        }
        candidate
    }

    /// The panel, while it is open.
    pub(crate) fn sources_window(&mut self, ctx: &egui::Context) {
        let Some(mut panel) = self.sources_panel.take() else {
            return;
        };
        // Tests finish in the background.
        if let Some((name, running)) = &panel.test
            && let Some(outcome) = running.poll()
        {
            panel.result = Some((name.clone(), outcome));
            panel.test = None;
        }
        if let Some(form) = &mut panel.form
            && let Some(running) = &form.test
            && let Some(outcome) = running.poll()
        {
            form.result = Some(outcome);
            form.test = None;
        }
        let testing = panel.test.is_some() || panel.form.as_ref().is_some_and(|f| f.test.is_some());
        if testing {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        let timeout = self.source_timeout();
        let listed = self.user.sources.list();
        let (mut open, mut close) = (true, false);
        let mut action = FormAction::None;
        let title = match &panel.form {
            None => "Data sources".to_owned(),
            Some(form) => match &form.old {
                None => "New data source".to_owned(),
                Some(name) => format!("Data source {name}"),
            },
        };
        egui::Window::new(title)
            .id(egui::Id::unique("data_sources_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .min_width(600.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| match panel.form.as_mut() {
                Some(form) => action = form_ui(ui, form, timeout),
                None => close = self.list_ui(ui, &mut panel, &listed, timeout),
            });

        match action {
            FormAction::Save => {
                if let Some(form) = panel.form.take() {
                    match self.save_source_form(&form) {
                        Ok(name) => {
                            panel.selected = Some(name);
                            panel.error = None;
                        }
                        Err(why) => {
                            panel.form = Some(SourceForm {
                                error: Some(why),
                                ..form
                            });
                        }
                    }
                }
            }
            FormAction::Cancel => panel.form = None,
            FormAction::None => {}
        }
        if open && !close {
            self.sources_panel = Some(panel);
        }
    }

    /// The list of definitions and its buttons. True closes the panel.
    fn list_ui(
        &mut self,
        ui: &mut egui::Ui,
        panel: &mut SourcesPanel,
        listed: &Result<Vec<SourceDef>, String>,
        timeout: Duration,
    ) -> bool {
        ui.weak(
            "Defined for you on this computer, passwords included. A workbook refers to a \
             data source by name, as source(\"Sales\"), so another computer needs the same \
             names defined.",
        );
        ui.add_space(6.0);
        let empty = Vec::new();
        let defs = match listed {
            Ok(defs) => defs,
            Err(why) => {
                ui.colored_label(error_color(ui), why);
                &empty
            }
        };
        if panel
            .selected
            .as_ref()
            .is_some_and(|s| !defs.iter().any(|d| &d.name == s))
        {
            panel.selected = None;
        }
        egui::ScrollArea::vertical()
            .id_salt("sources_list")
            .max_height(260.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if defs.is_empty() && listed.is_ok() {
                    ui.weak("No data sources are defined on this computer.");
                    return;
                }
                egui::Grid::new("sources_grid")
                    .num_columns(4)
                    .striped(true)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for head in ["Name", "Kind", "Where", "Writes"] {
                            ui.label(RichText::new(head).strong());
                        }
                        ui.end_row();
                        for def in defs {
                            let chosen = panel.selected.as_deref() == Some(def.name.as_str());
                            let response = ui.selectable_label(chosen, &def.name);
                            if response.clicked() {
                                panel.selected = Some(def.name.clone());
                            }
                            if response.double_clicked() {
                                panel.form = Some(SourceForm::edit(def.clone()));
                            }
                            ui.label(def.kind.label());
                            ui.label(RichText::new(def.target()).monospace());
                            ui.label(if def.writes { "Allowed" } else { "Read only" });
                            ui.end_row();
                        }
                    });
            });
        ui.add_space(6.0);

        let selected = panel
            .selected
            .as_ref()
            .and_then(|s| defs.iter().find(|d| &d.name == s))
            .cloned();
        if let Some(name) = panel.confirm_delete.clone() {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Delete the data source {name}? Its password is deleted with it."
                ));
                if ui.button("Delete it").clicked() {
                    match self.user.sources.remove(&name) {
                        Ok(()) => panel.error = None,
                        Err(why) => panel.error = Some(why),
                    }
                    panel.confirm_delete = None;
                }
                if ui.button("Keep").clicked() {
                    panel.confirm_delete = None;
                }
            });
        }
        let mut close = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(listed.is_ok(), egui::Button::new("New..."))
                .clicked()
            {
                panel.form = Some(SourceForm::new());
            }
            let some = selected.is_some();
            if ui.add_enabled(some, egui::Button::new("Edit...")).clicked()
                && let Some(def) = &selected
            {
                panel.form = Some(SourceForm::edit(def.clone()));
            }
            if ui.add_enabled(some, egui::Button::new("Copy...")).clicked()
                && let Some(def) = &selected
            {
                panel.form = Some(SourceForm::copy(def, self.copy_name(&def.name)));
            }
            if ui.add_enabled(some, egui::Button::new("Delete")).clicked()
                && let Some(def) = &selected
            {
                panel.confirm_delete = Some(def.name.clone());
            }
            let idle = panel.test.is_none();
            if ui
                .add_enabled(some && idle, egui::Button::new("Test"))
                .clicked()
                && let Some(def) = &selected
            {
                panel.result = None;
                panel.test = Some((def.name.clone(), RunningTest::start(def.clone(), timeout)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button("Close").clicked();
            });
        });
        if let Some((name, running)) = &panel.test {
            let mut cancel = false;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!(
                    "Testing {name}... {} s",
                    running.started.elapsed().as_secs()
                ));
                cancel = ui.button("Cancel").clicked();
            });
            if cancel {
                panel.test = None;
            }
        } else if let Some((name, outcome)) = &panel.result {
            outcome_line(ui, &format!("{name}: "), outcome);
        }
        if let Some(why) = &panel.error {
            ui.colored_label(error_color(ui), why);
        }
        close
    }
}

fn outcome_line(ui: &mut egui::Ui, prefix: &str, outcome: &TestOutcome) {
    let (mark, color) = if outcome.ok {
        ("Passed", crate::style::Palette::of_ui(ui).success)
    } else {
        ("Failed", error_color(ui))
    };
    ui.colored_label(color, format!("{prefix}{mark}: {}", outcome.message));
}

/// The form of one definition.
fn form_ui(ui: &mut egui::Ui, form: &mut SourceForm, timeout: Duration) -> FormAction {
    form_fields(ui, form);
    ui.add_space(6.0);
    let def = form.definition();
    // The target in full, so that it is plain where a secret will be sent.
    let target = match &def {
        Ok(def) => def.target(),
        Err(_) => form.draft.target(),
    };
    ui.horizontal(|ui| {
        ui.label("Where");
        ui.label(RichText::new(target).monospace());
    });
    if let Err(why) = &def {
        ui.colored_label(error_color(ui), why);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| match &form.test {
        Some(running) => {
            ui.spinner();
            ui.label(format!(
                "Testing... {} s",
                running.started.elapsed().as_secs()
            ));
            if ui.button("Cancel test").clicked() {
                form.test = None;
            }
        }
        None => {
            let can = def.is_ok();
            if ui.add_enabled(can, egui::Button::new("Test")).clicked()
                && let Ok(def) = &def
            {
                form.result = None;
                form.test = Some(RunningTest::start(def.clone(), timeout));
            }
        }
    });
    if let Some(outcome) = &form.result {
        outcome_line(ui, "", outcome);
    }
    if let Some(why) = &form.error {
        ui.colored_label(error_color(ui), why);
    }
    ui.separator();
    let mut action = FormAction::None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(def.is_ok(), egui::Button::new("Save"))
            .clicked()
        {
            action = FormAction::Save;
        }
        if ui.button("Cancel").clicked() {
            action = FormAction::Cancel;
        }
        if form.result.as_ref().is_some_and(|r| !r.ok) {
            ui.weak("The last test failed; the definition can still be saved.");
        }
    });
    action
}

#[cfg(test)]
#[path = "sources_ui_tests.rs"]
mod sources_ui_tests;
