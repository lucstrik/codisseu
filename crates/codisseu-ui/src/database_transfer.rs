//! Snapshot export and explicit, previewed imports. No database reads for grid exports.
use super::*;
use codisseu_core::{
    DbCommand,
    remote::{Cell, Engine, RemoteCommand, TableRef},
    transfer::{
        self, FileCommand, FileService, ImportApproval, ImportMapping, ImportRequest, ImportTarget,
        NoSqlImportRequest, SqlFlavor, TransferData, TransferFormat, TransferOptions,
        TransferRequest,
    },
    transfer_nosql::{NoSqlExportRequest, NoSqlImportTarget, TransferBytes, UnknownTtlPolicy},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialog {
    Export,
    Import,
    NativeExport,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Job {
    Render,
    Read,
    Write,
    Directory,
    Clipboard,
}
const FORMATS: &[(&str, &str)] = &[
    ("insert", "SQL Inserts"),
    ("sql-updates", "SQL Updates"),
    ("where-clause", "Where Clause"),
    ("xlsx", "Excel (xlsx)"),
    ("csv", "CSV"),
    ("tsv", "TSV"),
    ("pipe", "Pipe-separated"),
    ("semicolon", "Semicolon-separated"),
    ("csv-scripted", "CSV (scripted)"),
    ("html-groovy", "HTML (groovy)"),
    ("html-js", "HTML (js)"),
    ("json", "JSON"),
    ("markdown", "Markdown"),
    ("one-row", "One-row"),
    ("pretty", "Pretty"),
    ("python-dataframe", "Python-DataFrame"),
    ("sql-insert-multirow", "SQL-Insert-Multirow"),
    ("sql-insert-statements", "SQL-Insert-Statements"),
    ("xml", "XML"),
    ("ndjson", "NDJSON"),
    ("codisseu-json", "Codisseu JSON (tipado)"),
    ("codisseu-ndjson", "Codisseu NDJSON (tipado)"),
];
fn format(id: &str) -> TransferFormat {
    match id {
        "csv" => TransferFormat::Csv,
        "tsv" => TransferFormat::Tsv,
        "json" => TransferFormat::Json,
        "ndjson" => TransferFormat::Ndjson,
        "codisseu-json" => TransferFormat::CodisseuJson,
        "codisseu-ndjson" => TransferFormat::CodisseuNdjson,
        "insert" => TransferFormat::SqlInsert,
        id => TransferFormat::Extra(id.into()),
    }
}
fn extension(id: &str) -> &str {
    match id {
        "insert" => "sql",
        "csv" => "csv",
        "tsv" => "tsv",
        "json" | "codisseu-json" => "json",
        "ndjson" | "codisseu-ndjson" => "jsonl",
        id => codisseu_core::transfer_formats_extra::extension(id),
    }
}
fn label(id: &str) -> &str {
    FORMATS.iter().find(|v| v.0 == id).map_or(id, |v| v.1)
}
fn is_sql(id: &str) -> bool {
    matches!(
        id,
        "insert" | "sql-updates" | "sql-insert-multirow" | "sql-insert-statements"
    )
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, id: &str) -> bool {
    ui.horizontal(|ui| {
        let l = ui.add_sized([104., 28.], egui::Label::new(label));
        ui.add(
            egui::TextEdit::singleline(value)
                .id_salt(id)
                .desired_width((ui.available_width() - 4.).max(60.))
                .margin(egui::vec2(7., 5.)),
        )
        .labelled_by(l.id)
        .changed()
    })
    .inner
}
pub(super) struct TransferUi {
    cached: Option<std::sync::Arc<TransferData>>,
    cache_key: Option<(String, u64)>,
    presentation_revision: u64,
    copy_format: String,
    read_key: Option<String>,
    render_key: Option<String>,
    files: FileService,
    dialog: Option<Dialog>,
    job: Option<Job>,
    format: String,
    options: TransferOptions,
    snapshot: Option<TransferData>,
    rendered: Option<Vec<u8>>,
    dirty: bool,
    error: Option<String>,
    notice: String,
    path: String,
    overwrite: bool,
    namespace: String,
    table: String,
    proposals: std::collections::BTreeMap<(usize, usize), Cell>,
    include_proposed: bool,
    from_grid: bool,
    mapping: Vec<(bool, String)>,
    allow_partial: bool,
    identity: String,
    approval: Option<u64>,
    apply_pending: bool,
    preview_pending: bool,
    native_raw: Option<String>,
    native_key: Option<String>,
    native_pending: bool,
    keys: String,
    filter: String,
    ttl: u8,
    ttl_seconds: u32,
    browser: bool,
    browser_path: String,
    browser_entries: Vec<transfer::DirectoryEntry>,
    browser_truncated: bool,
}
impl Default for TransferUi {
    fn default() -> Self {
        Self {
            cached: None,
            cache_key: None,
            presentation_revision: 0,
            copy_format: "csv".into(),
            read_key: None,
            render_key: None,
            files: FileService::new(),
            dialog: None,
            job: None,
            format: "csv".into(),
            options: TransferOptions::default(),
            snapshot: None,
            rendered: None,
            dirty: false,
            error: None,
            notice: String::new(),
            path: String::new(),
            overwrite: false,
            namespace: String::new(),
            table: String::new(),
            proposals: Default::default(),
            include_proposed: false,
            from_grid: false,
            mapping: Vec::new(),
            allow_partial: false,
            identity: String::new(),
            approval: None,
            apply_pending: false,
            preview_pending: false,
            native_raw: None,
            native_key: None,
            native_pending: false,
            keys: String::new(),
            filter: "{}".into(),
            ttl: 0,
            ttl_seconds: 3600,
            browser: false,
            browser_path: String::new(),
            browser_entries: Vec::new(),
            browser_truncated: false,
        }
    }
}
impl TransferUi {
    fn export_data(&self) -> Option<TransferData> {
        let mut data = self.snapshot.clone()?;
        if self.include_proposed {
            for (&(row, column), cell) in &self.proposals {
                if let Some(value) = data.rows.get_mut(row).and_then(|r| r.get_mut(column)) {
                    *value = cell.clone();
                }
            }
        }
        Some(data)
    }
    fn native_source_key(&self) -> String {
        format!("{:?}", (&self.keys, &self.table, &self.filter))
    }
    fn source_key(&self) -> String {
        format!(
            "{:?}",
            (
                &self.path,
                format(&self.format),
                self.options.header,
                &self.options.null_marker
            )
        )
    }
    fn options(&self) -> TransferOptions {
        let mut o = self.options.clone();
        o.format = format(&self.format);
        o.sql_table = Some(TableRef {
            namespace: self.namespace.clone(),
            name: self.table.clone(),
        });
        o
    }
    fn invalidate(&mut self) {
        self.approval = None;
        self.preview_pending = false;
        self.error = None;
    }
    fn dispatch(&mut self, c: FileCommand, job: Job) {
        let read = matches!(
            c,
            FileCommand::Read { .. } | FileCommand::ReadRaw { .. } | FileCommand::ReadXlsx { .. }
        );
        let render = matches!(c, FileCommand::Render { .. });
        match self.files.dispatch(c) {
            Ok(()) => {
                self.job = Some(job);
                if read {
                    self.read_key = Some(self.source_key());
                }
                if render && job == Job::Render {
                    self.render_key =
                        Some(format!("{:?}", (self.options(), self.include_proposed)));
                }
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn set_format(&mut self, id: &str) {
        if self.format != id {
            self.format = id.into();
            self.notice.clear();
            if id != "xlsx" {
                self.copy_format = id.into();
            }
            self.options.transpose = false;
            self.dirty = true;
            self.rendered = None;
            if self.dialog == Some(Dialog::Export) {
                let mut p = PathBuf::from(&self.path);
                p.set_extension(extension(id));
                self.path = p.to_string_lossy().into();
            }
        }
    }
    fn set_data(&mut self, data: TransferData) {
        self.mapping = data.columns.iter().map(|n| (true, n.clone())).collect();
        self.snapshot = Some(data);
        self.invalidate();
    }
}
impl CodisseuApp {
    /// A selected project must wait for accepted file writes and import ACKs
    /// before its transfer presentation can be cleared for another project.
    pub(super) fn transfer_work_pending(&self) -> bool {
        self.transfer_ui.files.is_busy()
            || self.transfer_ui.apply_pending
            || self.transfer_ui.preview_pending
            || self.transfer_ui.native_pending
    }

    pub(super) fn transfer_modal_open(&self) -> bool {
        self.transfer_views.columns_open
            || self.transfer_views.grid.local.open.is_some()
            || self.query_library_modal_open()
            || self.transfer_ui.dialog.is_some()
            || self.transfer_ui.browser
            || self.query_export_modal_open()
            || self.sql_productivity_modal_open()
    }
    fn transfer_identity(&self) -> String {
        if self.database_ui.remote_results {
            self.service
                .remote_db
                .active
                .as_ref()
                .map_or_else(String::new, |c| {
                    format!("{}:{}", c.id, self.service.remote_db.session_generation)
                })
        } else {
            format!("sqlite:{:?}", self.service.db.config)
        }
    }
    pub(super) fn result_snapshot(&self) -> Option<std::sync::Arc<TransferData>> {
        self.transfer_ui.cached.clone()
    }
    pub(super) fn result_query(&self) -> Option<&str> {
        if self.database_ui.remote_results {
            self.database_ui.result_sql.as_deref()
        } else {
            self.database_ui.sqlite_result_sql.as_deref()
        }
    }
    fn begin_transfer(&mut self, dialog: Dialog) {
        if dialog == Dialog::Import {
            self.service.remote_db.import_report = None;
            self.service.db.import_report = None;
        }
        let data = self.result_snapshot().map(|d| (*d).clone());
        let identity = self.transfer_identity();
        let query = self.result_query().map(str::to_owned);
        let state = &mut self.transfer_ui;
        state.proposals = self.database_ui.staged.clone();
        state.include_proposed = false;
        state.dialog = Some(dialog);
        state.from_grid = false;
        if dialog == Dialog::Export {
            state.format = state.copy_format.clone();
        }
        if dialog == Dialog::Import
            && !matches!(
                state.format.as_str(),
                "csv"
                    | "tsv"
                    | "pipe"
                    | "semicolon"
                    | "csv-scripted"
                    | "json"
                    | "ndjson"
                    | "codisseu-json"
                    | "codisseu-ndjson"
                    | "xlsx"
            )
        {
            state.format = "csv".into();
        }

        state.error = None;
        state.notice.clear();
        state.approval = None;
        state.preview_pending = false;
        state.identity = identity;
        state.rendered = None;
        state.native_raw = None;
        state.native_pending = false;
        state.overwrite = false;
        state.allow_partial = false;
        state.options.query = query;
        state.options.sql_key_columns.clear();
        state.options.sql_flavor = SqlFlavor::Sqlite;
        state.namespace = "main".into();
        state.table = "MY_TABLE".into();
        if let Some(c) = self
            .service
            .remote_db
            .active
            .as_ref()
            .filter(|_| self.database_ui.remote_results)
        {
            state.options.sql_flavor = if matches!(c.engine, Engine::Mysql | Engine::AuroraMysql) {
                SqlFlavor::Mysql
            } else {
                SqlFlavor::Postgres
            };
            state.namespace = if c.engine.is_sql() {
                if matches!(c.engine, Engine::Postgres | Engine::AuroraPostgres) {
                    "public".into()
                } else {
                    c.database.clone()
                }
            } else {
                String::new()
            };
        }
        if self.database_ui.remote_results
            && let Some((table, cols)) = &self.service.remote_db.origin
        {
            state.namespace = table.namespace.clone();
            state.table = table.name.clone();
            state.options.sql_key_columns = cols
                .iter()
                .filter(|c| c.primary_key)
                .map(|c| c.name.clone())
                .collect();
        }
        if let Some(data) = data {
            state.set_data(data);
        } else {
            state.snapshot = None;
            state.mapping.clear();
        }
        if dialog == Dialog::Import {
            state.snapshot = None;
            state.mapping.clear();
        }
        state.dirty = dialog == Dialog::Export;
        state.path = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(format!(
                "result.{}",
                if dialog == Dialog::NativeExport {
                    "json"
                } else {
                    extension(&state.format)
                }
            ))
            .to_string_lossy()
            .into_owned();
    }
    pub(super) fn result_transfer_toolbar(&mut self, ui: &mut egui::Ui) {
        let data = self.result_snapshot();
        let available = data.is_some();
        let query = self.result_query().map(str::to_owned);
        let revision = self.result_transfer_revision();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(data) = &data {
                let grid_mode =
                    self.transfer_views.mode == database_transfer_views::ResultView::Grid;
                if ui
                    .add_enabled_ui(grid_mode, |ui| {
                        self.transfer_views.grid.local.toolbar(ui, &self.tokens)
                    })
                    .inner
                {
                    self.transfer_views.grid.changed();
                }
                let find = components::database_icon_button(
                    ui,
                    Icon::Search,
                    "Find on Current Page",
                    grid_mode,
                );
                self.transfer_views.grid.find_button = Some(find.id);
                if find.clicked() {
                    let grid = &mut self.transfer_views.grid;
                    grid.find_open = !grid.find_open;
                    grid.focus_find = grid.find_open;
                    grid.changed();
                }
                self.transfer_views
                    .toolbar(ui, data, revision, query.as_deref(), &self.tokens);
            }
            if components::database_icon_button(ui, Icon::Import, "Import Data from File…", true)
                .clicked()
            {
                self.begin_transfer(Dialog::Import);
            }
            if components::database_icon_button(
                ui,
                Icon::Import,
                "Copy to Database…",
                available && !self.transfer_is_native(),
            )
            .clicked()
            {
                self.begin_transfer(Dialog::Import);
                self.transfer_ui.path.clear();
                self.transfer_ui.from_grid = true;
                if let Some(data) = data.as_ref() {
                    self.transfer_ui.set_data((**data).clone());
                }
            }
            let export = components::database_icon_button(
                ui,
                Icon::Export,
                "Export Data…",
                available || self.transfer_is_native(),
            );
            if export.clicked() {
                self.begin_transfer(if available {
                    Dialog::Export
                } else {
                    Dialog::NativeExport
                });
            }
            export.context_menu(|ui| {
                if ui
                    .add_enabled(
                        self.can_export_query(),
                        egui::Button::new("Exportar consulta completa (CSV)…"),
                    )
                    .clicked()
                {
                    self.begin_query_export();
                    ui.close();
                }
                if self.transfer_is_native()
                    && ui.button("Export Native Data… · binary / TTL").clicked()
                {
                    self.begin_transfer(Dialog::NativeExport);
                    self.transfer_ui.path = PathBuf::from(&self.transfer_ui.path)
                        .with_extension("json")
                        .to_string_lossy()
                        .into();
                    ui.close();
                }
            });
            let mut chosen = self.transfer_ui.copy_format.clone();
            egui::ComboBox::from_id_salt("results.extractor")
                .width(53.)
                .truncate()
                .height(380.)
                .selected_text(label(&chosen))
                .show_ui(ui, |ui| {
                    for (id, name) in FORMATS.iter().filter(|(id, _)| *id != "xlsx") {
                        ui.selectable_value(&mut chosen, id.to_string(), *name);
                    }
                })
                .response
                .on_hover_text("Data extractor · formato de exportação e Ctrl+C");
            if chosen != self.transfer_ui.copy_format {
                self.transfer_ui.set_format(&chosen);
                if let Some(focus) = self.grid_focus {
                    ui.memory_mut(|memory| memory.request_focus(focus));
                }
            }
        });
    }
    pub(super) fn poll_transfer(&mut self, ctx: &egui::Context) {
        let cache_key = (self.transfer_identity(), self.worker_result_revision());
        if self.transfer_ui.cache_key.as_ref() != Some(&cache_key) {
            self.transfer_ui.cached = if self.database_ui.remote_results {
                self.service
                    .remote_db
                    .result
                    .as_ref()
                    .map(TransferData::from_remote)
            } else {
                self.service
                    .db
                    .result
                    .as_ref()
                    .map(TransferData::from_sqlite)
            }
            .map(std::sync::Arc::new);
            self.transfer_ui.cache_key = Some(cache_key);
            self.transfer_ui.presentation_revision =
                self.transfer_ui.presentation_revision.wrapping_add(1);
        }

        if self.transfer_ui.files.poll() {
            let state = &mut self.transfer_ui;
            state.error = state.files.error.clone();
            match state.job.take() {
                Some(Job::Render) => {
                    if state.render_key.as_ref()
                        == Some(&format!("{:?}", (state.options(), state.include_proposed)))
                    {
                        state.rendered = state.files.rendered.take();
                    } else {
                        state.dirty = true;
                        state.files.rendered = None;
                    }
                }
                Some(Job::Clipboard) => {
                    if let Some(bytes) = state.files.rendered.take() {
                        match String::from_utf8(bytes) {
                            Ok(text) => {
                                ctx.copy_text(text);
                                state.notice = "Seleção copiada no formato escolhido".into();
                            }
                            Err(_) => {
                                state.error = Some("XLSX é binário: use Export to File".into())
                            }
                        }
                    }
                }
                Some(Job::Read) => {
                    if state.read_key.as_ref() != Some(&state.source_key()) {
                        state.files.data = None;
                        state.files.raw_text = None;
                        state.error = Some(
                            "Origem do arquivo alterada durante a leitura. Clique Load novamente."
                                .into(),
                        );
                        return;
                    }
                    if let Some(data) = state.files.data.take() {
                        state.set_data(data);
                    }
                    if let Some(text) = state.files.raw_text.take() {
                        state.native_raw = Some(text);
                        state.invalidate();
                    }
                }
                Some(Job::Write) => {
                    if let Some(r) = &state.files.report {
                        state.notice =
                            format!("Arquivo salvo · {} bytes · {}", r.bytes, r.path.display());
                    }
                }
                Some(Job::Directory) => {
                    if let Some(d) = state.files.directory.take() {
                        state.browser_path = d.path.to_string_lossy().into();
                        state.browser_entries = d.entries;
                        state.browser_truncated = d.truncated;
                    }
                }
                None => {}
            }
        }
        let identity = self.transfer_identity();
        if self.transfer_ui.dialog.is_some() && identity != self.transfer_ui.identity {
            self.transfer_ui.invalidate();
            self.transfer_ui.error =
                Some("Conexão alterada. Reabra a transferência na origem desejada.".into());
        }
        if self.transfer_ui.apply_pending
            && !self.service.remote_db.is_busy()
            && !self.service.db.is_busy()
        {
            self.transfer_ui.apply_pending = false;
            let report = if self.database_ui.remote_results {
                self.service.remote_db.import_report.as_ref()
            } else {
                self.service.db.import_report.as_ref()
            };
            self.transfer_ui.notice = report.map(|r| r.summary.clone()).unwrap_or_else(|| {
                "Operação encerrada. Confira o relatório antes de repetir o lote.".into()
            });
            let error = if self.database_ui.remote_results {
                self.service.remote_db.error.as_ref()
            } else {
                self.service.db.last_error.as_ref()
            };
            self.transfer_ui.error = error.cloned();
        }
        if self.transfer_ui.preview_pending
            && !self.service.remote_db.is_busy()
            && !self.service.db.is_busy()
        {
            let state = &mut self.transfer_ui;
            let p = if self.database_ui.remote_results {
                self.service.remote_db.import_preview.as_ref()
            } else {
                self.service.db.import_preview.as_ref()
            };
            if let Some(p) = p {
                state.approval = Some(p.preview_id);
                state.notice = format!("{}\n{}", p.description, p.warnings.join("\n"));
            } else if let Some(e) = if self.database_ui.remote_results {
                self.service.remote_db.error.as_ref()
            } else {
                self.service.db.last_error.as_ref()
            } {
                state.error = Some(e.clone());
            }
            state.preview_pending = false;
        }
        if self.transfer_ui.native_pending && !self.service.remote_db.is_busy() {
            self.transfer_ui.native_pending = false;
            if self.transfer_ui.native_key.as_ref() != Some(&self.transfer_ui.native_source_key()) {
                self.transfer_ui.native_raw = None;
                self.transfer_ui.error = Some(
                    "Seleção de registros alterada durante a leitura. Leia os registros novamente."
                        .into(),
                );
            } else if let Some(raw) = self.service.remote_db.native_export.clone() {
                self.transfer_ui.native_raw = Some(raw);
            } else {
                self.transfer_ui.error = self.service.remote_db.error.clone();
            }
        }
        if self.transfer_ui.dirty
            && !self.transfer_ui.files.is_busy()
            && self.transfer_ui.dialog == Some(Dialog::Export)
        {
            let state = &mut self.transfer_ui;
            state.dirty = false;
            if let Some(data) = state.export_data() {
                let options = state.options();
                state.dispatch(FileCommand::Render { options, data }, Job::Render);
            }
        }
        if self.transfer_ui.files.is_busy()
            || self.transfer_ui.preview_pending
            || self.transfer_ui.apply_pending
            || self.transfer_ui.native_pending
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(20));
        }
    }
    pub(super) fn transfer_windows(&mut self, ctx: &egui::Context) {
        match self.transfer_ui.dialog {
            Some(Dialog::Export) => self.export_window(ctx),
            Some(Dialog::Import) => self.import_window(ctx),
            Some(Dialog::NativeExport) => self.native_export_window(ctx),
            None => {}
        }
        if self.transfer_ui.browser {
            self.transfer_browser(ctx);
        }
    }
    fn export_window(&mut self, ctx: &egui::Context) {
        let mut full_query = false;
        let mut close = false;
        let mut save = false;
        let mut browse = false;
        let mut copy = false;
        let frame = egui::Frame::new()
            .fill(color(self.tokens.panel))
            .stroke(egui::Stroke::new(1., color(self.tokens.popup_border)))
            .corner_radius(self.tokens.control_corner_radius)
            .inner_margin(12);
        egui::Modal::new(egui::Id::unique("transfer.export")).frame(frame).show(ctx,|ui|{
   ui.set_width(810_f32.min(ctx.content_rect().width()-56.));self.tokens.form_style(ui);
   ui.horizontal(|ui|{ui.label(RichText::new("Export Data").strong());ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{close=components::database_icon_button(ui,Icon::Close,"Fechar exportação",true).clicked();});});ui.separator();
   full_query=ui.add_enabled(self.can_export_query(), self.tokens.dialog_button("Exportar consulta completa (CSV)…",false)).clicked();
   let state=&mut self.transfer_ui;
   let body_height=(ctx.content_rect().height()-170.).clamp(220.,415.);
   let right_width=(ui.available_width()-313.).max(150.);
   ui.horizontal_top(|ui|{
    ui.allocate_ui_with_layout(egui::vec2(292.,body_height),egui::Layout::top_down(egui::Align::Min),|ui|{
     ui.set_min_height(body_height);let before=format!("{:?}",state.options());
     let mut chosen=state.format.clone();ui.horizontal(|ui|{ui.add_sized([85.,28.],egui::Label::new("Extractor:"));egui::ComboBox::from_id_salt("export.extractor").width(185.).height(400.).selected_text(label(&chosen)).show_ui(ui,|ui|{for(id,name)in FORMATS{ui.selectable_value(&mut chosen,id.to_string(),*name);}});});state.set_format(&chosen);
     ui.add_space(10.);
     ui.add_enabled_ui(!is_sql(&state.format),|ui|{ui.checkbox(&mut state.options.transpose,"Transpose");});
     if state.format=="xlsx"{ui.checkbox(&mut state.options.xlsx_query_sheet,"Output query to separate sheet");ui.checkbox(&mut state.options.xlsx_exact_numbers,"Preservar números longos como texto");}else{ui.checkbox(&mut state.options.header,"Add column header");ui.checkbox(&mut state.options.row_header,"Add row header");}
     if matches!(state.format.as_str(),"csv"|"tsv"|"pipe"|"semicolon"|"csv-scripted"){field(ui,"NULL:",&mut state.options.null_marker,"export.null");ui.checkbox(&mut state.options.spreadsheet_safe,"Proteger fórmulas em planilhas").on_hover_text("Transformação explícita: prefixa texto que pode ser interpretado como fórmula.");}
     if is_sql(&state.format){field(ui,"Schema:",&mut state.namespace,"export.schema");field(ui,"Table:",&mut state.table,"export.table");ui.label(RichText::new("SQL gerado para copiar ou salvar; não é executado.").small().color(color(self.tokens.muted)));}
     if before!=format!("{:?}",state.options()){state.dirty=true;state.rendered=None;}
     if !state.proposals.is_empty()&&ui.checkbox(&mut state.include_proposed,"Incluir alterações locais ainda não enviadas").changed(){state.dirty=true;state.rendered=None;}
     ui.add_space(12.);let output_label=ui.label(RichText::new("Output file").strong());
     ui.horizontal(|ui|{let width=(ui.available_width()-30.).max(80.);ui.add(egui::TextEdit::singleline(&mut state.path).id_salt("export.path").desired_width(width)).labelled_by(output_label.id);browse=components::database_icon_button(ui,Icon::Folder,"Escolher arquivo de exportação",!state.files.is_busy()).clicked();});
     ui.checkbox(&mut state.overwrite,"Substituir arquivo existente");ui.add_space(10.);
     if let Some(d)=&state.snapshot{ui.label(format!("{} linhas · {} colunas",d.rows.len(),d.columns.len()));ui.label(RichText::new(if state.include_proposed{"Snapshot com propostas locais · sem enviar ao banco"}else if d.source_truncated{"Página parcial · somente as linhas carregadas"}else{"Snapshot da última leitura · sem nova consulta"}).small().color(color(self.tokens.muted)));}
    });
    let (divider,_)=ui.allocate_exact_size(egui::vec2(1.,body_height),egui::Sense::hover());ui.painter().rect_filled(divider,0.,color(self.tokens.border));
    ui.allocate_ui_with_layout(egui::vec2(right_width,body_height),egui::Layout::top_down(egui::Align::Min),|ui|{ui.label(RichText::new("Preview").strong());let preview=if state.format=="xlsx"{"Arquivo Excel binário (.xlsx)\nPlanilhas: Result 1 e Query (quando disponível)".to_string()}else if state.dirty||state.files.is_busy(){"Preparando prévia…".into()}else{state.rendered.as_ref().and_then(|v|std::str::from_utf8(v).ok()).map(|s|s.chars().take(16000).collect()).unwrap_or_default()};
     egui::Frame::new().fill(color(self.tokens.shell_panel_background)).inner_margin(8).show(ui,|ui|{ui.set_width((ui.available_width()-16.).max(120.));egui::ScrollArea::both().id_salt("export.preview").max_height(body_height-40.).min_scrolled_height(body_height-40.).codisseu().show(ui,|ui|{ui.add(egui::Label::new(RichText::new(&preview).monospace()).selectable(true).wrap_mode(egui::TextWrapMode::Extend));});});
    });
   });
   ui.horizontal(|ui|{ui.set_min_height(20.);if let Some(e)=&state.error{crate::status_messages::status_message(ui, e).on_hover_text(e);}else{ui.add(egui::Label::new(&state.notice).truncate()).on_hover_text(&state.notice);}});ui.separator();
   ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{save=ui.add_enabled(!state.files.is_busy()&&!state.dirty&&state.rendered.is_some(),self.tokens.dialog_button("Export to File",true)).clicked();close|=ui.add(self.tokens.dialog_button("Cancel",false)).clicked();copy=ui.add_enabled(state.format!="xlsx"&&!state.files.is_busy()&&!state.dirty&&state.rendered.is_some(),self.tokens.dialog_button("Copy to Clipboard",false)).clicked();});
  });
        if full_query {
            self.close_transfer();
            self.begin_query_export();
        }
        if browse {
            self.open_transfer_browser();
        }
        if copy
            && let Some(bytes) = &self.transfer_ui.rendered
            && let Ok(text) = std::str::from_utf8(bytes)
        {
            ctx.copy_text(text.to_owned());
            self.transfer_ui.notice = "Copiado para a área de transferência".into();
        }
        if save {
            let state = &mut self.transfer_ui;
            if let Some(data) = state.export_data() {
                state.dispatch(
                    FileCommand::Write {
                        path: state.path.clone().into(),
                        options: state.options(),
                        data,
                        overwrite: state.overwrite,
                    },
                    Job::Write,
                );
            }
        }
        if close {
            self.close_transfer();
        }
    }
    fn close_transfer(&mut self) {
        if self.transfer_ui.apply_pending {
            let command = if self.database_ui.remote_results {
                Command::RemoteDatabase(RemoteCommand::Cancel)
            } else {
                Command::Database(DbCommand::Cancel)
            };
            match self.service.dispatch(command) {
                Ok(()) => {
                    self.transfer_ui.notice =
                        "Cancelamento solicitado · aguardando o resultado real do lote…".into()
                }
                Err(e) => self.transfer_ui.error = Some(e),
            }
            return;
        }

        self.transfer_ui.dialog = None;
        self.transfer_ui.approval = None;
        self.transfer_ui.browser = false;
        if self.transfer_ui.files.is_busy() {
            let _ = self.transfer_ui.files.dispatch(FileCommand::Cancel);
        }
        self.transfer_ui.snapshot = None;
        self.transfer_ui.rendered = None;
        self.transfer_ui.native_raw = None;
    }
    fn open_transfer_browser(&mut self) {
        let state = &mut self.transfer_ui;
        state.browser = true;
        let path = PathBuf::from(&state.path);
        let directory = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        state.browser_path = directory.to_string_lossy().into();
        state.dispatch(
            FileCommand::ListDirectory {
                path: directory.into(),
            },
            Job::Directory,
        );
    }
    fn transfer_browser(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut choose = None;
        let mut refresh = None;
        egui::Modal::new(egui::Id::unique("transfer.browser"))
            .frame(
                egui::Frame::new()
                    .fill(color(self.tokens.panel))
                    .stroke(egui::Stroke::new(1., color(self.tokens.popup_border)))
                    .corner_radius(self.tokens.control_corner_radius)
                    .inner_margin(12),
            )
            .show(ctx, |ui| {
                ui.set_width(650_f32.min(ctx.content_rect().width() - 56.));
                ui.label(RichText::new("Escolher arquivo").strong());
                let state = &mut self.transfer_ui;
                ui.horizontal(|ui| {
                    if components::database_icon_button(
                        ui,
                        Icon::Previous,
                        "Pasta superior",
                        !state.files.is_busy(),
                    )
                    .clicked()
                    {
                        refresh = PathBuf::from(&state.browser_path)
                            .parent()
                            .map(std::path::Path::to_path_buf);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut state.browser_path)
                            .desired_width((ui.available_width() - 70.).max(120.)),
                    );
                    if ui
                        .add_enabled(!state.files.is_busy(), egui::Button::new("Abrir"))
                        .clicked()
                    {
                        refresh = Some(PathBuf::from(&state.browser_path));
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("transfer.files")
                    .max_height(310.)
                    .min_scrolled_height(310.)
                    .codisseu()
                    .show(ui, |ui| {
                        for entry in &state.browser_entries {
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(20., 22.),
                                    egui::Sense::hover(),
                                );
                                components::paint_icon(
                                    ui,
                                    rect,
                                    if entry.is_dir {
                                        Icon::Folder
                                    } else {
                                        Icon::File
                                    },
                                    color(self.tokens.database_grid_foreground),
                                );
                                let r = ui.selectable_label(
                                    state.path == entry.path.to_string_lossy(),
                                    &entry.name,
                                );
                                if r.clicked() {
                                    if entry.is_dir {
                                        refresh = Some(entry.path.clone());
                                    } else {
                                        choose = Some(entry.path.to_string_lossy().into_owned());
                                    }
                                }
                            });
                        }
                    });
                if state.browser_truncated {
                    ui.label("Mostrando os primeiros 256 itens. Refine o caminho.");
                }
                if let Some(e) = &state.error {
                    crate::status_messages::status_message(ui, e);
                }
                close = ui.button("Fechar").clicked();
            });
        if let Some(path) = refresh {
            self.transfer_ui
                .dispatch(FileCommand::ListDirectory { path }, Job::Directory);
        }
        if let Some(path) = choose {
            self.transfer_ui.path = path;
            self.transfer_ui.invalidate();
            if self.transfer_ui.dialog == Some(Dialog::Import) {
                self.transfer_ui.snapshot = None;
                self.transfer_ui.native_raw = None;
            }
            close = true;
        }
        if close {
            self.transfer_ui.browser = false;
        }
    }

    pub(super) fn result_transfer_revision(&self) -> u64 {
        self.transfer_ui.presentation_revision
    }
    fn worker_result_revision(&self) -> u64 {
        if self.database_ui.remote_results {
            self.service.remote_db.revision
        } else {
            self.service.db.result_revision().unwrap_or(0)
        }
    }
    pub(super) fn alternative_results(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(data) = self.result_snapshot() else {
            return false;
        };
        let revision = self.result_transfer_revision();
        let query = self.result_query().map(str::to_owned);
        self.transfer_views
            .show(ui, &data, revision, query.as_deref(), &self.tokens)
    }
    pub(super) fn sync_grid_selection(&mut self) {
        let identity = self.transfer_identity();
        let revision = if self.database_ui.remote_results {
            self.service
                .remote_db
                .result
                .as_ref()
                .map(|_| self.service.remote_db.revision)
        } else {
            self.service.db.result_revision()
        };
        self.grid_selection.sync_revision(&identity, revision);
        if let Some(data) = self.result_snapshot() {
            self.transfer_views
                .grid
                .sync_source(&identity, &data.columns);
        } else {
            self.transfer_views.grid = database_grid::GridPresentation::default();
        }
    }
    pub(super) fn copy_result_selection(&mut self, ctx: &egui::Context) {
        if self.transfer_modal_open() || self.database_ui.edit.is_some() {
            return;
        }
        if ctx.memory(|m| m.focused()) != self.grid_focus || self.grid_focus.is_none() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
            if let Some(data) = self.result_snapshot()
                && !data.rows.is_empty()
            {
                self.grid_selection.row_clicked(0, Modifiers::NONE);
                self.grid_selection
                    .row_clicked(data.rows.len() - 1, Modifiers::SHIFT);
            }
            return;
        }
        let copy = ctx.input_mut(|i| {
            let event = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            if event {
                i.events.retain(|e| !matches!(e, egui::Event::Copy));
                true
            } else {
                i.consume_key(Modifiers::COMMAND, Key::C)
            }
        });
        if !copy || self.grid_selection.is_empty() {
            return;
        }
        if self.transfer_ui.copy_format == "xlsx" {
            self.service.last_error =
                Some("Excel é binário: use Export Data para salvar .xlsx".into());
            return;
        }
        let Some(data) = self.result_snapshot() else {
            return;
        };
        let mut visible = None;
        if !self.database_ui.staged.is_empty() {
            let mut changed = (*data).clone();
            for (&(row, column), cell) in &self.database_ui.staged {
                if let Some(value) = changed.rows.get_mut(row).and_then(|r| r.get_mut(column)) {
                    *value = cell.clone();
                }
            }
            visible = Some(changed);
        }
        let snapshot = visible.as_ref().unwrap_or(&data);
        let selection = if self.transfer_views.mode == database_transfer_views::ResultView::Grid
            && self.transfer_views.grid.has_projection()
        {
            self.grid_selection.snapshot_projected(
                snapshot,
                &self.transfer_views.grid.visible_rows(),
                &self.transfer_views.grid.visible_columns(),
            )
        } else {
            self.grid_selection.snapshot(snapshot)
        };
        let selected = match selection {
            Ok(data) => data,
            Err(e) => {
                self.service.last_error = Some(e);
                return;
            }
        };
        if !self.grid_selection.has_rows()
            && selected.rows.len() == 1
            && selected.columns.len() == 1
        {
            ctx.copy_text(selected.rows[0][0].clipboard_text());
            return;
        }
        let mut options = self.transfer_ui.options.clone();
        options.format = format(&self.transfer_ui.copy_format);
        options.query = self.result_query().map(str::to_owned);
        options.header = false;
        options.sql_flavor = if !self.database_ui.remote_results {
            SqlFlavor::Sqlite
        } else if self
            .service
            .remote_db
            .active
            .as_ref()
            .is_some_and(|c| matches!(c.engine, Engine::Mysql | Engine::AuroraMysql))
        {
            SqlFlavor::Mysql
        } else {
            SqlFlavor::Postgres
        };
        options.sql_table = self
            .service
            .remote_db
            .origin
            .as_ref()
            .filter(|_| self.database_ui.remote_results)
            .map(|(t, _)| t.clone())
            .or_else(|| {
                Some(TableRef {
                    namespace: String::new(),
                    name: "MY_TABLE".into(),
                })
            });
        options.sql_key_columns.clear();
        self.transfer_ui.dispatch(
            FileCommand::Render {
                options,
                data: selected,
            },
            Job::Clipboard,
        );
    }
    fn transfer_is_native(&self) -> bool {
        self.database_ui.remote_results
            && self
                .service
                .remote_db
                .active
                .as_ref()
                .is_some_and(|c| !c.engine.is_sql())
    }
    fn import_window(&mut self, ctx: &egui::Context) {
        let native = self.transfer_is_native();
        let mongo = self
            .service
            .remote_db
            .active
            .as_ref()
            .is_some_and(|c| c.engine == Engine::Mongo);
        let identity_ok = self.transfer_ui.identity == self.transfer_identity();
        let busy = self.service.remote_db.is_busy() || self.service.db.is_busy();
        let mut close = false;
        let mut browse = false;
        let mut read = false;
        let mut preview = false;
        let mut apply = false;
        let frame = egui::Frame::new()
            .fill(color(self.tokens.panel))
            .stroke(egui::Stroke::new(1., color(self.tokens.popup_border)))
            .corner_radius(self.tokens.control_corner_radius)
            .inner_margin(12);
        egui::Modal::new(egui::Id::unique("transfer.import")).frame(frame).show(ctx,|ui|{
   ui.set_width(776_f32.min(ctx.content_rect().width()-56.));self.tokens.form_style(ui);
   ui.horizontal(|ui|{ui.label(RichText::new(if self.transfer_ui.from_grid{"Copy to Database"}else{"Import Data from File"}).strong());ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{close=components::database_icon_button(ui,Icon::Close,"Fechar importação",true).clicked();});});ui.separator();
   let state=&mut self.transfer_ui;
   egui::ScrollArea::vertical().id_salt("import.form").max_height((ctx.content_rect().height()-220.).clamp(250.,570.)).codisseu().show(ui,|ui|{
    let mut changed=false;let mut source_changed=false;
    if state.from_grid{ui.label("From: Result 1 · snapshot da última leitura");}else{
    ui.horizontal(|ui|{let label=ui.add_sized([100.,28.],egui::Label::new("Path:"));source_changed|=ui.add(egui::TextEdit::singleline(&mut state.path).id_salt("import.path").desired_width((ui.available_width()-106.).max(120.))).labelled_by(label.id).changed();browse=components::database_icon_button(ui,Icon::Folder,"Escolher arquivo para importar",!state.files.is_busy()).clicked();read=ui.add_enabled(!state.path.is_empty()&&!state.files.is_busy(),egui::Button::new("Load")).clicked();});
    if native{ui.label(RichText::new("Formato nativo Codisseu JSON · MongoDB também aceita Extended JSON / NDJSON com _id").color(color(self.tokens.muted)));}
    else{ui.horizontal(|ui|{ui.add_sized([100.,28.],egui::Label::new("Format:"));egui::ComboBox::from_id_salt("import.format").selected_text(label(&state.format)).show_ui(ui,|ui|{for(id,name)in FORMATS.iter().filter(|(id,_)|matches!(*id,"csv"|"tsv"|"pipe"|"semicolon"|"csv-scripted"|"xlsx"|"json"|"ndjson"|"codisseu-json"|"codisseu-ndjson")){source_changed|=ui.selectable_value(&mut state.format,id.to_string(),*name).changed();}});source_changed|=ui.checkbox(&mut state.options.header,"First row is header").changed();});source_changed|=field(ui,"NULL marker:",&mut state.options.null_marker,"import.null");}
    }
    ui.add_space(12.);ui.label(RichText::new("Destination").strong());
    if !native{changed|=field(ui,"Schema:",&mut state.namespace,"import.schema");changed|=field(ui,"Table:",&mut state.table,"import.table");}
    else{
     if mongo{changed|=field(ui,"Collection:",&mut state.table,"import.collection");ui.label("Documentos com _id explícito · somente inclusão");}
     else{ui.horizontal(|ui|{ui.add_sized([100.,28.],egui::Label::new("Unknown TTL:"));egui::ComboBox::from_id_salt("import.ttl").selected_text(match state.ttl{1=>"Never expire",2=>"New TTL",_=>"Reject"}).show_ui(ui,|ui|{changed|=ui.selectable_value(&mut state.ttl,0,"Reject · preservar explicitamente").changed();changed|=ui.selectable_value(&mut state.ttl,1,"Never expire · decisão explícita").changed();changed|=ui.selectable_value(&mut state.ttl,2,"New TTL").changed();});if state.ttl==2{changed|=ui.add(egui::DragValue::new(&mut state.ttl_seconds).range(1..=2592000).suffix(" s")).changed();}});}
    }
    ui.add_space(12.);
    if !native&&let Some(data)=&state.snapshot{
     ui.label(RichText::new(format!("Mapping · {} rows",data.rows.len())).strong());
     egui::Grid::new("import.mapping").num_columns(3).striped(false).show(ui,|ui|{ui.label("Import");ui.label("From column");ui.label("To column");ui.end_row();for(index,(enabled,name))in state.mapping.iter_mut().enumerate(){changed|=ui.checkbox(enabled,"").changed();ui.label(&data.columns[index]);changed|=ui.add(egui::TextEdit::singleline(name).id_salt(("import.map",index)).desired_width(240.)).changed();ui.end_row();}});
     if data.source_truncated{changed|=ui.checkbox(&mut state.allow_partial,"Importar somente este resultado parcial carregado").changed();}
     ui.add_space(10.);ui.label(RichText::new("Data Preview · primeiras 8 linhas").strong());
     egui::ScrollArea::horizontal().id_salt("import.datapreview").max_height(170.).codisseu().show(ui,|ui|{egui::Grid::new("import.data").striped(false).show(ui,|ui|{for c in &data.columns{ui.label(RichText::new(c).strong());}ui.end_row();for row in data.rows.iter().take(8){for c in row{ui.add(egui::Label::new(RichText::new(c.display().chars().take(80).collect::<String>()).monospace()).truncate());}ui.end_row();}});});
    }
    if native{if let Some(raw)=&state.native_raw{ui.label(format!("Arquivo carregado · {} bytes · prévia valida tipos, TTL e destino",raw.len()));}else{ui.label("Carregue o arquivo antes de gerar a prévia.");}}
    if changed||source_changed{state.invalidate();}
if source_changed&&!state.path.is_empty(){state.snapshot=None;state.native_raw=None;}
   });
   ui.separator();ui.label(RichText::new(if !self.database_ui.remote_results{"SQLite · INSERT atômico no arquivo existente; confirmação ao aplicar"}else if native{"Create-only · cada registro pode persistir separadamente; falhas são detalhadas"}else{"INSERT na sessão atual · transação manual · Commit separado"}).color(color(self.tokens.muted)));
   if let Some(e)=&state.error{crate::status_messages::status_message(ui, e);}
if !state.notice.is_empty(){ui.add(egui::Label::new(&state.notice).wrap());}
   let report=if self.database_ui.remote_results{self.service.remote_db.import_report.as_ref()}else{self.service.db.import_report.as_ref()};if let Some(r)=report{ui.label(format!("{} · aplicados {} / {} · falhas {} · incertos {}{}",r.summary,r.applied,r.attempted,r.failed,r.unknown,if r.transaction_pending{" · Commit pendente"}else{""}));}
   ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{apply=ui.add_enabled(identity_ok&&!busy&&state.approval.is_some(),self.tokens.dialog_button("Apply Import",true)).clicked();preview=ui.add_enabled(identity_ok&&!busy&&!state.files.is_busy()&&if native{state.native_raw.is_some()}else{state.snapshot.is_some()},self.tokens.dialog_button("Preview",false)).clicked();close|=ui.add(self.tokens.dialog_button("Cancel",false)).clicked();});
  });
        if browse {
            self.open_transfer_browser();
        }
        if read {
            let state = &mut self.transfer_ui;
            state.invalidate();
            if native {
                state.dispatch(
                    FileCommand::ReadRaw {
                        path: state.path.clone().into(),
                    },
                    Job::Read,
                );
            } else if state.format == "xlsx" {
                state.dispatch(
                    FileCommand::ReadXlsx {
                        path: state.path.clone().into(),
                        header: state.options.header,
                    },
                    Job::Read,
                );
            } else {
                state.dispatch(
                    FileCommand::Read {
                        path: state.path.clone().into(),
                        options: state.options(),
                    },
                    Job::Read,
                );
            }
        }
        if preview {
            self.preview_transfer_import();
        }
        if apply {
            self.apply_transfer_import();
        }
        if close {
            self.close_transfer();
        }
    }
    fn preview_transfer_import(&mut self) {
        self.transfer_ui.invalidate();
        let state = &self.transfer_ui;
        if self.database_ui.remote_results {
            let Some(c) = self.service.remote_db.active.as_ref() else {
                return;
            };
            let id = c.id.clone();
            let generation = self.service.remote_db.session_generation;
            let command = if c.engine.is_sql() {
                let Some(data) = state.snapshot.clone() else {
                    return;
                };
                RemoteCommand::PreviewImport(TransferRequest {
                    datasource_id: id,
                    session_generation: generation,
                    payload: ImportRequest {
                        target: ImportTarget::SqlTable(TableRef {
                            namespace: state.namespace.clone(),
                            name: state.table.clone(),
                        }),
                        data,
                        mapping: state
                            .mapping
                            .iter()
                            .enumerate()
                            .filter(|(_, v)| v.0)
                            .map(|(i, v)| ImportMapping {
                                source_column: i,
                                target_column: v.1.clone(),
                            })
                            .collect(),
                        allow_partial: state.allow_partial,
                    },
                })
            } else {
                RemoteCommand::PreviewNoSqlImport(TransferRequest {
                    datasource_id: id,
                    session_generation: generation,
                    payload: NoSqlImportRequest {
                        json: state.native_raw.clone().unwrap_or_default(),
                        target: match c.engine {
                            Engine::Mongo => {
                                NoSqlImportTarget::MongoCollection(state.table.clone())
                            }
                            Engine::Redis => NoSqlImportTarget::RedisKeys,
                            _ => NoSqlImportTarget::MemcachedKeys,
                        },
                        ttl_policy: match state.ttl {
                            1 => UnknownTtlPolicy::NeverExpire,
                            2 => UnknownTtlPolicy::AfterSeconds(state.ttl_seconds as u64),
                            _ => UnknownTtlPolicy::Reject,
                        },
                    },
                })
            };
            self.service.remote_db.import_preview = None;
            match self.service.dispatch(Command::RemoteDatabase(command)) {
                Ok(()) => self.transfer_ui.preview_pending = true,
                Err(e) => self.transfer_ui.error = Some(e),
            }
        } else {
            let Some(path) = self.service.db.config.clone() else {
                self.transfer_ui.error = Some("Conecte um arquivo SQLite existente".into());
                return;
            };
            let Some(data) = state.snapshot.clone() else {
                return;
            };
            let request = ImportRequest {
                target: ImportTarget::SqlTable(TableRef {
                    namespace: state.namespace.clone(),
                    name: state.table.clone(),
                }),
                data,
                mapping: state
                    .mapping
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.0)
                    .map(|(i, v)| ImportMapping {
                        source_column: i,
                        target_column: v.1.clone(),
                    })
                    .collect(),
                allow_partial: state.allow_partial,
            };
            self.service.db.import_preview = None;
            match self
                .service
                .dispatch(Command::Database(DbCommand::PreviewFileImport {
                    path,
                    request,
                })) {
                Ok(()) => self.transfer_ui.preview_pending = true,
                Err(e) => self.transfer_ui.error = Some(e),
            }
        }
    }
    fn apply_transfer_import(&mut self) {
        let Some(preview_id) = self.transfer_ui.approval.take() else {
            return;
        };
        let command = if self.database_ui.remote_results {
            let Some(c) = &self.service.remote_db.active else {
                return;
            };
            let request = TransferRequest {
                datasource_id: c.id.clone(),
                session_generation: self.service.remote_db.session_generation,
                payload: ImportApproval { preview_id },
            };
            Command::RemoteDatabase(if c.engine.is_sql() {
                RemoteCommand::ApplyImport(request)
            } else {
                RemoteCommand::ApplyNoSqlImport(request)
            })
        } else {
            let Some(path) = self.service.db.config.clone() else {
                return;
            };
            Command::Database(DbCommand::ImportFileBatch { path, preview_id })
        };
        self.transfer_ui.notice = "Aplicando lote aprovado…".into();
        match self.service.dispatch(command) {
            Ok(()) => self.transfer_ui.apply_pending = true,
            Err(e) => self.transfer_ui.error = Some(e),
        }
    }
    fn native_export_window(&mut self, ctx: &egui::Context) {
        let identity_ok = self.transfer_ui.identity == self.transfer_identity();
        let mut close = false;
        let mut export = false;
        let mut save = false;
        let mut browse = false;
        egui::Window::new("Export Native Data")
            .id(egui::Id::unique("transfer.native"))
            .collapsible(false)
            .default_width(700.)
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                let state = &mut self.transfer_ui;
                let before = state.native_source_key();
                ui.label("JSON tipado · valores binários e TTL preservados · limite 500 registros");
                if self
                    .service
                    .remote_db
                    .active
                    .as_ref()
                    .is_some_and(|c| c.engine == Engine::Mongo)
                {
                    field(ui, "Collection:", &mut state.table, "native.collection");
                    field(ui, "Filter JSON:", &mut state.filter, "native.filter");
                } else {
                    ui.label(
                        "Chaves explícitas, uma por linha (UTF-8). Nenhuma enumeração automática.",
                    );
                    ui.add(
                        egui::TextEdit::multiline(&mut state.keys)
                            .desired_rows(5)
                            .desired_width(650.),
                    );
                }
                if before != state.native_source_key() {
                    state.native_raw = None;
                }
                export = ui
                    .add_enabled(
                        identity_ok
                            && self.service.remote_db.active.is_some()
                            && !self.service.remote_db.is_busy(),
                        egui::Button::new("Read explicit records"),
                    )
                    .clicked();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.path)
                            .desired_width((ui.available_width() - 30.).max(100.)),
                    );
                    browse = components::database_icon_button(
                        ui,
                        Icon::Folder,
                        "Escolher arquivo JSON nativo",
                        !state.files.is_busy(),
                    )
                    .clicked();
                });
                ui.checkbox(&mut state.overwrite, "Substituir arquivo existente");
                if let Some(raw) = &state.native_raw {
                    ui.label(format!("{} bytes · snapshot nativo carregado", raw.len()));
                    egui::ScrollArea::both()
                        .max_height(250.)
                        .codisseu()
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(raw.chars().take(12000).collect::<String>())
                                        .monospace(),
                                )
                                .selectable(true),
                            );
                        });
                    if ui.button("Copy to Clipboard").clicked() {
                        ctx.copy_text(raw.clone());
                    }
                    save = ui
                        .add_enabled(
                            !state.files.is_busy(),
                            self.tokens.dialog_button("Export to File", true),
                        )
                        .clicked();
                }
                if let Some(e) = &state.error {
                    crate::status_messages::status_message(ui, e);
                }
                crate::status_messages::status_message(ui, &state.notice);
                close = ui.button("Close").clicked();
            });
        if export {
            let state = &self.transfer_ui;
            if let Some(c) = &self.service.remote_db.active {
                let payload = if c.engine == Engine::Mongo {
                    match serde_json::from_str(&state.filter) {
                        Ok(filter) => Some(NoSqlExportRequest::Mongo {
                            collection: state.table.clone(),
                            filter,
                            limit: 500,
                        }),
                        Err(e) => {
                            self.transfer_ui.error = Some(e.to_string());
                            None
                        }
                    }
                } else {
                    Some(NoSqlExportRequest::Keys {
                        keys: state
                            .keys
                            .lines()
                            .filter(|v| !v.is_empty())
                            .map(|v| TransferBytes::Utf8(v.into()))
                            .collect(),
                    })
                };
                if let Some(payload) = payload {
                    let command = RemoteCommand::ExportNoSql(TransferRequest {
                        datasource_id: c.id.clone(),
                        session_generation: self.service.remote_db.session_generation,
                        payload,
                    });
                    self.service.remote_db.native_export = None;
                    match self.service.dispatch(Command::RemoteDatabase(command)) {
                        Ok(()) => {
                            self.transfer_ui.native_pending = true;
                            self.transfer_ui.native_key =
                                Some(self.transfer_ui.native_source_key());
                        }
                        Err(e) => self.transfer_ui.error = Some(e),
                    }
                }
            }
        }
        if save {
            let state = &mut self.transfer_ui;
            if let Some(text) = state.native_raw.clone() {
                state.dispatch(
                    FileCommand::WriteRaw {
                        path: state.path.clone().into(),
                        text,
                        overwrite: state.overwrite,
                    },
                    Job::Write,
                );
            }
        }
        if browse {
            self.open_transfer_browser();
        }
        if close {
            self.close_transfer();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::remote::Cell;
    use std::{
        thread,
        time::{Duration, Instant},
    };
    fn data() -> TransferData {
        TransferData {
            columns: vec!["id".into(), "text".into()],
            rows: vec![vec![
                Cell::Number("1".into()),
                Cell::Text("quote,\"x\"\nline".into()),
            ]],
            source_truncated: false,
        }
    }
    fn app(path: &std::path::Path) -> CodisseuApp {
        CodisseuApp::new(AppService::new(path.join("session.json")))
    }
    fn finish(app: &mut CodisseuApp, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.transfer_ui.files.is_busy() || app.transfer_ui.dirty {
            assert!(Instant::now() < deadline);
            app.poll_transfer(ctx);
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn project_transfer_waits_for_accepted_write_then_drops_old_project_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("shipping");
        let b = dir.path().join("game");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let mut app = app(dir.path());
        let ctx = egui::Context::default();
        app.service
            .dispatch(Command::OpenWorkspace(a.clone()))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            app.service.poll();
            app.sync_database_ui();
            if !app.service.is_busy() {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        let snapshot = data();
        app.transfer_ui.cached = Some(std::sync::Arc::new(snapshot.clone()));
        app.transfer_ui.snapshot = Some(snapshot.clone());
        app.transfer_ui.rendered = Some(b"Shipping export".to_vec());
        app.transfer_ui.path = "Shipping private path".into();
        app.transfer_ui.dialog = Some(Dialog::Export);
        let output = dir.path().join("accepted.csv");
        app.transfer_ui.dispatch(
            FileCommand::Write {
                path: output.clone(),
                options: TransferOptions::default(),
                data: snapshot,
                overwrite: false,
            },
            Job::Write,
        );
        assert!(app.transfer_work_pending());
        app.request_workspace_switch(b.clone());
        app.poll_workspace_switch(&ctx);
        assert_eq!(app.service.workspace.as_ref(), Some(&a));
        assert!(app.workspace_switch_pending());
        assert!(app.transfer_ui.snapshot.is_some());
        finish(&mut app, &ctx);
        assert!(!app.transfer_work_pending());
        let bytes = std::fs::read(&output).unwrap();
        assert!(!bytes.is_empty());
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.workspace_switch_pending() || app.service.is_busy() {
            app.poll_workspace_switch(&ctx);
            app.service.poll();
            app.sync_database_ui();
            app.poll_transfer(&ctx);
            assert!(Instant::now() < deadline, "{:?}", app.service.last_error);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.service.workspace.as_ref(), Some(&b));
        assert!(app.transfer_ui.cached.is_none());
        assert!(app.transfer_ui.snapshot.is_none());
        assert!(app.transfer_ui.rendered.is_none());
        assert!(app.transfer_ui.path.is_empty());
        assert!(app.transfer_ui.dialog.is_none());
        assert!(app.transfer_ui.files.report.is_none());
        assert_eq!(std::fs::read(output).unwrap(), bytes);
    }
    #[test]
    fn copy_uses_visible_proposal_while_export_defaults_to_confirmed_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let ctx = egui::Context::default();
        let original = data();
        app.transfer_ui.cached = Some(std::sync::Arc::new(original.clone()));
        app.database_ui.remote_results = true;
        app.database_ui
            .staged
            .insert((0, 1), Cell::Text("proposed but not submitted".into()));
        app.grid_selection.clicked(0, 1, Modifiers::NONE);
        let input = egui::RawInput {
            events: vec![egui::Event::Copy],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let response = ui.interact(
                egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(100., 30.)),
                egui::Id::unique("test.grid.value"),
                egui::Sense::click(),
            );
            response.request_focus();
            app.grid_focus = Some(response.id);
            app.copy_result_selection(ui.ctx());
        });
        assert!(output.platform_output.commands.iter().any(
            |c| matches!(c,egui::OutputCommand::CopyText(t) if t=="proposed but not submitted")
        ));
        output.textures_delta.clear();
        app.begin_transfer(Dialog::Export);
        assert_eq!(app.transfer_ui.export_data().unwrap(), original);
        app.transfer_ui.include_proposed = true;
        assert_eq!(
            app.transfer_ui.export_data().unwrap().rows[0][1],
            Cell::Text("proposed but not submitted".into())
        );
        assert_eq!(app.transfer_ui.snapshot.as_ref().unwrap(), &original);
    }
    #[test]
    fn transfer_dialog_blocks_global_query_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("query.sql");
        std::fs::write(&file, "SELECT 1;").unwrap();
        let db = dir.path().join("fixture.sqlite");
        rusqlite::Connection::open(&db).unwrap();
        let mut app = app(dir.path());
        for command in [
            Command::OpenFile(file),
            Command::Database(DbCommand::Connect(db)),
        ] {
            app.service.dispatch(command).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.service.is_busy() {
                assert!(Instant::now() < deadline);
                app.service.poll();
                thread::sleep(Duration::from_millis(1));
            }
        }
        app.begin_transfer(Dialog::Export);
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![
                egui::Event::ModifiersChanged(Modifiers::COMMAND),
                egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::COMMAND,
                },
            ],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        assert!(!app.service.db.is_busy());
        assert!(app.service.db.result.is_none());
        assert!(app.service.last_error.is_none());
    }
    #[test]
    fn changing_file_source_before_read_ack_never_imports_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.csv");
        std::fs::write(&first, "id,text\n1,first\n").unwrap();
        let mut app = app(dir.path());
        let ctx = egui::Context::default();
        app.begin_transfer(Dialog::Import);
        app.transfer_ui.path = first.to_string_lossy().into();
        app.transfer_ui.dispatch(
            FileCommand::Read {
                path: first,
                options: TransferOptions::default(),
            },
            Job::Read,
        );
        app.transfer_ui.path = dir.path().join("other.csv").to_string_lossy().into();
        finish(&mut app, &ctx);
        assert!(app.transfer_ui.snapshot.is_none());
        assert!(
            app.transfer_ui
                .error
                .as_deref()
                .unwrap()
                .contains("alterada")
        );
        assert!(app.transfer_ui.approval.is_none());
    }
    #[test]
    fn edited_export_options_discard_obsolete_render_and_keep_query_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let ctx = egui::Context::default();
        app.database_ui.sqlite_result_sql = Some("SELECT original;".into());
        app.begin_transfer(Dialog::Export);
        app.transfer_ui.set_data(data());
        let options = app.transfer_ui.options();
        app.transfer_ui.dirty = false;
        app.transfer_ui.dispatch(
            FileCommand::Render {
                options,
                data: data(),
            },
            Job::Render,
        );
        app.transfer_ui.set_format("json");
        app.database_ui.sqlite_result_sql = Some("SELECT edited_later;".into());
        finish(&mut app, &ctx);
        let bytes = app.transfer_ui.rendered.as_ref().unwrap();
        let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        assert_eq!(json[0]["id"], 1);
        assert_eq!(json[0]["text"], "quote,\"x\"\nline");
        assert_eq!(
            app.transfer_ui.options.query.as_deref(),
            Some("SELECT original;")
        );
    }
    #[test]
    fn frame_poll_reuses_typed_snapshot_and_xlsx_does_not_replace_clipboard_extractor() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let ctx = egui::Context::default();
        let file = dir.path().join("fixture.sqlite");
        rusqlite::Connection::open(&file).unwrap();
        for command in [
            DbCommand::Connect(file),
            DbCommand::Execute {
                sql: "SELECT 1 AS id".into(),
            },
        ] {
            app.service.dispatch(Command::Database(command)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.service.is_busy() {
                assert!(Instant::now() < deadline);
                app.service.poll();
                thread::sleep(Duration::from_millis(1));
            }
        }
        app.poll_transfer(&ctx);
        let before = app.result_snapshot().unwrap();
        app.poll_transfer(&ctx);
        assert!(std::sync::Arc::ptr_eq(
            &before,
            &app.result_snapshot().unwrap()
        ));
        app.transfer_ui.set_format("json");
        app.transfer_ui.set_format("xlsx");
        assert_eq!(app.transfer_ui.copy_format, "json");
        assert_eq!(app.transfer_ui.format, "xlsx");
    }
}
