//! Remote datasource model and single-session worker. Secrets never serialize.
pub use crate::mongo_options::{MongoAuthentication, MongoOptions, MongoReadPreference};
use crate::transfer::{
    ImportApproval, ImportPreview, ImportReport, ImportRequest, NoSqlImportRequest, TransferRequest,
};
use crate::transfer_nosql::NoSqlExportRequest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Engine {
    Mysql,
    AuroraMysql,
    Postgres,
    AuroraPostgres,
    Mongo,
    Redis,
    Memcached,
}
impl Engine {
    pub const ALL: [Self; 7] = [
        Self::Mysql,
        Self::AuroraMysql,
        Self::Postgres,
        Self::AuroraPostgres,
        Self::Mongo,
        Self::Redis,
        Self::Memcached,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Mysql => "MySQL",
            Self::AuroraMysql => "Aurora MySQL",
            Self::Postgres => "PostgreSQL",
            Self::AuroraPostgres => "Aurora PostgreSQL",
            Self::Mongo => "MongoDB",
            Self::Redis => "Redis",
            Self::Memcached => "Memcached",
        }
    }
    pub fn port(self) -> u16 {
        match self {
            Self::Mysql | Self::AuroraMysql => 3306,
            Self::Postgres | Self::AuroraPostgres => 5432,
            Self::Mongo => 27017,
            Self::Redis => 6379,
            Self::Memcached => 11211,
        }
    }
    pub fn is_sql(self) -> bool {
        matches!(
            self,
            Self::Mysql | Self::AuroraMysql | Self::Postgres | Self::AuroraPostgres
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub database: String,
    #[serde(default)]
    pub auth_database: String,
    #[serde(default)]
    pub mongo: MongoOptions,
    #[serde(default)]
    pub ca_file: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub ca_pem: Arc<str>,
    pub username: String,
    pub tls: bool,
    pub read_only: bool,
    pub timeout_secs: u64,
    pub idle_secs: u64,
    pub row_limit: usize,
    #[serde(default = "default_save_credentials")]
    pub save_credentials: bool,
}
fn default_save_credentials() -> bool {
    true
}
impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "MySQL @ localhost".into(),
            engine: Engine::Mysql,
            host: "localhost".into(),
            port: 3306,
            database: String::new(),
            auth_database: String::new(),
            mongo: MongoOptions::default(),
            ca_file: String::new(),
            ca_pem: Arc::from(""),
            username: String::new(),
            tls: true,
            read_only: true,
            timeout_secs: 10,
            idle_secs: 600,
            row_limit: 500,
            save_credentials: true,
        }
    }
}
impl ConnectionConfig {
    pub fn automatic_rds_ca(&self) -> bool {
        crate::tls_roots::automatic_aws_roots(self).is_some()
    }
    pub fn validate(&self) -> Result<(), String> {
        for s in [
            &self.id,
            &self.name,
            &self.host,
            &self.database,
            &self.username,
            &self.auth_database,
        ] {
            if s.len() > 1024 || s.contains(['\0', '\n', '\r']) {
                return Err("Campo de conexão inválido ou muito longo".into());
            }
        }
        if self.engine == Engine::Mongo {
            self.mongo.validate()?;
            if self.mongo.authentication != MongoAuthentication::Automatic
                && self.username.is_empty()
            {
                return Err(
                    "Informe o usuário para o mecanismo de autenticação Mongo selecionado".into(),
                );
            }
        }
        if !self.tls && (!self.ca_file.is_empty() || !self.ca_pem.is_empty()) {
            return Err("Certificado CA exige TLS ativado".into());
        }
        if self.ca_pem.len() > crate::tls_roots::MAX_CA_BYTES {
            return Err("Certificado CA excede256KiB".into());
        }
        if self.host.contains(['@', '/', ' ']) || self.ca_file.len() > 4096 {
            return Err("Use somente o hostname ou IP no campo Host".into());
        }
        if self.id.is_empty()
            || self.name.trim().is_empty()
            || self.host.trim().is_empty()
            || self.port == 0
        {
            return Err("Informe nome, host e porta válidos".into());
        }
        if !(1..=120).contains(&self.timeout_secs)
            || !(5..=3600).contains(&self.idle_secs)
            || !(1..=500).contains(&self.row_limit)
        {
            return Err("Timeout 1–120s, inatividade 5–3600s e limite 1–500 linhas".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Cell {
    Null,
    Text(String),
    Number(String),
    Bool(bool),
    Json(String),
    Binary(Vec<u8>),
}
impl Cell {
    pub fn clipboard_text(&self) -> String {
        match self {
            Self::Text(s) | Self::Number(s) | Self::Json(s) => s.clone(),
            Self::Binary(bytes) => {
                let mut text = String::from("0x");
                for byte in bytes {
                    use std::fmt::Write;
                    let _ = write!(text, "{byte:02X}");
                }
                text
            }
            _ => self.display(),
        }
    }
    pub fn display(&self) -> String {
        match self {
            Self::Null => "NULL".into(),
            Self::Text(s) if s.is_empty() => "\"\"".into(),
            Self::Text(s) | Self::Number(s) | Self::Json(s) => s.chars().take(512).collect(),
            Self::Bool(v) => v.to_string(),
            Self::Binary(v) => format!("BINARY[{}]", v.len()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteColumn {
    pub name: String,
    pub data_type: String,
    pub primary_key: bool,
    pub nullable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRef {
    pub namespace: String,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct DbObject {
    pub namespace: String,
    pub name: String,
    pub kind: String,
    pub columns: Vec<RemoteColumn>,
}
#[derive(Clone, Debug)]
pub struct RemoteResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    pub truncated: bool,
    pub elapsed_ms: u128,
    pub affected_rows: u64,
}
#[derive(Clone, Debug)]
pub struct CellEdit {
    pub row: Vec<Cell>,
    pub column: String,
    pub value: Cell,
}
#[derive(Clone, Debug)]
pub struct EditBatch {
    pub table: TableRef,
    pub columns: Vec<RemoteColumn>,
    pub edits: Vec<CellEdit>,
}

/// Keyset page over table names; filters are literal prefixes, never SQL patterns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogRequest {
    pub namespace: String,
    pub prefix: String,
    pub after: Option<String>,
    pub before: Option<String>,
}
#[derive(Clone, Debug)]
pub struct CatalogPage {
    pub request: CatalogRequest,
    pub objects: Vec<DbObject>,
    pub previous: Option<String>,
    pub next: Option<String>,
}
#[derive(Clone, Debug)]
pub struct CatalogPosition {
    pub request: CatalogRequest,
    pub previous: Option<String>,
    pub next: Option<String>,
}
pub enum RemoteCommand {
    ExportQuery(TransferRequest<crate::query_export::QueryExportRequest>),
    PreviewImport(TransferRequest<ImportRequest>),
    ApplyImport(TransferRequest<ImportApproval>),
    ExportNoSql(TransferRequest<NoSqlExportRequest>),
    PreviewNoSqlImport(TransferRequest<NoSqlImportRequest>),
    ApplyNoSqlImport(TransferRequest<ImportApproval>),
    Save(ConnectionConfig),
    SaveRemembered {
        config: ConnectionConfig,
        password: String,
    },
    Delete(String),
    DeleteRemembered(String),
    ConnectRemembered {
        config: ConnectionConfig,
        password: String,
        test: bool,
    },
    Connect {
        config: ConnectionConfig,
        password: String,
        test: bool,
    },
    Disconnect,
    Inspect,
    ListNamespaces,
    InspectNamespace(String),
    CatalogPage(CatalogRequest),
    CompleteTables {
        namespace: String,
        prefix: String,
    },
    InspectColumns(TableRef),
    Execute(String),
    ExecutePage(crate::query_page::BoundQueryPage),
    Explain(TransferRequest<String>),
    ExecuteBound {
        sql: String,
        datasource_id: String,
    },
    /// Explicit execution of a saved console may acquire its registered source.
    ExecuteConsole {
        sql: String,
        datasource_id: String,
    },
    LoadTable(TableRef),
    LoadTablePage {
        table: TableRef,
        offset: u64,
    },
    Apply(EditBatch),
    Commit,
    Rollback,
    Cancel,
    CreateConsole,
    CreateConsoleFor(String),
    DeleteConsole(PathBuf),
    RenameConsole {
        path: PathBuf,
        name: String,
    },
}
#[derive(Clone, Debug)]
pub struct ConsoleRename {
    pub source: PathBuf,
    pub path: PathBuf,
    pub warning: Option<String>,
}
impl std::fmt::Debug for RemoteCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteCommand")
            .field("kind", &std::mem::discriminant(self))
            .finish_non_exhaustive()
    }
}
enum Event {
    ProjectLoaded {
        directory: PathBuf,
        profiles: Vec<ConnectionConfig>,
        bindings: BTreeMap<PathBuf, String>,
        error: Option<String>,
        legacy_available: bool,
    },
    LegacyAvailable(bool),
    QueryExport(crate::query_export::QueryExportReport),
    ImportPreview(ImportPreview),
    ImportReport(ImportReport),
    NativeExport(String),
    Profiles(Vec<ConnectionConfig>),
    ProfilesChanged(Vec<ConnectionConfig>),
    Saved(Vec<ConnectionConfig>),
    Bindings(BTreeMap<PathBuf, String>),
    Connected(ConnectionConfig),
    ConnectedForQuery(ConnectionConfig),
    Suspended,
    Resumed,
    Disconnected(String),
    CancelFinished,
    Schema(Vec<DbObject>),
    Namespaces(Vec<String>),
    NamespaceObjects(CatalogPage),
    CompletionTables(CatalogPage),
    TableColumns(TableRef, Vec<RemoteColumn>),
    Result(RemoteResult, Option<(TableRef, Vec<RemoteColumn>)>),
    Applied(RemoteResult),
    Transaction(bool, String),
    Tested,
    Error(String),
    Console(PathBuf),
    ConsoleRenamed(ConsoleRename),
    ConsoleDeleted(PathBuf),
}
enum Request {
    Command(Box<RemoteCommand>),
    Project(Option<PathBuf>),
    ClaimLegacy(PathBuf),
}
/// Validated on the caller's frame; fields cannot be fabricated by a caller.
pub struct PreparedProjectChange {
    project: Option<PathBuf>,
    unchanged: bool,
}
/// Session-restore filter for managed query files. Call on a filesystem worker.
pub fn restorable_project_console(
    anchor: &std::path::Path,
    project: Option<&std::path::Path>,
    path: &std::path::Path,
) -> bool {
    crate::remote_scope::restorable_console(anchor, project, path)
}
pub struct RemoteService {
    /// The accepted canonical project identity, independent of datasource IDs.
    pub project: Option<PathBuf>,
    /// Unassigned legacy data is only advertised by existence, never by names.
    pub legacy_available: bool,
    pub console_deleted: Option<PathBuf>,
    console_delete_pending: bool,
    pub console_renamed: Option<ConsoleRename>,
    console_rename_pending: Option<PathBuf>,
    pub export_report: Option<crate::query_export::QueryExportReport>,
    pub export_progress: crate::query_export::QueryExportProgress,
    pub import_preview: Option<ImportPreview>,
    pub import_report: Option<ImportReport>,
    pub native_export: Option<String>,
    pub profiles: Vec<ConnectionConfig>,
    pub active: Option<ConnectionConfig>,
    pub objects: Vec<DbObject>,
    pub namespaces: Vec<String>,
    pub namespaces_loaded: bool,
    pub namespace_objects: BTreeMap<String, Vec<DbObject>>,
    pub namespace_pages: BTreeMap<String, CatalogPosition>,
    pub completion_tables: BTreeMap<(String, String), Vec<DbObject>>,
    pub completion_more: std::collections::BTreeSet<(String, String)>,
    pub table_columns: BTreeMap<(String, String), Vec<RemoteColumn>>,
    pub result: Option<RemoteResult>,
    pub origin: Option<(TableRef, Vec<RemoteColumn>)>,
    pub transaction: bool,
    pub status: String,
    pub error: Option<String>,
    /// Identity of the current query result. Transfer acknowledgements do not replace it.
    pub revision: u64,
    /// Changes even when reconnecting the same saved datasource.
    pub session_generation: u64,
    /// Logical connection identity. Automatic idle suspension does not change it.
    pub connection_generation: u64,
    /// The logical datasource remains selected, with its idle socket released.
    pub suspended: bool,
    pub console_path: Option<PathBuf>,
    pub console_bindings: BTreeMap<PathBuf, String>,
    console_directory: PathBuf,
    busy: bool,
    loading_profiles: bool,
    project_pending: bool,
    project_scoped: bool,
    queued_project: Option<Request>,
    cancel_pending: bool,
    sender: Option<SyncSender<Request>>,
    receiver: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    cancel_epoch: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
}
impl RemoteService {
    pub fn new(path: PathBuf) -> Self {
        Self::with_scope(path, false)
    }
    /// Window-independent profile anchor; starts with no project's connections.
    pub fn new_project_scoped(anchor: PathBuf) -> Self {
        Self::with_scope(anchor, true)
    }
    fn with_scope(path: PathBuf, project_scoped: bool) -> Self {
        let path = std::path::absolute(&path).unwrap_or(path);
        let initial_path = if project_scoped {
            crate::remote_scope::empty_path(&path)
        } else {
            path.clone()
        };
        let directory = initial_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("consoles");
        let console_directory = std::fs::canonicalize(&directory).unwrap_or(directory);
        let (tx, rx) = mpsc::sync_channel(1);
        let (etx, erx) = mpsc::sync_channel(8);
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let export_progress = crate::query_export::QueryExportProgress::default();
        let worker_export_progress = export_progress.clone();
        let cancel_epoch = Arc::new(AtomicU64::new(0));
        let epoch = cancel_epoch.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let s = stopped.clone();
        std::thread::Builder::new()
            .name("codisseu-datasources".into())
            .spawn(move || {
                worker(
                    path,
                    project_scoped,
                    rx,
                    etx,
                    c,
                    epoch,
                    worker_export_progress,
                );
                s.store(true, Ordering::SeqCst);
            })
            .expect("Worker de banco");
        Self {
            project: None,
            legacy_available: false,
            console_deleted: None,
            console_delete_pending: false,
            console_renamed: None,
            console_rename_pending: None,
            export_report: None,
            export_progress,
            import_preview: None,
            import_report: None,
            native_export: None,
            profiles: vec![],
            active: None,
            objects: vec![],
            namespaces: vec![],
            namespaces_loaded: false,
            namespace_objects: BTreeMap::new(),
            table_columns: BTreeMap::new(),
            namespace_pages: BTreeMap::new(),
            completion_tables: BTreeMap::new(),
            completion_more: Default::default(),
            result: None,
            origin: None,
            transaction: false,
            status: "Desconectado".into(),
            error: None,
            revision: 0,
            session_generation: 0,
            connection_generation: 0,
            suspended: false,
            console_path: None,
            console_bindings: BTreeMap::new(),
            console_directory,
            busy: true,
            loading_profiles: true,
            project_pending: false,
            project_scoped,
            queued_project: None,
            cancel_pending: false,
            sender: Some(tx),
            receiver: erx,
            cancel,
            cancel_epoch,
            stopped,
        }
    }
    pub fn is_busy(&self) -> bool {
        self.busy || self.cancel_pending || self.project_pending
    }
    /// Queue a metadata transition on the existing connection owner. No file IO
    /// or session close runs on the GUI thread. Old metadata disappears now.
    pub fn set_project(&mut self, project: Option<PathBuf>) -> Result<(), String> {
        let prepared = self.prepare_project_change(project)?;
        self.apply_project_change(prepared);
        Ok(())
    }
    pub fn prepare_project_change(
        &self,
        project: Option<PathBuf>,
    ) -> Result<PreparedProjectChange, String> {
        if !self.project_scoped {
            return Err("Este serviço de banco não usa escopo de projeto".into());
        }
        if self.project == project {
            return Ok(PreparedProjectChange {
                project,
                unchanged: true,
            });
        }
        if self.transaction {
            return Err("Faça Commit ou Rollback antes de trocar o projeto".into());
        }
        if self.has_pending_operation() {
            return Err("Aguarde a operação de banco antes de trocar o projeto".into());
        }
        if self.sender.is_none() || self.worker_stopped() {
            return Err("Worker de banco encerrado".into());
        }
        Ok(PreparedProjectChange {
            project,
            unchanged: false,
        })
    }
    /// Commit immediately after other workspace services accepted their switch.
    /// Queue availability is handled by poll, never by a partially applied core ACK.
    pub fn apply_project_change(&mut self, prepared: PreparedProjectChange) {
        if prepared.unchanged {
            return;
        }
        self.queued_project = Some(Request::Project(prepared.project.clone()));
        self.project = prepared.project;
        self.clear_project_metadata();
    }
    pub fn claim_legacy_project(&mut self) -> Result<(), String> {
        let project = self
            .project
            .clone()
            .ok_or("Abra o projeto correto antes de vincular os bancos anteriores")?;
        if !self.project_scoped || !self.legacy_available {
            return Err("Não há cadastro anterior sem projeto para vincular".into());
        }
        if self.is_busy() || self.transaction {
            return Err(
                "Conclua as operações e transações antes de vincular os bancos anteriores".into(),
            );
        }
        if !self.profiles.is_empty() || !self.console_bindings.is_empty() {
            return Err(
                "Este projeto já possui conexões ou queries; seus dados foram preservados".into(),
            );
        }
        self.sender
            .as_ref()
            .ok_or("Worker encerrado")?
            .try_send(Request::ClaimLegacy(project))
            .map_err(|_| "Worker de banco indisponível")?;
        self.clear_project_metadata();
        Ok(())
    }
    fn clear_project_metadata(&mut self) {
        self.project_pending = true;
        self.busy = true;
        self.active = None;
        self.profiles.clear();
        self.console_bindings.clear();
        self.console_path = None;
        self.console_deleted = None;
        self.console_renamed = None;
        self.console_rename_pending = None;
        self.console_delete_pending = false;
        self.clear_catalog();
        self.result = None;
        self.origin = None;
        self.transaction = false;
        self.suspended = false;
        self.export_report = None;
        self.import_preview = None;
        self.import_report = None;
        self.native_export = None;
        self.error = None;
        self.session_generation = self.session_generation.saturating_add(1);
        self.connection_generation = self.connection_generation.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
        self.status = "Carregando bancos do projeto".into();
    }
    /// Startup metadata loading may run alongside document-session restoration.
    /// Accepted datasource commands still prevent a concurrent session restore.
    pub fn has_pending_operation(&self) -> bool {
        self.project_pending || (self.is_busy() && !self.loading_profiles)
    }
    pub fn is_console_path(&self, path: &std::path::Path) -> bool {
        self.console_bindings.contains_key(path) || path.starts_with(&self.console_directory)
    }
    pub fn console_delete_pending(&self) -> bool {
        self.console_delete_pending
    }
    pub fn console_rename_pending(&self) -> bool {
        self.console_rename_pending.is_some()
    }
    pub fn console_rename_target(
        &self,
        path: &std::path::Path,
        name: &str,
    ) -> Result<PathBuf, String> {
        if !self.console_bindings.contains_key(path) {
            return Err("Query não cadastrada".into());
        }
        crate::console_files::rename_target(
            &self.console_directory,
            &self.console_bindings,
            path,
            name,
        )
    }
    pub fn clear_catalog(&mut self) {
        self.objects.clear();
        self.namespaces.clear();
        self.namespaces_loaded = false;
        self.namespace_objects.clear();
        self.namespace_pages.clear();
        self.completion_tables.clear();
        self.completion_more.clear();
        self.table_columns.clear();
    }
    pub fn console_target(&self, path: &std::path::Path) -> Result<Option<&str>, String> {
        if let Some(id) = self.console_bindings.get(path) {
            return Ok(Some(id));
        }
        if self.is_console_path(path) {
            return Err(
                "Console sem vínculo de datasource válido; restaure o vínculo antes de executar"
                    .into(),
            );
        }
        Ok(None)
    }
    pub fn worker_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
    pub fn dispatch(&mut self, cmd: RemoteCommand) -> Result<(), String> {
        if let RemoteCommand::Delete(id) | RemoteCommand::DeleteRemembered(id) = &cmd
            && self.transaction
            && self.active.as_ref().is_some_and(|c| &c.id == id)
        {
            return Err("Faça Commit ou Rollback antes de excluir a conexão".into());
        }
        let deleting = matches!(&cmd, RemoteCommand::DeleteConsole(_));
        let renaming = if let RemoteCommand::DeleteConsole(path) = &cmd {
            if !self.console_bindings.contains_key(path) {
                return Err("Query não cadastrada".into());
            }
            Some(path.clone())
        } else if let RemoteCommand::RenameConsole { path, name } = &cmd {
            self.console_rename_target(path, name)?;
            Some(path.clone())
        } else {
            None
        };
        if self.transaction
            && matches!(
                cmd,
                RemoteCommand::Connect { .. } | RemoteCommand::ConnectRemembered { .. }
            )
        {
            return Err("Faça Commit ou Rollback antes de trocar/testar a conexão".into());
        }
        if matches!(cmd, RemoteCommand::Cancel) {
            self.cancel_epoch.fetch_add(1, Ordering::SeqCst);
            self.import_preview = None;
            self.cancel.store(true, Ordering::SeqCst);
            // A queued reply may already exist even while the GUI reports busy.
            // Wait for a separate cancellation ACK before accepting another job.
            self.cancel_pending = true;
            self.busy = true;
            self.status = "Cancelamento solicitado; aguardando fechamento da conexão".into();
            return Ok(());
        }
        if self.is_busy() {
            return Err("Aguarde ou cancele a operação atual".into());
        }
        if let RemoteCommand::ExecuteConsole { sql, datasource_id } = &cmd {
            let source = self
                .profiles
                .iter()
                .find(|c| &c.id == datasource_id)
                .ok_or("Conexão desta query não encontrada; restaure o cadastro da origem")?;
            source.validate()?;
            if sql.trim().is_empty() || sql.len() > crate::SQL_BYTE_LIMIT {
                return Err("Comando vazio ou excede 256 KiB".into());
            }
            if self.transaction && self.active.as_ref().is_none_or(|c| &c.id != datasource_id) {
                return Err(
                    "Faça Commit ou Rollback antes de executar a query de outra conexão".into(),
                );
            }
        }
        if matches!(
            cmd,
            RemoteCommand::Apply(_)
                | RemoteCommand::ApplyImport(_)
                | RemoteCommand::ApplyNoSqlImport(_)
        ) && self.active.as_ref().is_some_and(|active| {
            self.profiles
                .iter()
                .any(|p| p.id == active.id && p.read_only)
        }) {
            return Err("Datasource configurado como somente leitura; reconecte para aplicar as opções da sessão".into());
        }
        if let RemoteCommand::ExportQuery(request) = &cmd {
            request.payload.validate()?;
            if self.transaction {
                return Err("Faça Commit ou Rollback antes de exportar a consulta completa".into());
            }
            if self.active.as_ref().is_none_or(|c| !c.engine.is_sql()) {
                return Err("Exportação da consulta completa exige conexão SQL".into());
            }
        }
        let binding = command_binding(&cmd);
        if let Some((id, generation)) = binding
            && (self.active.as_ref().is_none_or(|c| c.id != id)
                || generation != self.session_generation)
        {
            return Err("Sessão/datasource mudou; abra novamente a transferência".into());
        }
        if let RemoteCommand::ExecuteBound { datasource_id, .. } = &cmd
            && self.active.as_ref().is_none_or(|c| c.id != *datasource_id)
        {
            return Err(
                "Console vinculado a outro datasource; conecte a origem deste console".into(),
            );
        }
        if let RemoteCommand::ExecuteBound { sql, .. } = &cmd
            && (sql.len() > crate::SQL_BYTE_LIMIT || sql.trim().is_empty())
        {
            return Err("Comando vazio ou excede 256 KiB".into());
        }
        if let RemoteCommand::Execute(s) = &cmd
            && (s.len() > crate::SQL_BYTE_LIMIT || s.trim().is_empty())
        {
            return Err("Comando vazio ou excede 256 KiB".into());
        }
        if let RemoteCommand::Explain(request) = &cmd
            && (request.payload.len() > crate::SQL_BYTE_LIMIT || request.payload.trim().is_empty())
        {
            return Err("Plano exige uma consulta de até 256 KiB".into());
        }
        if let RemoteCommand::ExecutePage(request) = &cmd {
            if self
                .active
                .as_ref()
                .is_none_or(|c| c.id != request.datasource_id)
                || self.connection_generation != request.connection_generation
            {
                return Err("Sessão/datasource mudou; execute a consulta novamente".into());
            }
            request.payload.validate()?;
            if self.active.as_ref().is_none_or(|c| !c.engine.is_sql()) {
                return Err("Paginação exige conexão SQL".into());
            }
        }
        if matches!(
            cmd,
            RemoteCommand::Execute(_)
                | RemoteCommand::Explain(_)
                | RemoteCommand::LoadTable(_)
                | RemoteCommand::LoadTablePage { .. }
                | RemoteCommand::ExecutePage(_)
                | RemoteCommand::ExecuteConsole { .. }
                | RemoteCommand::ExecuteBound { .. }
        ) {
            self.result = None;
            self.origin = None;
        }
        if let RemoteCommand::Connect { config, .. }
        | RemoteCommand::ConnectRemembered { config, .. }
        | RemoteCommand::SaveRemembered { config, .. }
        | RemoteCommand::Save(config) = &cmd
        {
            config.validate()?;
        }
        if let RemoteCommand::InspectNamespace(namespace) = &cmd {
            validate_catalog_name(namespace)?;
        }
        match &cmd {
            RemoteCommand::CatalogPage(request) => validate_catalog_request(request)?,
            RemoteCommand::CompleteTables { namespace, prefix } => {
                validate_catalog_request(&CatalogRequest {
                    namespace: namespace.clone(),
                    prefix: prefix.clone(),
                    ..Default::default()
                })?
            }
            _ => {}
        }
        if let RemoteCommand::InspectColumns(table) = &cmd {
            validate_catalog_name(&table.namespace)?;
            validate_catalog_name(&table.name)?;
        }
        if matches!(
            cmd,
            RemoteCommand::Connect { .. } | RemoteCommand::ConnectRemembered { .. }
        ) {
            self.active = None;
            self.suspended = false;
            self.transaction = false;
            self.result = None;
            self.origin = None;
            self.clear_catalog();
        }
        if matches!(
            cmd,
            RemoteCommand::PreviewImport(_) | RemoteCommand::PreviewNoSqlImport(_)
        ) {
            self.import_preview = None;
            self.import_report = None;
        }
        if matches!(cmd, RemoteCommand::ExportNoSql(_)) {
            self.native_export = None;
        }
        if matches!(cmd, RemoteCommand::ExportQuery(_)) {
            self.export_report = None;
            self.export_progress.reset();
        }
        self.cancel.store(false, Ordering::SeqCst);
        self.sender
            .as_ref()
            .ok_or("Worker encerrado")?
            .try_send(Request::Command(Box::new(cmd)))
            .map_err(|_| "Worker indisponível")?;
        self.busy = true;
        if renaming.is_some() {
            self.console_delete_pending = deleting;
            self.console_deleted = None;
            self.console_rename_pending = renaming;
            self.console_renamed = None;
        }
        self.error = None;
        self.status = "Operação em andamento…".into();
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(request) = self.queued_project.take() {
            match self.sender.as_ref().map(|sender| sender.try_send(request)) {
                Some(Ok(())) => (),
                Some(Err(mpsc::TrySendError::Full(request))) => self.queued_project = Some(request),
                Some(Err(mpsc::TrySendError::Disconnected(_))) | None => {
                    self.project_pending = false;
                    self.busy = false;
                    self.loading_profiles = false;
                    self.error =
                        Some("Worker de banco encerrado; projeto sem conexões carregadas".into());
                    self.status = self.error.clone().unwrap_or_default();
                    changed = true;
                }
            }
        }
        loop {
            let event = match self.receiver.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if self.sender.take().is_some() {
                        self.busy = false;
                        self.cancel_pending = false;
                        self.loading_profiles = false;
                        self.project_pending = false;
                        self.console_rename_pending = None;
                        self.console_delete_pending = false;
                        self.active = None;
                        self.suspended = false;
                        self.clear_catalog();
                        self.transaction = false;
                        self.origin = None;
                        self.error = Some("Worker de banco encerrou; reconecte a aplicação".into());
                        self.status = self.error.clone().unwrap_or_default();
                        changed = true;
                    }
                    break;
                }
            };
            changed = true;
            let connected_for_query = matches!(&event, Event::ConnectedForQuery(_));
            match event {
                Event::ProjectLoaded {
                    directory,
                    profiles,
                    bindings,
                    error,
                    legacy_available,
                } => {
                    self.console_directory = directory;
                    self.profiles = profiles;
                    self.console_bindings = bindings;
                    self.busy = false;
                    self.loading_profiles = false;
                    self.project_pending = false;
                    self.status = error
                        .clone()
                        .unwrap_or_else(|| "Bancos do projeto carregados".into());
                    self.error = error;
                    self.legacy_available = legacy_available;
                }
                Event::LegacyAvailable(available) => self.legacy_available = available,
                Event::QueryExport(report) => {
                    self.status = format!(
                        "CSV salvo: {} linhas · {} bytes · {} ms",
                        report.rows, report.bytes, report.elapsed_ms
                    );
                    self.export_report = Some(report);
                    self.busy = false;
                }
                Event::ImportPreview(preview) => {
                    self.import_preview = Some(preview);
                    self.import_report = None;
                    self.busy = false;
                    self.status = "Prévia preparada; nenhuma linha inserida".into();
                }
                Event::ImportReport(report) => {
                    self.status = report.summary.clone();
                    self.transaction = report.transaction_pending;
                    self.import_report = Some(report);
                    self.import_preview = None;
                    self.busy = false;
                }
                Event::NativeExport(text) => {
                    self.native_export = Some(text);
                    self.busy = false;
                    self.status = "Snapshot nativo preparado".into();
                }
                Event::ProfilesChanged(p) => {
                    self.profiles = p;
                }
                Event::Saved(p) => {
                    self.profiles = p;
                    self.busy = false;
                    self.status =
                        "Configuração salva; reconecte para aplicar alterações à sessão ativa"
                            .into();
                }
                Event::Profiles(p) => {
                    if self.project_pending {
                        continue;
                    }
                    self.profiles = p;
                    if !self.loading_profiles {
                        self.busy = false;
                    }
                }
                Event::Bindings(b) => {
                    if self.project_pending && self.loading_profiles {
                        self.loading_profiles = false;
                        continue;
                    }
                    self.console_bindings = b;
                    if self.loading_profiles {
                        self.loading_profiles = false;
                        self.busy = false;
                    }
                }
                Event::Connected(c) | Event::ConnectedForQuery(c) => {
                    self.session_generation = self.session_generation.saturating_add(1);
                    self.connection_generation = self.connection_generation.saturating_add(1);
                    self.suspended = false;
                    self.import_preview = None;
                    self.import_report = None;
                    self.native_export = None;
                    self.active = Some(c);
                    self.clear_catalog();
                    self.result = None;
                    self.origin = None;
                    self.transaction = false;
                    self.busy = connected_for_query;
                    self.status = "Conectado · sessão única".into();
                }
                Event::Suspended => {
                    // Reserve the next physical-session generation immediately,
                    // invalidating dialogs prepared against the released socket.
                    self.session_generation = self.session_generation.saturating_add(1);
                    self.suspended = true;
                    self.import_preview = None;
                    self.native_export = None;
                    self.status =
                        "Conexão em espera; retomada automática na próxima leitura".into();
                }
                Event::Resumed => {
                    self.suspended = false;
                    // This is an intermediate acknowledgement. Keep busy until
                    // the requested read produces its own final acknowledgement.
                    self.status = "Conexão retomada".into();
                }
                Event::CancelFinished => {
                    self.cancel_pending = false;
                    self.busy = false;
                }
                Event::Disconnected(s) => {
                    self.import_preview = None;
                    self.native_export = None;
                    self.active = None;
                    self.suspended = false;
                    self.clear_catalog();
                    self.transaction = false;
                    self.busy = false;
                    self.status = s;
                }
                Event::Schema(o) => {
                    self.objects = o;
                    self.busy = false;
                    self.status = format!("{} objetos · catálogo limitado", self.objects.len());
                }
                Event::Namespaces(names) => {
                    self.namespaces = names;
                    self.namespaces_loaded = true;
                    self.busy = false;
                    self.status = format!(
                        "{} namespaces · catálogo limitado a 256",
                        self.namespaces.len()
                    );
                }
                Event::NamespaceObjects(page) => {
                    let namespace = page.request.namespace.clone();
                    if !self.namespace_objects.contains_key(&namespace)
                        && self.namespace_objects.len() >= 8
                        && let Some((old, _)) = self.namespace_objects.pop_first()
                    {
                        self.namespace_pages.remove(&old);
                    }
                    self.status = format!(
                        "{}: {} tabelas/views nesta página{}",
                        namespace,
                        page.objects.len(),
                        if page.next.is_some() {
                            " · há mais"
                        } else {
                            ""
                        }
                    );
                    self.namespace_pages.insert(
                        namespace.clone(),
                        CatalogPosition {
                            request: page.request,
                            previous: page.previous,
                            next: page.next,
                        },
                    );
                    self.namespace_objects.insert(namespace, page.objects);
                    self.busy = false;
                }
                Event::CompletionTables(page) => {
                    let key = (page.request.namespace, page.request.prefix);
                    if !self.completion_tables.contains_key(&key)
                        && self.completion_tables.len() >= 8
                        && let Some((old, _)) = self.completion_tables.pop_first()
                    {
                        self.completion_more.remove(&old);
                    }
                    if page.next.is_some() {
                        self.completion_more.insert(key.clone());
                    } else {
                        self.completion_more.remove(&key);
                    }
                    self.status = format!(
                        "{} sugestões de tabelas{}",
                        page.objects.len(),
                        if page.next.is_some() {
                            " · refine o prefixo para ver outras"
                        } else {
                            ""
                        }
                    );
                    self.completion_tables.insert(key, page.objects);
                    self.busy = false;
                }
                Event::TableColumns(table, columns) => {
                    self.status = format!(
                        "{}.{}: {} colunas",
                        table.namespace,
                        table.name,
                        columns.len()
                    );
                    let key = (table.namespace, table.name);
                    if !self.table_columns.contains_key(&key) && self.table_columns.len() >= 32 {
                        self.table_columns.pop_first();
                    }
                    self.table_columns.insert(key, columns);
                    self.busy = false;
                }
                Event::Result(r, origin) => {
                    self.status = format!(
                        "{} linhas · {} ms{}",
                        r.rows.len(),
                        r.elapsed_ms,
                        if r.truncated {
                            " · resultado limitado"
                        } else {
                            ""
                        }
                    );
                    self.result = Some(r);
                    self.origin = origin;
                    self.revision += 1;
                    self.busy = false;
                }
                Event::Applied(r) => {
                    self.status =
                        format!("{} alterações enviadas; Commit pendente", r.affected_rows);
                    self.transaction = true;
                    self.origin = None;
                    self.busy = false;
                    self.revision += 1;
                }
                Event::Transaction(t, s) => {
                    self.transaction = t;
                    if !s.is_empty() {
                        self.busy = false;
                        self.status = s;
                    }
                }
                Event::Tested => {
                    self.busy = false;
                    self.status = "Teste de conexão concluído; conexão de teste fechada".into();
                }
                Event::Console(p) => {
                    self.console_path = Some(p);
                    self.busy = false;
                }
                Event::ConsoleDeleted(path) => {
                    self.console_rename_pending = None;
                    self.console_delete_pending = false;
                    self.console_deleted = Some(path);
                    self.busy = false;
                }
                Event::ConsoleRenamed(receipt) => {
                    self.console_rename_pending = None;
                    self.console_delete_pending = false;
                    self.error = receipt.warning.clone();
                    self.console_renamed = Some(receipt);
                    self.busy = false;
                }
                Event::Error(e) => {
                    self.console_rename_pending = None;
                    self.console_delete_pending = false;
                    if !self.loading_profiles {
                        self.busy = false;
                    }
                    self.error = Some(e.clone());
                    self.status = e;
                }
            }
        }
        changed
    }
}
impl Drop for RemoteService {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.sender.take();
    }
}

enum Session {
    Sql(Box<crate::remote_sql::SqlSession>),
    NoSql(crate::remote_nosql::NoSqlSession),
}
impl Session {
    async fn close(self) {
        match self {
            Self::Sql(mut s) => {
                let _ = s.close().await;
            }
            Self::NoSql(s) => s.close().await,
        }
    }
}
pub fn preview(batch: &EditBatch, engine: Engine) -> Result<String, String> {
    crate::remote_sql::preview(batch, engine)
}
fn same_credential_target(a: &ConnectionConfig, b: &ConnectionConfig) -> bool {
    a.id == b.id
        && a.engine == b.engine
        && a.host.eq_ignore_ascii_case(&b.host)
        && a.port == b.port
        && a.username == b.username
        && a.database == b.database
        && a.auth_database == b.auth_database
        && (a.engine != Engine::Mongo
            || (a.mongo.replica_set == b.mongo.replica_set
                && a.mongo.authentication == b.mongo.authentication))
}
fn redact(error: String, password: &str) -> String {
    if password.is_empty() {
        error
    } else {
        error.replace(password, "[senha]")
    }
}
fn read_profiles(path: &std::path::Path) -> Result<Vec<ConnectionConfig>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 256 * 1024 {
        return Err("Cadastro excede limite".into());
    }
    let profiles: Vec<ConnectionConfig> =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|_| "Cadastro de datasources inválido")?;
    if profiles.len() > 128 {
        return Err("Máximo 128 datasources".into());
    }
    for c in &profiles {
        c.validate()?
    }
    Ok(profiles)
}
pub(super) fn read_bindings(
    path: &std::path::Path,
    directory: &std::path::Path,
) -> Result<BTreeMap<PathBuf, String>, String> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.to_string()),
    };
    if metadata.len() >= 256 * 1024 {
        return Err("Índice de queries excede o limite de 256 KiB".into());
    }
    let bindings: BTreeMap<PathBuf, String> =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|_| "Índice de queries inválido; arquivo preservado")?;
    if bindings
        .iter()
        .any(|(p, id)| !p.starts_with(directory) || id.len() > 1024)
    {
        return Err("Índice de queries contém vínculo fora do projeto; arquivo preservado".into());
    }
    Ok(bindings)
}
fn save_profiles(path: &std::path::Path, profiles: &[ConnectionConfig]) -> Result<(), String> {
    if serde_json::to_vec(profiles)
        .map_err(|e| e.to_string())?
        .len()
        > 256 * 1024
    {
        return Err(
            "Cadastro excede256KiB; certificados públicos conhecidos podem usar modo automático"
                .into(),
        );
    }
    atomic_json(path, profiles)
}
struct ProfileMutation {
    _lock: crate::remote_scope::MetadataLock,
    path: PathBuf,
    profiles: Vec<ConnectionConfig>,
    id: String,
}
fn project_metadata_guard(
    storage: &mut Option<Result<crate::remote_scope::Storage, String>>,
    project: Option<&std::path::Path>,
    path: &std::path::Path,
) -> Result<Option<crate::remote_scope::MetadataLock>, String> {
    storage
        .as_mut()
        .map(|s| {
            s.as_mut()
                .map_err(|e| e.clone())?
                .metadata_guard(project, path)
        })
        .transpose()
}
impl ProfileMutation {
    fn prepare(
        path: &std::path::Path,
        expected: &[ConnectionConfig],
        id: &str,
    ) -> Result<Self, String> {
        let lock = crate::remote_scope::MetadataLock::acquire(path)?;
        let profiles = read_profiles(path)?;
        if profiles.iter().find(|c| c.id == id) != expected.iter().find(|c| c.id == id) {
            return Err(
                "Esta conexão foi alterada em outra janela; reabra o projeto antes de gravar"
                    .into(),
            );
        }
        Ok(Self {
            _lock: lock,
            path: path.to_path_buf(),
            profiles,
            id: id.into(),
        })
    }
    fn commit(
        mut self,
        replacement: Option<ConnectionConfig>,
    ) -> Result<Vec<ConnectionConfig>, String> {
        self.profiles.retain(|c| c.id != self.id);
        if let Some(config) = replacement {
            self.profiles.push(config);
        }
        if self.profiles.len() > 128 {
            return Err("Máximo 128 datasources".into());
        }
        save_profiles(&self.path, &self.profiles)?;
        Ok(self.profiles)
    }
}
pub(crate) fn atomic_json<T: Serialize + ?Sized>(
    path: &std::path::Path,
    profiles: &T,
) -> Result<(), String> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let mut tmp =
        tempfile::NamedTempFile::new_in(path.parent().unwrap_or(std::path::Path::new(".")))
            .map_err(|e| e.to_string())?;
    use std::io::Write;
    tmp.write_all(&serde_json::to_vec_pretty(profiles).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
enum PreparedImport {
    Sql(crate::remote_sql::PreparedSqlImport),
    NoSql(crate::transfer_nosql::PreparedNoSqlImport),
}
fn command_binding(command: &RemoteCommand) -> Option<(&str, u64)> {
    match command {
        RemoteCommand::ExportQuery(r) => Some((&r.datasource_id, r.session_generation)),
        RemoteCommand::PreviewImport(r) => Some((&r.datasource_id, r.session_generation)),
        RemoteCommand::ApplyImport(r) | RemoteCommand::ApplyNoSqlImport(r) => {
            Some((&r.datasource_id, r.session_generation))
        }
        RemoteCommand::ExportNoSql(r) => Some((&r.datasource_id, r.session_generation)),
        RemoteCommand::PreviewNoSqlImport(r) => Some((&r.datasource_id, r.session_generation)),
        RemoteCommand::Explain(r) => Some((&r.datasource_id, r.session_generation)),
        _ => None,
    }
}

/// Only commands whose adapters enforce reads may acquire a fresh idle socket.
/// Writes, Commit and Rollback must never continue on a replacement transaction.
fn resumes_idle_session(command: &RemoteCommand) -> bool {
    matches!(
        command,
        RemoteCommand::Execute(_)
            | RemoteCommand::ExecutePage(_)
            | RemoteCommand::ExecuteConsole { .. }
            | RemoteCommand::ExecuteBound { .. }
            | RemoteCommand::Explain(_)
            | RemoteCommand::ExportQuery(_)
            | RemoteCommand::ExportNoSql(_)
            | RemoteCommand::Inspect
            | RemoteCommand::ListNamespaces
            | RemoteCommand::InspectNamespace(_)
            | RemoteCommand::CatalogPage(_)
            | RemoteCommand::CompleteTables { .. }
            | RemoteCommand::InspectColumns(_)
            | RemoteCommand::LoadTable(_)
            | RemoteCommand::LoadTablePage { .. }
    )
}

async fn connect_session(
    config: &ConnectionConfig,
    password: &str,
    cancel: Arc<AtomicBool>,
) -> Result<Session, String> {
    let connect = async {
        if config.engine.is_sql() {
            crate::remote_sql::SqlSession::connect(config, password)
                .await
                .map(|session| Session::Sql(Box::new(session)))
        } else {
            crate::remote_nosql::NoSqlSession::connect(config, password)
                .await
                .map(Session::NoSql)
        }
    };
    let mut session = tokio::select! {
        result = connect => result?,
        _ = cancelled(cancel.clone()) => return Err("Conexão cancelada".into()),
        _ = tokio::time::sleep(Duration::from_secs(config.timeout_secs + 2)) => {
            return Err("Prazo da conexão excedido".into());
        }
    };
    if cancel.load(Ordering::SeqCst) {
        session.close().await;
        return Err("Conexão cancelada; nenhuma sessão mantida".into());
    }
    if let Session::Sql(sql) = &mut session {
        sql.set_cancel_flag(cancel);
    }
    Ok(session)
}

fn worker(
    anchor: PathBuf,
    project_scoped: bool,
    rx: Receiver<Request>,
    tx: SyncSender<Event>,
    cancel: Arc<AtomicBool>,
    cancel_epoch: Arc<AtomicU64>,
    export_progress: crate::query_export::QueryExportProgress,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = tx.send(Event::Error(e.to_string()));
            return;
        }
    };
    let scoped_anchor = anchor.clone();
    let mut storage = project_scoped.then(|| crate::remote_scope::Storage::new(anchor.clone()));
    let mut path = if project_scoped {
        storage
            .as_ref()
            .and_then(|s| s.as_ref().ok())
            .and_then(|s| s.path(None).ok())
            .unwrap_or_else(|| crate::remote_scope::empty_path(&anchor))
    } else {
        anchor
    };
    let mut profile_writable = true;
    if let Some(Err(error)) = &storage {
        profile_writable = false;
        let _ = tx.send(Event::Error(error.clone()));
    }
    let _ =
        tx.send(Event::LegacyAvailable(storage.as_ref().is_some_and(|s| {
            s.as_ref().is_ok_and(|s| s.legacy_available())
        })));
    let mut profiles = match read_profiles(&path) {
        Ok(p) => p,
        Err(e) => {
            profile_writable = !project_scoped;
            let _ = tx.send(Event::Error(e));
            vec![]
        }
    };
    let _ = tx.send(Event::Profiles(profiles.clone()));
    let mut bindings_path = path.with_extension("console-bindings.json");
    let mut consoles_dir = path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("consoles");
    let mut bindings: BTreeMap<PathBuf, String> = std::fs::metadata(&bindings_path)
        .ok()
        .filter(|m| m.len() < 256 * 1024)
        .and_then(|_| std::fs::read(&bindings_path).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    bindings.retain(|p, id| p.starts_with(&consoles_dir) && id.len() <= 1024);
    let _ = tx.send(Event::Bindings(bindings.clone()));
    let mut prepared_import: Option<(String, u64, u64, u64, PreparedImport)> = None;
    let mut preview_sequence = 0u64;
    let mut session: Option<Session> = None;
    let mut cleanup_blocked: Option<Box<crate::remote_sql::SqlSession>> = None;
    let mut config: Option<ConnectionConfig> = None;
    let mut password = String::new();
    let mut session_generation = 0u64;
    let mut connection_generation = 0u64;
    let mut current_project: Option<PathBuf> = None;
    let credentials = crate::remote_credentials::CredentialStore::new();
    let mut last = Instant::now();
    loop {
        let received = rx.recv_timeout(Duration::from_millis(200));
        if cancel.load(Ordering::SeqCst) {
            if let Some(s) = session.take() {
                match s {
                    Session::Sql(mut sql) => {
                        if let Err(error) = rt.block_on(sql.close()) {
                            cleanup_blocked = Some(sql);
                            let _ = tx.send(Event::Error(format!(
                                "Reconexão bloqueada: encerramento remoto não confirmado. {error}"
                            )));
                        }
                    }
                    Session::NoSql(s) => rt.block_on(s.close()),
                }
            }
            if config.is_some() {
                let message = if cleanup_blocked.is_some() {
                    "Cancelamento solicitado; encerramento remoto não confirmado; reconexão bloqueada"
                } else {
                    "Conexão encerrada após cancelamento"
                };
                let _ = tx.send(Event::Disconnected(message.into()));
            }
            config = None;
            password.clear();
            prepared_import = None;
            cancel.store(false, Ordering::SeqCst);
            if tx.send(Event::CancelFinished).is_err() {
                break;
            }
            // Any command already queued belongs to the cancelled operation.
            if matches!(received, Err(mpsc::RecvTimeoutError::Disconnected)) {
                break;
            }
            // Scope metadata transitions have their own acknowledgement. A
            // shutdown cancellation must not strand the project's busy gate.
            if !matches!(&received, Ok(Request::Project(_) | Request::ClaimLegacy(_))) {
                continue;
            }
        }
        // Check the deadline before a newly arrived command too. The server's
        // idle deadline may otherwise win the race with our periodic wake-up.
        if session.is_some()
            && config
                .as_ref()
                .is_some_and(|c| last.elapsed() >= Duration::from_secs(c.idle_secs))
        {
            let transaction = matches!(&session, Some(Session::Sql(s)) if s.transaction_active());
            if let Some(s) = session.take() {
                rt.block_on(s.close());
            }
            prepared_import = None;
            let event = if transaction {
                let message = closed_message(
                    config.as_ref().map(|c| c.engine),
                    "Sessão fechada por inatividade",
                );
                config = None;
                password.clear();
                Event::Disconnected(message)
            } else {
                session_generation = session_generation.saturating_add(1);
                Event::Suspended
            };
            if tx.send(event).is_err() {
                break;
            }
        }
        let request = match received {
            Ok(c) => c,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
        };
        let cmd = match request {
            Request::Command(cmd) => *cmd,
            request => {
                let (project, claim) = match request {
                    Request::Project(project) => (project, false),
                    Request::ClaimLegacy(project) => (Some(project), true),
                    Request::Command(_) => unreachable!(),
                };
                current_project = project.clone();
                let loaded = (|| {
                    if matches!(&session, Some(Session::Sql(s)) if s.transaction_active()) {
                        return Err("Faça Commit ou Rollback antes de trocar o projeto".into());
                    }
                    if let Some(old) = session.take() {
                        match old {
                            Session::Sql(mut sql) => {
                                if let Err(error) = rt.block_on(sql.close()) {
                                    cleanup_blocked = Some(sql);
                                    return Err(format!(
                                        "Encerramento remoto não confirmado; reconexão bloqueada: {error}"
                                    ));
                                }
                            }
                            Session::NoSql(s) => rt.block_on(s.close()),
                        }
                    }
                    config = None;
                    password.clear();
                    prepared_import = None;
                    if matches!(storage, Some(Err(_))) {
                        storage = Some(crate::remote_scope::Storage::new(scoped_anchor.clone()));
                    }
                    let scope = storage
                        .as_mut()
                        .ok_or("Armazenamento de projeto indisponível")?
                        .as_mut()
                        .map_err(|error| error.clone())?;
                    scope.refresh_owner()?;
                    if claim {
                        scope.claim(project.as_deref().ok_or("Projeto não selecionado")?)?;
                    }
                    let next = scope.path(project.as_deref())?;
                    path = next;
                    bindings_path = path.with_extension("console-bindings.json");
                    consoles_dir = path
                        .parent()
                        .ok_or("Cadastro sem diretório")?
                        .join("consoles");
                    let next_profiles = read_profiles(&path)?;
                    let next_bindings = read_bindings(&bindings_path, &consoles_dir)?;
                    Ok((next_profiles, next_bindings))
                })();
                session_generation = session_generation.saturating_add(1);
                connection_generation = connection_generation.saturating_add(1);
                let error = match loaded {
                    Ok((next_profiles, next_bindings)) => {
                        profiles = next_profiles;
                        bindings = next_bindings;
                        profile_writable = true;
                        None
                    }
                    Err(error) => {
                        profiles.clear();
                        bindings.clear();
                        profile_writable = false;
                        Some(error)
                    }
                };
                let available = storage
                    .as_ref()
                    .is_some_and(|s| s.as_ref().is_ok_and(|s| s.legacy_available()));
                if tx
                    .send(Event::ProjectLoaded {
                        directory: consoles_dir.clone(),
                        profiles: profiles.clone(),
                        bindings: bindings.clone(),
                        error,
                        legacy_available: available,
                    })
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };
        if !profile_writable {
            if tx.send(Event::Error("Cadastro ou associação de bancos inválido; arquivos preservados. Reabra o projeto após corrigir o arquivo.".into())).is_err() { break; }
            continue;
        }
        last = Instant::now();
        let metadata = matches!(
            cmd,
            RemoteCommand::Save(_)
                | RemoteCommand::SaveRemembered { .. }
                | RemoteCommand::Delete(_)
                | RemoteCommand::DeleteRemembered(_)
                | RemoteCommand::CreateConsole
                | RemoteCommand::CreateConsoleFor(_)
                | RemoteCommand::RenameConsole { .. }
                | RemoteCommand::DeleteConsole(_)
        );
        let _metadata_scope_guard = if metadata {
            match project_metadata_guard(&mut storage, current_project.as_deref(), &path) {
                Ok(guard) => guard,
                Err(error) => {
                    if tx.send(Event::Error(error)).is_err() {
                        break;
                    }
                    continue;
                }
            }
        } else {
            None
        };
        if !metadata
            && let Some(mut blocked) = cleanup_blocked.take()
            && let Err(error) = rt.block_on(blocked.close())
        {
            cleanup_blocked = Some(blocked);
            if tx
                .send(Event::Error(format!(
                    "Reconexão bloqueada: encerramento da sessão anterior não confirmado. {error}"
                )))
                .is_err()
            {
                break;
            }
            continue;
        }
        let credential_connect = matches!(cmd, RemoteCommand::ConnectRemembered { .. });
        let cmd = match cmd {
            RemoteCommand::ConnectRemembered {
                config,
                password,
                test,
            } => RemoteCommand::Connect {
                config,
                password,
                test,
            },
            other => other,
        };
        // The worker can release a socket before the UI has polled Suspended.
        // Recheck request identity here, without discarding the logical source.
        if let Some((id, generation)) = command_binding(&cmd)
            && (config.as_ref().is_none_or(|c| c.id != id) || generation != session_generation)
        {
            if tx
                .send(Event::Error(
                    "Sessão/datasource mudou; abra novamente a transferência".into(),
                ))
                .is_err()
            {
                break;
            }
            continue;
        }
        if let RemoteCommand::ExecutePage(request) = &cmd
            && (config
                .as_ref()
                .is_none_or(|c| c.id != request.datasource_id)
                || connection_generation != request.connection_generation)
        {
            if tx
                .send(Event::Error(
                    "Sessão/datasource mudou; execute a consulta novamente".into(),
                ))
                .is_err()
            {
                break;
            }
            continue;
        }
        let operation_timeout = if let RemoteCommand::ExportQuery(request) = &cmd {
            request.payload.timeout_secs + 2
        } else {
            config.as_ref().map_or(12, |c| c.timeout_secs + 2)
        };
        let recoverable_command = matches!(
            cmd,
            RemoteCommand::ExportQuery(_)
                | RemoteCommand::Execute(_)
                | RemoteCommand::Explain(_)
                | RemoteCommand::ExecutePage(_)
                | RemoteCommand::ExecuteConsole { .. }
                | RemoteCommand::ExecuteBound { .. }
                | RemoteCommand::Inspect
                | RemoteCommand::ListNamespaces
                | RemoteCommand::InspectNamespace(_)
                | RemoteCommand::CatalogPage(_)
                | RemoteCommand::CompleteTables { .. }
                | RemoteCommand::InspectColumns(_)
                | RemoteCommand::LoadTable(_)
                | RemoteCommand::LoadTablePage { .. }
                | RemoteCommand::PreviewImport(_)
                | RemoteCommand::ApplyImport(_)
        );
        if matches!(
            cmd,
            RemoteCommand::Connect { .. }
                | RemoteCommand::Disconnect
                | RemoteCommand::Execute(_)
                | RemoteCommand::ExecutePage(_)
                | RemoteCommand::ExecuteConsole { .. }
                | RemoteCommand::ExecuteBound { .. }
                | RemoteCommand::Commit
                | RemoteCommand::Rollback
                | RemoteCommand::Apply(_)
                | RemoteCommand::PreviewImport(_)
                | RemoteCommand::PreviewNoSqlImport(_)
        ) {
            prepared_import = None;
        }
        let progress = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut native_apply_started = false;
        let mut native_outcome_unknown = false;
        let mut import_attempted = 0;
        let mut force_close = false;
        let event:Result<Event,String>=rt.block_on(async {
            if let RemoteCommand::ExecuteConsole { datasource_id, .. } = &cmd
                && config.as_ref().is_none_or(|c| &c.id != datasource_id) {
                let target=profiles.iter().find(|c| &c.id==datasource_id)
                    .ok_or("Conexão desta query não encontrada; restaure o cadastro da origem")?.clone();
                target.validate()?;
                if matches!(&session, Some(Session::Sql(s)) if s.transaction_active()) {
                    return Err("Faça Commit ou Rollback antes de executar a query de outra conexão".into());
                }
                // Exact persisted identity only. Never borrow another source's secret.
                let secret=if target.save_credentials {
                    credentials.load(&target).await.map_err(|e|e.to_string())?
                        .map(|s|s.expose().to_owned()).unwrap_or_default()
                } else {String::new()};
                if cancel.load(Ordering::SeqCst) {return Err("Execução cancelada antes de conectar".into());}
                if let Some(previous)=session.take() {previous.close().await;}
                config=None; password.clear();
                let opened=connect_session(&target,&secret,cancel.clone()).await.map_err(|e|redact(e,&secret))?;
                if cancel.load(Ordering::SeqCst) {opened.close().await;return Err("Execução cancelada; nenhuma sessão mantida".into());}
                password=secret;config=Some(target.clone());session=Some(opened);
                session_generation=session_generation.saturating_add(1);
                connection_generation=connection_generation.saturating_add(1);
                // Intermediate acknowledgement must not release the command slot.
                let _=tx.send(Event::ConnectedForQuery(target));
            }
            if session.is_none() && resumes_idle_session(&cmd) {
                let cfg=config.as_ref().ok_or("Conecte um datasource primeiro")?;
                let restored=connect_session(cfg,&password,cancel.clone()).await.map_err(|e|redact(e,&password))?;
                session=Some(restored);
                let _=tx.send(Event::Resumed);
            }
            match cmd {
                RemoteCommand::SaveRemembered{config:mut c,password:p}=>{
                    crate::tls_roots::prepare_saved_certificate(&mut c)?;
                    let old=profiles.iter().find(|v|v.id==c.id).cloned();
                    let mutation=ProfileMutation::prepare(&path,&profiles,&c.id)?;
                    if c.save_credentials {
                        if !p.is_empty(){credentials.store(&c,p).await.map_err(|e|e.to_string())?;}
                    } else if old.as_ref().is_some_and(|v|v.save_credentials) {
                        credentials.delete(&c).await.map_err(|e|e.to_string())?;
                    }
                    profiles=mutation.commit(Some(c.clone()))?;
                    if let Some(old)=old.filter(|old|old.save_credentials && !same_credential_target(old,&c))
                        && let Err(error)=credentials.delete(&old).await {
                        let _=tx.send(Event::Saved(profiles.clone()));
                        return Err(error.to_string());
                    }
                    Ok(Event::Saved(profiles.clone()))
                },
                RemoteCommand::DeleteRemembered(id)=>{
                    let mutation=ProfileMutation::prepare(&path,&profiles,&id)?;
                    if let Some(c)=profiles.iter().find(|c|c.id==id && c.save_credentials){credentials.delete(c).await.map_err(|e|e.to_string())?;}
                    profiles=mutation.commit(None)?;
                    if config.as_ref().is_some_and(|c|c.id==id) { if let Some(s)=session.take(){s.close().await;}config=None;password.clear();prepared_import=None;let _=tx.send(Event::Disconnected("Desconectado".into())); }
                    Ok(Event::Profiles(profiles.clone()))
                },
                RemoteCommand::Save(mut c)=>{crate::tls_roots::prepare_saved_certificate(&mut c)?;let mutation=ProfileMutation::prepare(&path,&profiles,&c.id)?;profiles=mutation.commit(Some(c))?;Ok(Event::Saved(profiles.clone()))},
                RemoteCommand::Delete(id)=>{let mutation=ProfileMutation::prepare(&path,&profiles,&id)?;profiles=mutation.commit(None)?;
                    if config.as_ref().is_some_and(|c|c.id==id) { if let Some(s)=session.take(){s.close().await;}config=None;password.clear();prepared_import=None;let _=tx.send(Event::Disconnected("Desconectado".into())); }
                    Ok(Event::Profiles(profiles.clone()))},
                RemoteCommand::Connect{config:mut c,password:mut p,test}=>{
                    crate::tls_roots::prepare_saved_certificate(&mut c)?;
                    // A test also owns the sole connection slot.
                    if let Some(s)=session.take(){s.close().await;}config=None;password.clear();
                    if credential_connect && c.save_credentials && p.is_empty()
                        && let Some(secret)=credentials.load(&c).await.map_err(|e|e.to_string())? {p=secret.expose().to_owned();}
                    let s=connect_session(&c,&p,cancel.clone()).await.map_err(|e|redact(e,&p))?;
                    if test{s.close().await;Ok(Event::Tested)}else{
                        let _scope=match project_metadata_guard(&mut storage,current_project.as_deref(),&path){Ok(g)=>g,Err(e)=>{s.close().await;return Err(e)}};
                        let mutation=match ProfileMutation::prepare(&path,&profiles,&c.id){Ok(m)=>m,Err(e)=>{s.close().await;return Err(e)}};
                        if credential_connect {
                            let update=if c.save_credentials && !p.is_empty() {credentials.store(&c,p.clone()).await}
                                else if !c.save_credentials && profiles.iter().any(|old|old.id==c.id && old.save_credentials){credentials.delete(&c).await}
                                else{Ok(())};
                            if let Err(error)=update {s.close().await;return Err(error.to_string());}
                        }
                        if cancel.load(Ordering::SeqCst) {s.close().await;return Err("Conexão cancelada; nenhuma sessão mantida".into());}
                        let old=profiles.iter().find(|old|old.id==c.id && old.save_credentials && !same_credential_target(old,&c)).cloned();
                        profiles=match mutation.commit(Some(c.clone())){Ok(next)=>next,Err(error)=>{s.close().await;return Err(error)}};
                        let _=tx.send(Event::ProfilesChanged(profiles.clone()));
                        if credential_connect && let Some(old)=old && let Err(error)=credentials.delete(&old).await {s.close().await;return Err(error.to_string());}
                        config=Some(c.clone());password=p;session=Some(s);session_generation=session_generation.saturating_add(1);connection_generation=connection_generation.saturating_add(1);Ok(Event::Connected(c))}
                },
                RemoteCommand::Disconnect=>{let message=closed_message(config.as_ref().map(|c|c.engine),"Desconectado");if let Some(s)=session.take(){s.close().await}config=None;password.clear();Ok(Event::Disconnected(message))},
                command @ (RemoteCommand::CreateConsole | RemoteCommand::CreateConsoleFor(_)) => {
                    let c=match command { RemoteCommand::CreateConsoleFor(id)=>profiles.iter().find(|c|c.id==id).ok_or("Fonte não cadastrada")?, _=>config.as_ref().ok_or("Conecte um datasource primeiro")? };
                    let _lock=crate::remote_scope::MetadataLock::acquire(&bindings_path)?;
                    bindings=read_bindings(&bindings_path,&consoles_dir)?;
                    let file=crate::console_files::create(&consoles_dir,&bindings_path,&mut bindings,c)?;
                    let _=tx.send(Event::Bindings(bindings.clone()));Ok(Event::Console(file))
                },
                RemoteCommand::DeleteConsole(source)=>{
                    let _lock=crate::remote_scope::MetadataLock::acquire(&bindings_path)?;
                    bindings=read_bindings(&bindings_path,&consoles_dir)?;
                    crate::console_files::delete(&consoles_dir,&bindings_path,&mut bindings,&source)?;
                    let _=tx.send(Event::Bindings(bindings.clone()));Ok(Event::ConsoleDeleted(source))
                },
                RemoteCommand::RenameConsole{path:source,name}=>{
                    let _lock=crate::remote_scope::MetadataLock::acquire(&bindings_path)?;
                    bindings=read_bindings(&bindings_path,&consoles_dir)?;
                    let receipt=crate::console_files::rename(&consoles_dir,&bindings_path,&mut bindings,&source,&name)?;
                    let _=tx.send(Event::Bindings(bindings.clone()));Ok(Event::ConsoleRenamed(receipt))
                },
                cmd=>{
                    let cfg=config.as_ref().ok_or("Conecte um datasource primeiro")?;let s=session.as_mut().ok_or("Sessão encerrada; reconecte")?;
                    let query_timeout=if matches!(&cmd, RemoteCommand::ExecuteConsole{..}) {cfg.timeout_secs+2} else {operation_timeout};
                    let op=async {match (cmd,s){
                        (RemoteCommand::ExportQuery(request),Session::Sql(s))=>{
                            if request.datasource_id!=cfg.id{return Err("Datasource de exportação mudou".into());}
                            s.export_query(&request.payload,export_progress.clone()).await.map(Event::QueryExport)
                        },
                        (RemoteCommand::PreviewImport(request),Session::Sql(s))=>{
                            if request.datasource_id!=cfg.id{return Err("Datasource mudou após escolher o arquivo".into());}
                            let partial=request.payload.data.source_truncated;let plan=s.prepare_import(&request.payload).await?;preview_sequence+=1;
                            let preview=ImportPreview {preview_id:preview_sequence,rows:plan.len(),columns:plan.columns.clone(),description:format!("INSERT parametrizado em {}.{}; sem UPSERT; Commit manual",plan.table.namespace,plan.table.name),warnings:if partial{vec![format!("Origem truncada: somente{}linhas carregadas",plan.len()),"Nenhum registro existente será substituído. Conflito reverte todo o lote.".into()]}else{vec!["Nenhum registro existente será substituído. Conflito reverte todo o lote.".into()]}};
                            prepared_import=Some((request.datasource_id,request.session_generation,preview_sequence,cancel_epoch.load(Ordering::SeqCst),PreparedImport::Sql(plan)));Ok(Event::ImportPreview(preview))
                        },
                        (RemoteCommand::ApplyImport(request),Session::Sql(s))=>{
                            let (id,generation,token,epoch,PreparedImport::Sql(plan))=prepared_import.take().ok_or("Prepare uma prévia antes de importar")? else{return Err("Prévia não corresponde ao driver".into())};
                            if epoch!=cancel_epoch.load(Ordering::SeqCst) || id!=cfg.id || id!=request.datasource_id || generation!=request.session_generation || token!=request.payload.preview_id{return Err("Prévia obsoleta; prepare novamente".into());}
                            if profiles.iter().any(|p|p.id==cfg.id && p.read_only){return Err("Datasource salvo somente leitura; reconecte".into());}
                            s.apply_import(&plan).await.map(Event::ImportReport)
                        },
                        (RemoteCommand::ExportNoSql(request),Session::NoSql(s))=>{
                            if request.datasource_id!=cfg.id{return Err("Datasource de exportação mudou".into());}
                            s.export_source(cfg,&request.payload).await?.to_json().map(Event::NativeExport)
                        },
                        (RemoteCommand::PreviewNoSqlImport(request),Session::NoSql(s))=>{
                            if request.datasource_id!=cfg.id || cfg.read_only || profiles.iter().any(|p|p.id==cfg.id && p.read_only){return Err("Conexão somente leitura ou datasource mudou".into());}
                            let plan=crate::transfer_nosql::prepare_import(&request.payload.json,cfg.engine,request.payload.target,request.payload.ttl_policy)?;
                            let checked=s.preview_import(cfg,&plan).await?;preview_sequence+=1;
                            let mut warnings=checked.warnings;warnings.push(format!("{} conflitos existentes; grupo não atômico; um prefixo confirmado pode permanecer em caso de erro",checked.conflicts.len()));
                            let preview=ImportPreview{preview_id:preview_sequence,rows:plan.len(),columns:vec![],description:plan.summary(),warnings};
                            prepared_import=Some((request.datasource_id,request.session_generation,preview_sequence,cancel_epoch.load(Ordering::SeqCst),PreparedImport::NoSql(plan)));Ok(Event::ImportPreview(preview))
                        },
                        (RemoteCommand::ApplyNoSqlImport(request),Session::NoSql(s))=>{
                            let(id,generation,token,epoch,PreparedImport::NoSql(plan))=prepared_import.take().ok_or("Prepare uma prévia antes de importar")? else{return Err("Prévia não corresponde ao driver".into())};
                            if epoch!=cancel_epoch.load(Ordering::SeqCst) || id!=cfg.id || id!=request.datasource_id || generation!=request.session_generation || token!=request.payload.preview_id || cfg.read_only || profiles.iter().any(|p|p.id==cfg.id && p.read_only){return Err("Prévia obsoleta ou somente leitura; prepare novamente".into());}
                            import_attempted=plan.len();native_apply_started=true;
                            let report=s.apply_import(cfg,&plan,progress.clone()).await;
                            native_outcome_unknown=report.outcome_unknown;
                            Ok(Event::ImportReport(ImportReport{attempted:plan.len(),applied:report.applied,failed:usize::from(report.failed_index.is_some()),unknown:usize::from(report.outcome_unknown),transaction_pending:false,atomic:false,summary:format!("{} confirmados; {}",report.applied,report.error.unwrap_or_else(||"importação concluída; alterações individuais persistidas".into()))}))
                        },
                        (RemoteCommand::Inspect,Session::Sql(s))=>s.inspect().await.map(Event::Schema),
                        (RemoteCommand::ListNamespaces,Session::Sql(s))=>s.namespaces().await.map(Event::Namespaces),
                        (RemoteCommand::InspectNamespace(namespace),Session::Sql(s))=>s.inspect_namespace_page(CatalogRequest {namespace,..Default::default()}).await.map(Event::NamespaceObjects),
                        (RemoteCommand::CatalogPage(request),Session::Sql(s))=>s.inspect_namespace_page(request).await.map(Event::NamespaceObjects),
                        (RemoteCommand::CompleteTables{namespace,prefix},Session::Sql(s))=>s.inspect_namespace_page(CatalogRequest{namespace,prefix,..Default::default()}).await.map(Event::CompletionTables),
                        (RemoteCommand::InspectColumns(table),Session::Sql(s))=>s.columns_for_table(&table).await.map(|columns| Event::TableColumns(table,columns)),
                        (RemoteCommand::Inspect,Session::NoSql(s))=>s.schema(cfg).await.map(Event::Schema),
                        (RemoteCommand::Explain(request),Session::Sql(s))=>{
                            if request.datasource_id != cfg.id {return Err("Datasource do plano mudou".into());}
                            s.explain(&request.payload).await.map(|r|Event::Result(r,None))
                        },
                        (RemoteCommand::ExecutePage(request),Session::Sql(s))=>{
                            let r=s.execute_page(&request.payload).await?;
                            let origin=if cfg.read_only{None}else{s.editable_origin(&request.payload.sql,&r).await?};
                            Ok(Event::Result(r,origin))
                        },
                        (RemoteCommand::Execute(q)|RemoteCommand::ExecuteBound{sql:q,..}|RemoteCommand::ExecuteConsole{sql:q,..},Session::Sql(s))=>{
                            let r=s.execute(&q).await?;
                            let origin=if cfg.read_only{None}else{s.editable_origin(&q,&r).await?};
                            Ok(Event::Result(r,origin))
                        },
                        (RemoteCommand::Execute(q)|RemoteCommand::ExecuteBound{sql:q,..}|RemoteCommand::ExecuteConsole{sql:q,..},Session::NoSql(s))=>s.execute(&q,cfg).await.map(|r|Event::Result(r,None)),
                        (RemoteCommand::LoadTable(t),Session::Sql(s))=>s.load_table(&t).await.map(|(r,c)|Event::Result(r,Some((t,c)))),
                        (RemoteCommand::LoadTablePage{table:t,offset},Session::Sql(s))=>s.load_table_page(&t,offset).await.map(|(r,c)|Event::Result(r,Some((t,c)))),
                        (RemoteCommand::Apply(batch),Session::Sql(s))=>s.apply(&batch).await.map(Event::Applied),
                        (RemoteCommand::Commit,Session::Sql(s))=>s.commit().await.map(|_|Event::Transaction(false,"Commit concluído".into())),
                        (RemoteCommand::Rollback,Session::Sql(s))=>s.rollback().await.map(|_|Event::Transaction(false,"Rollback concluído; recarregue os dados".into())),
                        _=>Err("Operação não disponível neste driver".into()),
                    }};
                    tokio::select! {
                        r=op=>r,
                        _=async {loop {tokio::time::sleep(Duration::from_millis(25)).await;if cancel.load(Ordering::SeqCst){break}}}=>{force_close=true;Err("Cancelamento solicitado: conexão fechada; interrupção no servidor não confirmada".into())},
                        _=tokio::time::sleep(Duration::from_secs(query_timeout))=>{force_close=true;Err("Prazo excedido: conexão fechada; conclusão remota não confirmada".into())},
                    }
                }
            }
        });
        if force_close && let Some(Session::Sql(sql)) = &mut session {
            sql.mark_interrupted();
        }
        // Work performed by the server is not idle time. Start the inactivity
        // interval when the operation finishes, including a long bounded read.
        last = Instant::now();
        if native_outcome_unknown {
            if let Some(s) = session.take() {
                rt.block_on(s.close());
            }
            config = None;
            prepared_import = None;
            password.clear();
            let _ = tx.send(Event::Disconnected(
                "Resultado remoto incerto; sessão fechada. Não repetir automaticamente".into(),
            ));
        }
        let event = if native_apply_started && event.is_err() {
            let applied = progress.load(Ordering::SeqCst);
            if let Some(s) = session.take() {
                rt.block_on(s.close());
            }
            config = None;
            prepared_import = None;
            password.clear();
            let _ = tx.send(Event::Disconnected(
                "Importação interrompida; reconecte e confira o destino".into(),
            ));
            Ok(Event::ImportReport(ImportReport {
                attempted: import_attempted,
                applied,
                failed: 0,
                unknown: 1,
                transaction_pending: false,
                atomic: false,
                summary: format!(
                    "{applied} registros confirmados podem permanecer; item em andamento incerto. Nenhuma repetição automática."
                ),
            }))
        } else {
            event
        };
        match event {
            Ok(e) => {
                if tx.send(e).is_err() {
                    break;
                }
                if let Some(Session::Sql(s)) = &session {
                    let _ = tx.send(Event::Transaction(s.transaction_active(), String::new()));
                }
            }
            Err(e) => {
                if recoverable_command
                    && !force_close
                    && let Some(Session::Sql(sql)) = session.as_ref()
                    && !sql.is_closed()
                {
                    let _ = tx.send(Event::Transaction(sql.transaction_active(), String::new()));
                    if tx
                        .send(Event::Error(format!(
                            "{} · sessão preservada",
                            redact(e, &password)
                        )))
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                if metadata {
                    if tx.send(Event::Error(e)).is_err() {
                        break;
                    }
                    continue;
                }
                let closed = closed_message(config.as_ref().map(|c| c.engine), "conexão fechada");
                let cleanup = if let Some(s) = session.take() {
                    match s {
                        Session::Sql(mut sql) => match rt.block_on(sql.close()) {
                            Ok(()) => None,
                            Err(error) => {
                                cleanup_blocked = Some(sql);
                                Some(error)
                            }
                        },
                        Session::NoSql(s) => {
                            rt.block_on(s.close());
                            None
                        }
                    }
                } else {
                    None
                };
                let e = if let Some(cleanup) = cleanup {
                    format!(
                        "{e} · Reconexão bloqueada: encerramento no servidor não confirmado: {cleanup}"
                    )
                } else {
                    e
                };
                config = None;
                prepared_import = None;
                let e = format!("{} · {closed}", redact(e, &password));
                password.clear();
                let _ = tx.send(Event::Disconnected(
                    "Conexão fechada após erro; reconecte".into(),
                ));
                if tx.send(Event::Error(e)).is_err() {
                    break;
                }
            }
        }
    }
    if let Some(s) = session {
        rt.block_on(s.close())
    }
    if let Some(mut blocked) = cleanup_blocked {
        let _ = rt.block_on(blocked.close());
    }
}

fn closed_message(engine: Option<Engine>, reason: &str) -> String {
    match engine {
        Some(engine) if engine.is_sql() => format!("{reason}; transação não confirmada revertida"),
        Some(_) => {
            format!("{reason}; itens NoSQL confirmados permanecem; não há rollback implícito")
        }
        None => reason.into(),
    }
}

fn validate_catalog_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 1024 || name.contains(['\0', '\n', '\r']) {
        Err("Nome de namespace/tabela inválido ou excede 1024 bytes".into())
    } else {
        Ok(())
    }
}

async fn cancelled(flag: Arc<AtomicBool>) {
    loop {
        tokio::time::sleep(Duration::from_millis(25)).await;
        if flag.load(Ordering::SeqCst) {
            return;
        }
    }
}

pub(crate) fn validate_catalog_request(request: &CatalogRequest) -> Result<(), String> {
    validate_catalog_name(&request.namespace)?;
    if request.prefix.len() > 1024 || request.prefix.contains(['\0', '\n', '\r']) {
        return Err("Prefixo inválido; máximo1024 bytes".into());
    }
    for cursor in [&request.after, &request.before].into_iter().flatten() {
        validate_catalog_name(cursor)?;
    }
    if request.after.is_some() && request.before.is_some() {
        return Err("Use apenas um cursor de catálogo".into());
    }
    Ok(())
}

#[cfg(test)]
mod close_message_tests {
    use super::*;
    #[test]
    fn saved_console_connect_ack_keeps_slot_busy_and_rejects_missing_source_or_pending_switch() {
        let directory = tempfile::tempdir().unwrap();
        let mut service = RemoteService::new(directory.path().join("profiles.json"));
        let (events, receiver) = mpsc::sync_channel(8);
        service.receiver = receiver;
        service.loading_profiles = false;
        let source = ConnectionConfig {
            id: "qa".into(),
            ..Default::default()
        };
        service.profiles = vec![
            source.clone(),
            ConnectionConfig {
                id: "dev".into(),
                ..source.clone()
            },
        ];
        events.send(Event::ConnectedForQuery(source)).unwrap();
        service.poll();
        assert!(
            service.is_busy(),
            "intermediate Connect cannot admit another command"
        );
        assert!(service.dispatch(RemoteCommand::Disconnect).is_err());
        events
            .send(Event::Result(
                RemoteResult {
                    columns: vec!["value".into()],
                    rows: vec![vec![Cell::Number("123".into())]],
                    truncated: false,
                    elapsed_ms: 1,
                    affected_rows: 0,
                },
                None,
            ))
            .unwrap();
        service.poll();
        assert!(!service.is_busy());
        let revision = service.revision;
        for (sql, datasource_id) in [("SELECT 1", "missing"), ("", "qa")] {
            assert!(
                service
                    .dispatch(RemoteCommand::ExecuteConsole {
                        sql: sql.into(),
                        datasource_id: datasource_id.into()
                    })
                    .is_err()
            );
            assert_eq!(service.revision, revision);
            assert!(
                service.result.is_some(),
                "rejected requests preserve the old result"
            );
        }
        service.transaction = true;
        assert!(
            service
                .dispatch(RemoteCommand::ExecuteConsole {
                    sql: "SELECT 1".into(),
                    datasource_id: "dev".into()
                })
                .is_err()
        );
        assert_eq!(service.active.as_ref().unwrap().id, "qa");
        assert!(service.transaction);
        assert!(service.result.is_some());
    }

    #[test]
    fn transfer_acknowledgements_preserve_result_revision_and_loaded_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let mut service = RemoteService::new(directory.path().join("profiles.json"));
        let (events, receiver) = mpsc::sync_channel(8);
        service.receiver = receiver;
        service.loading_profiles = false;
        service.revision = 41;
        let columns = vec!["id".to_owned(), "name".to_owned()];
        let rows = vec![vec![
            Cell::Number("1".into()),
            Cell::Text("original".into()),
        ]];
        service.result = Some(RemoteResult {
            columns: columns.clone(),
            rows: rows.clone(),
            truncated: false,
            elapsed_ms: 3,
            affected_rows: 0,
        });
        let origin = TableRef {
            namespace: "public".into(),
            name: "fixture".into(),
        };
        service.origin = Some((origin.clone(), Vec::new()));
        let assert_snapshot = |service: &RemoteService| {
            assert_eq!(
                service.revision, 41,
                "transfer metadata must not invalidate staged grid proposals"
            );
            let result = service
                .result
                .as_ref()
                .expect("loaded SELECT remains available");
            assert_eq!(result.columns, columns);
            assert_eq!(result.rows, rows);
            assert_eq!(result.elapsed_ms, 3);
            assert!(!result.truncated);
            assert_eq!(
                service.origin.as_ref().map(|(table, _)| table),
                Some(&origin)
            );
        };
        service.busy = true;
        events
            .send(Event::ImportPreview(ImportPreview {
                preview_id: 7,
                rows: 1,
                columns: columns.clone(),
                description: "bound INSERT".into(),
                warnings: vec![],
            }))
            .unwrap();
        assert!(service.poll());
        assert!(!service.is_busy());
        assert_eq!(service.import_preview.as_ref().unwrap().preview_id, 7);
        assert_eq!(service.status, "Prévia preparada; nenhuma linha inserida");
        assert_snapshot(&service);

        service.busy = true;
        events
            .send(Event::ImportReport(ImportReport {
                attempted: 1,
                applied: 1,
                failed: 0,
                unknown: 0,
                transaction_pending: true,
                atomic: true,
                summary: "Commit pendente".into(),
            }))
            .unwrap();
        assert!(service.poll());
        assert!(!service.is_busy());
        assert!(service.transaction);
        assert!(service.import_preview.is_none());
        assert_eq!(service.import_report.as_ref().unwrap().applied, 1);
        assert_eq!(service.status, "Commit pendente");
        assert_snapshot(&service);

        service.busy = true;
        events
            .send(Event::NativeExport("native fixture snapshot".into()))
            .unwrap();
        assert!(service.poll());
        assert!(!service.is_busy());
        assert_eq!(
            service.native_export.as_deref(),
            Some("native fixture snapshot")
        );
        assert_eq!(service.status, "Snapshot nativo preparado");
        assert_snapshot(&service);

        events
            .send(Event::Result(
                RemoteResult {
                    columns: vec!["next".into()],
                    rows: vec![],
                    truncated: false,
                    elapsed_ms: 1,
                    affected_rows: 0,
                },
                None,
            ))
            .unwrap();
        assert!(service.poll());
        assert_eq!(
            service.revision, 42,
            "a new actual SELECT still changes result identity"
        );
        assert_eq!(service.result.as_ref().unwrap().columns, vec!["next"]);
    }

    #[test]
    fn closing_nosql_never_claims_reversal_of_confirmed_items() {
        for engine in [Engine::Mongo, Engine::Redis, Engine::Memcached] {
            let message = closed_message(Some(engine), "Fechada");
            assert!(message.contains("confirmados permanecem"));
            assert!(!message.contains("revertida"));
        }
        assert!(
            closed_message(Some(Engine::Postgres), "Fechada").contains("não confirmada revertida")
        );
        assert_eq!(closed_message(None, "Fechada"), "Fechada");
    }
    #[test]
    fn mongo_credential_cleanup_uses_the_same_auth_scope_as_the_vault() {
        let a = ConnectionConfig {
            engine: Engine::Mongo,
            ..Default::default()
        };
        let mut b = a.clone();
        b.mongo.replica_set = "fixture-rs".into();
        assert!(!same_credential_target(&a, &b));
        b = a.clone();
        b.mongo.authentication = MongoAuthentication::ScramSha1;
        assert!(!same_credential_target(&a, &b));
        b = a.clone();
        b.mongo.read_preference = MongoReadPreference::SecondaryPreferred;
        b.mongo.direct_connection = false;
        assert!(same_credential_target(&a, &b));
    }
}
